//! Real `llama.cpp` inference engine for GGUF models.
//!
//! Implements `pocket_engine::Engine` backed by `llama-cpp-2` with Apple Metal
//! GPU acceleration, chat template application, sampler chaining, and dedicated
//! OS thread decode streaming.

pub mod backend;
pub mod decode;
pub mod multimodal;
pub mod sampler;
pub mod template;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use pocket_types::{
    EngineKind, GenerateRequest, GenerateStats, LoadOptions, LoadedModel, ModelFormat,
    ModelManifest, PocketError,
};

use crate::traits::{Engine, TokenSink};

struct LoadedModelInner {
    context: Arc<Mutex<LlamaContext<'static>>>,
    mtmd: Option<Arc<Mutex<llama_cpp_2::mtmd::MtmdContext>>>,
    model: Arc<LlamaModel>,
    #[allow(dead_code)]
    manifest: ModelManifest,
    resident_bytes: u64,
    n_ctx: u32,
}

/// Real GGUF inference engine backed by llama.cpp.
pub struct LlamaCppEngine {
    models_dir: Option<PathBuf>,
    loaded: Mutex<HashMap<String, Arc<LoadedModelInner>>>,
    cancelled: Mutex<HashMap<String, Arc<AtomicBool>>>,
    next_handle: AtomicU64,
}

impl LlamaCppEngine {
    /// Creates a new `LlamaCppEngine`.
    ///
    /// If `models_dir` is `Some`, relative model paths are resolved against
    /// `<models_dir>/<hf_repo>/<hf_revision>/<file_path>`, matching the directory
    /// structure used by `pocket-models::plan_download`.
    pub fn new(models_dir: Option<PathBuf>) -> Self {
        Self {
            models_dir,
            loaded: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
        }
    }

    /// Resolves the on-disk path for a model's weights file.
    fn resolve_model_path(
        &self,
        manifest: &ModelManifest,
        file_path: &str,
    ) -> Result<PathBuf, PocketError> {
        let p = Path::new(file_path);

        // 1. Absolute path on disk
        if p.is_absolute() && p.exists() {
            return Ok(p.to_path_buf());
        }

        // 2. Models base directory (passed in or from POCKET_MODELS_DIR)
        let base_dir = self
            .models_dir
            .clone()
            .or_else(|| std::env::var("POCKET_MODELS_DIR").ok().map(PathBuf::from));

        if let Some(ref dir) = base_dir {
            let candidate = dir
                .join(&manifest.hf_repo)
                .join(&manifest.hf_revision)
                .join(file_path);
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        // 3. Direct relative path from cwd
        if p.exists() {
            return Ok(p.to_path_buf());
        }

        // 4. Relative <hf_repo>/<hf_revision>/<file_path>
        let repo_candidate = Path::new(&manifest.hf_repo)
            .join(&manifest.hf_revision)
            .join(file_path);
        if repo_candidate.exists() {
            return Ok(repo_candidate);
        }

        if let Some(ref dir) = base_dir {
            let expected = dir
                .join(&manifest.hf_repo)
                .join(&manifest.hf_revision)
                .join(file_path);
            return Err(PocketError::NotFound {
                what: format!("model weights not found at {}", expected.display()),
            });
        }

        Err(PocketError::NotFound {
            what: format!("model weights not found for {file_path}"),
        })
    }
}

impl Default for LlamaCppEngine {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait::async_trait]
impl Engine for LlamaCppEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::LlamaCpp
    }

    fn supports(&self, model: &ModelManifest) -> bool {
        model.format == ModelFormat::Gguf
    }

    async fn load(
        &self,
        model: ModelManifest,
        opts: LoadOptions,
    ) -> Result<LoadedModel, PocketError> {
        let gguf_file = model
            .files
            .iter()
            .find(|f| f.path.ends_with(".gguf") && !f.path.to_lowercase().contains("mmproj"))
            .or_else(|| model.files.iter().find(|f| f.path.ends_with(".gguf")))
            .or_else(|| model.files.first())
            .ok_or_else(|| PocketError::InvalidInput {
                message: format!("no weights file specified in manifest for {}", model.id),
            })?;

        let path = self.resolve_model_path(&model, &gguf_file.path)?;
        let file_size = std::fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or(gguf_file.size);

        let backend = backend::get_backend()?;

        // Configure model parameters: mmap enabled, offload all layers to Metal GPU
        let model_params = LlamaModelParams::default()
            .with_use_mmap(true)
            .with_n_gpu_layers(999);

        let llama_model =
            LlamaModel::load_from_file(backend, &path, &model_params).map_err(|e| {
                PocketError::EngineFailed {
                    engine: EngineKind::LlamaCpp,
                    message: format!("failed to load GGUF model from {}: {e}", path.display()),
                }
            })?;

        // Multimodal projector (mmproj) loading:
        // Heuristic: Since `ModelFile` has no `role` field in the catalog schema, locate the
        // mmproj projector file by checking if `path` contains "mmproj" (case-insensitive).
        let mmproj_file = multimodal::find_mmproj_file(&model.files);
        let mut resident_bytes = llama_model.size().max(file_size);

        let mtmd = if let Some(mf) = mmproj_file {
            let mmproj_path = self.resolve_model_path(&model, &mf.path).map_err(|e| {
                PocketError::EngineFailed {
                    engine: EngineKind::LlamaCpp,
                    message: format!("failed to resolve mmproj path {}: {e}", mf.path),
                }
            })?;

            let mmproj_path_str =
                mmproj_path
                    .to_str()
                    .ok_or_else(|| PocketError::EngineFailed {
                        engine: EngineKind::LlamaCpp,
                        message: format!("mmproj path {:?} is not valid UTF-8", mmproj_path),
                    })?;

            let mtmd_params = llama_cpp_2::mtmd::MtmdContextParams::default();
            let mtmd_ctx = llama_cpp_2::mtmd::MtmdContext::init_from_file(
                mmproj_path_str,
                &llama_model,
                &mtmd_params,
            )
            .map_err(|e| PocketError::EngineFailed {
                engine: EngineKind::LlamaCpp,
                message: format!(
                    "failed to load mmproj projector from {}: {e}",
                    mmproj_path.display()
                ),
            })?;

            let mmproj_size = std::fs::metadata(&mmproj_path)
                .map(|m| m.len())
                .unwrap_or(mf.size);
            resident_bytes += mmproj_size;

            Some(Arc::new(Mutex::new(mtmd_ctx)))
        } else {
            None
        };

        // Determine context length: min(model.context_length, policy_ctx).
        // The platform-specific default (iPhone 4096 / Mac 8192) is applied by the caller
        // via `opts.context_length`; when unset, default to 8192.
        let policy_ctx = opts.context_length.unwrap_or(8192);
        let n_ctx = if model.context_length > 0 {
            model.context_length.min(policy_ctx)
        } else {
            policy_ctx
        };

        let ctx_params = LlamaContextParams::default().with_n_ctx(std::num::NonZeroU32::new(n_ctx));

        // Enable Flash Attention when Metal GPU acceleration is active on Apple platforms.
        // In llama.cpp, llama_flash_attn_type values are: -1 = auto, 0 = disabled, 1 = enabled.
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let ctx_params = ctx_params.with_flash_attention_policy(1);

        let context = llama_model.new_context(backend, ctx_params).map_err(|e| {
            PocketError::EngineFailed {
                engine: EngineKind::LlamaCpp,
                message: format!("failed to create llama context: {e}"),
            }
        })?;

        // SAFETY: `LlamaModel` is held inside the same `Arc<LoadedModelInner>` as the context.
        // It is guaranteed to outlive the context, and context drop occurs before model drop.
        let ctx_static: LlamaContext<'static> = unsafe { std::mem::transmute(context) };

        let handle = format!("llama-{}", self.next_handle.fetch_add(1, Ordering::SeqCst));
        let inner = LoadedModelInner {
            context: Arc::new(Mutex::new(ctx_static)),
            mtmd,
            model: Arc::new(llama_model),
            manifest: model,
            resident_bytes,
            n_ctx,
        };

        self.loaded
            .lock()
            .unwrap()
            .insert(handle.clone(), Arc::new(inner));

        Ok(LoadedModel { handle })
    }

    async fn unload(&self, handle: String) -> Result<(), PocketError> {
        self.loaded.lock().unwrap().remove(&handle);
        Ok(())
    }

    async fn generate(
        &self,
        handle: String,
        req: GenerateRequest,
        sink: Arc<dyn TokenSink>,
    ) -> Result<GenerateStats, PocketError> {
        let loaded = self
            .loaded
            .lock()
            .unwrap()
            .get(&handle)
            .cloned()
            .ok_or_else(|| PocketError::EngineFailed {
                engine: EngineKind::LlamaCpp,
                message: format!("no model loaded for handle '{handle}'"),
            })?;

        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.cancelled
            .lock()
            .unwrap()
            .insert(req.request_id.clone(), cancel_flag.clone());

        let is_multimodal = multimodal::has_image_parts(&req.messages);

        let prompt_input = if is_multimodal {
            let mtmd_arc = match &loaded.mtmd {
                Some(m) => m.clone(),
                None => {
                    self.cancelled.lock().unwrap().remove(&req.request_id);
                    let err = PocketError::EngineFailed {
                        engine: EngineKind::LlamaCpp,
                        message: format!(
                            "request contains image parts but model '{}' has no mmproj projector loaded",
                            loaded.manifest.id
                        ),
                    };
                    sink.on_error(err.clone());
                    return Err(err);
                }
            };

            let image_paths = multimodal::extract_image_paths(&req.messages);

            let bitmaps = {
                let mtmd_guard = match mtmd_arc.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                match multimodal::load_image_bitmaps(&mtmd_guard, &image_paths) {
                    Ok(b) => b,
                    Err(err) => {
                        self.cancelled.lock().unwrap().remove(&req.request_id);
                        sink.on_error(err.clone());
                        return Err(err);
                    }
                }
            };

            let prompt = match template::apply_model_template(&loaded.model, &req.messages) {
                Ok(p) => p,
                Err(err) => {
                    self.cancelled.lock().unwrap().remove(&req.request_id);
                    sink.on_error(err.clone());
                    return Err(err);
                }
            };

            let chunks = {
                let mtmd_guard = match mtmd_arc.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                match multimodal::tokenize_multimodal_prompt(&mtmd_guard, &prompt, &bitmaps) {
                    Ok(c) => c,
                    Err(err) => {
                        self.cancelled.lock().unwrap().remove(&req.request_id);
                        sink.on_error(err.clone());
                        return Err(err);
                    }
                }
            };

            let total_prompt = chunks.total_tokens();
            if total_prompt >= loaded.n_ctx as usize {
                self.cancelled.lock().unwrap().remove(&req.request_id);
                let err = PocketError::InvalidInput {
                    message: format!(
                        "prompt length ({} tokens) exceeds context size ({})",
                        total_prompt, loaded.n_ctx
                    ),
                };
                sink.on_error(err.clone());
                return Err(err);
            }

            decode::PromptInput::Multimodal { chunks, mtmd_arc }
        } else {
            // Text-only request keeps existing ENG-05 path completely unchanged
            let prompt = template::apply_model_template(&loaded.model, &req.messages)?;
            let prompt_tokens = loaded.model.vocab().tokenize(prompt.as_bytes(), true, true);

            if prompt_tokens.len() >= loaded.n_ctx as usize {
                self.cancelled.lock().unwrap().remove(&req.request_id);
                let err = PocketError::InvalidInput {
                    message: format!(
                        "prompt length ({} tokens) exceeds context size ({})",
                        prompt_tokens.len(),
                        loaded.n_ctx
                    ),
                };
                sink.on_error(err.clone());
                return Err(err);
            }

            decode::PromptInput::TextTokens(prompt_tokens)
        };

        let (tx, mut rx) = tokio::sync::mpsc::channel::<decode::DecodeMessage>(32);
        let context_arc = loaded.context.clone();
        let model_arc = loaded.model.clone();
        let n_ctx = loaded.n_ctx;
        let req_clone = req.clone();
        let cancel_flag_clone = cancel_flag.clone();

        // Run decode loop on a dedicated OS thread (not tokio async task)
        std::thread::spawn(move || {
            decode::run_decode_thread(
                context_arc,
                model_arc,
                prompt_input,
                req_clone,
                cancel_flag_clone,
                n_ctx,
                tx,
            );
        });

        let mut final_stats = None;
        let mut final_error = None;

        while let Some(msg) = rx.recv().await {
            match msg {
                decode::DecodeMessage::Delta(delta) => {
                    sink.on_delta(delta);
                }
                decode::DecodeMessage::Done(stats) => {
                    sink.on_done(stats.clone());
                    final_stats = Some(stats);
                    break;
                }
                decode::DecodeMessage::Error(err) => {
                    sink.on_error(err.clone());
                    final_error = Some(err);
                    break;
                }
            }
        }

        self.cancelled.lock().unwrap().remove(&req.request_id);

        if let Some(stats) = final_stats {
            Ok(stats)
        } else if let Some(err) = final_error {
            Err(err)
        } else {
            let err = PocketError::EngineFailed {
                engine: EngineKind::LlamaCpp,
                message: "decode thread terminated unexpectedly without returning stats".into(),
            };
            sink.on_error(err.clone());
            Err(err)
        }
    }

    async fn embed(
        &self,
        _handle: String,
        _inputs: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, PocketError> {
        Err(PocketError::Internal {
            message: "LlamaCppEngine::embed: not supported, use the dedicated embedding engine"
                .into(),
        })
    }

    fn cancel(&self, request_id: String) {
        if let Some(flag) = self.cancelled.lock().unwrap().get(&request_id) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn resident_bytes(&self, handle: String) -> u64 {
        self.loaded
            .lock()
            .unwrap()
            .get(&handle)
            .map(|l| l.resident_bytes)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pocket_types::{LicenseInfo, Modality, ModelId};

    fn dummy_manifest(format: ModelFormat) -> ModelManifest {
        ModelManifest {
            id: ModelId::parse("test/model@rev").unwrap(),
            display_name: "Test Model".into(),
            family: "Test".into(),
            params_b: 0.1,
            quant: "Q8_0".into(),
            format,
            modalities: vec![Modality::Text],
            context_length: 2048,
            files: vec![],
            license: LicenseInfo {
                spdx: "MIT".into(),
                commercial_ok: true,
                url: String::new(),
            },
            hf_repo: "test/model".into(),
            hf_revision: "rev".into(),
            min_ram_bytes: 0,
            est_runtime_bytes: 0,
            tags: vec![],
            curated: true,
        }
    }

    #[test]
    fn engine_reports_llamacpp_kind() {
        let engine = LlamaCppEngine::default();
        assert_eq!(engine.kind(), EngineKind::LlamaCpp);
    }

    #[test]
    fn supports_only_gguf_format() {
        let engine = LlamaCppEngine::default();
        assert!(engine.supports(&dummy_manifest(ModelFormat::Gguf)));
        assert!(!engine.supports(&dummy_manifest(ModelFormat::MlxSafetensors)));
        assert!(!engine.supports(&dummy_manifest(ModelFormat::CoreMlPackage)));
        assert!(!engine.supports(&dummy_manifest(ModelFormat::WhisperKit)));
    }

    #[tokio::test]
    async fn embed_returns_unsupported_error() {
        let engine = LlamaCppEngine::default();
        let result = engine.embed("any-handle".into(), vec!["test".into()]).await;
        match result {
            Err(PocketError::Internal { message }) => {
                assert!(message.contains("not supported, use the dedicated embedding engine"));
            }
            other => panic!("expected PocketError::Internal, got {other:?}"),
        }
    }

    #[test]
    fn cancellation_flag_bookkeeping() {
        let engine = LlamaCppEngine::default();
        let flag = Arc::new(AtomicBool::new(false));
        engine
            .cancelled
            .lock()
            .unwrap()
            .insert("req-1".into(), flag.clone());

        assert!(!flag.load(Ordering::SeqCst));
        engine.cancel("req-1".into());
        assert!(flag.load(Ordering::SeqCst));

        // Cancelling unknown request does not panic
        engine.cancel("req-unknown".into());
    }

    #[test]
    fn resident_bytes_returns_zero_for_unknown_handle() {
        let engine = LlamaCppEngine::default();
        assert_eq!(engine.resident_bytes("non-existent".into()), 0);
    }

    // =========================================================================
    // Integration test with real GGUF fixture
    // =========================================================================
    //
    // To run this test with a real tiny model:
    // 1. Download a tiny GGUF (e.g. SmolLM2-135M-Instruct Q8_0, ~145MB):
    //    curl -L -o /tmp/smollm2-135m-q8_0.gguf \
    //      https://huggingface.co/HuggingFaceTB/SmolLM2-135M-Instruct-GGUF/resolve/main/smollm2-135m-instruct-q8_0.gguf
    // 2. Run:
    //    POCKET_TEST_GGUF_PATH=/tmp/smollm2-135m-q8_0.gguf cargo test -p pocket-engine --features slow-tests -- --nocapture
    //
    // If POCKET_TEST_GGUF_PATH is unset, the test skips cleanly.
    #[cfg(feature = "slow-tests")]
    #[tokio::test]
    async fn real_gguf_inference_and_cancellation() {
        use pocket_types::{ChatMessage, ContentPart};

        let gguf_path = match std::env::var("POCKET_TEST_GGUF_PATH") {
            Ok(p) if Path::new(&p).exists() => p,
            _ => {
                eprintln!(
                    "SKIPPING real_gguf_inference_and_cancellation: POCKET_TEST_GGUF_PATH unset or file does not exist. \
                     Download a small instruct GGUF (e.g. SmolLM2-135M Q8_0) and set POCKET_TEST_GGUF_PATH to run."
                );
                return;
            }
        };

        struct CollectingSink {
            deltas: Mutex<Vec<String>>,
            done: Mutex<Option<GenerateStats>>,
        }

        impl TokenSink for CollectingSink {
            fn on_delta(&self, delta: pocket_types::GenerateDelta) {
                self.deltas.lock().unwrap().push(delta.text);
            }
            fn on_done(&self, stats: GenerateStats) {
                *self.done.lock().unwrap() = Some(stats);
            }
            fn on_error(&self, err: PocketError) {
                panic!("unexpected on_error: {err:?}");
            }
        }

        let engine = Arc::new(LlamaCppEngine::default());

        let mut manifest = dummy_manifest(ModelFormat::Gguf);
        manifest.files = vec![pocket_types::ModelFile {
            path: gguf_path.clone(),
            size: std::fs::metadata(&gguf_path).unwrap().len(),
            sha256: String::new(),
            url: String::new(),
        }];

        // 1. Load model
        let loaded = engine
            .load(manifest, LoadOptions::default())
            .await
            .expect("failed loading real GGUF");

        let handle = loaded.handle;
        assert!(engine.resident_bytes(handle.clone()) > 0);

        // 2. Generate test
        let sink = Arc::new(CollectingSink {
            deltas: Mutex::new(Vec::new()),
            done: Mutex::new(None),
        });

        let req = GenerateRequest {
            request_id: "req-real-1".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text {
                    text: "What is 1+1?".into(),
                }],
            }],
            tools: vec![],
            params: pocket_types::SamplingParams {
                max_tokens: 32,
                temperature: 0.1,
                ..Default::default()
            },
            reasoning: pocket_types::ReasoningMode::Off,
        };

        let stats = engine
            .generate(handle.clone(), req, sink.clone())
            .await
            .expect("generation failed");

        assert!(stats.completion_tokens > 0);
        assert!(stats.ttft_ms > 0 || stats.completion_tokens > 0);
        let full_text: String = sink.deltas.lock().unwrap().join("");
        assert!(!full_text.is_empty(), "expected non-empty output");

        // 3. Cancellation test: launch large generation and cancel within 100ms
        let cancel_sink = Arc::new(CollectingSink {
            deltas: Mutex::new(Vec::new()),
            done: Mutex::new(None),
        });

        let cancel_req = GenerateRequest {
            request_id: "req-cancel-1".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text {
                    text: "Write a 500-word essay about the history of mathematics.".into(),
                }],
            }],
            tools: vec![],
            params: pocket_types::SamplingParams {
                max_tokens: 500,
                temperature: 0.7,
                ..Default::default()
            },
            reasoning: pocket_types::ReasoningMode::Off,
        };

        let engine_clone = engine.clone();
        let handle_clone = handle.clone();
        let cancel_task = tokio::spawn(async move {
            engine_clone
                .generate(handle_clone, cancel_req, cancel_sink.clone())
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.cancel("req-cancel-1".into());

        let cancel_stats = cancel_task
            .await
            .unwrap()
            .expect("cancellation generate failed");
        assert_eq!(cancel_stats.finish, pocket_types::FinishReason::Cancelled);

        // 4. Unload
        engine.unload(handle).await.expect("unload failed");
    }

    #[tokio::test]
    async fn load_with_missing_files_returns_error() {
        let engine = LlamaCppEngine::default();
        let mut manifest = dummy_manifest(ModelFormat::Gguf);
        manifest.files = vec![
            pocket_types::ModelFile {
                path: "nonexistent_model.gguf".into(),
                size: 100,
                sha256: "".into(),
                url: "".into(),
            },
            pocket_types::ModelFile {
                path: "nonexistent_mmproj.gguf".into(),
                size: 50,
                sha256: "".into(),
                url: "".into(),
            },
        ];

        let res = engine.load(manifest, LoadOptions::default()).await;
        assert!(res.is_err());
    }

    // =========================================================================
    // Integration test with real VLM GGUF fixture
    // =========================================================================
    //
    // To run this test with a real VLM model (e.g. SmolVLM-500M or MobileVLM):
    // 1. Download model weights and mmproj:
    //    curl -L -o /tmp/model.gguf <url-to-vlm-gguf>
    //    curl -L -o /tmp/mmproj.gguf <url-to-mmproj-gguf>
    // 2. Run:
    //    POCKET_TEST_VLM_GGUF_PATH=/tmp/model.gguf \
    //    POCKET_TEST_VLM_MMPROJ_PATH=/tmp/mmproj.gguf \
    //    cargo test -p pocket-engine --features slow-tests -- --nocapture
    //
    // If either POCKET_TEST_VLM_GGUF_PATH or POCKET_TEST_VLM_MMPROJ_PATH is unset,
    // the test skips cleanly.
    #[cfg(feature = "slow-tests")]
    #[tokio::test]
    async fn real_vlm_gguf_inference() {
        use pocket_types::{ChatMessage, ContentPart};

        let (gguf_path, mmproj_path) = match (
            std::env::var("POCKET_TEST_VLM_GGUF_PATH"),
            std::env::var("POCKET_TEST_VLM_MMPROJ_PATH"),
        ) {
            (Ok(g), Ok(m)) if Path::new(&g).exists() && Path::new(&m).exists() => (g, m),
            _ => {
                eprintln!(
                    "SKIPPING real_vlm_gguf_inference: POCKET_TEST_VLM_GGUF_PATH and/or \
                     POCKET_TEST_VLM_MMPROJ_PATH unset or file does not exist. \
                     Set both variables to point to a valid VLM GGUF and mmproj GGUF."
                );
                return;
            }
        };

        // Create a minimal 2x2 24-bit uncompressed BMP test image
        let temp_dir = std::env::temp_dir();
        let img_path = temp_dir.join("pocket_test_fixture.bmp");
        #[rustfmt::skip]
        let bmp_bytes: [u8; 70] = [
            // Bitmap file header (14 bytes)
            b'B', b'M',
            70, 0, 0, 0, // file size (70 bytes)
            0, 0, 0, 0,  // reserved
            54, 0, 0, 0, // offset to pixel data
            // DIB header BITMAPINFOHEADER (40 bytes)
            40, 0, 0, 0, // header size
            2, 0, 0, 0,  // width = 2
            2, 0, 0, 0,  // height = 2
            1, 0,        // planes = 1
            24, 0,       // bpp = 24
            0, 0, 0, 0,  // compression = BI_RGB
            16, 0, 0, 0, // image size (16 bytes = 2 rows * 8 bytes)
            0, 0, 0, 0,  // x pixels/meter
            0, 0, 0, 0,  // y pixels/meter
            0, 0, 0, 0,  // total colors
            0, 0, 0, 0,  // important colors
            // Pixel data: bottom row (2 pixels RGB = BGR in BMP: 6 bytes + 2 pad = 8 bytes)
            0, 0, 255,   0, 255, 0,   0, 0,
            // Top row
            255, 0, 0,   255, 255, 255, 0, 0,
        ];
        std::fs::write(&img_path, bmp_bytes).expect("failed writing test fixture image");

        struct CollectingSink {
            deltas: Mutex<Vec<String>>,
            done: Mutex<Option<GenerateStats>>,
        }

        impl TokenSink for CollectingSink {
            fn on_delta(&self, delta: pocket_types::GenerateDelta) {
                self.deltas.lock().unwrap().push(delta.text);
            }
            fn on_done(&self, stats: GenerateStats) {
                *self.done.lock().unwrap() = Some(stats);
            }
            fn on_error(&self, err: PocketError) {
                panic!("unexpected on_error: {err:?}");
            }
        }

        let engine = Arc::new(LlamaCppEngine::default());

        let mut manifest = dummy_manifest(ModelFormat::Gguf);
        manifest.files = vec![
            pocket_types::ModelFile {
                path: gguf_path.clone(),
                size: std::fs::metadata(&gguf_path).unwrap().len(),
                sha256: String::new(),
                url: String::new(),
            },
            pocket_types::ModelFile {
                path: mmproj_path.clone(),
                size: std::fs::metadata(&mmproj_path).unwrap().len(),
                sha256: String::new(),
                url: String::new(),
            },
        ];

        // 1. Load VLM model + mmproj projector
        let loaded = engine
            .load(manifest, LoadOptions::default())
            .await
            .expect("failed loading VLM GGUF and mmproj");

        let handle = loaded.handle;
        assert!(engine.resident_bytes(handle.clone()) > 0);

        // 2. Multimodal generation test: user message with ImageRef + Text
        let sink = Arc::new(CollectingSink {
            deltas: Mutex::new(Vec::new()),
            done: Mutex::new(None),
        });

        let req = GenerateRequest {
            request_id: "req-vlm-1".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                parts: vec![
                    ContentPart::ImageRef {
                        path: img_path.to_str().unwrap().into(),
                    },
                    ContentPart::Text {
                        text: "Describe this image in detail.".into(),
                    },
                ],
            }],
            tools: vec![],
            params: pocket_types::SamplingParams {
                max_tokens: 32,
                temperature: 0.1,
                ..Default::default()
            },
            reasoning: pocket_types::ReasoningMode::Off,
        };

        let stats = engine
            .generate(handle.clone(), req, sink.clone())
            .await
            .expect("VLM generation failed");

        assert!(stats.completion_tokens > 0);
        let full_text = sink.deltas.lock().unwrap().join("");
        assert!(
            !full_text.is_empty(),
            "expected non-empty VLM description output"
        );

        // 3. Unload
        engine.unload(handle).await.expect("unload failed");
        let _ = std::fs::remove_file(img_path);
    }
}
