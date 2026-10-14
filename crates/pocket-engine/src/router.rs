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
}
