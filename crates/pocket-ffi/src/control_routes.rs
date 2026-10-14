//! NET-12 Control API (`/pocket/v1/catalog`, `/pocket/v1/status`, model
//! download/delete). Lives in `pocket-ffi` since it calls straight back into
//! `PocketCore`'s own methods via a `Weak<PocketCore>` (D-era self-reference,
//! see `core.rs`'s `self_weak`) - no logic is duplicated here, these are
//! thin HTTP wrappers.
use std::sync::{Arc, Weak};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::core::PocketCore;
use crate::error::PocketError;
use crate::types::CatalogFilter;

fn core(state: &Weak<PocketCore>) -> Result<Arc<PocketCore>, StatusCode> {
    state.upgrade().ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

fn error_response(e: PocketError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, code) = match &e {
        PocketError::NotFound { .. } => (StatusCode::NOT_FOUND, "not_found"),
        PocketError::WillNotFit { .. } => (StatusCode::CONFLICT, "will_not_fit"),
        PocketError::NotEntitled { .. } => (StatusCode::FORBIDDEN, "not_entitled"),
        PocketError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    (status, Json(serde_json::json!({ "error": { "message": e.to_string(), "code": code } })))
}

async fn get_catalog(State(state): State<Weak<PocketCore>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let core = core(&state)?;
    let entries = core.catalog(CatalogFilter {
        modality: None,
        curated_only: false,
        installed_only: false,
    });
    Ok(Json(serde_json::json!({ "entries": entries.len() })))
}

#[derive(Serialize)]
struct StatusResponse {
    loaded_models: usize,
    server: ServerStatusJson,
}

#[derive(Serialize)]
struct ServerStatusJson {
    running: bool,
    lan_enabled: bool,
    port: u16,
}

async fn get_status(State(state): State<Weak<PocketCore>>) -> Result<Json<StatusResponse>, StatusCode> {
    let core = core(&state)?;
    let server = core.server_status();
    Ok(Json(StatusResponse {
        loaded_models: core.loaded_models().len(),
        server: ServerStatusJson {
            running: server.running,
            lan_enabled: server.lan_enabled,
            port: server.port,
        },
    }))
}

async fn start_download(
    State(state): State<Weak<PocketCore>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let core = core(&state).map_err(|s| (s, Json(serde_json::json!({ "error": { "message": "core not available", "code": "unavailable" } }))))?;
    let plan = core.plan_download(id).map_err(error_response)?;
    core.start_download_via_coordinator(plan).map_err(error_response)?;
    Ok(StatusCode::ACCEPTED)
}

async fn delete_model(
    State(state): State<Weak<PocketCore>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let core = core(&state).map_err(|s| (s, Json(serde_json::json!({ "error": { "message": "core not available", "code": "unavailable" } }))))?;
    core.delete_model(id).map_err(error_response)?;
    Ok(StatusCode::OK)
}

pub fn router(core_weak: Weak<PocketCore>) -> Router {
    Router::new()
        .route("/pocket/v1/catalog", get(get_catalog))
        .route("/pocket/v1/status", get(get_status))
        .route("/pocket/v1/models/{id}/download", post(start_download))
        .route("/pocket/v1/models/{id}", delete(delete_model))
        .with_state(core_weak)
}
