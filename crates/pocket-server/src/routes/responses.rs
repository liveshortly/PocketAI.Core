//! NET-06: `POST /v1/responses` — OpenAI Responses API as a *stateless
//! translation layer* over the same engine chat path `/v1/chat/completions`
//! uses (see `bin/pocket-serve.rs`): `input`/`instructions` are translated to
//! chat messages, the engine streams tokens through a `TokenSink`, and the
//! output is re-shaped into Responses objects / SSE events. No response
//! storage: `previous_response_id` and `store: true` are 400 "stateless
//! server".
//!
//! Like `routes::embeddings`/`routes::models`, this router carries no auth;
//! the composer layers it (pocket-serve adds its bearer-key middleware,
//! the LAN surface uses `require_auth`).
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures_util::{Stream, StreamExt};
use pocket_engine::{Engine, TokenSink};
use pocket_types::{
    ChatMessage, ContentPart, GenerateDelta, GenerateRequest, GenerateStats, LoadOptions,
    PocketError, ReasoningMode, SamplingParams,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::openai_error_json;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Items(Vec<InputItem>),
}

/// A message input item (`{type: "message", role, content}`; `type` may be
/// omitted, as the Responses API allows).
#[derive(Debug, Deserialize)]
pub struct InputItem {
    #[serde(default, rename = "type")]
    pub item_type: Option<String>,
    pub role: String,
    pub content: ItemContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ItemContent {
    Text(String),
    Parts(Vec<ItemContentPart>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemContentPart {
    InputText { text: String },
    OutputText { text: String },
}

#[derive(Debug, Deserialize)]
pub struct ResponsesRequest {
    pub model: String,
    pub input: ResponsesInput,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub store: Option<bool>,
}

/// Translate Responses-API input to the internal chat-messages form the
/// existing chat path generates from. `instructions` becomes the leading
/// system message.
pub fn to_chat_messages(instructions: Option<String>, input: ResponsesInput) -> Vec<ChatMessage> {
    let text_message = |role: &str, text: String| ChatMessage {
        role: role.to_string(),
        parts: vec![ContentPart::Text { text }],
    };
    let mut messages = Vec::new();
    if let Some(sys) = instructions {
        messages.push(text_message("system", sys));
    }
    match input {
        ResponsesInput::Text(text) => messages.push(text_message("user", text)),
        ResponsesInput::Items(items) => {
            for item in items {
                let text = match item.content {
                    ItemContent::Text(t) => t,
                    ItemContent::Parts(parts) => parts
                        .into_iter()
                        .map(|p| match p {
                            ItemContentPart::InputText { text }
                            | ItemContentPart::OutputText { text } => text,
                        })
                        .collect::<Vec<_>>()
                        .join(""),
                };
                messages.push(text_message(&item.role, text));
            }
        }
    }
    messages
}

/// The Responses object (`object: "response"`); `usage` only once stats exist.
fn response_object(
    resp_id: &str,
    item_id: &str,
    created_at: i64,
    model: &str,
    status: &str,
    text: &str,
    stats: Option<&GenerateStats>,
) -> Value {
    let output = if status == "in_progress" {
        json!([])
    } else {
        json!([{
            "type": "message",
            "id": item_id,
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": text, "annotations": [] }],
        }])
    };
    let mut obj = json!({
        "id": resp_id,
        "object": "response",
        "created_at": created_at,
        "model": model,
        "status": status,
        "output": output,
    });
    if let Some(s) = stats {
        obj["usage"] = json!({
            "input_tokens": s.prompt_tokens,
            "output_tokens": s.completion_tokens,
            "total_tokens": s.prompt_tokens + s.completion_tokens,
        });
    }
    obj
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn error_response(status: StatusCode, message: &str, code: &str) -> Response {
    (
        status,
        Json(openai_error_json(message, "invalid_request_error", code)),
    )
        .into_response()
}

// Same stub the pocket-serve chat path feeds `Engine::load` (that one is
// private to the bin target); real servers resolve manifests from the store.
fn stub_manifest(model_id: &str) -> pocket_types::ModelManifest {
    pocket_types::ModelManifest {
        id: pocket_types::ModelId {
            value: model_id.to_string(),
        },
        display_name: model_id.to_string(),
        family: "mock".into(),
        params_b: 0.0,
        quant: "none".into(),
        format: pocket_types::ModelFormat::Gguf,
        modalities: vec![pocket_types::Modality::Text],
        context_length: 4096,
        files: vec![],
        license: pocket_types::LicenseInfo {
            spdx: "N/A".into(),
            commercial_ok: true,
            url: String::new(),
        },
        hf_repo: String::new(),
        hf_revision: String::new(),
        min_ram_bytes: 0,
        est_runtime_bytes: 0,
        tags: vec![],
        curated: false,
    }
}

struct AccumulatingSink {
    text: Mutex<String>,
}
impl TokenSink for AccumulatingSink {
    fn on_delta(&self, delta: GenerateDelta) {
        self.text.lock().unwrap().push_str(&delta.text);
    }
    fn on_done(&self, _stats: GenerateStats) {}
    fn on_error(&self, _err: PocketError) {}
}

/// Streams Responses SSE events (`response.output_text.delta`,
/// `response.completed`) as the engine produces tokens.
// TODO(NET-06): function-call output items when the chat path emits tool
// calls (GenerateDelta.tool_calls); text-only until AGT-09 forces it.
struct SseSink {
    tx: tokio::sync::mpsc::UnboundedSender<(&'static str, Value)>,
    resp_id: String,
    item_id: String,
    model: String,
    created_at: i64,
    text: Mutex<String>,
}
impl TokenSink for SseSink {
    fn on_delta(&self, delta: GenerateDelta) {
        if delta.text.is_empty() {
            return;
        }
        self.text.lock().unwrap().push_str(&delta.text);
        let _ = self.tx.send((
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "item_id": self.item_id,
                "output_index": 0,
                "content_index": 0,
                "delta": delta.text,
            }),
        ));
    }
    fn on_done(&self, stats: GenerateStats) {
        let text = self.text.lock().unwrap().clone();
        let _ = self.tx.send((
            "response.completed",
            json!({
                "type": "response.completed",
                "response": response_object(
                    &self.resp_id, &self.item_id, self.created_at, &self.model,
                    "completed", &text, Some(&stats),
                ),
            }),
        ));
    }
    fn on_error(&self, err: PocketError) {
        let _ = self.tx.send((
            "error",
            json!({ "type": "error", "message": err.to_string() }),
        ));
    }
}

async fn create_response(
    State(engine): State<Arc<dyn Engine>>,
    Json(req): Json<ResponsesRequest>,
) -> Response {
    if req.previous_response_id.is_some() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "stateless server",
            "previous_response_id_unsupported",
        );
    }
    if req.store == Some(true) {
        return error_response(StatusCode::BAD_REQUEST, "stateless server", "store_unsupported");
    }

    let loaded = match engine
        .load(stub_manifest(&req.model), LoadOptions::default())
        .await
    {
        Ok(l) => l,
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "engine_failed")
        }
    };

    let gen_req = GenerateRequest {
        request_id: ulid::Ulid::new().to_string(),
        messages: to_chat_messages(req.instructions, req.input),
        tools: vec![],
        params: SamplingParams {
            temperature: req.temperature.unwrap_or(0.7),
            top_p: req.top_p.unwrap_or(0.95),
            top_k: 40,
            min_p: 0.0,
            max_tokens: req.max_output_tokens.unwrap_or(1024),
            stop: vec![],
            seed: None,
            repetition_penalty: 1.1,
        },
        reasoning: ReasoningMode::Off,
    };
    let resp_id = format!("resp_{}", ulid::Ulid::new());
    let item_id = format!("msg_{}", ulid::Ulid::new());
    let created_at = now_secs();

    if !req.stream {
        let sink = Arc::new(AccumulatingSink {
            text: Mutex::new(String::new()),
        });
        let stats = match engine.generate(loaded.handle, gen_req, sink.clone()).await {
            Ok(s) => s,
            Err(e) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &e.to_string(),
                    "engine_failed",
                )
            }
        };
        let text = sink.text.lock().unwrap().clone();
        return Json(response_object(
            &resp_id,
            &item_id,
            created_at,
            &req.model,
            "completed",
            &text,
            Some(&stats),
        ))
        .into_response();
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<(&'static str, Value)>();
    let _ = tx.send((
        "response.created",
        json!({
            "type": "response.created",
            "response": response_object(
                &resp_id, &item_id, created_at, &req.model, "in_progress", "", None,
            ),
        }),
    ));
    let sink = Arc::new(SseSink {
        tx,
        resp_id,
        item_id,
        model: req.model,
        created_at,
        text: Mutex::new(String::new()),
    });
    tokio::spawn(async move {
        let _ = engine.generate(loaded.handle, gen_req, sink).await;
    });
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = Box::pin(
        tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
            .map(|(name, v)| Ok(Event::default().event(name).data(v.to_string()))),
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response()
}

pub fn router(engine: Arc<dyn Engine>) -> Router {
    Router::new()
        .route("/v1/responses", post(create_response))
        .with_state(engine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use pocket_engine::MockEngine;
    use tower::ServiceExt;

    fn app() -> Router {
        router(Arc::new(MockEngine::new(2000.0)))
    }

    async fn post_json(app: Router, body: &str) -> (StatusCode, Vec<u8>) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, bytes.to_vec())
    }

    fn one_text(m: &ChatMessage) -> &str {
        match &m.parts[..] {
            [ContentPart::Text { text }] => text,
            other => panic!("expected one text part, got {other:?}"),
        }
    }

    #[test]
    fn instructions_become_system_message_and_string_input_a_user_message() {
        let messages = to_chat_messages(
            Some("be brief".into()),
            ResponsesInput::Text("hello".into()),
        );
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(one_text(&messages[0]), "be brief");
        assert_eq!(messages[1].role, "user");
        assert_eq!(one_text(&messages[1]), "hello");
    }

    #[test]
    fn items_input_translates_roles_and_joins_content_parts() {
        let input: ResponsesInput = serde_json::from_str(
            r#"[
                {"type": "message", "role": "user", "content": "first"},
                {"role": "assistant", "content": [{"type": "output_text", "text": "re"}, {"type": "output_text", "text": "ply"}]},
                {"role": "user", "content": [{"type": "input_text", "text": "second"}]}
            ]"#,
        )
        .unwrap();
        let messages = to_chat_messages(None, input);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(one_text(&messages[0]), "first");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(one_text(&messages[1]), "reply");
        assert_eq!(messages[2].role, "user");
        assert_eq!(one_text(&messages[2]), "second");
    }

    #[tokio::test]
    async fn previous_response_id_and_store_true_are_rejected_as_stateless() {
        let (status, body) = post_json(
            app(),
            r#"{"model":"m","input":"hi","previous_response_id":"resp_123"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["message"], "stateless server");

        let (status, body) = post_json(app(), r#"{"model":"m","input":"hi","store":true}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["message"], "stateless server");

        // store:false is what stateless clients send; it must pass.
        let (status, _) = post_json(app(), r#"{"model":"m","input":"hi","store":false}"#).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn non_stream_response_matches_openai_responses_shape() {
        let (status, body) = post_json(
            app(),
            r#"{"model":"m","input":"hi","instructions":"be brief","stream":false}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let actual: Value = serde_json::from_slice(&body).unwrap();
        assert!(actual["id"].as_str().unwrap().starts_with("resp_"));
        assert_eq!(actual["object"], "response");
        assert!(actual["created_at"].is_number());
        assert_eq!(actual["model"], "m");
        assert_eq!(actual["status"], "completed");
        let item = &actual["output"][0];
        assert_eq!(item["type"], "message");
        assert!(item["id"].as_str().unwrap().starts_with("msg_"));
        assert_eq!(item["role"], "assistant");
        assert_eq!(item["status"], "completed");
        let content = &item["content"][0];
        assert_eq!(content["type"], "output_text");
        assert!(!content["text"].as_str().unwrap().is_empty());
        assert_eq!(content["annotations"], json!([]));
        assert!(actual["usage"]["input_tokens"].is_number());
        assert!(actual["usage"]["output_tokens"].is_number());
        assert_eq!(
            actual["usage"]["total_tokens"].as_u64().unwrap(),
            actual["usage"]["input_tokens"].as_u64().unwrap()
                + actual["usage"]["output_tokens"].as_u64().unwrap()
        );
    }

    #[tokio::test]
    async fn stream_emits_created_then_text_deltas_then_completed() {
        let (status, body) =
            post_json(app(), r#"{"model":"m","input":"hi","stream":true}"#).await;
        assert_eq!(status, StatusCode::OK);
        let text = String::from_utf8(body).unwrap();

        // Parse SSE blocks into (event name, data json) pairs.
        let events: Vec<(String, Value)> = text
            .split("\n\n")
            .filter(|b| b.contains("event:"))
            .map(|block| {
                let mut name = String::new();
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(v) = line.strip_prefix("event: ") {
                        name = v.to_string();
                    } else if let Some(v) = line.strip_prefix("data: ") {
                        data.push_str(v);
                    }
                }
                (name, serde_json::from_str(&data).unwrap())
            })
            .collect();

        assert_eq!(events.first().unwrap().0, "response.created");
        let created = &events[0].1;
        assert_eq!(created["type"], "response.created");
        assert_eq!(created["response"]["status"], "in_progress");
        assert_eq!(created["response"]["object"], "response");

        let deltas: Vec<&Value> = events
            .iter()
            .filter(|(n, _)| n == "response.output_text.delta")
            .map(|(_, v)| v)
            .collect();
        assert!(!deltas.is_empty());
        for d in &deltas {
            assert!(d["item_id"].as_str().unwrap().starts_with("msg_"));
            assert_eq!(d["output_index"], 0);
            assert_eq!(d["content_index"], 0);
            assert!(d["delta"].is_string());
        }
        let joined: String = deltas
            .iter()
            .map(|d| d["delta"].as_str().unwrap())
            .collect();

        let (last_name, last) = events.last().unwrap();
        assert_eq!(last_name, "response.completed");
        assert_eq!(last["response"]["status"], "completed");
        assert_eq!(
            last["response"]["output"][0]["content"][0]["text"], joined,
            "final text must equal the concatenated deltas"
        );
        assert!(last["response"]["usage"]["output_tokens"].as_u64().unwrap() > 0);
    }
}
