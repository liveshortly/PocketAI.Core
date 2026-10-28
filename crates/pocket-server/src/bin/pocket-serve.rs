//! NET-16: headless server binary for L8's SDK tests to point an
//! OpenAI-compatible client at. `pocket-server` doesn't depend on
//! `pocket-ffi` (that'd be a cycle - `pocket-ffi` depends on
//! `pocket-server`), so this binary builds its own minimal
//! `/v1/chat/completions` directly against `pocket_engine::Engine`,
//! independent of (and smaller than) `pocket-ffi::chat_routes`.
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures_util::{Stream, StreamExt};
use pocket_engine::{Engine, MockEngine, TokenSink};
use pocket_types::{
    ChatMessage, ContentPart, GenerateDelta, GenerateRequest, GenerateStats, LoadOptions,
    ModelManifest, PocketError, ReasoningMode, SamplingParams,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<MessageIn>,
    #[serde(default)]
    stream: bool,
}

#[derive(Deserialize)]
struct MessageIn {
    role: String,
    content: String,
}

fn dummy_manifest(model_id: &str) -> ModelManifest {
    ModelManifest {
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

fn random_key() -> String {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).expect("OS CSPRNG");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

struct AppState {
    engine: Arc<dyn Engine>,
    key: String,
}

fn has_valid_key(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {expected}"))
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
    chat_id: String,
    created: i64,
    model: String,
}
impl TokenSink for ChannelSink {
    fn on_delta(&self, delta: GenerateDelta) {
        let _ = self.tx.send(json!({
            "id": self.chat_id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
            "choices": [{ "index": 0, "delta": { "content": delta.text }, "finish_reason": Value::Null }],
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
            .send(json!({ "error": { "message": err.to_string() } }));
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChatCompletionsRequest>,
) -> Response {
    if !has_valid_key(&headers, &state.key) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": { "message": "unauthorized" } })),
        )
            .into_response();
    }

    let loaded = match state
        .engine
        .load(dummy_manifest(&req.model), LoadOptions::default())
        .await
    {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": e.to_string() } })),
            )
                .into_response()
        }
    };
    let messages: Vec<ChatMessage> = req
        .messages
        .iter()
        .map(|m| ChatMessage {
            role: m.role.clone(),
            parts: vec![ContentPart::Text {
                text: m.content.clone(),
            }],
        })
        .collect();
    let gen_req = GenerateRequest {
        request_id: ulid::Ulid::new().to_string(),
        messages,
        tools: vec![],
        params: SamplingParams {
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.0,
            max_tokens: 1024,
            stop: vec![],
            seed: None,
            repetition_penalty: 1.1,
        },
        reasoning: ReasoningMode::Off,
    };
    let chat_id = format!("chatcmpl-{}", ulid::Ulid::new());
    let created = now_secs();

    if !req.stream {
        let sink = Arc::new(AccumulatingSink {
            text: std::sync::Mutex::new(String::new()),
        });
        let stats = match state
            .engine
            .generate(loaded.handle, gen_req, sink.clone())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": { "message": e.to_string() } })),
                )
                    .into_response()
            }
        };
        let text = sink.text.lock().unwrap().clone();
        return Json(json!({
            "id": chat_id,
            "object": "chat.completion",
            "created": created,
            "model": req.model,
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": stats.prompt_tokens,
                "completion_tokens": stats.completion_tokens,
                "total_tokens": stats.prompt_tokens + stats.completion_tokens,
            },
        }))
        .into_response();
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let _ = tx.send(json!({
        "id": chat_id, "object": "chat.completion.chunk", "created": created, "model": req.model,
        "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "" }, "finish_reason": Value::Null }],
    }));
    let sink = Arc::new(ChannelSink {
        tx,
        chat_id,
        created,
        model: req.model.clone(),
    });
    let engine = Arc::clone(&state.engine);
    tokio::spawn(async move {
        let _ = engine.generate(loaded.handle, gen_req, sink).await;
    });
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = Box::pin(
        tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
            .map(|chunk| Ok(Event::default().data(chunk.to_string())))
            .chain(tokio_stream::once(Ok(Event::default().data("[DONE]")))),
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--gguf") {
        eprintln!("--gguf is not wired into pocket-serve yet: the llama.cpp engine exists (pocket_engine::llama::LlamaCppEngine) but this binary only runs --mock-engine.");
        std::process::exit(1);
    }

    let key = random_key();
    let router = build_router(Arc::new(MockEngine::new(2000.0)), key.clone());

    let (cert_der, key_der) = {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params =
            rcgen::CertificateParams::new(vec!["localhost".into(), "pocketai.local".into()])
                .expect("params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        let cert = params.self_signed(&key_pair).expect("self sign");
        (cert.der().to_vec(), key_pair.serialize_der())
    };

    let handle = pocket_server::start_with_routes(
        pocket_server::ServerConfig::default(),
        "pocket-serve".to_string(),
        router,
        cert_der,
        key_der,
    )
    .await
    .expect("start server");
    println!("base_url=https://127.0.0.1:{}", handle.port);
    println!("api_key={key}");

    tokio::signal::ctrl_c().await.ok();
    handle.stop().await;
}

fn build_router(engine: Arc<dyn Engine>, key: String) -> Router {
    let state = Arc::new(AppState { engine, key });
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn rejects_missing_or_wrong_bearer_key() {
        let app = build_router(Arc::new(MockEngine::new(2000.0)), "secret".into());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer wrong")
                    .body(Body::from(
                        r#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn non_stream_response_matches_openai_shape() {
        let app = build_router(Arc::new(MockEngine::new(2000.0)), "secret".into());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer secret")
                    .body(Body::from(r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"stream":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let actual: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(actual["object"], "chat.completion");
        assert_eq!(actual["choices"][0]["message"]["role"], "assistant");
        assert!(!actual["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(actual["choices"][0]["finish_reason"], "stop");
    }
}
