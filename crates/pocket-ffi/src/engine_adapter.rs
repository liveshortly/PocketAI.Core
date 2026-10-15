use std::sync::Arc;

use crate::error::PocketError;
use crate::traits::{Engine, TokenSink};
use crate::types::*;

/// Wraps a Rust `pocket_engine::Engine` (async) behind the FFI-local `Engine` trait.
pub struct PocketEngineAdapter<E: pocket_engine::Engine> {
    inner: E,
}

impl<E: pocket_engine::Engine> PocketEngineAdapter<E> {
    pub fn new(inner: E) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &E {
        &self.inner
    }
}

impl<E: pocket_engine::Engine> Engine for PocketEngineAdapter<E> {
    fn kind(&self) -> EngineKind {
        engine_kind_to_ffi(self.inner.kind())
    }

    fn supports(&self, model: ModelManifest) -> bool {
        let p_manifest = manifest_to_pocket(&model);
        self.inner.supports(&p_manifest)
    }

    fn load(&self, model: ModelManifest, opts: LoadOptions) -> Result<LoadedModel, PocketError> {
        let p_manifest = manifest_to_pocket(&model);
        let p_opts = load_options_to_pocket(opts);
        let loaded = crate::core::runtime()
            .block_on(self.inner.load(p_manifest, p_opts))
            .map_err(|e| PocketError::EngineFailed {
                engine: self.kind(),
                message: e.to_string(),
            })?;
        Ok(loaded_model_to_ffi(loaded))
    }

    fn unload(&self, handle: String) -> Result<(), PocketError> {
        crate::core::runtime()
            .block_on(self.inner.unload(handle))
            .map_err(|e| PocketError::EngineFailed {
                engine: self.kind(),
                message: e.to_string(),
            })
    }

    fn generate(
        &self,
        handle: String,
        req: GenerateRequest,
        sink: Arc<dyn TokenSink>,
    ) -> Result<GenerateStats, PocketError> {
        let p_req = generate_request_to_pocket(req);
        let sink_bridge = Arc::new(SinkBridge::new(sink, self.kind()));
        let stats = crate::core::runtime()
            .block_on(self.inner.generate(handle, p_req, sink_bridge))
            .map_err(|e| PocketError::EngineFailed {
                engine: self.kind(),
                message: e.to_string(),
            })?;
        Ok(generate_stats_to_ffi(stats))
    }

    fn embed(&self, handle: String, inputs: Vec<String>) -> Result<Vec<Vec<f32>>, PocketError> {
        crate::core::runtime()
            .block_on(self.inner.embed(handle, inputs))
            .map_err(|e| PocketError::EngineFailed {
                engine: self.kind(),
                message: e.to_string(),
            })
    }

    fn cancel(&self, request_id: String) {
        self.inner.cancel(request_id);
    }

    fn resident_bytes(&self, handle: String) -> u64 {
        self.inner.resident_bytes(handle)
    }
}

/// Bridges an FFI `TokenSink` to a `pocket_engine::TokenSink`.
pub struct SinkBridge {
    sink: Arc<dyn TokenSink>,
    engine_kind: EngineKind,
}

impl SinkBridge {
    pub fn new(sink: Arc<dyn TokenSink>, engine_kind: EngineKind) -> Self {
        Self { sink, engine_kind }
    }
}

impl pocket_engine::TokenSink for SinkBridge {
    fn on_delta(&self, delta: pocket_types::GenerateDelta) {
        self.sink.on_delta(generate_delta_to_ffi(delta));
    }

    fn on_done(&self, stats: pocket_types::GenerateStats) {
        self.sink.on_done(generate_stats_to_ffi(stats));
    }

    fn on_error(&self, err: pocket_types::PocketError) {
        self.sink.on_error(PocketError::EngineFailed {
            engine: self.engine_kind,
            message: err.to_string(),
        });
    }
}

pub fn engine_kind_to_ffi(k: pocket_types::EngineKind) -> EngineKind {
    match k {
        pocket_types::EngineKind::Mlx => EngineKind::Mlx,
        pocket_types::EngineKind::LlamaCpp => EngineKind::LlamaCpp,
        pocket_types::EngineKind::CoreMl => EngineKind::CoreMl,
        pocket_types::EngineKind::WhisperKit => EngineKind::WhisperKit,
        pocket_types::EngineKind::AppleSpeech => EngineKind::AppleSpeech,
        pocket_types::EngineKind::AppleFoundation => EngineKind::AppleFoundation,
        pocket_types::EngineKind::Remote => EngineKind::Remote,
    }
}

pub fn engine_kind_from_ffi(k: EngineKind) -> pocket_types::EngineKind {
    match k {
        EngineKind::Mlx => pocket_types::EngineKind::Mlx,
        EngineKind::LlamaCpp => pocket_types::EngineKind::LlamaCpp,
        EngineKind::CoreMl => pocket_types::EngineKind::CoreMl,
        EngineKind::WhisperKit => pocket_types::EngineKind::WhisperKit,
        EngineKind::AppleSpeech => pocket_types::EngineKind::AppleSpeech,
        EngineKind::AppleFoundation => pocket_types::EngineKind::AppleFoundation,
        EngineKind::Remote => pocket_types::EngineKind::Remote,
    }
}

pub fn manifest_to_pocket(m: &ModelManifest) -> pocket_types::ModelManifest {
    pocket_types::ModelManifest {
        id: pocket_types::ModelId {
            value: m.id.clone(),
        },
        display_name: m.display_name.clone(),
        family: m.family.clone(),
        params_b: m.params_b,
        quant: m.quant.clone(),
        format: match m.format {
            ModelFormat::MlxSafetensors => pocket_types::ModelFormat::MlxSafetensors,
            ModelFormat::Gguf => pocket_types::ModelFormat::Gguf,
            ModelFormat::CoreMlPackage => pocket_types::ModelFormat::CoreMlPackage,
            ModelFormat::WhisperKit => pocket_types::ModelFormat::WhisperKit,
        },
        modalities: m
            .modalities
            .iter()
            .map(|modality| match modality {
                Modality::Text => pocket_types::Modality::Text,
                Modality::Vision => pocket_types::Modality::Vision,
                Modality::Embedding => pocket_types::Modality::Embedding,
                Modality::SpeechToText => pocket_types::Modality::SpeechToText,
                Modality::TextToSpeech => pocket_types::Modality::TextToSpeech,
                Modality::Image => pocket_types::Modality::Image,
            })
            .collect(),
        context_length: m.context_length,
        files: m
            .files
            .iter()
            .map(|f| pocket_types::ModelFile {
                path: f.path.clone(),
                size: f.size,
                sha256: f.sha256.clone(),
                url: f.url.clone(),
            })
            .collect(),
        license: pocket_types::LicenseInfo {
            spdx: m.license.spdx.clone(),
            commercial_ok: m.license.commercial_ok,
            url: m.license.url.clone(),
        },
        hf_repo: m.hf_repo.clone(),
        hf_revision: m.hf_revision.clone(),
        min_ram_bytes: m.min_ram_bytes,
        est_runtime_bytes: m.est_runtime_bytes,
        tags: m.tags.clone(),
        curated: m.curated,
    }
}

pub fn load_options_to_pocket(opts: LoadOptions) -> pocket_types::LoadOptions {
    pocket_types::LoadOptions {
        context_length: opts.context_length,
        pinned_engine: opts.pinned_engine.map(engine_kind_from_ffi),
    }
}

pub fn loaded_model_to_ffi(m: pocket_types::LoadedModel) -> LoadedModel {
    LoadedModel { handle: m.handle }
}

pub fn generate_request_to_pocket(req: GenerateRequest) -> pocket_types::GenerateRequest {
    pocket_types::GenerateRequest {
        request_id: req.request_id,
        messages: req
            .messages
            .into_iter()
            .map(|m| pocket_types::ChatMessage {
                role: m.role,
                parts: m
                    .parts
                    .into_iter()
                    .map(|p| match p {
                        ContentPart::Text { text } => pocket_types::ContentPart::Text { text },
                        ContentPart::ImageRef { path } => {
                            pocket_types::ContentPart::ImageRef { path }
                        }
                    })
                    .collect(),
            })
            .collect(),
        tools: req
            .tools
            .into_iter()
            .map(|t| pocket_types::ToolSpec {
                name: t.name,
                description: t.description,
                parameters_json_schema: t.parameters_json_schema,
            })
            .collect(),
        params: pocket_types::SamplingParams {
            temperature: req.params.temperature,
            top_p: req.params.top_p,
            top_k: req.params.top_k,
            min_p: req.params.min_p,
            max_tokens: req.params.max_tokens,
            stop: req.params.stop,
            seed: req.params.seed,
            repetition_penalty: req.params.repetition_penalty,
        },
        reasoning: match req.reasoning {
            ReasoningMode::Off => pocket_types::ReasoningMode::Off,
            ReasoningMode::On => pocket_types::ReasoningMode::On,
            ReasoningMode::Auto => pocket_types::ReasoningMode::Auto,
        },
    }
}

pub fn generate_delta_to_ffi(delta: pocket_types::GenerateDelta) -> GenerateDelta {
    GenerateDelta {
        text: delta.text,
        reasoning: delta.reasoning,
        tool_calls: delta
            .tool_calls
            .into_iter()
            .map(|tc| ToolCallDelta {
                id: tc.id,
                name: tc.name,
                arguments_json_fragment: tc.arguments_json_fragment,
            })
            .collect(),
    }
}

pub fn finish_reason_to_ffi(r: pocket_types::FinishReason) -> FinishReason {
    match r {
        pocket_types::FinishReason::Stop => FinishReason::Stop,
        pocket_types::FinishReason::Length => FinishReason::Length,
        pocket_types::FinishReason::ToolCalls => FinishReason::ToolCalls,
        pocket_types::FinishReason::ContentFilter => FinishReason::ContentFilter,
        pocket_types::FinishReason::Cancelled => FinishReason::Cancelled,
        pocket_types::FinishReason::Error => FinishReason::Error,
    }
}

pub fn generate_stats_to_ffi(stats: pocket_types::GenerateStats) -> GenerateStats {
    GenerateStats {
        prompt_tokens: stats.prompt_tokens,
        completion_tokens: stats.completion_tokens,
        ttft_ms: stats.ttft_ms,
        decode_tps: stats.decode_tps,
        finish: finish_reason_to_ffi(stats.finish),
    }
}
