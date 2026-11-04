//! NET-10's route layer: `POST /pocket/v1/pair` + `GET /pocket/v1/pair/{id}`.
//! Lives in `pocket-ffi` (not `pocket-server`) since it needs `PocketCore`'s
//! device/peer storage and the secret `pairing_offer()` generated - routes
//! compose into `pocket_server::start_with_routes` by value (D31).
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use crate::traits::PlatformHooks;

const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
enum PendingStatus {
    Pending,
    Approved { token: String },
    Denied,
}

struct PendingPairing {
    status: Mutex<PendingStatus>,
    notify: Notify,
    device_name: String,
    device_pubkey: Vec<u8>,
    platform: String,
    /// NET-23: the Mac-side SAS for a manual-code request; `None` for QR.
    sas: Option<String>,
    /// NET-23: a manual-code phone never saw the QR's host name; it gets it
    /// with the approval instead.
    host_name: Option<String>,
}

/// Shared pairing state, owned by `PocketCore` so `pairing_offer()` (called
/// before or after the server is running) and the routes (built fresh each
/// `server_start`) see the same single-use secret and pending exchanges.
pub struct PairingRoutesState {
    store: Arc<pocket_store::Store>,
    platform: Arc<dyn PlatformHooks>,
    current_secret: Mutex<Option<(String, i64)>>,
    /// NET-23: the typed-code alternative to `current_secret`, plus this
    /// Mac's cert fingerprint for the SAS shown in the Allow sheet.
    manual_code: Mutex<Option<pocket_trust::ManualCode>>,
    /// (cert fingerprint, host display name) the manual code was issued for.
    manual_host: Mutex<(String, String)>,
    pending: Mutex<HashMap<String, Arc<PendingPairing>>>,
}

impl PairingRoutesState {
    pub fn new(store: Arc<pocket_store::Store>, platform: Arc<dyn PlatformHooks>) -> Self {
        Self {
            store,
            platform,
            current_secret: Mutex::new(None),
            manual_code: Mutex::new(None),
            manual_host: Mutex::new((String::new(), String::new())),
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_current_secret(&self, secret: String, expires_at_ms: i64) {
        *self.current_secret.lock().unwrap() = Some((secret, expires_at_ms));
    }

    /// NET-23: a fresh manual code replaces any earlier one (and its tries).
    pub fn set_manual_code(
        &self,
        code: String,
        expires_at_ms: i64,
        cert_fingerprint: String,
        host_name: String,
    ) {
        *self.manual_host.lock().unwrap() = (cert_fingerprint, host_name);
        *self.manual_code.lock().unwrap() =
            Some(pocket_trust::ManualCode::new(code, expires_at_ms));
    }

    /// MAC-07: incoming requests still awaiting an Allow/Deny decision -
    /// `(id, device_name, platform, sas)`; `sas` only for manual-code ones.
    pub fn pending_requests(&self) -> Vec<(String, String, String, Option<String>)> {
        self.pending
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, p)| matches!(*p.status.lock().unwrap(), PendingStatus::Pending))
            .map(|(id, p)| {
                (
                    id.clone(),
                    p.device_name.clone(),
                    p.platform.clone(),
                    p.sas.clone(),
                )
            })
            .collect()
    }

    fn now_ms(&self) -> i64 {
        self.platform.now_ms()
    }

    pub fn approve(&self, pairing_id: &str) -> Result<(), &'static str> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .get(pairing_id)
            .cloned()
            .ok_or("not found")?;
        {
            let mut status = pending.status.lock().unwrap();
            if *status != PendingStatus::Pending {
                return Err("already decided");
            }
            let token = generate_token();
            let token_hash = sha256_hex(&token);
            let now = self.now_ms();
            let _ = pocket_store::repos::devices::DevicesRepo::new(&self.store).create(
                &pocket_store::DeviceRow {
                    id: pairing_id.to_string(),
                    name: pending.device_name.clone(),
                    platform: pending.platform.clone(),
                    public_key: pending.device_pubkey.clone(),
                    token_hash: hex_decode(&token_hash),
                    role: "paired".into(),
                    paired_at: now,
                    last_seen_at: None,
                    last_addr: None,
                    revoked_at: None,
                },
            );
            *status = PendingStatus::Approved { token };
        }
        pending.notify.notify_waiters();
        Ok(())
    }

    pub fn deny(&self, pairing_id: &str) -> Result<(), &'static str> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .get(pairing_id)
            .cloned()
            .ok_or("not found")?;
        *pending.status.lock().unwrap() = PendingStatus::Denied;
        pending.notify.notify_waiters();
        Ok(())
    }
}

fn generate_token() -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0))
        .collect()
}

/// Constant-time comparison for the single-use secret (NET-10: "validates
/// the secret (constant-time)").
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[derive(Debug, Deserialize)]
struct PairRequest {
    /// QR flow: the offer's 128-bit secret.
    #[serde(default)]
    secret: String,
    /// NET-23 manual flow: the 8-character code typed on the phone. When
    /// present it is checked instead of `secret`.
    #[serde(default)]
    code: String,
    device_name: String,
    #[serde(default)]
    device_pubkey: String,
    platform: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PairResponse {
    pairing_id: String,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    message: String,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    attempts_left: Option<u32>,
}

fn unauthorized(
    code: &'static str,
    message: &str,
    attempts_left: Option<u32>,
) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody {
            error: ErrorDetail {
                message: message.into(),
                code,
                attempts_left,
            },
        }),
    )
}

/// NET-23: redeems the manual code (single use, 5 tries) and returns the
/// SAS this Mac will show, computed over its own cert fingerprint.
fn redeem_manual(
    state: &PairingRoutesState,
    typed: &str,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    use pocket_trust::ManualCodeError as E;
    let redeemed = pocket_trust::redeem_manual_code(
        &mut state.manual_code.lock().unwrap(),
        typed,
        state.now_ms(),
    )
    .map_err(|e| match e {
        E::NoCode => unauthorized("no_code", "no pairing code is active on this Mac", None),
        E::Expired => unauthorized("code_expired", "the pairing code expired", None),
        E::Wrong { attempts_left } => {
            unauthorized("wrong_code", "wrong pairing code", Some(attempts_left))
        }
        E::Exhausted => unauthorized(
            "code_locked",
            "too many wrong tries; the pairing code no longer works",
            Some(0),
        ),
    })?;
    pocket_trust::pairing_sas(&state.manual_host.lock().unwrap().0, &redeemed.code)
        .ok_or_else(|| unauthorized("no_code", "no pairing code is active on this Mac", None))
}

async fn create_pairing(
    State(state): State<Arc<PairingRoutesState>>,
    Json(req): Json<PairRequest>,
) -> Result<Json<PairResponse>, (StatusCode, Json<ErrorBody>)> {
    let sas = if req.code.is_empty() {
        None
    } else {
        Some(redeem_manual(&state, &req.code)?)
    };
    let now = state.now_ms();
    let valid = sas.is_some() || {
        let current = state.current_secret.lock().unwrap();
        match &*current {
            Some((secret, expires_at_ms)) => {
                now < *expires_at_ms && constant_time_eq(secret, &req.secret)
            }
            None => false,
        }
    };
    if !valid {
        return Err(unauthorized(
            "invalid_secret",
            "pairing secret is invalid, expired, or already used",
            None,
        ));
    }
    // Single-use: clear it so a replay of the same request is rejected.
    // (A manual code was already consumed by `redeem_manual`.)
    if sas.is_none() {
        *state.current_secret.lock().unwrap() = None;
    }

    let pairing_id = ulid::Ulid::new().to_string();
    state.pending.lock().unwrap().insert(
        pairing_id.clone(),
        Arc::new(PendingPairing {
            status: Mutex::new(PendingStatus::Pending),
            notify: Notify::new(),
            device_name: req.device_name,
            device_pubkey: req.device_pubkey.into_bytes(),
            platform: req.platform,
            host_name: sas
                .as_ref()
                .map(|_| state.manual_host.lock().unwrap().1.clone()),
            sas,
        }),
    );

    Ok(Json(PairResponse { pairing_id }))
}

#[derive(Debug, Serialize)]
struct PairStatusResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_name: Option<String>,
}

async fn poll_pairing(
    State(state): State<Arc<PairingRoutesState>>,
    Path(pairing_id): Path<String>,
) -> Result<Json<PairStatusResponse>, StatusCode> {
    let pending = state
        .pending
        .lock()
        .unwrap()
        .get(&pairing_id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;

    let already_decided = !matches!(*pending.status.lock().unwrap(), PendingStatus::Pending);
    if !already_decided {
        let _ = tokio::time::timeout(LONG_POLL_TIMEOUT, pending.notify.notified()).await;
    }

    let status = pending.status.lock().unwrap().clone();
    Ok(Json(match status {
        PendingStatus::Pending => PairStatusResponse {
            status: "pending",
            token: None,
            host_name: None,
        },
        PendingStatus::Approved { token } => PairStatusResponse {
            status: "approved",
            token: Some(token),
            host_name: pending.host_name.clone(),
        },
        PendingStatus::Denied => PairStatusResponse {
            status: "denied",
            token: None,
            host_name: None,
        },
    }))
}

pub fn router(state: Arc<PairingRoutesState>) -> Router {
    Router::new()
        .route("/pocket/v1/pair", post(create_pairing))
        .route("/pocket/v1/pair/{id}", get(poll_pairing))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    struct FixedClock;
    impl PlatformHooks for FixedClock {
        fn keychain_get(&self, _key: String) -> Option<Vec<u8>> {
            None
        }
        fn keychain_set(&self, _key: String, _value: Vec<u8>) -> bool {
            true
        }
        fn device_profile(&self) -> crate::types::DeviceProfile {
            unimplemented!()
        }
        fn app_support_dir(&self) -> String {
            String::new()
        }
        fn now_ms(&self) -> i64 {
            1_000_000
        }
    }

    fn test_state() -> Arc<PairingRoutesState> {
        Arc::new(PairingRoutesState::new(
            Arc::new(pocket_store::Store::open_in_memory().unwrap()),
            Arc::new(FixedClock),
        ))
    }

    #[tokio::test]
    async fn valid_secret_creates_a_pending_pairing() {
        let state = test_state();
        state.set_current_secret("s3cr3t".into(), 2_000_000);
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/pocket/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"secret":"s3cr3t","device_name":"iPhone","device_pubkey":"abc","platform":"ios"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn wrong_secret_is_rejected() {
        let state = test_state();
        state.set_current_secret("s3cr3t".into(), 2_000_000);
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/pocket/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"secret":"wrong","device_name":"iPhone","device_pubkey":"abc","platform":"ios"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn expired_secret_is_rejected() {
        let state = test_state();
        state.set_current_secret("s3cr3t".into(), 500_000); // before now_ms() == 1_000_000
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/pocket/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"secret":"s3cr3t","device_name":"iPhone","device_pubkey":"abc","platform":"ios"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn secret_is_single_use() {
        let state = test_state();
        state.set_current_secret("s3cr3t".into(), 2_000_000);

        let body = || {
            Body::from(
                r#"{"secret":"s3cr3t","device_name":"iPhone","device_pubkey":"abc","platform":"ios"}"#,
            )
        };
        let req = || {
            Request::builder()
                .method("POST")
                .uri("/pocket/v1/pair")
                .header("content-type", "application/json")
                .body(body())
                .unwrap()
        };

        let first = router(state.clone()).oneshot(req()).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);

        let second = router(state).oneshot(req()).await.unwrap();
        assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn approve_delivers_a_token_to_the_poller() {
        let state = test_state();
        state.set_current_secret("s3cr3t".into(), 2_000_000);
        let app = router(state.clone());

        let create = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/pocket/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"secret":"s3cr3t","device_name":"iPhone","device_pubkey":"abc","platform":"ios"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(create.into_body(), usize::MAX)
            .await
            .unwrap();
        let pairing_id = serde_json::from_slice::<PairResponse>(&body)
            .unwrap()
            .pairing_id;

        state.approve(&pairing_id).unwrap();

        let poll_app = router(state);
        let poll = poll_app
            .oneshot(
                Request::builder()
                    .uri(format!("/pocket/v1/pair/{pairing_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(poll.status(), StatusCode::OK);
        let body = axum::body::to_bytes(poll.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["status"], "approved");
        assert!(parsed["token"].is_string());
    }

    #[tokio::test]
    async fn deny_reports_denied() {
        let state = test_state();
        state.set_current_secret("s3cr3t".into(), 2_000_000);
        let app = router(state.clone());

        let create = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/pocket/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"secret":"s3cr3t","device_name":"iPhone","device_pubkey":"abc","platform":"ios"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(create.into_body(), usize::MAX)
            .await
            .unwrap();
        let pairing_id = serde_json::from_slice::<PairResponse>(&body)
            .unwrap()
            .pairing_id;

        state.deny(&pairing_id).unwrap();

        let poll_app = router(state);
        let poll = poll_app
            .oneshot(
                Request::builder()
                    .uri(format!("/pocket/v1/pair/{pairing_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(poll.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["status"], "denied");
    }
    // ---- NET-23 manual code ----

    const MAC_FP: &str = "q83vASNFZ4mrze8BI0VniavN7wEjRWeJq83vASNFZ4k";

    async fn post_code(
        state: &Arc<PairingRoutesState>,
        code: &str,
    ) -> (StatusCode, serde_json::Value) {
        let body = serde_json::json!({
            "code": code, "device_name": "iPhone", "device_pubkey": "", "platform": "ios",
        });
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/pocket/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn manual_code_creates_a_pending_pairing_carrying_the_mac_sas() {
        let state = test_state();
        state.set_manual_code(
            "ABCDEFGH".into(),
            2_000_000,
            MAC_FP.into(),
            "Rohit's Mac".into(),
        );
        let (status, body) = post_code(&state, "abcd-efgh").await;
        assert_eq!(status, StatusCode::OK);
        let pending = state.pending_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].3, pocket_trust::pairing_sas(MAC_FP, "ABCDEFGH"));

        // Approval hands the manual phone the host name it never saw.
        let id = body["pairing_id"].as_str().unwrap().to_string();
        state.approve(&id).unwrap();
        let poll = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/pocket/v1/pair/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(poll.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["status"], "approved");
        assert_eq!(parsed["host_name"], "Rohit's Mac");
    }

    #[tokio::test]
    async fn qr_pairing_has_no_sas() {
        let state = test_state();
        state.set_current_secret("s3cr3t".into(), 2_000_000);
        let body =
            r#"{"secret":"s3cr3t","device_name":"iPhone","device_pubkey":"","platform":"ios"}"#;
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/pocket/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.pending_requests()[0].3, None);
    }

    #[tokio::test]
    async fn manual_code_is_single_use_over_http() {
        let state = test_state();
        state.set_manual_code("ABCDEFGH".into(), 2_000_000, MAC_FP.into(), "Mac".into());
        assert_eq!(post_code(&state, "ABCDEFGH").await.0, StatusCode::OK);
        let (status, body) = post_code(&state, "ABCDEFGH").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "no_code");
    }

    #[tokio::test]
    async fn expired_manual_code_is_rejected() {
        let state = test_state();
        state.set_manual_code("ABCDEFGH".into(), 500_000, MAC_FP.into(), "Mac".into());
        let (status, body) = post_code(&state, "ABCDEFGH").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "code_expired");
        assert!(state.pending_requests().is_empty());
    }

    #[tokio::test]
    async fn five_wrong_codes_kill_the_code() {
        let state = test_state();
        state.set_manual_code("ABCDEFGH".into(), 2_000_000, MAC_FP.into(), "Mac".into());
        for left in [4, 3, 2, 1] {
            let (status, body) = post_code(&state, "ZZZZZZZZ").await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body["error"]["code"], "wrong_code");
            assert_eq!(body["error"]["attempts_left"], left);
        }
        let (_, body) = post_code(&state, "ZZZZZZZZ").await;
        assert_eq!(body["error"]["code"], "code_locked");
        // The right code no longer works either.
        let (status, body) = post_code(&state, "ABCDEFGH").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "no_code");
        assert!(state.pending_requests().is_empty());
    }

    #[tokio::test]
    async fn a_manual_code_does_not_open_the_qr_secret_path() {
        // A wrong code must not fall through to (or consume) the QR secret.
        let state = test_state();
        state.set_current_secret("s3cr3t".into(), 2_000_000);
        state.set_manual_code("ABCDEFGH".into(), 2_000_000, MAC_FP.into(), "Mac".into());
        let body =
            r#"{"secret":"s3cr3t","code":"ZZZZZZZZ","device_name":"iPhone","platform":"ios"}"#;
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/pocket/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(state.current_secret.lock().unwrap().is_some());
    }
}
