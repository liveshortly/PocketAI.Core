//! NET-05: `POST /v1/chat/completions`, non-stream and stream. Thin HTTP
//! wrapper over `PocketCore::generate_chat` (D32/D34 `self_weak` pattern,
//! same shape as `control_routes.rs`/`pairing_routes.rs`).
use std::convert::Infallible;
use std::sync::{Arc, Weak};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::core::PocketCore;
use crate::error::PocketError;
use crate::traits::TokenSink;
use crate::types::{ChatMessage, ContentPart, GenerateDelta, GenerateStats, SamplingParams};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Parts(Vec<ContentPartIn>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentPartIn {
    Text { text: String },
    ImageUrl { image_url: ImageUrlIn },
}

#[derive(Debug, Deserialize)]
struct ImageUrlIn {
    url: String,
}

#[derive(Debug, Deserialize)]
struct MessageIn {
    role: String,
    content: MessageContent,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<MessageIn>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
}

fn openai_error(status: StatusCode, message: &str, code: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "message": message, "type": "invalid_request_error", "code": code } })),
    )
        .into_response()
}

/// Marker: the only failure mode is a remote (non-`data:`) image URL.
struct RemoteImageRejected;

fn to_ffi_messages(messages: Vec<MessageIn>) -> Result<Vec<ChatMessage>, RemoteImageRejected> {
    messages
        .into_iter()
        .map(|m| {
            let parts = match m.content {
                MessageContent::Text(text) => vec![ContentPart::Text { text }],
                MessageContent::Parts(parts) => parts
                    .into_iter()
                    .map(|p| match p {
                        ContentPartIn::Text { text } => Ok(ContentPart::Text { text }),
                        ContentPartIn::ImageUrl { image_url } => {
                            if image_url.url.starts_with("data:") {
                                Ok(ContentPart::ImageRef {
                                    path: image_url.url,
                                })
                            } else {
                                Err(RemoteImageRejected)
                            }
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            Ok(ChatMessage {
                role: m.role,
                parts,
            })
        })
        .collect()
}

fn params_from(req: &ChatCompletionsRequest) -> SamplingParams {
    SamplingParams {
        temperature: req.temperature.unwrap_or(0.7),
        top_p: 0.95,
        top_k: 40,
        min_p: 0.0,
        max_tokens: req.max_tokens.unwrap_or(1024),
        stop: vec![],
        seed: None,
        repetition_penalty: 1.1,
    }
}

fn engine_error_response(e: PocketError) -> Response {
    match e {
        PocketError::NotFound { what } => openai_error(
            StatusCode::NOT_FOUND,
            &format!("The model '{what}' does not exist or is not installed."),
            "model_not_found",
        ),
        PocketError::WillNotFit { needed_bytes, available_bytes, advice } => (
            StatusCode::CONFLICT,
            Json(json!({ "error": { "message": advice, "type": "invalid_request_error", "code": "will_not_fit", "needed_bytes": needed_bytes, "available_bytes": available_bytes } })),
        )
            .into_response(),
        other => openai_error(StatusCode::INTERNAL_SERVER_ERROR, &other.to_string(), "engine_failed"),
    }
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

struct ChannelSink {
    tx: tokio::sync::mpsc::UnboundedSender<Value>,
    /// Set for streams: cancels the request once nobody is reading (covers a
    /// client that left before the engine registered it).
    cancel: Option<(Weak<PocketCore>, String)>,
    chat_id: String,
    created: i64,
    model: String,
}

impl TokenSink for ChannelSink {
    fn on_delta(&self, delta: GenerateDelta) {
        if self.tx.is_closed() {
            if let Some((core, request_id)) = &self.cancel {
                if let Some(core) = core.upgrade() {
                    core.cancel_conversation(request_id.clone());
                }
            }
            return;
        }
        let mut delta_obj = json!({ "content": delta.text });
        if !delta.reasoning.is_empty() {
            delta_obj["reasoning_content"] = json!(delta.reasoning);
        }
        let _ = self.tx.send(json!({
            "id": self.chat_id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
            "choices": [{ "index": 0, "delta": delta_obj, "finish_reason": Value::Null }],
        }));
    }
    fn on_done(&self, stats: GenerateStats) {
        let _ = self.tx.send(json!({
            "id": self.chat_id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
            "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
        }));
        let _ = self.tx.send(json!({
            "id": self.chat_id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
            "choices": [],
            "usage": {
                "prompt_tokens": stats.prompt_tokens,
                "completion_tokens": stats.completion_tokens,
                "total_tokens": stats.prompt_tokens + stats.completion_tokens,
            },
        }));
    }
    fn on_error(&self, err: PocketError) {
        let _ = self
            .tx
            .send(json!({ "error": { "message": err.to_string(), "code": "engine_failed" } }));
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// NET-18: cancels the engine request when the SSE body is dropped - the
/// client went away (a failed write, at the latest on the next 10 s ping) or
/// the device was disconnected. After a normal finish it's a no-op.
struct CancelOnDrop {
    core: Weak<PocketCore>,
    request_id: String,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(core) = self.core.upgrade() {
            core.cancel_conversation(self.request_id.clone());
        }
    }
}

async fn chat_completions(
    State(core_weak): State<Weak<PocketCore>>,
    identity: Option<Extension<pocket_server::Identity>>,
    Json(req): Json<ChatCompletionsRequest>,
) -> Response {
    let Some(core) = core_weak.upgrade() else {
        return openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "core not available",
            "unavailable",
        );
    };
    let params = params_from(&req);
    let messages = match to_ffi_messages(req.messages) {
        Ok(m) => m,
        Err(RemoteImageRejected) => {
            return openai_error(
                StatusCode::BAD_REQUEST,
                "remote images are not fetched",
                "unsupported_image_url",
            )
        }
    };
    let model = req.model.clone();
    let chat_id = format!("chatcmpl-{}", ulid::Ulid::new());
    let created = now_secs();
    let request_id = ulid::Ulid::new().to_string();
    // NET-20: a device's request is dropped (if not started) or cancelled
    // when that device is released.
    let device = match identity {
        Some(Extension(pocket_server::Identity::Device(id))) => Some(id),
        _ => None,
    };
    let dropped = device
        .as_deref()
        .map(|id| core.register_device_request(id, &request_id));

    if !req.stream {
        let sink = Arc::new(AccumulatingSink {
            text: std::sync::Mutex::new(String::new()),
        });
        let sink_clone = Arc::clone(&sink);
        let stats = match tokio::task::spawn_blocking(move || {
            core.generate_chat_for(
                request_id,
                &model,
                messages,
                params,
                sink_clone as Arc<dyn TokenSink>,
                dropped.as_deref(),
            )
        })
        .await
        {
            Ok(Ok(stats)) => stats,
            Ok(Err(e)) => return engine_error_response(e),
            Err(e) => {
                return openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &e.to_string(),
                    "internal",
                )
            }
        };
        let text = sink.text.lock().unwrap().clone();
        return Json(json!({
            "id": chat_id,
            "object": "chat.completion",
            "created": created,
            "model": req.model,
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": text },
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": stats.prompt_tokens,
                "completion_tokens": stats.completion_tokens,
                "total_tokens": stats.prompt_tokens + stats.completion_tokens,
            },
        }))
        .into_response();
    }

    // Streaming: `generate_chat` runs synchronously on a blocking thread;
    // its TokenSink pushes chunks into an unbounded channel that the SSE
    // response streams out as they arrive.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let _ = tx.send(json!({
        "id": chat_id, "object": "chat.completion.chunk", "created": created, "model": model,
        "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "" }, "finish_reason": Value::Null }],
    }));
    let sink = Arc::new(ChannelSink {
        tx,
        cancel: Some((Arc::downgrade(&core), request_id.clone())),
        chat_id,
        created,
        model,
    });
    // NET-17: a device's stream ends when that device is disconnected.
    let device_stop = device.map(|id| core.register_device_stream(&id));
    let cancel = CancelOnDrop {
        core: Arc::downgrade(&core),
        request_id: request_id.clone(),
    };
    tokio::task::spawn_blocking(move || {
        let _ = core.generate_chat_for(
            request_id,
            &req.model,
            messages,
            params,
            sink as Arc<dyn TokenSink>,
            dropped.as_deref(),
        );
    });

    let events = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
        .map(move |chunk| {
            let _keep = &cancel;
            Ok(Event::default().data(chunk.to_string()))
        })
        .chain(tokio_stream::once(Ok(Event::default().data("[DONE]"))));
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
        match device_stop {
            Some(stop) => Box::pin(events.take_until(stop)),
            None => Box::pin(events),
        };
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response()
}

pub fn router(core_weak: Weak<PocketCore>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(core_weak)
}
