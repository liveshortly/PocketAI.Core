use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pocket_types::{EgressEntry, PocketError};

/// Implemented by whatever owns the Swift `DownloadCoordinator` registration
/// (kept decoupled from `pocket-ffi` to avoid a layering cycle — the FFI
/// crate wires its `DownloadCoordinator` into this hook).
pub trait OfflinePauseHook: Send + Sync {
    fn pause_all_internet_transfers(&self);
}

/// The only allowed internet HTTP client (NET-15): refuses requests while
/// `offline_mode` is on, records every request as an `EgressEntry`. Paired-Mac
/// LAN traffic goes through a separate `PeerClient`/pinned-TLS path and never
/// touches this type, so it is unaffected by offline mode.
pub struct PocketHttp {
    client: reqwest::Client,
    offline: AtomicBool,
    egress: Mutex<Vec<EgressEntry>>,
    pause_hook: Mutex<Option<Arc<dyn OfflinePauseHook>>>,
}

impl Default for PocketHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl PocketHttp {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            offline: AtomicBool::new(false),
            egress: Mutex::new(Vec::new()),
            pause_hook: Mutex::new(None),
        }
    }

    pub fn register_pause_hook(&self, hook: Arc<dyn OfflinePauseHook>) {
        *self.pause_hook.lock().unwrap() = Some(hook);
    }

    pub fn is_offline(&self) -> bool {
        self.offline.load(Ordering::SeqCst)
    }

    /// Turning offline mode on pauses active/queued internet transfers via
    /// the registered hook. Turning it off does **not** auto-resume anything.
    pub fn set_offline_mode(&self, on: bool) {
        let was_off = !self.offline.swap(on, Ordering::SeqCst);
        if on && was_off {
            if let Some(hook) = self.pause_hook.lock().unwrap().as_ref() {
                hook.pause_all_internet_transfers();
            }
        }
    }

    pub async fn get(&self, url: &str, purpose: &str, initiated_by: &str) -> Result<Vec<u8>, PocketError> {
        if self.is_offline() {
            return Err(PocketError::Network {
                message: "Offline mode is on".into(),
            });
        }
        let resp = self.client.get(url).send().await.map_err(|e| PocketError::Network {
            message: e.to_string(),
        })?;
        let bytes = resp.bytes().await.map_err(|e| PocketError::Network {
            message: e.to_string(),
        })?;
        self.record(url, purpose, 0, bytes.len() as u64, initiated_by);
        Ok(bytes.to_vec())
    }

    fn record(&self, url: &str, purpose: &str, bytes_out: u64, bytes_in: u64, initiated_by: &str) {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| url.to_string());
        self.egress.lock().unwrap().push(EgressEntry {
            ts: now_ms(),
            host,
            purpose: purpose.into(),
            bytes_out,
            bytes_in,
            initiated_by: initiated_by.into(),
        });
    }

    pub fn egress_log(&self) -> Vec<EgressEntry> {
        self.egress.lock().unwrap().clone()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    struct RecordingHook {
        paused: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl OfflinePauseHook for RecordingHook {
        fn pause_all_internet_transfers(&self) {
            self.paused.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn spawn_local_http_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = b"hello";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn refuses_requests_while_offline() {
        let http = PocketHttp::new();
        http.set_offline_mode(true);
        let err = http.get("http://example.invalid/catalog", "catalog_refresh", "app").await.unwrap_err();
        match err {
            PocketError::Network { message } => assert_eq!(message, "Offline mode is on"),
            other => panic!("expected Network error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn records_egress_on_success() {
        let http = PocketHttp::new();
        let url = spawn_local_http_server();
        let body = http.get(&url, "catalog_refresh", "app").await.unwrap();
        assert_eq!(body, b"hello");

        let log = http.egress_log();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].purpose, "catalog_refresh");
        assert_eq!(log[0].bytes_in, 5);
    }

    #[tokio::test]
    async fn activating_offline_mode_calls_pause_hook_once() {
        let http = PocketHttp::new();
        let paused = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        http.register_pause_hook(Arc::new(RecordingHook { paused: paused.clone() }));

        http.set_offline_mode(true);
        assert_eq!(paused.load(Ordering::SeqCst), 1);

        // Re-activating (already on) must not call the hook again.
        http.set_offline_mode(true);
        assert_eq!(paused.load(Ordering::SeqCst), 1);

        // Turning off does not call the hook (no auto-resume semantics here).
        http.set_offline_mode(false);
        assert_eq!(paused.load(Ordering::SeqCst), 1);
    }
}
