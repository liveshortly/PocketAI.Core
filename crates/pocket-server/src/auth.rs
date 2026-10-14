use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    limiters: Mutex<HashMap<String, RateLimiter<governor::state::NotKeyed, governor::state::InMemoryState, governor::clock::DefaultClock>>>,
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
        assert_eq!(authenticate(Some("wrong"), false, &store), Err(AuthError::Unauthorized));
    }

    #[test]
    fn missing_bearer_token_is_unauthorized() {
        let store = store_with_device("devtoken", "dev1", true);
        assert_eq!(authenticate(None, false, &store), Err(AuthError::Unauthorized));
    }

    #[test]
    fn local_api_key_accepted_on_loopback_only() {
        let store = FakeStore {
            devices: HashMap::new(),
            local_key_hash: Some(sha256_hex("localkey")),
            pro: false,
        };
        assert_eq!(authenticate(Some("localkey"), true, &store), Ok(Identity::LocalApiKey));
        assert_eq!(authenticate(Some("localkey"), false, &store), Err(AuthError::Unauthorized));
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

    #[test]
    fn error_json_shapes_differ_between_v1_and_pocket_v1() {
        let openai = openai_error_json("bad request", "invalid_request_error", "missing_field");
        assert!(openai["error"]["type"].is_string());

        let pocket = pocket_error_json("bad request", "missing_field");
        assert!(pocket["error"]["type"].is_null());
        assert_eq!(pocket["error"]["message"], "bad request");
    }
}
