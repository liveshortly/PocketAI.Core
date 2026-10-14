//! NET-08: `/v1/audio/transcriptions`, `/v1/audio/speech`,
//! `/v1/images/generations`. Thin HTTP wrappers over the registered
//! `SpeechToTextEngine`/`TextToSpeechEngine`/`ImageEngine` (same
//! `self_weak: Weak<PocketCore>` pattern as `chat_routes.rs`/`control_routes.rs`).
//! `ImageEngine::generate`'s `sink` mirrors `TokenSink` (A§4.5: "progress +
//! preview") - there's no separate final-bytes channel in the FFI contract,
//! so the final image is assembled by concatenating `on_delta`'s text
//! chunks as base64 and decoding once `on_done` fires (D39).
use std::sync::{Arc, Weak};

use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::core::PocketCore;
use crate::error::PocketError;
use crate::traits::TokenSink;
use crate::types::{GenerateDelta, GenerateStats};

fn error_response(status: StatusCode, e: PocketError) -> Response {
    (
        status,
        Json(json!({ "error": { "message": e.to_string() } })),
    )
        .into_response()
}

fn core(state: &Weak<PocketCore>) -> Result<Arc<PocketCore>, StatusCode> {
    state.upgrade().ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

fn unavailable_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": { "message": "core not available" } })),
    )
        .into_response()
}

struct AccumulatingSink {
    text: std::sync::Mutex<String>,
}
impl TokenSink for AccumulatingSink {
    fn on_delta(&self, delta: GenerateDelta) {
        self.text.lock().unwrap().push_str(&delta.text);
    }
    fn on_done(&self, _stats: GenerateStats) {}
    fn on_error(&self, _err: PocketError) {}
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pocket-{prefix}-{}", ulid::Ulid::new()))
}

async fn transcribe(
    State(core_weak): State<Weak<PocketCore>>,
    mut multipart: Multipart,
) -> Response {
    let core = match core(&core_weak) {
        Ok(c) => c,
        Err(_) => return unavailable_response(),
    };

    let mut file_bytes: Option<Vec<u8>> = None;
    let mut language: Option<String> = None;
    let mut response_format = "json".to_string();
    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("").to_string().as_str() {
            "file" => file_bytes = field.bytes().await.ok().map(|b| b.to_vec()),
            "language" => language = field.text().await.ok(),
            "response_format" => {
                response_format = field.text().await.unwrap_or_else(|_| "json".into())
            }
            _ => {}
        }
    }
    let Some(bytes) = file_bytes else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": { "message": "missing 'file' field" } })),
        )
            .into_response();
    };

    let path = temp_path("stt");
    if let Err(e) = tokio::fs::write(&path, &bytes).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": { "message": e.to_string() } })),
        )
            .into_response();
    }

    let engine = match core.stt_engine() {
        Ok(e) => e,
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, e);
        }
    };
    let sink = Arc::new(AccumulatingSink {
        text: std::sync::Mutex::new(String::new()),
    });
    let path_str = path.to_string_lossy().to_string();
    let sink_clone = Arc::clone(&sink);
    let result = tokio::task::spawn_blocking(move || {
        engine.transcribe(path_str, language, sink_clone as Arc<dyn TokenSink>)
    })
    .await;
    let _ = tokio::fs::remove_file(&path).await;

    match result {
        Ok(Ok(())) => {
            let text = sink.text.lock().unwrap().clone();
            if response_format == "text" {
                text.into_response()
            } else {
                Json(json!({ "text": text })).into_response()
            }
        }
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": { "message": e.to_string() } })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct SpeechRequest {
    input: String,
    #[serde(default = "default_voice")]
    voice: String,
}
fn default_voice() -> String {
    "default".into()
}

async fn speech(
    State(core_weak): State<Weak<PocketCore>>,
    Json(req): Json<SpeechRequest>,
) -> Response {
    let core = match core(&core_weak) {
        Ok(c) => c,
        Err(_) => return unavailable_response(),
    };
    let engine = match core.tts_engine() {
        Ok(e) => e,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let path = temp_path("tts");
    let path_str = path.to_string_lossy().to_string();
    let result =
        tokio::task::spawn_blocking(move || engine.synthesize(req.input, req.voice, path_str))
            .await;

    let synth_result = match result {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": e.to_string() } })),
            )
                .into_response()
        }
    };
    if let Err(e) = synth_result {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, e);
    }

    let wav = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": e.to_string() } })),
            )
                .into_response()
        }
    };
    let _ = tokio::fs::remove_file(&path).await;

    let mut headers = HeaderMap::new();
    headers.insert("content-type", "audio/wav".parse().unwrap());
    (headers, wav).into_response()
}

#[derive(Deserialize)]
struct ImagesRequest {
    prompt: String,
    #[serde(default)]
    negative_prompt: String,
    #[serde(default = "default_n")]
    n: u32,
    #[serde(default = "default_size")]
    size: String,
    #[serde(default = "default_steps")]
    steps: u32,
    #[serde(default)]
    seed: Option<i64>,
}
fn default_n() -> u32 {
    1
}
fn default_size() -> String {
    "512x512".into()
}
fn default_steps() -> u32 {
    20
}
const ALLOWED_SIZES: &[&str] = &["512x512", "768x768", "1024x1024"];

async fn images_generations(
    State(core_weak): State<Weak<PocketCore>>,
    Json(req): Json<ImagesRequest>,
) -> Response {
    let core = match core(&core_weak) {
        Ok(c) => c,
        Err(_) => return unavailable_response(),
    };
    if req.n == 0 || req.n > 4 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": { "message": "'n' must be between 1 and 4" } })),
        )
            .into_response();
    }
    if !ALLOWED_SIZES.contains(&req.size.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": { "message": format!("'size' must be one of {ALLOWED_SIZES:?}") } })),
        )
            .into_response();
    }
    let engine = match core.image_engine() {
        Ok(e) => e,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };

    let mut images = Vec::with_capacity(req.n as usize);
    for i in 0..req.n {
        let sink = Arc::new(AccumulatingSink {
            text: std::sync::Mutex::new(String::new()),
        });
        let engine = Arc::clone(&engine);
        let prompt = req.prompt.clone();
        let negative = req.negative_prompt.clone();
        let size = req.size.clone();
        let steps = req.steps;
        let seed = req.seed.unwrap_or(0) + i as i64;
        let sink_clone = Arc::clone(&sink);
        let result = tokio::task::spawn_blocking(move || {
            engine.generate(
                prompt,
                negative,
                size,
                steps,
                seed,
                sink_clone as Arc<dyn TokenSink>,
            )
        })
        .await;
        match result {
            Ok(Ok(())) => images.push(json!({ "b64_json": sink.text.lock().unwrap().clone() })),
            Ok(Err(e)) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": { "message": e.to_string() } })),
                )
                    .into_response()
            }
        }
    }

    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Json(json!({ "created": created, "data": images })).into_response()
}

pub fn router(core_weak: Weak<PocketCore>) -> Router {
    Router::new()
        .route("/v1/audio/transcriptions", post(transcribe))
        .route("/v1/audio/speech", post(speech))
        .route("/v1/images/generations", post(images_generations))
        .with_state(core_weak)
}
