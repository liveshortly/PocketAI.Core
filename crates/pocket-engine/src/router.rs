use pocket_types::{EngineKind, ModelFormat, ModelId, ModelManifest, PocketError};

#[derive(Clone, Debug, Default)]
pub struct EnginePrefs {
    pub pinned_engine: Option<EngineKind>,
}

/// `Engine` instances capable of serving a request, keyed by kind.
pub trait EngineRegistry {
    fn supports(&self, kind: EngineKind, model: &ModelManifest) -> bool;
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
        let m = manifest(ModelFormat::MlxSafetensors, "mac:01J.../mlx-community/x@rev");
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
}
