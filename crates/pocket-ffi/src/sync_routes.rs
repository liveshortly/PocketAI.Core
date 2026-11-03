//! NET-22: chat sync over the paired, pinned-TLS channel.
//!
//! Mac side: `/pocket/v1/sync/*` routes (behind the device bearer token like
//! every other device route, and Pro-gated in [`PocketCore::sync_store`]).
//! iPhone side: [`run`], driven by `PocketCore::sync_now` through a Swift
//! [`SyncTransport`] that owns TLS pinning + the token. One sync is
//! hello -> pull pages -> push pages -> blobs; the op log is the outbox, and
//! every page advances the version vectors, so a dropped sync resumes.
use std::sync::{Arc, Weak};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
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
    match ident {
        Some(Extension(Identity::Device(id))) => id.clone(),
        _ => fallback.to_string(),
    }
}

async fn hello(
    State(state): State<Weak<PocketCore>>,
    ident: Option<Extension<Identity>>,
    Json(req): Json<Hello>,
) -> RouteResult<Json<Hello>> {
    let core = store_of(&state)?;
    let store = core.sync_store().map_err(err)?;
    sync::seed(store).map_err(|e| err(storage(e)))?;
    sync::gc(store, core.now_ms()).map_err(|e| err(storage(e)))?;
    note_peer(
        store,
        &caller(&ident, &req.device_id),
        &req.vv,
        Some(core.now_ms()),
    )
    .map_err(err)?;
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
    let store = core.sync_store().map_err(err)?;
    if let Some(Extension(Identity::Device(id))) = &ident {
        note_peer(store, id, &req.vv, None).map_err(err)?;
    }
    let (ops, more) = sync::ops_since(store, &req.vv, req.limit.clamp(1, PAGE * 2))
        .map_err(|e| err(storage(e)))?;
    Ok(Json(PullResponse { ops, more }))
}

async fn push(
    State(state): State<Weak<PocketCore>>,
    Json(req): Json<PushRequest>,
) -> RouteResult<Json<VvResponse>> {
    let core = store_of(&state)?;
    let store = core.sync_store().map_err(err)?;
    sync::apply_remote(store, &req.ops).map_err(|e| err(storage(e)))?;
    Ok(Json(VvResponse {
        vv: sync::version_vector(store).map_err(|e| err(storage(e)))?,
    }))
}

async fn missing(State(state): State<Weak<PocketCore>>) -> RouteResult<Json<Missing>> {
    let core = store_of(&state)?;
    let store = core.sync_store().map_err(err)?;
    Ok(Json(Missing {
        hashes: sync::missing_blobs(store).map_err(|e| err(storage(e)))?,
    }))
}

async fn get_blob(
    State(state): State<Weak<PocketCore>>,
    Path(hash): Path<String>,
) -> RouteResult<Vec<u8>> {
    let core = store_of(&state)?;
    let store = core.sync_store().map_err(err)?;
    sync::blob_path(store, &hash)
        .and_then(|p| std::fs::read(p).ok())
        .ok_or_else(|| StatusCode::NOT_FOUND.into_response())
}

async fn put_blob(
    State(state): State<Weak<PocketCore>>,
    Path(hash): Path<String>,
    body: Bytes,
) -> RouteResult<StatusCode> {
    let core = store_of(&state)?;
    let store = core.sync_store().map_err(err)?;
    match sync::put_blob(store, &hash, &body).map_err(|e| err(storage(e)))? {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::BAD_REQUEST.into_response()),
    }
}

pub fn router(core_weak: Weak<PocketCore>) -> Router {
    Router::new()
        .route("/pocket/v1/sync/hello", post(hello))
        .route("/pocket/v1/sync/pull", post(pull))
        .route("/pocket/v1/sync/push", post(push))
        .route("/pocket/v1/sync/blobs/missing", get(missing))
        .route("/pocket/v1/sync/blobs/{hash}", get(get_blob).put(put_blob))
        // Op pages and photos are bigger than axum's 2 MB default.
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(core_weak)
}
