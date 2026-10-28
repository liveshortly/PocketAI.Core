use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pocket_types::{EngineKind, ModelFormat, ModelId, ModelManifest, PocketError};

use crate::traits::Engine;

#[derive(Clone, Debug, Default)]
pub struct EnginePrefs {
    pub pinned_engine: Option<EngineKind>,
}

/// ENG-13: holds one `Engine` per `EngineKind` (e.g. a Swift `RemoteEngine`
/// registered under `EngineKind::Remote`) and resolves a model to whichever
/// one `route()` says should serve it.
#[derive(Default)]
pub struct EngineRegistry {
    engines: Mutex<HashMap<EngineKind, Arc<dyn Engine>>>,
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, kind: EngineKind, engine: Arc<dyn Engine>) {
        self.engines.lock().unwrap().insert(kind, engine);
    }

    pub fn resolve(
        &self,
        model: &ModelManifest,
        prefs: &EnginePrefs,
    ) -> Result<Arc<dyn Engine>, PocketError> {
        let kind = route(model, prefs)?;
        self.engines
            .lock()
            .unwrap()
            .get(&kind)
            .cloned()
            .ok_or_else(|| PocketError::EngineFailed {
                engine: kind,
                message: format!("no engine registered for {kind:?}"),
            })
    }
}

/// ENG-03: a composite `Engine` that dispatches each call to whichever
/// sub-engine owns (or can serve) the model, promoting the D95/D96
/// single-slot + Swift-`RoutingEngine` workaround into one tested Rust router.
///
/// Sub-engines are tried in priority order on `load`; the chosen one owns that
/// handle, and `unload`/`generate`/`embed`/`resident_bytes` dispatch back to it
/// by handle. `cancel` broadcasts (a request id isn't tied to a handle).
///
/// ponytail: priority-order first-supporting match, same policy the Swift
/// composite used. The per-format [`route`]/[`EngineRegistry`] policy is the
/// upgrade path if models ever need pinning to a specific kind here.
pub struct CompositeEngine {
    kind: EngineKind,
    engines: Vec<Arc<dyn Engine>>,
    handles: Mutex<HashMap<String, Arc<dyn Engine>>>,
}

impl CompositeEngine {
    /// `kind` is the nominal kind this composite reports (it has no single real
    /// one); `engines` are tried in order on load.
    pub fn new(kind: EngineKind, engines: Vec<Arc<dyn Engine>>) -> Self {
        Self {
            kind,
            engines,
            handles: Mutex::new(HashMap::new()),
        }
    }

    fn owner(&self, handle: &str) -> Result<Arc<dyn Engine>, PocketError> {
        self.handles
            .lock()
            .unwrap()
            .get(handle)
            .cloned()
            .ok_or_else(|| PocketError::EngineFailed {
                engine: self.kind,
                message: format!("unknown handle {handle}"),
            })
    }
}

#[async_trait::async_trait]
impl Engine for CompositeEngine {
    fn kind(&self) -> EngineKind {
        self.kind
    }

    fn supports(&self, model: &ModelManifest) -> bool {
        self.engines.iter().any(|e| e.supports(model))
    }

    async fn load(
        &self,
        model: ModelManifest,
        opts: pocket_types::LoadOptions,
    ) -> Result<pocket_types::LoadedModel, PocketError> {
        let engine = self
            .engines
            .iter()
            .find(|e| e.supports(&model))
            .cloned()
            .ok_or_else(|| PocketError::EngineFailed {
                engine: self.kind,
                message: format!("no engine supports model {}", model.id),
            })?;
        let loaded = engine.load(model, opts).await?;
        self.handles
            .lock()
            .unwrap()
            .insert(loaded.handle.clone(), engine);
        Ok(loaded)
    }

    async fn unload(&self, handle: String) -> Result<(), PocketError> {
        // Remove ownership first so a failed unload doesn't strand the entry.
        let engine = self.handles.lock().unwrap().remove(&handle);
        match engine {
            Some(engine) => engine.unload(handle).await,
            None => Ok(()),
        }
    }

    async fn generate(
        &self,
        handle: String,
        req: pocket_types::GenerateRequest,
        sink: Arc<dyn crate::traits::TokenSink>,
    ) -> Result<pocket_types::GenerateStats, PocketError> {
        self.owner(&handle)?.generate(handle, req, sink).await
    }

    async fn embed(
        &self,
        handle: String,
        inputs: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, PocketError> {
        self.owner(&handle)?.embed(handle, inputs).await
    }

    fn cancel(&self, request_id: String) {
        for engine in &self.engines {
            engine.cancel(request_id.clone());
        }
    }

    fn resident_bytes(&self, handle: String) -> u64 {
        self.handles
            .lock()
            .unwrap()
            .get(&handle)
            .map(|e| e.resident_bytes(handle.clone()))
            .unwrap_or(0)
    }
}

pub fn route(model: &ModelManifest, prefs: &EnginePrefs) -> Result<EngineKind, PocketError> {
    if let Some(pinned) = prefs.pinned_engine {
        return Ok(pinned);
    }
    if is_remote(&model.id) {
        return Ok(EngineKind::Remote);
    }
    match model.format {
        ModelFormat::MlxSafetensors => Ok(EngineKind::Mlx),
        ModelFormat::Gguf => Ok(EngineKind::LlamaCpp),
        ModelFormat::CoreMlPackage => Ok(EngineKind::CoreMl),
        ModelFormat::WhisperKit => Ok(EngineKind::WhisperKit),
    }
}

fn is_remote(id: &ModelId) -> bool {
    id.is_remote()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pocket_types::{LicenseInfo, Modality};

    fn manifest(format: ModelFormat, id: &str) -> ModelManifest {
        ModelManifest {
            id: ModelId::parse(id).unwrap(),
            display_name: "Test".into(),
            family: "Test".into(),
            params_b: 1.0,
            quant: "4bit".into(),
            format,
            modalities: vec![Modality::Text],
            context_length: 4096,
            files: vec![],
            license: LicenseInfo {
                spdx: "MIT".into(),
                commercial_ok: true,
                url: String::new(),
            },
            hf_repo: id.split('@').next().unwrap().to_string(),
            hf_revision: "abc".into(),
            min_ram_bytes: 0,
            est_runtime_bytes: 0,
            tags: vec![],
            curated: true,
        }
    }

    #[test]
    fn routes_mlx_safetensors_to_mlx() {
        let m = manifest(ModelFormat::MlxSafetensors, "mlx-community/x@rev");
        assert_eq!(route(&m, &EnginePrefs::default()).unwrap(), EngineKind::Mlx);
    }

    #[test]
    fn routes_gguf_to_llamacpp() {
        let m = manifest(ModelFormat::Gguf, "ggml-org/x@rev");
        assert_eq!(
            route(&m, &EnginePrefs::default()).unwrap(),
            EngineKind::LlamaCpp
        );
    }

    #[test]
    fn routes_remote_model_id_to_remote() {
        let m = manifest(
            ModelFormat::MlxSafetensors,
            "mac:01J.../mlx-community/x@rev",
        );
        assert_eq!(
            route(&m, &EnginePrefs::default()).unwrap(),
            EngineKind::Remote
        );
    }

    #[test]
    fn user_pinned_engine_wins() {
        let m = manifest(ModelFormat::Gguf, "ggml-org/x@rev");
        let prefs = EnginePrefs {
            pinned_engine: Some(EngineKind::Mlx),
        };
        assert_eq!(route(&m, &prefs).unwrap(), EngineKind::Mlx);
    }

    #[test]
    fn routes_core_ml_package() {
        let m = manifest(ModelFormat::CoreMlPackage, "apple/x@rev");
        assert_eq!(
            route(&m, &EnginePrefs::default()).unwrap(),
            EngineKind::CoreMl
        );
    }

    /// A minimal stand-in for Swift's `RemoteEngine` (L3), which talks to a
    /// paired Mac over HTTPS+SSE - all that matters here is that it reports
    /// `EngineKind::Remote` so the registry dispatches to it.
    struct MockRemoteEngine;

    #[async_trait::async_trait]
    impl Engine for MockRemoteEngine {
        fn kind(&self) -> EngineKind {
            EngineKind::Remote
        }
        fn supports(&self, _model: &ModelManifest) -> bool {
            true
        }
        async fn load(
            &self,
            _model: ModelManifest,
            _opts: pocket_types::LoadOptions,
        ) -> Result<pocket_types::LoadedModel, PocketError> {
            Ok(pocket_types::LoadedModel {
                handle: "remote-handle".into(),
            })
        }
        async fn unload(&self, _handle: String) -> Result<(), PocketError> {
            Ok(())
        }
        async fn generate(
            &self,
            _handle: String,
            _req: pocket_types::GenerateRequest,
            _sink: Arc<dyn crate::traits::TokenSink>,
        ) -> Result<pocket_types::GenerateStats, PocketError> {
            unimplemented!("not exercised by these tests")
        }
        async fn embed(
            &self,
            _handle: String,
            _inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, PocketError> {
            Ok(vec![])
        }
        fn cancel(&self, _request_id: String) {}
        fn resident_bytes(&self, _handle: String) -> u64 {
            0
        }
    }

    #[test]
    fn resolve_dispatches_a_remote_model_id_to_the_registered_remote_engine() {
        let registry = EngineRegistry::new();
        registry.register(EngineKind::Remote, Arc::new(MockRemoteEngine));

        let m = manifest(
            ModelFormat::MlxSafetensors,
            "mac:01J.../mlx-community/x@rev",
        );
        let engine = registry.resolve(&m, &EnginePrefs::default()).unwrap();
        assert_eq!(engine.kind(), EngineKind::Remote);
    }

    #[test]
    fn resolve_errors_when_no_engine_is_registered_for_the_routed_kind() {
        let registry = EngineRegistry::new();
        let m = manifest(
            ModelFormat::MlxSafetensors,
            "mac:01J.../mlx-community/x@rev",
        );
        match registry.resolve(&m, &EnginePrefs::default()) {
            Err(PocketError::EngineFailed {
                engine: EngineKind::Remote,
                ..
            }) => {}
            other => panic!(
                "expected EngineFailed{{engine: Remote, ..}}, got {}",
                other.is_ok()
            ),
        }
    }

    /// A configurable fake engine that records which calls it received and
    /// loads handles prefixed with its id, so composite dispatch is observable.
    struct FakeEngine {
        id: &'static str,
        kind: EngineKind,
        supports: bool,
        unloaded: Mutex<Vec<String>>,
        generated: Mutex<Vec<String>>,
        cancelled: Mutex<Vec<String>>,
    }

    impl FakeEngine {
        fn new(id: &'static str, kind: EngineKind, supports: bool) -> Arc<Self> {
            Arc::new(Self {
                id,
                kind,
                supports,
                unloaded: Mutex::new(vec![]),
                generated: Mutex::new(vec![]),
                cancelled: Mutex::new(vec![]),
            })
        }
    }

    #[async_trait::async_trait]
    impl Engine for FakeEngine {
        fn kind(&self) -> EngineKind {
            self.kind
        }
        fn supports(&self, _model: &ModelManifest) -> bool {
            self.supports
        }
        async fn load(
            &self,
            _model: ModelManifest,
            _opts: pocket_types::LoadOptions,
        ) -> Result<pocket_types::LoadedModel, PocketError> {
            Ok(pocket_types::LoadedModel {
                handle: format!("{}-handle", self.id),
            })
        }
        async fn unload(&self, handle: String) -> Result<(), PocketError> {
            self.unloaded.lock().unwrap().push(handle);
            Ok(())
        }
        async fn generate(
            &self,
            handle: String,
            _req: pocket_types::GenerateRequest,
            _sink: Arc<dyn crate::traits::TokenSink>,
        ) -> Result<pocket_types::GenerateStats, PocketError> {
            self.generated.lock().unwrap().push(handle);
            Ok(pocket_types::GenerateStats {
                prompt_tokens: 0,
                completion_tokens: 0,
                ttft_ms: 0,
                decode_tps: 0.0,
                finish: pocket_types::FinishReason::Stop,
            })
        }
        async fn embed(
            &self,
            _handle: String,
            inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, PocketError> {
            Ok(inputs.iter().map(|_| vec![1.0]).collect())
        }
        fn cancel(&self, request_id: String) {
            self.cancelled.lock().unwrap().push(request_id);
        }
        fn resident_bytes(&self, _handle: String) -> u64 {
            42
        }
    }

    #[tokio::test]
    async fn composite_loads_first_supporting_engine_and_dispatches_by_handle() {
        let mlx = FakeEngine::new("mlx", EngineKind::Mlx, false);
        let llama = FakeEngine::new("llama", EngineKind::LlamaCpp, true);
        let composite = CompositeEngine::new(
            EngineKind::Mlx,
            vec![mlx.clone(), llama.clone()],
        );

        let m = manifest(ModelFormat::Gguf, "ggml-org/x@rev");
        let loaded = composite
            .load(m, pocket_types::LoadOptions::default())
            .await
            .unwrap();
        // mlx doesn't support -> llama (next in priority) owns the handle.
        assert_eq!(loaded.handle, "llama-handle");
        assert_eq!(composite.resident_bytes(loaded.handle.clone()), 42);

        let req = pocket_types::GenerateRequest {
            request_id: "r1".into(),
            messages: vec![],
            tools: vec![],
            params: pocket_types::SamplingParams::default(),
            reasoning: pocket_types::ReasoningMode::Off,
        };
        composite
            .generate(loaded.handle.clone(), req, Arc::new(NoopSink))
            .await
            .unwrap();
        assert_eq!(*llama.generated.lock().unwrap(), vec![loaded.handle.clone()]);

        composite.unload(loaded.handle.clone()).await.unwrap();
        assert_eq!(*llama.unloaded.lock().unwrap(), vec![loaded.handle.clone()]);
        // Handle is gone now: resident_bytes falls back to 0.
        assert_eq!(composite.resident_bytes(loaded.handle), 0);
    }

    #[tokio::test]
    async fn composite_errors_on_unknown_handle_and_when_nothing_supports() {
        let llama = FakeEngine::new("llama", EngineKind::LlamaCpp, true);
        let composite = CompositeEngine::new(EngineKind::Mlx, vec![llama]);

        // generate on an unknown handle errors.
        let req = pocket_types::GenerateRequest {
            request_id: "r1".into(),
            messages: vec![],
            tools: vec![],
            params: pocket_types::SamplingParams::default(),
            reasoning: pocket_types::ReasoningMode::Off,
        };
        assert!(composite
            .generate("nope".into(), req, Arc::new(NoopSink))
            .await
            .is_err());
        // unload of an unknown handle is an idempotent no-op success.
        composite.unload("nope".into()).await.unwrap();

        // No sub-engine supports -> load errors.
        let rejecting = CompositeEngine::new(
            EngineKind::Mlx,
            vec![FakeEngine::new("x", EngineKind::Mlx, false)],
        );
        let m = manifest(ModelFormat::Gguf, "ggml-org/x@rev");
        assert!(rejecting
            .load(m, pocket_types::LoadOptions::default())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn composite_cancel_broadcasts_to_all_sub_engines() {
        let a = FakeEngine::new("a", EngineKind::Mlx, true);
        let b = FakeEngine::new("b", EngineKind::LlamaCpp, true);
        let composite = CompositeEngine::new(EngineKind::Mlx, vec![a.clone(), b.clone()]);
        composite.cancel("req-7".into());
        assert_eq!(*a.cancelled.lock().unwrap(), vec!["req-7".to_string()]);
        assert_eq!(*b.cancelled.lock().unwrap(), vec!["req-7".to_string()]);
    }

    struct NoopSink;
    impl crate::traits::TokenSink for NoopSink {
        fn on_delta(&self, _delta: pocket_types::GenerateDelta) {}
        fn on_done(&self, _stats: pocket_types::GenerateStats) {}
        fn on_error(&self, _err: PocketError) {}
    }
}
