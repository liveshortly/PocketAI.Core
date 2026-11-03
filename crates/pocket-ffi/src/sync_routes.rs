//! NET-22: chat sync over the paired, pinned-TLS channel.
//!
//! Mac side: `/pocket/v1/sync/*` routes (behind the device bearer token like
//! every other device route, and Pro-gated in [`PocketCore::sync_store`]).
//! iPhone side: [`run`], driven by `PocketCore::sync_now` through a Swift
//! [`SyncTransport`] that owns TLS pinning + the token. One sync is
//! hello -> pull pages -> push pages -> blobs; the op log is the outbox, and
//! every page advances the version vectors, so a dropped sync resumes.
//! `GET /pocket/v1/sync/wait?since=<cursor>` is the near-live Mac -> phone
//! signal (long-poll); a per-device toggle on the Mac answers 403
//! `sync_disabled`.
use std::sync::{Arc, Weak};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use pocket_server::Identity;
use pocket_store::sync::{self, Op, VersionVector};
use pocket_store::{SettingsRepo, Store};
use serde::{Deserialize, Serialize};

use crate::control_routes::error_response;
use crate::core::PocketCore;
use crate::error::PocketError;
use crate::traits::SyncTransport;
use crate::types::{SyncReport, SyncStatus};

const PAGE: usize = 500;

#[derive(Serialize, Deserialize)]
struct Hello {
    device_id: String,
    vv: VersionVector,
}

#[derive(Serialize, Deserialize)]
struct PullRequest {
    vv: VersionVector,
    limit: usize,
}

#[derive(Serialize, Deserialize)]
struct PullResponse {
    ops: Vec<Op>,
    more: bool,
}

#[derive(Serialize, Deserialize)]
struct PushRequest {
    ops: Vec<Op>,
}

#[derive(Serialize, Deserialize)]
struct VvResponse {
    vv: VersionVector,
}

#[derive(Serialize, Deserialize)]
struct Missing {
    hashes: Vec<String>,
}

fn storage(e: impl std::fmt::Display) -> PocketError {
    PocketError::Storage {
        message: e.to_string(),
    }
}

fn last_synced_key(peer: &str) -> String {
    format!("sync.last_synced.{peer}")
}

fn peer_has_key(peer: &str) -> String {
    format!("sync.peer_has.{peer}")
}

/// The peer's sync device id (op origin), learned at hello; drives
/// "Remove chats that came from <device>".
fn peer_device_key(peer: &str) -> String {
    format!("sync.peer_device.{peer}")
}

fn disabled_key(device: &str) -> String {
    format!("sync.disabled.{device}")
}

pub(crate) fn enabled_for(store: &Store, device: &str) -> bool {
    SettingsRepo::new(store)
        .get(&disabled_key(device))
        .ok()
        .flatten()
        .is_none()
}

pub(crate) fn set_enabled_for(store: &Store, device: &str, on: bool) -> Result<(), PocketError> {
    let repo = SettingsRepo::new(store);
    match on {
        true => repo.delete(&disabled_key(device)),
        false => repo.set(&disabled_key(device), "1"),
    }
    .map_err(storage)
}

fn note_peer_device(store: &Store, peer: &str, sync_device: &str) -> Result<(), PocketError> {
    SettingsRepo::new(store)
        .set(&peer_device_key(peer), sync_device)
        .map_err(storage)
}

pub(crate) fn remove_chats_from(store: &Store, peer: &str) -> Result<u32, PocketError> {
    let Some(origin) = SettingsRepo::new(store)
        .get(&peer_device_key(peer))
        .map_err(storage)?
    else {
        return Ok(0);
    };
    let n = sync::remove_chats_from(store, &origin).map_err(storage)?;
    Ok(n as u32)
}

/// Records what `peer` is known to hold of our ops (drives the outbox/status).
fn note_peer(
    store: &Store,
    peer: &str,
    peer_vv: &VersionVector,
    now: Option<i64>,
) -> Result<(), PocketError> {
    let me = sync::device_id(store).map_err(storage)?;
    let repo = SettingsRepo::new(store);
    repo.set(
        &peer_has_key(peer),
        &peer_vv.get(&me).copied().unwrap_or(0).to_string(),
    )
    .map_err(storage)?;
    if let Some(now) = now {
        repo.set(&last_synced_key(peer), &now.to_string())
            .map_err(storage)?;
    }
    Ok(())
}

pub(crate) fn status(store: &Store, peer: &str) -> Result<SyncStatus, PocketError> {
    let repo = SettingsRepo::new(store);
    let get = |k: String| {
        repo.get(&k)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
    };
    let (pending_ops, pending_conversation_ids) =
        sync::pending_conversations(store, get(peer_has_key(peer)).unwrap_or(0))
            .map_err(storage)?;
    Ok(SyncStatus {
        last_synced_at: get(last_synced_key(peer)),
        pending_ops,
        pending_conversation_ids,
    })
}

// ---- client driver (iPhone) ----

fn call<T: for<'de> Deserialize<'de>>(
    t: &dyn SyncTransport,
    method: &str,
    path: &str,
    body: &impl Serialize,
) -> Result<T, PocketError> {
    let body = match method {
        "GET" => Vec::new(),
        _ => serde_json::to_vec(body).map_err(|e| PocketError::Internal {
            message: e.to_string(),
        })?,
    };
    let resp = t.request(method.into(), path.into(), body)?;
    serde_json::from_slice(&resp).map_err(|e| PocketError::Network {
        message: format!("bad sync response from {path}: {e}"),
    })
}

pub(crate) fn run(
    store: &Store,
    peer: &str,
    t: &dyn SyncTransport,
    now: i64,
) -> Result<SyncReport, PocketError> {
    sync::seed(store).map_err(storage)?;
    sync::gc(store, now).map_err(storage)?;
    let me = sync::device_id(store).map_err(storage)?;
    let vv = sync::version_vector(store).map_err(storage)?;
    let hello: Hello = call(
        t,
        "POST",
        "/pocket/v1/sync/hello",
        &Hello { device_id: me, vv },
    )?;
    note_peer_device(store, peer, &hello.device_id)?;

    let mut pulled = 0;
    loop {
        let vv = sync::version_vector(store).map_err(storage)?;
        let page: PullResponse = call(
            t,
            "POST",
            "/pocket/v1/sync/pull",
            &PullRequest { vv, limit: PAGE },
        )?;
        pulled += sync::apply_remote(store, &page.ops).map_err(storage)?;
        if !page.more || page.ops.is_empty() {
            break;
        }
    }

    let mut server_vv = hello.vv;
    note_peer(store, peer, &server_vv, None)?;
    let mut pushed = 0;
    loop {
        let (ops, more) = sync::ops_since(store, &server_vv, PAGE).map_err(storage)?;
        if ops.is_empty() {
            break;
        }
        pushed += ops.len();
        let resp: VvResponse = call(t, "POST", "/pocket/v1/sync/push", &PushRequest { ops })?;
        let advanced = resp.vv != server_vv;
        server_vv = resp.vv;
        note_peer(store, peer, &server_vv, None)?;
        if !more || !advanced {
            break;
        }
    }

    // Blobs: a failed fetch stays missing and is retried next sync.
    let mut blobs_fetched = 0;
    for hash in sync::missing_blobs(store).map_err(storage)? {
        if let Ok(bytes) = t.request(
            "GET".into(),
            format!("/pocket/v1/sync/blobs/{hash}"),
            Vec::new(),
        ) {
            if sync::put_blob(store, &hash, &bytes).map_err(storage)? {
                blobs_fetched += 1;
            }
        }
    }
    let theirs: Missing = call(t, "GET", "/pocket/v1/sync/blobs/missing", &())?;
    let mut blobs_sent = 0;
    for hash in theirs.hashes {
        if let Some(bytes) = sync::blob_path(store, &hash).and_then(|p| std::fs::read(p).ok()) {
            t.request("PUT".into(), format!("/pocket/v1/sync/blobs/{hash}"), bytes)?;
            blobs_sent += 1;
        }
    }

    note_peer(store, peer, &server_vv, Some(now))?;
    Ok(SyncReport {
        pulled: pulled as u32,
        pushed: pushed as u32,
        blobs_fetched,
        blobs_sent,
        synced_at: now,
    })
}

// ---- Mac routes ----

type RouteResult<T> = Result<T, Response>;

fn store_of(state: &Weak<PocketCore>) -> RouteResult<Arc<PocketCore>> {
    state
        .upgrade()
        .ok_or_else(|| StatusCode::SERVICE_UNAVAILABLE.into_response())
}

fn err(e: PocketError) -> Response {
    error_response(e).into_response()
}

/// The calling device's pairing id when authenticated, else the sync id it sent.
fn caller(ident: &Option<Extension<Identity>>, fallback: &str) -> String {
    device(ident).unwrap_or(fallback).to_string()
}

fn device(ident: &Option<Extension<Identity>>) -> Option<&str> {
    match ident {
        Some(Extension(Identity::Device(id))) => Some(id),
        _ => None,
    }
}

/// Pro + this device's "Sync chats" toggle (403 `not_entitled` / `sync_disabled`).
fn gate<'a>(core: &'a PocketCore, ident: &Option<Extension<Identity>>) -> RouteResult<&'a Store> {
    core.sync_store(device(ident)).map_err(err)
}

async fn hello(
    State(state): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
    Json(req): Json<Hello>,
) -> RouteResult<Json<Hello>> {
    let core = store_of(&state)?;
    let store = gate(&core, &ident)?;
    sync::seed(store).map_err(|e| err(storage(e)))?;
    sync::gc(store, core.now_ms()).map_err(|e| err(storage(e)))?;
    let peer = caller(&ident, &req.device_id);
    note_peer(store, &peer, &req.vv, Some(core.now_ms())).map_err(err)?;
    note_peer_device(store, &peer, &req.device_id).map_err(err)?;
    core.emit_event("sync", "sync.applied", serde_json::json!({ "peer": peer, "applied": 0 }));
    Ok(Json(Hello {
        device_id: sync::device_id(store).map_err(|e| err(storage(e)))?,
        vv: sync::version_vector(store).map_err(|e| err(storage(e)))?,
    }))
}

async fn pull(
    State(state): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
    Json(req): Json<PullRequest>,
) -> RouteResult<Json<PullResponse>> {
    let core = store_of(&state)?;
    let store = gate(&core, &ident)?;
    if let Some(Extension(Identity::Device(id))) = &ident {
        note_peer(store, id, &req.vv, None).map_err(err)?;
    }
    let (ops, more) = sync::ops_since(store, &req.vv, req.limit.clamp(1, PAGE * 2))
        .map_err(|e| err(storage(e)))?;
    Ok(Json(PullResponse { ops, more }))
}

async fn push(
    State(state): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
    Json(req): Json<PushRequest>,
) -> RouteResult<Json<VvResponse>> {
    let core = store_of(&state)?;
    let store = gate(&core, &ident)?;
    let applied = sync::apply_remote(store, &req.ops).map_err(|e| err(storage(e)))?;
    let peer = device(&ident)
        .map(str::to_string)
        .or_else(|| req.ops.first().map(|o| o.device_id.clone()))
        .unwrap_or_default();
    core.emit_event("sync", "sync.applied", serde_json::json!({ "peer": peer, "applied": applied }));
    Ok(Json(VvResponse {
        vv: sync::version_vector(store).map_err(|e| err(storage(e)))?,
    }))
}

async fn missing(
    State(state): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
) -> RouteResult<Json<Missing>> {
    let core = store_of(&state)?;
    let store = gate(&core, &ident)?;
    Ok(Json(Missing {
        hashes: sync::missing_blobs(store).map_err(|e| err(storage(e)))?,
    }))
}

async fn get_blob(
    State(state): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
    Path(hash): Path<String>,
) -> RouteResult<Vec<u8>> {
    let core = store_of(&state)?;
    let store = gate(&core, &ident)?;
    sync::blob_path(store, &hash)
        .and_then(|p| std::fs::read(p).ok())
        .ok_or_else(|| StatusCode::NOT_FOUND.into_response())
}

async fn put_blob(
    State(state): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
    Path(hash): Path<String>,
    body: Bytes,
) -> RouteResult<StatusCode> {
    let core = store_of(&state)?;
    let store = gate(&core, &ident)?;
    match sync::put_blob(store, &hash, &body).map_err(|e| err(storage(e)))? {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::BAD_REQUEST.into_response()),
    }
}

/// How long `/sync/wait` holds a request, and how often it looks.
pub(crate) const WAIT_MS: u64 = 25_000;
const WAIT_POLL_MS: u64 = 150;

#[derive(Deserialize)]
struct WaitQuery {
    since: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct WaitResponse {
    pub(crate) cursor: i64,
}

/// Op count on this Mac (any origin) plus "Sync now" nudges. Grows on every
/// change; it can only drop after a Mac restart, which `!=` also treats as news.
fn cursor(core: &PocketCore, store: &Store) -> RouteResult<i64> {
    Ok(sync::change_cursor(store).map_err(|e| err(storage(e)))? + core.sync_nudges())
}

/// Near-live Mac -> phone: the phone holds this open while foreground and
/// connected. Returns `{cursor}` at once when `since` is absent or stale,
/// else as soon as the cursor moves, else after ~25 s. A changed cursor
/// means "run sync_now".
/// ponytail: watches the cursor every 150 ms (one tiny indexed SUM per
/// waiting phone) rather than wiring a notifier through every store write;
/// swap in a `tokio::sync::watch` bumped by `sync::emit`/`apply_remote` if
/// many phones ever wait at once.
async fn wait(
    State(state): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
    Query(q): Query<WaitQuery>,
) -> RouteResult<Json<WaitResponse>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(WAIT_MS);
    loop {
        // Re-checked every pass: Forget, Pro lapsing or the toggle going off
        // ends the wait with the matching error. No core held while asleep.
        let now = {
            let core = store_of(&state)?;
            let store = gate(&core, &ident)?;
            cursor(&core, store)?
        };
        if q.since != Some(now) || tokio::time::Instant::now() >= deadline {
            return Ok(Json(WaitResponse { cursor: now }));
        }
        tokio::time::sleep(std::time::Duration::from_millis(WAIT_POLL_MS)).await;
    }
}

pub fn router(core_weak: Weak<PocketCore>) -> Router {
    Router::new()
        .route("/pocket/v1/sync/hello", post(hello))
        .route("/pocket/v1/sync/wait", get(wait))
        .route("/pocket/v1/sync/pull", post(pull))
        .route("/pocket/v1/sync/push", post(push))
        .route("/pocket/v1/sync/blobs/missing", get(missing))
        .route("/pocket/v1/sync/blobs/{hash}", get(get_blob).put(put_blob))
        // Op pages and photos are bigger than axum's 2 MB default.
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(core_weak)
}
