use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pocket_types::{
    EngineKind, FinishReason, GenerateDelta, GenerateRequest, GenerateStats, LoadOptions,
    LoadedModel, ModelManifest, PocketError,
};

use crate::traits::{Engine, TokenSink};

/// Deterministic engine for tests and PreviewCore parity: generates a fixed
/// phrase word-by-word from a seed at a configurable tok/s, supports cancel
/// and hash-based embeddings, and reports fake resident bytes per handle.
pub struct MockEngine {
    tokens_per_sec: f32,
    cancelled: Mutex<HashMap<String, Arc<AtomicBool>>>,
    resident: Mutex<HashMap<String, u64>>,
    next_handle: AtomicU64,
}

impl MockEngine {
    pub fn new(tokens_per_sec: f32) -> Self {
        Self {
            tokens_per_sec,
            cancelled: Mutex::new(HashMap::new()),
            resident: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
        }
    }

    fn words_for_seed(seed: i64) -> Vec<String> {
        let base = ["the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog"];
        (0..16)
            .map(|i| base[(seed as usize + i) % base.len()].to_string())
            .collect()
    }
}

#[async_trait::async_trait]
impl Engine for MockEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Mlx
    }

    fn supports(&self, _model: &ModelManifest) -> bool {
        true
    }

    async fn load(
        &self,
        _model: ModelManifest,
        _opts: LoadOptions,
    ) -> Result<LoadedModel, PocketError> {
        let handle = format!("mock-{}", self.next_handle.fetch_add(1, Ordering::SeqCst));
        self.resident.lock().unwrap().insert(handle.clone(), 256 * 1024 * 1024);
        Ok(LoadedModel { handle })
    }

    async fn unload(&self, handle: String) -> Result<(), PocketError> {
        self.resident.lock().unwrap().remove(&handle);
        Ok(())
    }

    async fn generate(
        &self,
        _handle: String,
        req: GenerateRequest,
        sink: Arc<dyn TokenSink>,
    ) -> Result<GenerateStats, PocketError> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancelled
            .lock()
            .unwrap()
            .insert(req.request_id.clone(), flag.clone());

        let words = Self::words_for_seed(req.params.seed.unwrap_or(0));
        let delay_ms = (1000.0 / self.tokens_per_sec.max(0.01)) as u64;
        let mut completion_tokens = 0u32;
        for word in &words {
            if flag.load(Ordering::SeqCst) {
                let stats = GenerateStats {
                    prompt_tokens: 0,
                    completion_tokens,
                    ttft_ms: 0,
                    decode_tps: self.tokens_per_sec,
                    finish: FinishReason::Cancelled,
                };
                sink.on_done(stats.clone());
                return Ok(stats);
            }
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            sink.on_delta(GenerateDelta {
                text: format!("{word} "),
                reasoning: String::new(),
                tool_calls: Vec::new(),
            });
            completion_tokens += 1;
        }
        let stats = GenerateStats {
            prompt_tokens: req.messages.len() as u32,
            completion_tokens,
            ttft_ms: delay_ms as u32,
            decode_tps: self.tokens_per_sec,
            finish: FinishReason::Stop,
        };
        sink.on_done(stats.clone());
        Ok(stats)
    }

    async fn embed(&self, _handle: String, inputs: Vec<String>) -> Result<Vec<Vec<f32>>, PocketError> {
        Ok(inputs
            .iter()
            .map(|s| {
                let hash = s.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
                (0..8).map(|i| ((hash >> (i * 4)) & 0xF) as f32 / 15.0).collect()
            })
            .collect())
    }

    fn cancel(&self, request_id: String) {
        if let Some(flag) = self.cancelled.lock().unwrap().get(&request_id) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn resident_bytes(&self, handle: String) -> u64 {
        *self.resident.lock().unwrap().get(&handle).unwrap_or(&0)
    }
}
