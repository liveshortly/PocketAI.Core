use std::sync::Arc;

use crate::error::PocketError;
use crate::types::*;

#[uniffi::export(with_foreign)]
pub trait PlatformHooks: Send + Sync {
    fn keychain_get(&self, key: String) -> Option<Vec<u8>>;
    fn keychain_set(&self, key: String, value: Vec<u8>) -> bool;
    fn device_profile(&self) -> DeviceProfile;
    fn app_support_dir(&self) -> String;
    fn now_ms(&self) -> i64;
}

#[uniffi::export(with_foreign)]
pub trait DownloadCoordinator: Send + Sync {
    fn start(&self, plan: DownloadPlan) -> Result<(), PocketError>;
    fn pause(&self, id: ModelId) -> Result<(), PocketError>;
    fn resume(&self, id: ModelId) -> Result<(), PocketError>;
    fn cancel(&self, id: ModelId) -> Result<(), PocketError>;
    fn pause_all_internet_transfers(&self) -> Result<(), PocketError>;
}

/// Mirrors `pocket-engine::TokenSink` at the FFI boundary (DECISIONS.md D19):
/// batched, coalesced to >=16ms or >=8 tokens per call by the engine.
#[uniffi::export(with_foreign)]
pub trait TokenSink: Send + Sync {
    fn on_delta(&self, delta: GenerateDelta);
    fn on_done(&self, stats: GenerateStats);
    fn on_error(&self, err: PocketError);
}

/// `TokenSink` + `on_message_saved`, per A§4.6.
#[uniffi::export(with_foreign)]
pub trait ChatSink: Send + Sync {
    fn on_delta(&self, delta: GenerateDelta);
    fn on_done(&self, stats: GenerateStats);
    fn on_error(&self, err: PocketError);
    fn on_message_saved(&self, message: Message);
    /// AGT-12: a web tool ran while Tools is on (before the answer streams).
    fn on_tool_step(&self, step: ToolStep);
}

#[uniffi::export(with_foreign)]
pub trait EventSink: Send + Sync {
    fn on_event(&self, event: Event);
}

/// Mirrors `pocket-engine::Engine` at the FFI boundary. Not yet wired to a
/// real adapter into `pocket_engine::Engine` — register_engine just stores
/// the handle today (see DECISIONS.md D23).
#[uniffi::export(with_foreign)]
pub trait Engine: Send + Sync {
    fn kind(&self) -> EngineKind;
    fn supports(&self, model: ModelManifest) -> bool;
    fn load(&self, model: ModelManifest, opts: LoadOptions) -> Result<LoadedModel, PocketError>;
    fn unload(&self, handle: String) -> Result<(), PocketError>;
    fn generate(
        &self,
        handle: String,
        req: GenerateRequest,
        sink: Arc<dyn TokenSink>,
    ) -> Result<GenerateStats, PocketError>;
    fn embed(&self, handle: String, inputs: Vec<String>) -> Result<Vec<Vec<f32>>, PocketError>;
    fn cancel(&self, request_id: String);
    fn resident_bytes(&self, handle: String) -> u64;
}

#[uniffi::export(with_foreign)]
pub trait SpeechToTextEngine: Send + Sync {
    fn transcribe(
        &self,
        audio_path: String,
        lang: Option<String>,
        sink: Arc<dyn TokenSink>,
    ) -> Result<(), PocketError>;
}

#[uniffi::export(with_foreign)]
pub trait TextToSpeechEngine: Send + Sync {
    fn synthesize(&self, text: String, voice: String, out_path: String) -> Result<(), PocketError>;
}

#[uniffi::export(with_foreign)]
pub trait ImageEngine: Send + Sync {
    fn generate(
        &self,
        prompt: String,
        negative: String,
        size: String,
        steps: u32,
        seed: i64,
        sink: Arc<dyn TokenSink>,
    ) -> Result<(), PocketError>;
}

/// NET-22: the iPhone's pinned-TLS channel to the paired Mac, as `sync_now`
/// sees it. Swift implements it over `PeerClient` (pinned cert + device
/// bearer token): send `body` (empty for GET) to `path` on the Mac and return
/// the response body. Non-2xx must be an error; map HTTP 403 `not_entitled`
/// to `PocketError::NotEntitled`. Called synchronously off the main thread.
#[uniffi::export(with_foreign)]
pub trait SyncTransport: Send + Sync {
    fn request(&self, method: String, path: String, body: Vec<u8>) -> Result<Vec<u8>, PocketError>;
}
