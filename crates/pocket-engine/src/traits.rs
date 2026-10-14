use std::sync::Arc;

use pocket_types::{
    EngineKind, GenerateDelta, GenerateRequest, GenerateStats, LoadOptions, LoadedModel,
    ModelManifest, PocketError,
};

#[async_trait::async_trait]
pub trait Engine: Send + Sync {
    fn kind(&self) -> EngineKind;
    fn supports(&self, model: &ModelManifest) -> bool;
    async fn load(
        &self,
        model: ModelManifest,
        opts: LoadOptions,
    ) -> Result<LoadedModel, PocketError>;
    async fn unload(&self, handle: String) -> Result<(), PocketError>;
    async fn generate(
        &self,
        handle: String,
        req: GenerateRequest,
        sink: Arc<dyn TokenSink>,
    ) -> Result<GenerateStats, PocketError>;
    async fn embed(&self, handle: String, inputs: Vec<String>) -> Result<Vec<Vec<f32>>, PocketError>;
    fn cancel(&self, request_id: String);
    fn resident_bytes(&self, handle: String) -> u64;
}

pub trait TokenSink: Send + Sync {
    /// Batched: engines MUST coalesce to >=16ms or >=8 tokens per call.
    fn on_delta(&self, delta: GenerateDelta);
    fn on_done(&self, stats: GenerateStats);
    fn on_error(&self, err: PocketError);
}
