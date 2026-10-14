use std::sync::Arc;

use pocket_engine::{Engine, MockEngine, TokenSink};
use pocket_types::{GenerateRequest, ReasoningMode, SamplingParams};

/// Rust-to-Swift streaming callback, proving the pattern APL-02's `LiveCore`
/// will use for real chat streaming once the full FFI surface lands: a
/// `pocket-engine` engine drives this callback and Swift turns it into an
/// `AsyncThrowingStream`.
#[uniffi::export(with_foreign)]
pub trait GenerateSink: Send + Sync {
    fn on_delta(&self, text: String);
    fn on_done(&self, completion_tokens: u32);
    fn on_error(&self, message: String);
}

struct SinkAdapter(Arc<dyn GenerateSink>);

impl TokenSink for SinkAdapter {
    fn on_delta(&self, delta: pocket_types::GenerateDelta) {
        self.0.on_delta(delta.text);
    }
    fn on_done(&self, stats: pocket_types::GenerateStats) {
        self.0.on_done(stats.completion_tokens);
    }
    fn on_error(&self, err: pocket_types::PocketError) {
        self.0.on_error(err.to_string());
    }
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build mock_generate runtime")
    })
}

/// Blocks the calling (native) thread until generation finishes, driving
/// `sink` synchronously as tokens arrive. Swift calls this from a background
/// `Task` so it never blocks the UI thread. A real `uniffi::export(async)`
/// path needs a reactor already running in-process, which isn't guaranteed
/// here (see DECISIONS.md D21); this sidesteps that with a dedicated runtime.
#[uniffi::export]
pub fn mock_generate(seed: i64, tokens_per_sec: f32, sink: Arc<dyn GenerateSink>) {
    let engine = MockEngine::new(tokens_per_sec);
    let req = GenerateRequest {
        request_id: "mock".into(),
        messages: vec![],
        tools: vec![],
        params: SamplingParams {
            seed: Some(seed),
            ..Default::default()
        },
        reasoning: ReasoningMode::Off,
    };
    let adapter: Arc<dyn TokenSink> = Arc::new(SinkAdapter(sink));
    runtime().block_on(async {
        let _ = engine.generate("handle".into(), req, adapter).await;
    });
}
