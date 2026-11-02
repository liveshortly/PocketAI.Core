use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use governor::{Quota, RateLimiter};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    Device(String),
    LocalApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    Unauthorized,
    NotEntitled { feature: String },
    Locked,
}

pub fn sha256_hex(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whatever owns device/local-API-key storage and the Mac's current
/// entitlement state implements this; `pocket-trust`/`pocket-store` supply a
/// real one, tests use an in-memory fake.
pub trait AuthStore: Send + Sync {
    fn device_id_for_token_hash(&self, hash: &str) -> Option<String>;
    fn local_api_key_hash(&self) -> Option<String>;
    fn mac_is_pro(&self) -> bool;
    /// Called after a request authenticates as a device so the host can
    /// record `device.seen` (A§7 step 6: "Every request records device.seen").
    /// Default no-op for stores that don't track it (e.g. the loopback mock).
    fn record_seen(&self, _device_id: &str) {}
}

/// `Authorization: Bearer <token>` resolves to a device (paired, requires
/// current Pro entitlement on the Mac) or, loopback-only, a local API key
/// (free, for tools like Cursor/scripts).
pub fn authenticate(
    bearer_token: Option<&str>,
    is_loopback: bool,
    store: &dyn AuthStore,
) -> Result<Identity, AuthError> {
    let token = bearer_token.ok_or(AuthError::Unauthorized)?;
    let hash = sha256_hex(token);

    if let Some(device_id) = store.device_id_for_token_hash(&hash) {
        if !store.mac_is_pro() {
            return Err(AuthError::NotEntitled {
                feature: "lan_serving".into(),
            });
        }
        return Ok(Identity::Device(device_id));
    }

    if is_loopback {
        if let Some(expected) = store.local_api_key_hash() {
            if expected == hash {
                return Ok(Identity::LocalApiKey);
            }
        }
    }

    Err(AuthError::Unauthorized)
}

fn bearer_token(request: &Request) -> Option<String> {
    request
        .headers()
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|t| t.trim().to_string())
}

fn auth_error_response(path: &str, status: StatusCode, message: &str, code: &str) -> Response {
    // `/v1/*` is OpenAI-compatible (clients expect `error.type`); the
    // `/pocket/v1/*` device API uses the leaner shape.
    let body = if path.starts_with("/v1/") {
        openai_error_json(message, "invalid_request_error", code)
    } else {
        pocket_error_json(message, code)
    };
    (status, Json(body)).into_response()
}

/// NET-14: axum middleware guarding the device/API surface against the
/// threat A§9 names - "LAN attacker hits the API". A non-loopback (LAN)
/// request must carry `Authorization: Bearer <token>`; the token is hashed
/// with `sha256_hex` (the exact scheme pairing uses to store
/// `devices.token_hash`) and resolved via [`authenticate`]. Missing/unknown
/// token -> 401; a paired device on a non-Pro Mac -> 403. Wire it with
/// `from_fn_with_state` onto the protected routes only - the pairing
/// endpoints and `/pocket/v1/hello` are pre-token and must stay open.
///
/// Loopback (same-host) is trusted and passes through: A§9's threat model
/// is the LAN, and enforcing the loopback-only *local API key* for tools
/// like Cursor is MAC-06's separate, not-yet-wired scope. When the server
/// isn't served with `ConnectInfo` (e.g. unit tests), the peer is unknown
/// and treated as remote, so auth is enforced.
///
/// ponytail: `LockoutTracker`/`DeviceRateLimiter` (built & unit-tested in
/// this module, A§9's rate-limit/lockout controls) aren't driven in-band
/// yet - add them here keyed on the same `ConnectInfo` IP if the LAN
/// surface needs per-IP lockout / per-device throttling enforced live.
pub async fn require_auth(
    State(store): State<Arc<dyn AuthStore>>,
    mut request: Request,
    next: Next,
) -> Response {
    let is_loopback = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip().is_loopback())
        .unwrap_or(false);
    if is_loopback {
        return next.run(request).await;
    }

    let token = bearer_token(&request);
    let path = request.uri().path().to_string();
    match authenticate(token.as_deref(), false, store.as_ref()) {
        Ok(Identity::Device(device_id)) => {
            store.record_seen(&device_id);
            // NET-17/18: downstream routes (session close, per-device stream
            // teardown) need to know which device is calling.
            request
                .extensions_mut()
                .insert(Identity::Device(device_id));
            next.run(request).await
        }
        Ok(Identity::LocalApiKey) => next.run(request).await,
        Err(AuthError::NotEntitled { feature }) => auth_error_response(
            &path,
            StatusCode::FORBIDDEN,
            &format!("{feature} requires Pro"),
            "not_entitled",
        ),
        Err(_) => auth_error_response(
            &path,
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token",
            "unauthorized",
        ),
    }
}

const FAILED_AUTH_WINDOW: Duration = Duration::from_secs(60);
const FAILED_AUTH_THRESHOLD: u32 = 10;
const LOCKOUT_DURATION: Duration = Duration::from_secs(5 * 60);

struct IpState {
    failures_in_window: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
}

/// 10 failed auths/min/IP -> 5 min lockout.
pub struct LockoutTracker {
    state: Mutex<HashMap<IpAddr, IpState>>,
}

impl Default for LockoutTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl LockoutTracker {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_locked(&self, ip: IpAddr, now: Instant) -> bool {
        let state = self.state.lock().unwrap();
        matches!(state.get(&ip).and_then(|s| s.locked_until), Some(until) if now < until)
    }

    pub fn record_failure(&self, ip: IpAddr, now: Instant) {
        let mut state = self.state.lock().unwrap();
        let entry = state.entry(ip).or_insert(IpState {
            failures_in_window: 0,
            window_start: now,
            locked_until: None,
        });
        if now.duration_since(entry.window_start) > FAILED_AUTH_WINDOW {
            entry.window_start = now;
            entry.failures_in_window = 0;
        }
        entry.failures_in_window += 1;
        if entry.failures_in_window >= FAILED_AUTH_THRESHOLD {
            entry.locked_until = Some(now + LOCKOUT_DURATION);
        }
    }

    pub fn record_success(&self, ip: IpAddr) {
        self.state.lock().unwrap().remove(&ip);
    }
}

/// 20 requests/sec/device.
pub struct DeviceRateLimiter {
    limiters: Mutex<
        HashMap<
            String,
            RateLimiter<
                governor::state::NotKeyed,
                governor::state::InMemoryState,
                governor::clock::DefaultClock,
            >,
        >,
    >,
}

impl Default for DeviceRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceRateLimiter {
    pub fn new() -> Self {
        Self {
            limiters: Mutex::new(HashMap::new()),
        }
    }

    pub fn check(&self, device_id: &str) -> bool {
        let mut limiters = self.limiters.lock().unwrap();
        let limiter = limiters.entry(device_id.to_string()).or_insert_with(|| {
            RateLimiter::direct(Quota::per_second(NonZeroU32::new(20).unwrap()))
        });
        limiter.check().is_ok()
    }
}

pub fn openai_error_json(message: &str, error_type: &str, code: &str) -> serde_json::Value {
    serde_json::json!({ "error": { "message": message, "type": error_type, "code": code } })
}

pub fn pocket_error_json(message: &str, code: &str) -> serde_json::Value {
    serde_json::json!({ "error": { "message": message, "code": code } })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeStore {
        devices: HashMap<String, String>,
        local_key_hash: Option<String>,
        pro: bool,
    }

    impl AuthStore for FakeStore {
        fn device_id_for_token_hash(&self, hash: &str) -> Option<String> {
            self.devices.get(hash).cloned()
        }
        fn local_api_key_hash(&self) -> Option<String> {
            self.local_key_hash.clone()
        }
        fn mac_is_pro(&self) -> bool {
            self.pro
        }
    }

    fn store_with_device(token: &str, device_id: &str, pro: bool) -> FakeStore {
        let mut devices = HashMap::new();
        devices.insert(sha256_hex(token), device_id.to_string());
        FakeStore {
            devices,
            local_key_hash: None,
            pro,
        }
    }

    #[test]
    fn valid_device_token_with_pro_authenticates() {
        let store = store_with_device("devtoken", "dev1", true);
        assert_eq!(
            authenticate(Some("devtoken"), false, &store),
            Ok(Identity::Device("dev1".into()))
        );
    }

    #[test]
    fn device_token_without_pro_is_not_entitled() {
        let store = store_with_device("devtoken", "dev1", false);
        assert_eq!(
            authenticate(Some("devtoken"), false, &store),
            Err(AuthError::NotEntitled {
                feature: "lan_serving".into()
            })
        );
    }

    #[test]
    fn unknown_bearer_token_is_unauthorized() {
        let store = store_with_device("devtoken", "dev1", true);
        assert_eq!(
            authenticate(Some("wrong"), false, &store),
            Err(AuthError::Unauthorized)
        );
    }

    #[test]
    fn missing_bearer_token_is_unauthorized() {
        let store = store_with_device("devtoken", "dev1", true);
        assert_eq!(
            authenticate(None, false, &store),
            Err(AuthError::Unauthorized)
        );
    }

    #[test]
    fn local_api_key_accepted_on_loopback_only() {
        let store = FakeStore {
            devices: HashMap::new(),
            local_key_hash: Some(sha256_hex("localkey")),
            pro: false,
        };
        assert_eq!(
            authenticate(Some("localkey"), true, &store),
            Ok(Identity::LocalApiKey)
        );
        assert_eq!(
            authenticate(Some("localkey"), false, &store),
            Err(AuthError::Unauthorized)
        );
    }

    #[test]
    fn rate_limiter_allows_twenty_then_blocks() {
        let limiter = DeviceRateLimiter::new();
        for _ in 0..20 {
            assert!(limiter.check("dev1"));
        }
        assert!(!limiter.check("dev1"));
    }

    #[test]
    fn rate_limiter_is_per_device() {
        let limiter = DeviceRateLimiter::new();
        for _ in 0..20 {
            assert!(limiter.check("dev1"));
        }
        assert!(limiter.check("dev2"));
    }

    #[test]
    fn lockout_after_ten_failures_per_minute() {
        let tracker = LockoutTracker::new();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let t0 = Instant::now();
        assert!(!tracker.is_locked(ip, t0));
        for i in 0..10 {
            tracker.record_failure(ip, t0 + Duration::from_secs(i));
        }
        assert!(tracker.is_locked(ip, t0 + Duration::from_secs(10)));
        assert!(!tracker.is_locked(ip, t0 + LOCKOUT_DURATION + Duration::from_secs(11)));
    }

    #[test]
    fn success_clears_failure_count() {
        let tracker = LockoutTracker::new();
        let ip: IpAddr = "10.0.0.5".parse().unwrap();
        let t0 = Instant::now();
        for i in 0..5 {
            tracker.record_failure(ip, t0 + Duration::from_secs(i));
        }
        tracker.record_success(ip);
        for i in 0..5 {
            tracker.record_failure(ip, t0 + Duration::from_secs(20 + i));
        }
        assert!(!tracker.is_locked(ip, t0 + Duration::from_secs(25)));
    }

    #[tokio::test]
    async fn require_auth_passes_valid_token_and_401s_missing_or_wrong() {
        use axum::body::Body;
        use axum::http::Request as HttpRequest;
        use axum::{routing::get, Router};
        use tower::ServiceExt;

        let store: Arc<dyn AuthStore> = Arc::new(store_with_device("good-token", "dev1", true));
        let app = Router::new()
            .route("/v1/models", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(store, require_auth));

        let call = |auth: Option<&'static str>| {
            let app = app.clone();
            let mut builder = HttpRequest::builder().uri("/v1/models");
            if let Some(a) = auth {
                builder = builder.header("authorization", a);
            }
            let req = builder.body(Body::empty()).unwrap();
            async move { app.oneshot(req).await.unwrap().status() }
        };

        assert_eq!(call(Some("Bearer good-token")).await, StatusCode::OK);
        assert_eq!(call(None).await, StatusCode::UNAUTHORIZED);
        assert_eq!(call(Some("Bearer wrong")).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn require_auth_hands_the_device_identity_to_the_route() {
        use axum::body::Body;
        use axum::http::Request as HttpRequest;
        use axum::{routing::get, Extension, Router};
        use tower::ServiceExt;

        let store: Arc<dyn AuthStore> = Arc::new(store_with_device("good-token", "dev1", true));
        let app = Router::new()
            .route(
                "/pocket/v1/whoami",
                get(|Extension(id): Extension<Identity>| async move { format!("{id:?}") }),
            )
            .layer(axum::middleware::from_fn_with_state(store, require_auth));
        let req = HttpRequest::builder()
            .uri("/pocket/v1/whoami")
            .header("authorization", "Bearer good-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], br#"Device("dev1")"#);
    }

    #[test]
    fn error_json_shapes_differ_between_v1_and_pocket_v1() {
        let openai = openai_error_json("bad request", "invalid_request_error", "missing_field");
        assert!(openai["error"]["type"].is_string());

        let pocket = pocket_error_json("bad request", "missing_field");
        assert!(pocket["error"]["type"].is_null());
        assert_eq!(pocket["error"]["message"], "bad request");
    }
}
