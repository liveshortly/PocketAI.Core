//! NET-17 / NET-18: a paired device's live session on the Mac.
//!
//! The Mac can't dial the phone, so liveness is a client lease: every
//! authenticated request (and the phone's 15 s `POST /pocket/v1/heartbeat`)
//! bumps `devices.last_seen_at` in `require_auth`; `link_state` turns that
//! timestamp into Connected / Idle / Disconnected. `POST /pocket/v1/session/close`
//! is the phone's goodbye (background → Idle now, user Disconnect →
//! Disconnected now); `POST /pocket/v1/session/open` is its Connect, which
//! also lifts a Mac-side Disconnect. The Mac's own per-device Disconnect is
//! persisted in settings so it survives relaunch, and `reject_host_disconnected`
//! answers that device with a typed `disconnected_by_host` until it reconnects.
use std::collections::HashMap;
use std::sync::{Mutex, Weak};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use pocket_server::Identity;
use serde::Deserialize;
use tokio::sync::oneshot;

use crate::core::PocketCore;
use crate::types::DeviceLinkState;

/// Three missed 15 s heartbeats.
pub const LEASE_MS: i64 = 45_000;
pub const DISCONNECT_AFTER_MS: i64 = 5 * 60_000;
/// Error code (body `error.code` and the `x-pocket-error` header) a phone
/// gets while this Mac has it disconnected.
pub const DISCONNECTED_BY_HOST: &str = "disconnected_by_host";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Goodbye {
    pub at_ms: i64,
    /// The user tapped Disconnect (vs. the app going to the background).
    pub by_user: bool,
}

/// Pure lease rule, clock injected (NET-18 done-when).
pub fn link_state(
    last_seen_at: Option<i64>,
    goodbye: Option<Goodbye>,
    host_disconnected: bool,
    now_ms: i64,
) -> DeviceLinkState {
    if host_disconnected {
        return DeviceLinkState::DisconnectedByHost;
    }
    let Some(seen) = last_seen_at else {
        return DeviceLinkState::Disconnected;
    };
    let mut age = now_ms - seen;
    // A goodbye only counts if nothing was heard from the phone after it.
    if let Some(bye) = goodbye.filter(|b| b.at_ms >= seen) {
        if bye.by_user {
            return DeviceLinkState::Disconnected;
        }
        age = age.max(LEASE_MS);
    }
    if age < LEASE_MS {
        DeviceLinkState::Connected
    } else if age < DISCONNECT_AFTER_MS {
        DeviceLinkState::Idle
    } else {
        DeviceLinkState::Disconnected
    }
}

/// In-memory per-device session state (goodbyes and open streams). A
/// relaunch forgets goodbyes, which is fine: `last_seen_at` alone then
/// decays to Idle/Disconnected on its own.
#[derive(Default)]
pub struct Sessions {
    goodbyes: Mutex<HashMap<String, Goodbye>>,
    streams: Mutex<HashMap<String, Vec<oneshot::Sender<()>>>>,
}

impl Sessions {
    pub fn goodbye(&self, device_id: &str) -> Option<Goodbye> {
        self.goodbyes.lock().unwrap().get(device_id).copied()
    }

    pub fn record_goodbye(&self, device_id: &str, bye: Goodbye) {
        self.goodbyes
            .lock()
            .unwrap()
            .insert(device_id.to_string(), bye);
    }

    pub fn clear_goodbye(&self, device_id: &str) {
        self.goodbyes.lock().unwrap().remove(device_id);
    }

    /// An open SSE stream for `device_id`; it ends when the receiver fires.
    pub fn register_stream(&self, device_id: &str) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        let mut streams = self.streams.lock().unwrap();
        let list = streams.entry(device_id.to_string()).or_default();
        list.retain(|s| !s.is_closed());
        list.push(tx);
        rx
    }

    /// Ends every open stream of `device_id` (Disconnect, lease expiry).
    pub fn drop_streams(&self, device_id: &str) {
        if let Some(list) = self.streams.lock().unwrap().remove(device_id) {
            for tx in list {
                let _ = tx.send(());
            }
        }
    }

    pub fn devices_with_streams(&self) -> Vec<String> {
        let mut streams = self.streams.lock().unwrap();
        streams.retain(|_, list| {
            list.retain(|s| !s.is_closed());
            !list.is_empty()
        });
        streams.keys().cloned().collect()
    }
}

fn settings_key(device_id: &str) -> String {
    format!("device.disconnected_by_host.{device_id}")
}

pub fn is_host_disconnected(store: &pocket_store::Store, device_id: &str) -> bool {
    pocket_store::SettingsRepo::new(store)
        .get(&settings_key(device_id))
        .ok()
        .flatten()
        .is_some()
}

pub fn set_host_disconnected(
    store: &pocket_store::Store,
    device_id: &str,
    on: bool,
) -> Result<(), crate::error::PocketError> {
    let repo = pocket_store::SettingsRepo::new(store);
    let key = settings_key(device_id);
    if on { repo.set(&key, "true") } else { repo.delete(&key) }.map_err(|e| {
        crate::error::PocketError::Storage {
            message: e.to_string(),
        }
    })
}

fn device_id(ident: &Option<Extension<Identity>>) -> Option<&str> {
    match ident {
        Some(Extension(Identity::Device(id))) => Some(id),
        _ => None,
    }
}

/// Wraps the device API: a phone this Mac disconnected gets a typed 403
/// (not a generic auth failure) so it can say "Your Mac disconnected this iPhone".
pub async fn reject_host_disconnected(
    State(core): State<Weak<PocketCore>>,
    request: Request,
    next: Next,
) -> Response {
    let blocked = match (request.extensions().get::<Identity>(), core.upgrade()) {
        (Some(Identity::Device(id)), Some(core)) => core.is_host_disconnected(id),
        _ => false,
    };
    if !blocked {
        return next.run(request).await;
    }
    let message = "This Mac disconnected this device. Connect again from the device.";
    let body = if request.uri().path().starts_with("/v1/") {
        pocket_server::openai_error_json(message, "invalid_request_error", DISCONNECTED_BY_HOST)
    } else {
        pocket_server::pocket_error_json(message, DISCONNECTED_BY_HOST)
    };
    (
        StatusCode::FORBIDDEN,
        [("x-pocket-error", DISCONNECTED_BY_HOST)],
        Json(body),
    )
        .into_response()
}

/// `require_auth` already bumped `last_seen_at`; nothing else to do.
async fn heartbeat() -> StatusCode {
    StatusCode::NO_CONTENT
}

#[derive(Deserialize, Default)]
struct CloseBody {
    /// "user" (tapped Disconnect) or "background" (app left the foreground).
    #[serde(default)]
    reason: String,
}

async fn close_session(
    State(core): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
    body: Option<Json<CloseBody>>,
) -> StatusCode {
    if let (Some(id), Some(core)) = (device_id(&ident), core.upgrade()) {
        let by_user = body.is_some_and(|Json(b)| b.reason == "user");
        core.session_goodbye(id, by_user);
    }
    StatusCode::NO_CONTENT
}

async fn open_session(
    State(core): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
) -> StatusCode {
    if let (Some(id), Some(core)) = (device_id(&ident), core.upgrade()) {
        core.session_open(id);
    }
    StatusCode::NO_CONTENT
}

/// Behind `reject_host_disconnected`: a disconnected phone's heartbeat is
/// how it learns the Mac let it go.
pub fn guarded_router() -> Router {
    Router::new().route("/pocket/v1/heartbeat", post(heartbeat))
}

/// Outside the guard: goodbye and Connect must always reach the Mac.
pub fn router(core: Weak<PocketCore>) -> Router {
    Router::new()
        .route("/pocket/v1/session/close", post(close_session))
        .route("/pocket/v1/session/open", post(open_session))
        .with_state(core)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::PlatformHooks;
    use crate::types::{CoreConfig, Platform};
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;
    use tower::ServiceExt;

    const T0: i64 = 1_000_000;

    #[test]
    fn lease_connected_then_idle_then_disconnected() {
        let s = |now| link_state(Some(T0), None, false, now);
        assert_eq!(s(T0), DeviceLinkState::Connected);
        assert_eq!(s(T0 + LEASE_MS - 1), DeviceLinkState::Connected);
        assert_eq!(s(T0 + LEASE_MS), DeviceLinkState::Idle);
        assert_eq!(s(T0 + DISCONNECT_AFTER_MS - 1), DeviceLinkState::Idle);
        assert_eq!(s(T0 + DISCONNECT_AFTER_MS), DeviceLinkState::Disconnected);
        assert_eq!(link_state(None, None, false, T0), DeviceLinkState::Disconnected);
    }

    #[test]
    fn background_goodbye_is_idle_at_once_and_a_later_request_revives() {
        let bye = Some(Goodbye { at_ms: T0, by_user: false });
        assert_eq!(link_state(Some(T0), bye, false, T0 + 1), DeviceLinkState::Idle);
        assert_eq!(
            link_state(Some(T0), bye, false, T0 + DISCONNECT_AFTER_MS),
            DeviceLinkState::Disconnected
        );
        // Heard from again after the goodbye: the lease applies normally.
        assert_eq!(link_state(Some(T0 + 5), bye, false, T0 + 6), DeviceLinkState::Connected);
    }

    #[test]
    fn user_disconnect_and_host_disconnect_win() {
        let bye = Some(Goodbye { at_ms: T0, by_user: true });
        assert_eq!(link_state(Some(T0), bye, false, T0 + 1), DeviceLinkState::Disconnected);
        assert_eq!(
            link_state(Some(T0), None, true, T0),
            DeviceLinkState::DisconnectedByHost
        );
    }

    #[tokio::test]
    async fn drop_streams_ends_only_that_devices_streams() {
        let sessions = Sessions::default();
        let a = sessions.register_stream("a");
        let mut b = sessions.register_stream("b");
        assert_eq!(sessions.devices_with_streams().len(), 2);
        sessions.drop_streams("a");
        assert!(a.await.is_ok());
        assert!(b.try_recv().is_err());
        assert_eq!(sessions.devices_with_streams(), vec!["b".to_string()]);
    }

    struct Clock(AtomicI64, String);
    impl PlatformHooks for Clock {
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
            self.1.clone()
        }
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn core_with_device(clock: Arc<Clock>) -> Arc<PocketCore> {
        let core = PocketCore::open(
            CoreConfig {
                app_support_dir: clock.1.clone(),
                platform: Platform::MacOs,
            },
            clock,
        )
        .unwrap();
        pocket_store::DevicesRepo::new(core.store_for_test())
            .create(&pocket_store::DeviceRow {
                id: "dev1".into(),
                name: "iPhone".into(),
                platform: "ios".into(),
                public_key: vec![],
                token_hash: vec![1],
                role: "paired".into(),
                paired_at: 0,
                last_seen_at: None,
                last_addr: None,
                revoked_at: None,
            })
            .unwrap();
        core
    }

    /// The same layering `server_start` uses, with the device identity
    /// injected where `require_auth` would put it.
    fn app(core: &Arc<PocketCore>) -> Router {
        let weak = Arc::downgrade(core);
        guarded_router()
            .layer(axum::middleware::from_fn_with_state(
                weak.clone(),
                reject_host_disconnected,
            ))
            .merge(router(weak))
            .layer(Extension(Identity::Device("dev1".into())))
    }

    async fn post(app: &Router, path: &str, body: &str) -> Response {
        app.clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    fn state(core: &PocketCore) -> DeviceLinkState {
        core.devices()[0].link_state
    }

    #[tokio::test]
    async fn goodbye_and_host_disconnect_round_trip_through_the_routes() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(Clock(AtomicI64::new(T0), dir.path().display().to_string()));
        let core = core_with_device(clock.clone());
        let app = app(&core);
        let seen = |core: &PocketCore| {
            pocket_store::DevicesRepo::new(core.store_for_test())
                .mark_seen("dev1", clock.now_ms(), "")
                .unwrap()
        };

        seen(&core);
        assert_eq!(state(&core), DeviceLinkState::Connected);
        assert_eq!(core.server_status().connected_devices, 0); // not running

        // Background goodbye -> Idle immediately.
        assert_eq!(
            post(&app, "/pocket/v1/session/close", r#"{"reason":"background"}"#).await.status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(state(&core), DeviceLinkState::Idle);

        // Mac disconnects the phone: heartbeat gets the typed error, its
        // open stream is ended, and it survives a "relaunch" (settings).
        let stream = core.register_device_stream("dev1");
        core.set_device_disconnected("dev1".into(), true).unwrap();
        assert!(stream.await.is_ok());
        assert_eq!(state(&core), DeviceLinkState::DisconnectedByHost);
        let resp = post(&app, "/pocket/v1/heartbeat", "{}").await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(resp.headers()["x-pocket-error"], DISCONNECTED_BY_HOST);
        assert!(is_host_disconnected(core.store_for_test(), "dev1"));

        // Phone taps Connect -> allowed again.
        clock.0.fetch_add(10, Ordering::SeqCst);
        post(&app, "/pocket/v1/session/open", "").await;
        seen(&core);
        assert_eq!(post(&app, "/pocket/v1/heartbeat", "{}").await.status(), StatusCode::NO_CONTENT);
        assert_eq!(state(&core), DeviceLinkState::Connected);

        // No heartbeat for 45 s -> Idle; 5 min -> Disconnected.
        clock.0.fetch_add(LEASE_MS, Ordering::SeqCst);
        assert_eq!(state(&core), DeviceLinkState::Idle);
        clock.0.fetch_add(DISCONNECT_AFTER_MS, Ordering::SeqCst);
        assert_eq!(state(&core), DeviceLinkState::Disconnected);
    }
}
