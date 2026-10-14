//! ENG-09: after a model's first successful load, run a background
//! fixed-prompt generation to measure real throughput (A§4.4's `Benchmark`),
//! so `pocket_models::fit()` can report `Confidence::Measured` instead of
//! `Estimated`. Skipped if the caller is already busy generating for that
//! device, and cancelled (via `Engine::cancel`) if the caller becomes busy
//! mid-run.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use pocket_types::{
    Benchmark, ChatMessage, ContentPart, GenerateDelta, GenerateRequest, GenerateStats,
    ReasoningMode, SamplingParams,
};

use crate::traits::{Engine, TokenSink};

const BENCHMARK_PROMPT: &str =
    "The quick brown fox jumps over the lazy dog. Continue this story in a few sentences.";
const BENCHMARK_MAX_TOKENS: u32 = 64;

/// Caches one `Benchmark` per model id, keyed by whatever string the caller
/// uses to identify the model (its `ModelId`'s `value`).
#[derive(Default)]
pub struct BenchmarkStore {
    results: Mutex<HashMap<String, Benchmark>>,
}

impl BenchmarkStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, model_id: &str) -> Option<Benchmark> {
        self.results.lock().unwrap().get(model_id).cloned()
    }

    fn set(&self, model_id: String, benchmark: Benchmark) {
        self.results.lock().unwrap().insert(model_id, benchmark);
    }
}

struct BenchmarkSink {
    request_id: String,
    engine: Arc<dyn Engine>,
    busy: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    start: Instant,
    first_token_at: Mutex<Option<Instant>>,
    tokens: Mutex<u32>,
}

impl TokenSink for BenchmarkSink {
    fn on_delta(&self, _delta: GenerateDelta) {
        let mut first = self.first_token_at.lock().unwrap();
        if first.is_none() {
            *first = Some(Instant::now());
        }
        drop(first);
        *self.tokens.lock().unwrap() += 1;

        if self.busy.load(Ordering::SeqCst) && !self.cancelled.swap(true, Ordering::SeqCst) {
            self.engine.cancel(self.request_id.clone());
        }
    }
    fn on_done(&self, _stats: GenerateStats) {}
    fn on_error(&self, _err: pocket_types::PocketError) {}
}

/// Runs the benchmark for `model_id` on `handle` unless it's already
/// measured or `busy` is set (another generation is in flight for this
/// device). Returns `None` when skipped, already measured, cancelled
/// mid-run, or on engine error - none of those are failures worth
/// surfacing, since `fit()` just keeps reporting `Estimated`.
pub async fn benchmark_after_load(
    engine: Arc<dyn Engine>,
    handle: String,
    model_id: String,
    store: &BenchmarkStore,
    busy: &Arc<AtomicBool>,
) -> Option<Benchmark> {
    if busy.load(Ordering::SeqCst) || store.get(&model_id).is_some() {
        return None;
    }

    let request_id = format!("benchmark-{model_id}");
    let req = GenerateRequest {
        request_id: request_id.clone(),
        messages: vec![ChatMessage {
            role: "user".into(),
            parts: vec![ContentPart::Text {
                text: BENCHMARK_PROMPT.into(),
            }],
        }],
        tools: vec![],
        params: SamplingParams {
            temperature: 0.0,
            top_p: 1.0,
            top_k: 1,
            min_p: 0.0,
            max_tokens: BENCHMARK_MAX_TOKENS,
            stop: vec![],
            seed: Some(0),
            repetition_penalty: 1.0,
        },
        reasoning: ReasoningMode::Off,
    };

    let cancelled = Arc::new(AtomicBool::new(false));
    let sink = Arc::new(BenchmarkSink {
        request_id,
        engine: Arc::clone(&engine),
        busy: Arc::clone(busy),
        cancelled: Arc::clone(&cancelled),
        start: Instant::now(),
        first_token_at: Mutex::new(None),
        tokens: Mutex::new(0),
    });

    let result = engine.generate(handle.clone(), req, sink.clone()).await;
    if cancelled.load(Ordering::SeqCst) || result.is_err() {
        return None;
    }

    let tokens = *sink.tokens.lock().unwrap();
    if tokens == 0 {
        return None;
    }
    let total_elapsed = sink.start.elapsed();
    let ttft_ms = sink
        .first_token_at
        .lock()
        .unwrap()
        .map(|t| (t - sink.start).as_millis() as u32)
        .unwrap_or(0);
    let tps = tokens as f32 / total_elapsed.as_secs_f32().max(0.001);
    let benchmark = Benchmark {
        tps,
        ttft_ms,
        peak_bytes: engine.resident_bytes(handle),
    };
    store.set(model_id, benchmark.clone());
    Some(benchmark)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEngine;
    use pocket_types::LoadOptions;

    fn manifest() -> pocket_types::ModelManifest {
        pocket_types::ModelManifest {
            id: pocket_types::ModelId {
                value: "test/model".into(),
            },
            display_name: "Test".into(),
            family: "mock".into(),
            params_b: 0.0,
            quant: "none".into(),
            format: pocket_types::ModelFormat::Gguf,
            modalities: vec![],
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

    #[tokio::test]
    async fn records_a_benchmark_after_first_load() {
        let engine: Arc<dyn Engine> = Arc::new(MockEngine::new(2000.0));
        let loaded = engine
            .load(manifest(), LoadOptions::default())
            .await
            .unwrap();
        let store = BenchmarkStore::new();
        let busy = Arc::new(AtomicBool::new(false));

        let result = benchmark_after_load(
            Arc::clone(&engine),
            loaded.handle,
            "test/model".into(),
            &store,
            &busy,
        )
        .await;

        let benchmark = result.expect("expected a recorded benchmark");
        assert!(benchmark.tps > 0.0);
        assert_eq!(store.get("test/model").unwrap(), benchmark);
    }

    #[tokio::test]
    async fn skipped_when_already_busy_generating() {
        let engine: Arc<dyn Engine> = Arc::new(MockEngine::new(2000.0));
        let loaded = engine
            .load(manifest(), LoadOptions::default())
            .await
            .unwrap();
        let store = BenchmarkStore::new();
        let busy = Arc::new(AtomicBool::new(true));

        let result =
            benchmark_after_load(engine, loaded.handle, "test/model".into(), &store, &busy).await;

        assert!(result.is_none());
        assert!(store.get("test/model").is_none());
    }

    #[tokio::test]
    async fn skipped_when_already_measured() {
        let engine: Arc<dyn Engine> = Arc::new(MockEngine::new(2000.0));
        let loaded = engine
            .load(manifest(), LoadOptions::default())
            .await
            .unwrap();
        let store = BenchmarkStore::new();
        store.set(
            "test/model".into(),
            Benchmark {
                tps: 1.0,
                ttft_ms: 1,
                peak_bytes: 1,
            },
        );
        let busy = Arc::new(AtomicBool::new(false));

        let result =
            benchmark_after_load(engine, loaded.handle, "test/model".into(), &store, &busy).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cancelled_when_user_starts_generating_mid_benchmark() {
        // Slow enough (10 tok/s) that flipping `busy` after a short delay
        // lands mid-run, not after completion.
        let engine: Arc<dyn Engine> = Arc::new(MockEngine::new(10.0));
        let loaded = engine
            .load(manifest(), LoadOptions::default())
            .await
            .unwrap();
        let store = BenchmarkStore::new();
        let busy = Arc::new(AtomicBool::new(false));

        let busy_clone = Arc::clone(&busy);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            busy_clone.store(true, Ordering::SeqCst);
        });

        let result =
            benchmark_after_load(engine, loaded.handle, "test/model".into(), &store, &busy).await;

        assert!(result.is_none());
        assert!(store.get("test/model").is_none());
    }
}
