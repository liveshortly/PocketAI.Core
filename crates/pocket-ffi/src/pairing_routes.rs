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
}

/// Shared pairing state, owned by `PocketCore` so `pairing_offer()` (called
/// before or after the server is running) and the routes (built fresh each
/// `server_start`) see the same single-use secret and pending exchanges.
pub struct PairingRoutesState {
    store: Arc<pocket_store::Store>,
    platform: Arc<dyn PlatformHooks>,
    current_secret: Mutex<Option<(String, i64)>>,
    pending: Mutex<HashMap<String, Arc<PendingPairing>>>,
}

impl PairingRoutesState {
    pub fn new(store: Arc<pocket_store::Store>, platform: Arc<dyn PlatformHooks>) -> Self {
        Self {
            store,
            platform,
            current_secret: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_current_secret(&self, secret: String, expires_at_ms: i64) {
        *self.current_secret.lock().unwrap() = Some((secret, expires_at_ms));
    }

    /// MAC-07: incoming requests still awaiting an Allow/Deny decision -
    /// `(id, device_name, platform)`.
    pub fn pending_requests(&self) -> Vec<(String, String, String)> {
        self.pending
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, p)| matches!(*p.status.lock().unwrap(), PendingStatus::Pending))
            .map(|(id, p)| (id.clone(), p.device_name.clone(), p.platform.clone()))
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
    secret: String,
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
}

async fn create_pairing(
    State(state): State<Arc<PairingRoutesState>>,
    Json(req): Json<PairRequest>,
) -> Result<Json<PairResponse>, (StatusCode, Json<ErrorBody>)> {
    let now = state.now_ms();
    let valid = {
        let current = state.current_secret.lock().unwrap();
        match &*current {
            Some((secret, expires_at_ms)) => {
                now < *expires_at_ms && constant_time_eq(secret, &req.secret)
            }
            None => false,
        }
    };
    if !valid {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: ErrorDetail {
                    message: "pairing secret is invalid, expired, or already used".into(),
                    code: "invalid_secret",
                },
            }),
        ));
    }
    // Single-use: clear it so a replay of the same request is rejected.
    *state.current_secret.lock().unwrap() = None;

    let pairing_id = ulid::Ulid::new().to_string();
    state.pending.lock().unwrap().insert(
        pairing_id.clone(),
        Arc::new(PendingPairing {
            status: Mutex::new(PendingStatus::Pending),
            notify: Notify::new(),
            device_name: req.device_name,
            device_pubkey: req.device_pubkey.into_bytes(),
            platform: req.platform,
        }),
    );

    Ok(Json(PairResponse { pairing_id }))
}

#[derive(Debug, Serialize)]
struct PairStatusResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
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
        },
        PendingStatus::Approved { token } => PairStatusResponse {
            status: "approved",
            token: Some(token),
        },
        PendingStatus::Denied => PairStatusResponse {
            status: "denied",
            token: None,
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
}
