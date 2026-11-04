//! AGT-13 stage 2: `POST /pocket/v1/run` - the paired iPhone asks this Mac to
//! run approved code in the `mac_exec` sandbox.
//!
//! Refusals use the usual `{ "error": { "message", "code" } }` body:
//! 401 `unauthorized`, 403 `not_entitled` | `run_disabled` |
//! `disconnected_by_host`, 400 `bad_request`, 413 `code_too_large`,
//! 429 `rate_limited`, 409 `busy`, 501 `interpreter_missing`.
//! A run that executed answers 200 `{ok, output, error, duration_ms,
//! truncated, exit_code}` even if the program failed (`ok` false).
//!
//! Auth is done here from the bearer token (see `PocketCore::run_gate`), not
//! taken from `require_auth`, because that middleware lets loopback callers
//! through unauthenticated; a local process or web page must not reach this.
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

use crate::core::PocketCore;
use crate::mac_exec::{self, Language};

const RUNS_PER_MINUTE: usize = 10;

#[derive(Clone)]
struct RunState {
    core: Weak<PocketCore>,
    busy: Arc<AtomicBool>,
    recent: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

#[derive(Deserialize)]
struct RunBody {
    language: String,
    code: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

fn refuse(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": { "message": message, "code": code } }))).into_response()
}

/// Clears `busy`. Moved into the blocking task, so the slot frees only once
/// the run is really over (killed and scratch deleted), not when the client
/// leaves.
struct BusyGuard(Arc<AtomicBool>);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Sets the cancel flag when the handler future is dropped, which hyper does
/// when the client disconnects (Stop on the phone). Harmless after a normal
/// finish: the run is already over.
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Takes the one run slot (`Err` = busy) and runs `code` on a blocking
/// thread. Dropping the returned future kills the run and frees the slot
/// once the kill is done.
async fn exec(busy: &Arc<AtomicBool>, lang: Language, code: String, timeout: u64) -> Option<Result<mac_exec::Outcome, String>> {
    if busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return None;
    }
    let guard = BusyGuard(Arc::clone(busy));
    let cancel = Arc::new(AtomicBool::new(false));
    let _cancel_on_drop = CancelOnDrop(Arc::clone(&cancel));
    Some(
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            mac_exec::run(lang, &code, timeout, &cancel)
        })
        .await
        .unwrap_or_else(|_| Err("The run crashed.".into())),
    )
}

async fn run(State(st): State<RunState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(core) = st.core.upgrade() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let (device_id, device) = match core.run_gate(bearer) {
        Ok(d) => d,
        Err("unauthorized") => return refuse(StatusCode::UNAUTHORIZED, "unauthorized", "missing or invalid bearer token"),
        Err(code) => {
            let msg = match code {
                "run_disabled" => "Running code on this Mac is turned off.",
                "not_entitled" => "Not available on this Mac.",
                _ => "This device was disconnected by the Mac.",
            };
            return refuse(StatusCode::FORBIDDEN, code, msg);
        }
    };

    let req: RunBody = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return refuse(StatusCode::BAD_REQUEST, "bad_request", "expected {language, code, timeout_ms}"),
    };
    let log = |status: u16, ms: u64, note: &str| core.record_run(&device, &req.language, &req.code, status, ms, note);

    {
        let mut recent = st.recent.lock().unwrap();
        let q = recent.entry(device_id).or_default();
        let now = Instant::now();
        while q.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60)) {
            q.pop_front();
        }
        if q.len() >= RUNS_PER_MINUTE {
            log(429, 0, "rate_limited");
            return refuse(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "Too many runs; try again in a minute.");
        }
        q.push_back(now);
    }
    if req.code.len() > mac_exec::MAX_CODE_BYTES {
        log(413, 0, "code_too_large");
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "code_too_large", "Code is larger than 64 KB.");
    }
    let Some(lang) = Language::parse(&req.language) else {
        log(400, 0, "bad_language");
        return refuse(StatusCode::BAD_REQUEST, "bad_request", "language must be python, javascript or shell");
    };
    let (code, timeout) = (req.code.clone(), mac_exec::clamp_timeout(req.timeout_ms));
    let started = Instant::now();
    let Some(result) = exec(&st.busy, lang, code, timeout).await else {
        log(409, 0, "busy");
        return refuse(StatusCode::CONFLICT, "busy", "The Mac is already running something.");
    };
    let ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(o) => {
            log(200, o.duration_ms, o.error.as_deref().unwrap_or("ok"));
            Json(serde_json::json!({
                "ok": o.ok, "output": o.output, "error": o.error,
                "duration_ms": o.duration_ms, "truncated": o.truncated, "exit_code": o.exit_code,
            }))
            .into_response()
        }
        Err(msg) => {
            log(501, ms, "interpreter_missing");
            refuse(StatusCode::NOT_IMPLEMENTED, "interpreter_missing", &msg)
        }
    }
}

pub fn router(core: Weak<PocketCore>) -> Router {
    mac_exec::sweep_scratch();
    let st = RunState {
        core,
        busy: Arc::new(AtomicBool::new(false)),
        recent: Arc::default(),
    };
    Router::new()
        .route("/pocket/v1/run", post(run))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(st)
}

#[cfg(test)]
mod tests {
    //! Real runs under sandbox-exec, through a real axum server and raw TCP
    //! (the auth gate needs a paired device; `exec` is everything after it).
    use super::*;
    use std::process::Command;
    use tokio::io::AsyncWriteExt;

    fn alive(marker: &str) -> bool {
        let o = Command::new("/bin/ps").args(["-axo", "command"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).contains(marker)
    }

    async fn until(mut f: impl FnMut() -> bool, ms: u64) -> bool {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            if f() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        f()
    }

    #[tokio::test]
    async fn client_disconnect_kills_the_run_and_frees_the_slot() {
        let busy = Arc::new(AtomicBool::new(false));
        let b = Arc::clone(&busy);
        let app = Router::new().route(
            "/run",
            post(move |body: String| {
                let b = Arc::clone(&b);
                async move {
                    match exec(&b, Language::Shell, body, 30_000).await {
                        Some(Ok(o)) => o.output,
                        _ => "busy".into(),
                    }
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

        let code = "exec /bin/sleep 303";
        let mut c = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!("POST /run HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{code}", code.len());
        c.write_all(req.as_bytes()).await.unwrap();
        assert!(until(|| alive("sleep 303"), 5_000).await, "run never started");
        assert!(busy.load(Ordering::SeqCst));
        drop(c); // Stop on the phone
        assert!(until(|| !alive("sleep 303"), 3_000).await, "run survived the disconnect");
        assert!(until(|| !busy.load(Ordering::SeqCst), 3_000).await, "slot not freed");
        let left = std::fs::read_dir(std::env::temp_dir().canonicalize().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("pocketai-run-") && e.path().join("main.py").exists())
            .count();
        assert_eq!(left, 0);
    }
}
