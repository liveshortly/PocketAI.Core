use std::sync::{Arc, Mutex};

use crate::convert::*;
use crate::error::PocketError;
use crate::traits::*;
use crate::types::*;
use pocket_engine::{
    bm25_rank, chunk_document, cosine_rank, embedding_from_bytes, embedding_to_bytes,
    reciprocal_rank_fusion, ChunkRecord, ChunkerConfig, PageText,
};
use pocket_store::repos::chunks::{ChunkRow, ChunksRepo};
use pocket_store::repos::conversations::ConversationRow as StoreConversationRow;
use pocket_store::repos::devices::DevicesRepo;
use pocket_store::repos::documents::{DocumentRow, DocumentsRepo};
use pocket_store::repos::projects::{ProjectRow, ProjectsRepo};
use pocket_store::services::chat::{ChatService, ConversationQuery as StoreConversationQuery};
use pocket_store::Store;

/// Multi-thread, not current-thread: `server_start` spawns the axum server
/// as a background task and returns immediately. A current-thread runtime
/// only polls spawned tasks while something is actively inside `block_on`
/// on that same runtime - once `server_start`'s own `block_on` call
/// returned, the spawned server task would never be polled again and the
/// "running" server would silently stop accepting connections. A
/// multi-thread runtime has worker threads that keep driving it regardless.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build pocket-ffi server runtime")
    })
}

struct Inner {
    offline_mode: bool,
    license_entitlement: Option<Entitlements>,
    store_entitlement: Option<StoreEntitlement>,
    llama_engine: Arc<dyn Engine>,
    registered_engine: Option<Arc<dyn Engine>>,
    registered_stt: Option<Arc<dyn SpeechToTextEngine>>,
    registered_tts: Option<Arc<dyn TextToSpeechEngine>>,
    registered_image: Option<Arc<dyn ImageEngine>>,
    egress: Vec<EgressEntry>,
    server_handle: Option<pocket_server::ServerHandle>,
    download_plans: std::collections::HashMap<String, pocket_models::DownloadPlan>,
    tls_identity: Option<pocket_trust::TlsIdentity>,
    download_coordinator: Option<Arc<dyn DownloadCoordinator>>,
    /// model_id -> live engine handles loaded for it. `load_or_reuse` keeps
    /// one resident handle per (model, engine) and reuses it across turns, so
    /// weights load once instead of once per message; `unload` /
    /// `on_memory_warning` drain and free them (D35).
    loaded_handles: std::collections::HashMap<String, Vec<(Arc<dyn Engine>, String)>>,
    /// conversation_id -> (in-flight request_id, the engine running it), so
    /// Stop can reach `cancel(request_id)` on the engine that actually owns
    /// the request (F11) — the built-in llama engine as well as a registered
    /// one — not just the Swift stream consumer.
    active_requests: std::collections::HashMap<String, (String, Arc<dyn Engine>)>,
}

impl Inner {
    fn new(llama_engine: Arc<dyn Engine>) -> Self {
        Self {
            offline_mode: false,
            license_entitlement: None,
            store_entitlement: None,
            llama_engine,
            registered_engine: None,
            registered_stt: None,
            registered_tts: None,
            registered_image: None,
            egress: Vec::new(),
            server_handle: None,
            download_plans: std::collections::HashMap::new(),
            tls_identity: None,
            download_coordinator: None,
            loaded_handles: std::collections::HashMap::new(),
            active_requests: std::collections::HashMap::new(),
        }
    }
}

/// The public surface for Swift (A§4.5). Most methods carry real behavior now
/// (catalog, downloads, chat/send via the built-in llama.cpp adapter or a
/// registered engine, projects/RAG, server, licensing); the few remaining
/// stubs (`subscribe`, `on_thermal_change`) say so at their definition.
#[derive(uniffi::Object)]
pub struct PocketCore {
    #[allow(dead_code)]
    config: CoreConfig,
    platform: Arc<dyn PlatformHooks>,
    store: Arc<Store>,
    http: pocket_trust::PocketHttp,
    pairing_state: Arc<crate::pairing_routes::PairingRoutesState>,
    inner: Mutex<Inner>,
    /// Lets route handlers (built inside `&self` methods like
    /// `server_start`) hold a clonable, 'static handle back into this same
    /// `PocketCore` without `open()` needing a separate two-phase init.
    self_weak: std::sync::Weak<PocketCore>,
}

#[uniffi::export]
impl PocketCore {
    #[uniffi::constructor]
    pub fn open(
        config: CoreConfig,
        platform: Arc<dyn PlatformHooks>,
    ) -> Result<Arc<Self>, PocketError> {
        std::fs::create_dir_all(&config.app_support_dir).map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?;
        let db_path = std::path::Path::new(&config.app_support_dir).join("pocket.sqlite");
        let store = Arc::new(Store::open(&db_path).map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?);
        let pairing_state = Arc::new(crate::pairing_routes::PairingRoutesState::new(
            Arc::clone(&store),
            Arc::clone(&platform),
        ));
        let models_dir = std::path::Path::new(&config.app_support_dir).join("Models");
        let llama_engine = Arc::new(crate::engine_adapter::PocketEngineAdapter::new(
            pocket_engine::llama::LlamaCppEngine::new(Some(models_dir)),
        ));
        let inner = Inner::new(llama_engine);
        Ok(Arc::new_cyclic(|self_weak| Self {
            config,
            platform,
            store,
            http: pocket_trust::PocketHttp::new(),
            pairing_state,
            inner: Mutex::new(inner),
            self_weak: self_weak.clone(),
        }))
    }

    // ---- device + fit ----

    pub fn device_profile(&self) -> DeviceProfile {
        self.platform.device_profile()
    }

    pub fn on_memory_warning(&self) {
        // Free resident weights: drain every tracked handle and unload it
        // (ENG-04). An in-flight decode holds its own Arcs to the model and
        // context, so it finishes safely; the next send simply reloads.
        let handles: Vec<(Arc<dyn Engine>, String)> = {
            let mut inner = self.inner.lock().unwrap();
            inner
                .loaded_handles
                .drain()
                .flat_map(|(_, handles)| handles)
                .collect()
        };
        for (engine, handle) in handles {
            let _ = engine.unload(handle);
        }
    }

    pub fn on_thermal_change(&self, _state: ThermalState) {
        // Real behavior: cap generation, emit thermal.changed (ENG-04).
    }

    // ---- engines ----

    pub fn register_engine(&self, engine: Arc<dyn Engine>) {
        self.inner.lock().unwrap().registered_engine = Some(engine);
    }

    pub fn register_stt(&self, e: Arc<dyn SpeechToTextEngine>) {
        self.inner.lock().unwrap().registered_stt = Some(e);
    }
    pub fn register_tts(&self, e: Arc<dyn TextToSpeechEngine>) {
        self.inner.lock().unwrap().registered_tts = Some(e);
    }
    pub fn register_image(&self, e: Arc<dyn ImageEngine>) {
        self.inner.lock().unwrap().registered_image = Some(e);
    }

    // ---- catalog + models ----

    /// Real catalog: loads the bundled `pocket-models` catalog, computes a
    /// live `FitReport` per entry against this device's profile (via the
    /// chip bandwidth table), and joins in install state/bytes-done from
    /// `pocket-store`'s `ModelsRepo`.
    pub fn catalog(&self, filter: CatalogFilter) -> Vec<CatalogEntry> {
        let profile = self.platform.device_profile();
        let chip = pocket_models::resolve_chip(&profile.model_identifier);
        let models_repo = pocket_store::ModelsRepo::new(&self.store);
        let models_platform = ffi_platform_to_models(&profile.platform);

        let catalog = pocket_models::load();
        let mut entries: Vec<CatalogEntry> = catalog
            .entries
            .into_iter()
            .filter(|m| {
                if filter.curated_only && !m.curated {
                    return false;
                }
                if let Some(modality) = &filter.modality {
                    let wanted = modality_to_str(modality);
                    if !m.modalities.iter().any(|mm| mm == wanted) {
                        return false;
                    }
                }
                true
            })
            .filter_map(|m| {
                let row = models_repo.get(&m.id).ok().flatten();
                if filter.installed_only
                    && row.as_ref().map(|r| r.state.as_str()) != Some("installed")
                {
                    return None;
                }
                let weights_bytes: u64 = m.files.iter().map(|f| f.size).sum();
                let disk_bytes = weights_bytes;
                let fit = pocket_models::fit(
                    weights_bytes,
                    disk_bytes,
                    models_platform,
                    profile.app_memory_limit_bytes,
                    chip.bandwidth_gbps,
                    None,
                    // A benchmark the user actually ran turns the fit report's
                    // estimate into `Confidence::Measured` (ENG-09).
                    row.as_ref().and_then(measured_benchmark),
                );
                let state = row
                    .as_ref()
                    .map(|r| model_state_from_str(&r.state))
                    .unwrap_or(ModelState::Available);
                let bytes_done = row.map(|r| r.bytes_done).unwrap_or(0);
                Some(CatalogEntry {
                    manifest: manifest_from_models(&m),
                    fit: fit_report_from_models(&fit),
                    state,
                    bytes_done,
                })
            })
            .collect();

        // F25: surface off-catalog models in ANY state (downloading, paused,
        // failed, installed, …), so a noncurated Hugging Face download doesn't
        // vanish from Home/Models/Storage mid-transfer.
        let bundled_ids: std::collections::HashSet<String> =
            entries.iter().map(|e| e.manifest.id.clone()).collect();
        for row in models_repo.list().unwrap_or_default() {
            if bundled_ids.contains(&row.id)
                || filter.curated_only
                || (filter.installed_only && row.state != "installed")
            {
                continue;
            }
            if let Ok(m) = serde_json::from_str::<pocket_models::ModelManifest>(&row.manifest_json) {
                if let Some(modality) = &filter.modality {
                    let wanted = modality_to_str(modality);
                    if !m.modalities.iter().any(|mm| mm == wanted) {
                        continue;
                    }
                }
                let weights_bytes: u64 = m.files.iter().map(|f| f.size).sum();
                let fit = pocket_models::fit(
                    weights_bytes,
                    weights_bytes,
                    models_platform,
                    profile.app_memory_limit_bytes,
                    chip.bandwidth_gbps,
                    None,
                    measured_benchmark(&row),
                );
                entries.push(CatalogEntry {
                    manifest: manifest_from_models(&m),
                    fit: fit_report_from_models(&fit),
                    state: model_state_from_str(&row.state),
                    bytes_done: row.bytes_done,
                });
            }
        }
        entries
    }

    /// User-initiated Hugging Face search (CORE-10), egress-recorded and
    /// refused in offline mode via `PocketHttp`. Results are uncurated with
    /// a placeholder fit report - real fit needs `config.json`, fetched
    /// lazily when the user opens a result (not implemented yet).
    pub fn search_hub(
        &self,
        query: String,
        format: ModelFormat,
    ) -> Result<Vec<CatalogEntry>, PocketError> {
        let format_str = match format {
            ModelFormat::Gguf => "gguf",
            ModelFormat::CoreMlPackage => "coreml",
            ModelFormat::WhisperKit => "whisperkit",
            ModelFormat::MlxSafetensors => "mlx",
        };
        let manifests = runtime()
            .block_on(pocket_models::search_hub(
                &self.http,
                "https://huggingface.co",
                &query,
                format_str,
            ))
            .map_err(|e| match e {
                pocket_types::PocketError::Network { message } => PocketError::Network { message },
                other => PocketError::Internal {
                    message: other.to_string(),
                },
            })?;

        Ok(manifests
            .iter()
            .map(|m| CatalogEntry {
                manifest: manifest_from_models(m),
                fit: FitReport {
                    verdict: FitVerdict::NotSupported,
                    reason: "Open this model to check whether it fits".into(),
                    est_tokens_per_sec: None,
                    measured_tokens_per_sec: None,
                    est_runtime_bytes: 0,
                    disk_bytes: 0,
                    confidence: Confidence::Estimated,
                },
                state: ModelState::Available,
                bytes_done: 0,
            })
            .collect())
    }

    pub fn plan_download(&self, id: ModelId) -> Result<DownloadPlan, PocketError> {
        let manifest = self
            .find_manifest(&id)
            .ok_or_else(|| PocketError::NotFound { what: id.clone() })?;
        let profile = self.platform.device_profile();
        let plan =
            pocket_models::plan_download(&manifest, &self.models_dir(), profile.free_disk_bytes)
                .map_err(|e| match e {
                    pocket_models::DownloadError::DiskFull {
                        needed_bytes,
                        available_bytes,
                    } => PocketError::WillNotFit {
                        needed_bytes,
                        available_bytes,
                        advice: "Free up disk space before downloading this model".into(),
                    },
                    other => PocketError::Storage {
                        message: other.to_string(),
                    },
                })?;

        pocket_store::ModelsRepo::new(&self.store)
            .upsert(&pocket_store::ModelRow {
                id: id.clone(),
                manifest_json: serde_json::to_string(&manifest).unwrap_or_default(),
                state: "downloading".into(),
                bytes_done: 0,
                installed_at: None,
                last_used_at: None,
                pinned_engine: None,
                benchmark_json: None,
            })
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;

        let ffi_plan = DownloadPlan {
            model_id: id.clone(),
            files: plan
                .files
                .iter()
                .map(|f| PlannedFile {
                    url: f.url.clone(),
                    size: f.size,
                    sha256: f.sha256.clone(),
                    dest_part: f.dest_part.to_string_lossy().to_string(),
                    dest_final: f.dest_final.to_string_lossy().to_string(),
                })
                .collect(),
            total_bytes: plan.total_bytes,
            dest_dir: plan.dest_dir.to_string_lossy().to_string(),
        };
        self.inner.lock().unwrap().download_plans.insert(id, plan);
        Ok(ffi_plan)
    }

    /// Records aggregate downloaded bytes for a model. The coordinator sums
    /// completed + active file bytes and reports the total here (F07), so this
    /// must store the value as-is — it must not be a per-file figure.
    pub fn record_download_progress(&self, id: ModelId, _file: String, bytes_done: u64) {
        if let Ok(Some(mut row)) = pocket_store::ModelsRepo::new(&self.store).get(&id) {
            row.bytes_done = bytes_done;
            let _ = pocket_store::ModelsRepo::new(&self.store).upsert(&row);
        }
    }

    /// Sets a model's download-state string for coordinator-driven transitions
    /// ("downloading", "paused", "failed:network", …). Installation still goes
    /// through `verify_and_commit`; this never marks a model installed (F06/§8).
    pub fn set_download_state(&self, id: ModelId, state: String) {
        if state == "installed" {
            return; // only verify_and_commit may install
        }
        if let Ok(Some(mut row)) = pocket_store::ModelsRepo::new(&self.store).get(&id) {
            row.state = state;
            let _ = pocket_store::ModelsRepo::new(&self.store).upsert(&row);
        }
    }

    /// Cancels the in-flight generation for a conversation (F11). Reaches the
    /// engine's `cancel(request_id)` so a synchronous generate actually stops,
    /// not just the Swift stream consumer. No-op if nothing is running.
    pub fn cancel_conversation(&self, conversation_id: Id) {
        let entry = self
            .inner
            .lock()
            .unwrap()
            .active_requests
            .get(&conversation_id)
            .cloned();
        if let Some((request_id, engine)) = entry {
            engine.cancel(request_id);
        }
    }

    pub fn verify_and_commit(&self, id: ModelId) -> Result<(), PocketError> {
        let plan = self
            .inner
            .lock()
            .unwrap()
            .download_plans
            .get(&id)
            .cloned()
            .ok_or_else(|| PocketError::NotFound { what: id.clone() })?;

        match pocket_models::verify_and_commit(&plan) {
            Ok(()) => {
                let now = self.platform.now_ms();
                if let Ok(Some(mut row)) = pocket_store::ModelsRepo::new(&self.store).get(&id) {
                    row.state = "installed".into();
                    row.installed_at = Some(now);
                    row.bytes_done = plan.total_bytes;
                    let _ = pocket_store::ModelsRepo::new(&self.store).upsert(&row);
                }
                self.inner.lock().unwrap().download_plans.remove(&id);
                Ok(())
            }
            Err(e) => {
                if let pocket_models::DownloadError::ChecksumMismatch { .. } = &e {
                    if let Ok(Some(mut row)) = pocket_store::ModelsRepo::new(&self.store).get(&id) {
                        row.state = "failed:checksum".into();
                        let _ = pocket_store::ModelsRepo::new(&self.store).upsert(&row);
                    }
                }
                Err(PocketError::Storage {
                    message: e.to_string(),
                })
            }
        }
    }

    pub fn delete_model(&self, id: ModelId) -> Result<(), PocketError> {
        if let Some(manifest) = self.find_manifest(&id) {
            let dir = self
                .models_dir()
                .join(&manifest.hf_repo)
                .join(&manifest.hf_revision);
            let _ = std::fs::remove_dir_all(dir);
        }
        pocket_store::ModelsRepo::new(&self.store)
            .delete(&id)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;
        self.inner.lock().unwrap().download_plans.remove(&id);
        Ok(())
    }

    /// If an engine is registered, proves Rust calls back into it (its
    /// `.kind()`) rather than just holding the `Arc`.
    pub fn loaded_models(&self) -> Vec<LoadedModelInfo> {
        let inner = self.inner.lock().unwrap();
        match &inner.registered_engine {
            Some(engine) => vec![LoadedModelInfo {
                handle: "probe".into(),
                id: String::new(),
                engine: engine.kind(),
                resident_bytes: 0,
            }],
            None => Vec::new(),
        }
    }

    /// Runs ENG-09's fixed-prompt measurement for `id` on this device and
    /// persists the result to the model's row, then returns it. Loads or
    /// reuses the model through the very same `load_or_reuse` path a chat turn
    /// takes, so benchmarking an already-loaded model costs no second copy of
    /// the weights.
    ///
    /// Synchronous on purpose. `pocket_engine::benchmark_after_load` is async
    /// only because `pocket_engine::Engine` is; the FFI `Engine` trait here is
    /// the sync one, whose methods already `block_on` the shared runtime
    /// internally. Driving the async helper from a uniffi export would mean
    /// `block_on(benchmark_after_load(..))` whose `generate().await` lands in a
    /// sync FFI method that calls `block_on` on that same runtime - tokio
    /// panics on a nested block_on. So this path measures directly off the
    /// `GenerateStats` the sync engine already reports (`ttft_ms`,
    /// `decode_tps`), using pocket-engine's own prompt and token budget so the
    /// two paths can never drift into incomparable numbers.
    ///
    /// Refuses while a reply is generating: a benchmark run alongside real
    /// decoding measures contention, not the model (the same rule
    /// `benchmark_after_load`'s `busy` flag enforces).
    pub fn benchmark_model(&self, id: ModelId) -> Result<Benchmark, PocketError> {
        if !self.inner.lock().unwrap().active_requests.is_empty() {
            return Err(PocketError::InvalidInput {
                message: "Wait for the current reply to finish before benchmarking.".into(),
            });
        }
        let manifest = self
            .find_manifest(&id)
            .ok_or_else(|| PocketError::NotFound { what: id.clone() })?;
        let ffi_manifest = manifest_from_models(&manifest);
        let engine = self.pick_engine(&ffi_manifest)?;
        let opts = LoadOptions {
            context_length: self
                .is_builtin_llama(&engine)
                .then(|| self.effective_context_length(manifest.context_length)),
            pinned_engine: None,
        };
        let handle = self.load_or_reuse(&id, &engine, ffi_manifest, opts)?;

        let req = GenerateRequest {
            request_id: format!("benchmark-{id}"),
            messages: vec![ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text {
                    text: pocket_engine::BENCHMARK_PROMPT.into(),
                }],
            }],
            tools: vec![],
            params: SamplingParams {
                temperature: 0.0,
                top_p: 1.0,
                top_k: 1,
                min_p: 0.0,
                max_tokens: pocket_engine::BENCHMARK_MAX_TOKENS,
                stop: vec![],
                seed: Some(0),
                repetition_penalty: 1.0,
            },
            reasoning: ReasoningMode::Off,
        };

        let sink = Arc::new(AccumulatingSink::new(Arc::new(NullChatSink)));
        let started = std::time::Instant::now();
        let stats = engine.generate(handle.clone(), req, sink as Arc<dyn TokenSink>)?;
        let elapsed = started.elapsed().as_secs_f32().max(0.001);

        // Honest instruments: a run that produced nothing has nothing to
        // report, so say so rather than persisting a 0 tok/s "measurement".
        if stats.completion_tokens == 0 {
            return Err(PocketError::EngineFailed {
                engine: engine.kind(),
                message: "the model produced no tokens to measure".into(),
            });
        }
        let benchmark = Benchmark {
            // The engine's own decode rate when it reports one; wall clock
            // otherwise (a registered Swift engine may leave it at 0).
            tps: if stats.decode_tps > 0.0 {
                stats.decode_tps
            } else {
                stats.completion_tokens as f32 / elapsed
            },
            ttft_ms: stats.ttft_ms,
            peak_bytes: engine.resident_bytes(handle),
        };
        self.persist_benchmark(&id, &benchmark);
        Ok(benchmark)
    }

    /// The last benchmark persisted for `id`, or `None` if it was never run.
    /// A single row read, no inference - so the UI can show a previous result
    /// without making the user sit through another run.
    pub fn stored_benchmark(&self, id: ModelId) -> Option<Benchmark> {
        let row = pocket_store::ModelsRepo::new(&self.store)
            .get(&id)
            .ok()
            .flatten()?;
        benchmark_from_row(&row).map(|b| Benchmark {
            tps: b.tps,
            ttft_ms: b.ttft_ms,
            peak_bytes: b.peak_bytes,
        })
    }

    /// Unloads every engine handle loaded for `id`, freeing its resident
    /// weights (D35). Idempotent: unloading a model that isn't resident is a
    /// no-op success - there is simply nothing to free.
    pub fn unload(&self, id: ModelId) -> Result<(), PocketError> {
        // Drain under the lock, then unload without holding it (unload blocks
        // on the engine runtime).
        let handles = self
            .inner
            .lock()
            .unwrap()
            .loaded_handles
            .remove(&id)
            .unwrap_or_default();
        for (engine, handle) in handles {
            engine.unload(handle)?;
        }
        Ok(())
    }

    // ---- chat ----

    pub fn conversations(&self, q: ConversationQuery) -> Vec<ConversationSummary> {
        let service = ChatService::new(&self.store);
        let rows = service
            .list_conversations(&StoreConversationQuery {
                search: q.search,
                limit: q.limit as usize,
                offset: q.offset as usize,
            })
            .unwrap_or_default();
        rows.into_iter()
            .filter(|r| q.archived.is_none_or(|wanted| wanted == r.archived))
            .map(|r| ConversationSummary {
                id: r.id,
                title: r.title,
                updated_at: r.updated_at,
                archived: r.archived,
            })
            .collect()
    }

    pub fn create_conversation(&self, input: NewConversation) -> Result<Conversation, PocketError> {
        let now = self.platform.now_ms();
        let id = new_ulid();
        let row = StoreConversationRow {
            id: id.clone(),
            title: input.title.clone(),
            project_id: input.project_id.clone(),
            model_id: input.model_id.clone(),
            system_prompt: None,
            params_json: None,
            created_at: now,
            updated_at: now,
            archived: false,
        };
        ChatService::new(&self.store)
            .create_conversation(&row)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;
        Ok(Conversation {
            id,
            title: input.title,
            project_id: input.project_id,
            model_id: input.model_id,
            system_prompt: None,
            created_at: now,
            updated_at: now,
            archived: false,
        })
    }

    pub fn messages(&self, conversation_id: Id) -> Vec<Message> {
        let service = ChatService::new(&self.store);
        service
            .messages_for(&conversation_id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| Message {
                id: m.id,
                conversation_id: m.conversation_id,
                role: m.role,
                // Reconstruct full parts (text + images), backward-compatible
                // with old text-only rows (F13).
                parts: extract_parts(&m.content_json),
                reasoning: m.reasoning,
                stats: None,
                model_id: m.model_id,
                created_at: m.created_at,
                status: match m.status.as_str() {
                    "streaming" => MessageStatus::Streaming,
                    "error" => MessageStatus::Error,
                    "cancelled" => MessageStatus::Cancelled,
                    _ => MessageStatus::Done,
                },
            })
            .collect()
    }

    /// ENG-08 chat turn. Routes to a registered `Engine` when it supports the
    /// model, otherwise to the built-in llama.cpp adapter for GGUF models.
    ///
    /// Persists the user message first, resolves the conversation's model
    /// (or a runnable fallback), loads/reuses it, truncates history to fit the
    /// effective context, streams the reply through `sink`, then persists and
    /// returns the assistant message (titling the first exchange).
    pub fn send(
        &self,
        conversation_id: Id,
        input: UserInput,
        sink: Arc<dyn ChatSink>,
    ) -> Result<Message, PocketError> {
        self.send_with_reasoning(conversation_id, input, ReasoningMode::Off, sink)
    }

    /// UI-27: `send` with an explicit reasoning mode. `send` keeps the
    /// historical `ReasoningMode::Off` behavior for existing callers.
    pub fn send_with_reasoning(
        &self,
        conversation_id: Id,
        input: UserInput,
        reasoning: ReasoningMode,
        sink: Arc<dyn ChatSink>,
    ) -> Result<Message, PocketError> {
        let chat = ChatService::new(&self.store);
        let now = self.platform.now_ms();
        let user_message_id = new_ulid();
        chat.create_message(&pocket_store::MessageRow {
            id: user_message_id,
            conversation_id: conversation_id.clone(),
            role: "user".into(),
            // Persist the full parts (text + image refs), not just flattened text (F13).
            content_json: parts_to_content_json(&input.parts),
            reasoning: None,
            stats_json: None,
            model_id: None,
            created_at: now,
            status: "done".into(),
        })
        .map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?;

        self.generate_reply(conversation_id, reasoning, sink)
    }

    /// Shared reply-generation path for a fresh send and for a retry/regenerate
    /// (F12). Assumes the user turn is already persisted. Resolves the
    /// conversation's model by ID (F17), builds history with full parts so
    /// images reach the engine (F13), streams, persists the assistant message,
    /// and titles on the first exchange.
    fn generate_reply(
        &self,
        conversation_id: Id,
        reasoning: ReasoningMode,
        sink: Arc<dyn ChatSink>,
    ) -> Result<Message, PocketError> {
        let chat = ChatService::new(&self.store);
        // Resolve this conversation directly by ID (F17) — never page-scan the
        // first 50 conversations, which loses older chats' model/system prompt.
        let conversation = pocket_store::ConversationsRepo::new(&self.store)
            .get(&conversation_id)
            .ok()
            .flatten();
        // Use the conversation's bound model only if it is actually runnable
        // (installed locally, or a remote/Apple route); otherwise fall back to an
        // installed model instead of failing with a cryptic engine load error —
        // the common "bound to a recommended model that was never downloaded"
        // case ("no model found even though a model is present").
        let model_id = conversation
            .as_ref()
            .and_then(|c| c.model_id.clone())
            .filter(|id| self.is_model_runnable(id))
            .or_else(|| self.default_recommended_model_id())
            .ok_or_else(|| PocketError::InvalidInput {
                message: self.no_runnable_model_message(),
            })?;
        let manifest = self
            .find_manifest(&model_id)
            .ok_or_else(|| PocketError::NotFound {
                what: model_id.clone(),
            })?;
        let ffi_manifest = manifest_from_models(&manifest);
        let engine = self.pick_engine(&ffi_manifest)?;
        // One effective context drives both the engine load and history
        // truncation, so a 131k catalog ctx can't overrun an 8k/4k engine cap.
        // Only the built-in llama engine takes the explicit cap; a registered
        // engine (MLX) keeps its existing LoadOptions::default() behavior.
        let is_builtin = self.is_builtin_llama(&engine);
        let effective_ctx = if is_builtin {
            self.effective_context_length(manifest.context_length)
        } else {
            manifest.context_length
        };
        let opts = LoadOptions {
            context_length: is_builtin.then_some(effective_ctx),
            pinned_engine: None,
        };
        let handle = self.load_or_reuse(&model_id, &engine, ffi_manifest, opts)?;

        // `messages_for` already includes the user turn, so history alone is the
        // full prompt. Full parts (text + images) reach the engine (F13).
        let mut messages: Vec<ChatMessage> = chat
            .messages_for(&conversation_id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| ChatMessage {
                role: m.role,
                parts: extract_parts(&m.content_json),
            })
            .collect();
        let is_first_exchange = messages.len() <= 1;

        // A model without vision can't consume images: replaying stored
        // ImageRef parts into it fails ("no mmproj projector loaded"), so any
        // image in the history would break the conversation forever. Strip them.
        if !manifest.modalities.iter().any(|m| m == "vision") {
            for m in &mut messages {
                m.parts
                    .retain(|p| !matches!(p, ContentPart::ImageRef { .. }));
            }
        }

        if let Some(system_prompt) = conversation.as_ref().and_then(|c| c.system_prompt.clone()) {
            messages.insert(
                0,
                ChatMessage {
                    role: "system".into(),
                    parts: vec![ContentPart::Text {
                        text: system_prompt,
                    }],
                },
            );
        }

        const MAX_TOKENS: usize = 1024;
        truncate_to_fit(&mut messages, effective_ctx as usize, MAX_TOKENS);

        let request_id = new_ulid();
        // Record the in-flight request so cancel_conversation can reach the
        // owning engine mid-generation (F11), and reject a second concurrent
        // request for the same conversation — it would overwrite this marker
        // and interleave two replies.
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.active_requests.contains_key(&conversation_id) {
                return Err(PocketError::InvalidInput {
                    message: "a reply is already generating for this conversation".into(),
                });
            }
            inner.active_requests.insert(
                conversation_id.clone(),
                (request_id.clone(), Arc::clone(&engine)),
            );
        }
        let req = GenerateRequest {
            request_id,
            messages,
            tools: vec![],
            params: SamplingParams {
                temperature: 0.7,
                top_p: 0.95,
                top_k: 40,
                min_p: 0.0,
                max_tokens: MAX_TOKENS as u32,
                stop: vec![],
                seed: None,
                repetition_penalty: 1.1,
            },
            reasoning,
        };

        let adapter = Arc::new(AccumulatingSink::new(Arc::clone(&sink)));
        let gen_result = engine.generate(
            handle.clone(),
            req,
            adapter.clone() as Arc<dyn TokenSink>,
        );
        // Clear the in-flight marker on every terminal path (F11).
        self.inner
            .lock()
            .unwrap()
            .active_requests
            .remove(&conversation_id);
        let stats = gen_result?;
        let cancelled = matches!(stats.finish, FinishReason::Cancelled);

        let final_text = adapter.text();
        let final_reasoning = adapter.reasoning();
        let reasoning_col = (!final_reasoning.is_empty()).then_some(final_reasoning);
        let now = self.platform.now_ms();
        let message_id = new_ulid();
        chat.create_message(&pocket_store::MessageRow {
            id: message_id.clone(),
            conversation_id: conversation_id.clone(),
            role: "assistant".into(),
            content_json: serde_json::json!({ "text": final_text }).to_string(),
            reasoning: reasoning_col.clone(),
            stats_json: serde_json::to_string(&serde_json::json!({
                "prompt_tokens": stats.prompt_tokens,
                "completion_tokens": stats.completion_tokens,
            }))
            .ok(),
            model_id: Some(model_id),
            created_at: now,
            status: if cancelled { "cancelled" } else { "done" }.into(),
        })
        .map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?;

        // A stopped reply is partial: don't title the conversation off it.
        if is_first_exchange && !cancelled {
            self.generate_title(&conversation_id, &handle, &engine, &chat, &final_text);
        }

        let message = Message {
            id: message_id,
            conversation_id,
            role: "assistant".into(),
            parts: vec![ContentPart::Text { text: final_text }],
            reasoning: reasoning_col,
            stats: Some(stats),
            model_id: None,
            created_at: now,
            status: if cancelled {
                MessageStatus::Cancelled
            } else {
                MessageStatus::Done
            },
        };
        sink.on_message_saved(message.clone());
        Ok(message)
    }

    /// Regenerates the assistant reply for a conversation's latest turn (F12):
    /// drops the trailing assistant message (the failed/stopped attempt) and
    /// re-runs generation from the existing user history — never duplicating the
    /// user input, never sending the assistant text back as a prompt.
    pub fn regenerate(
        &self,
        conversation_id: Id,
        sink: Arc<dyn ChatSink>,
    ) -> Result<Message, PocketError> {
        let chat = ChatService::new(&self.store);
        if let Some(last) = chat
            .messages_for(&conversation_id)
            .unwrap_or_default()
            .into_iter()
            .last()
        {
            if last.role == "assistant" {
                let _ = pocket_store::MessagesRepo::new(&self.store).delete(&last.id);
            }
        }
        self.generate_reply(conversation_id, ReasoningMode::Off, sink)
    }

    /// NET-05: the OpenAI-compatible HTTP route's engine call. Same
    /// resolve-engine/load/generate steps as `send`, but parameterized by an
    /// explicit `model_id` and message list instead of a conversation (the
    /// HTTP route owns no conversation state) and generic over `sink` so the
    /// caller can choose accumulate-then-respond or stream-as-SSE.
    pub(crate) fn generate_chat(
        &self,
        model_id: &str,
        messages: Vec<ChatMessage>,
        params: SamplingParams,
        sink: Arc<dyn TokenSink>,
    ) -> Result<GenerateStats, PocketError> {
        let manifest =
            self.find_manifest(&model_id.to_string())
                .ok_or_else(|| PocketError::NotFound {
                    what: model_id.to_string(),
                })?;
        let ffi_manifest = manifest_from_models(&manifest);
        let engine = self.pick_engine(&ffi_manifest)?;
        let opts = LoadOptions {
            context_length: self
                .is_builtin_llama(&engine)
                .then(|| self.effective_context_length(manifest.context_length)),
            pinned_engine: None,
        };
        let handle = self.load_or_reuse(model_id, &engine, ffi_manifest, opts)?;
        let req = GenerateRequest {
            request_id: new_ulid(),
            messages,
            tools: vec![],
            params,
            reasoning: ReasoningMode::Off,
        };
        engine.generate(handle, req, sink)
    }

    pub fn stop(&self, _conversation_id: Id) {}

    pub fn delete_conversation(&self, id: Id) -> Result<(), PocketError> {
        pocket_store::ConversationsRepo::new(&self.store)
            .delete(&id)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    /// UI-08: swipe-to-archive on the chats list.
    pub fn archive_conversation(&self, id: Id, archived: bool) -> Result<(), PocketError> {
        pocket_store::ConversationsRepo::new(&self.store)
            .archive(&id, archived)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    pub fn rename_conversation(&self, id: Id, title: String) -> Result<(), PocketError> {
        let now = self.platform.now_ms();
        ChatService::new(&self.store)
            .set_title(&id, &title, now)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    /// Sets which model a conversation uses. Without this, selecting a model in
    /// the UI never reaches an already-created conversation, so `send_message`
    /// keeps resolving the conversation's original (possibly never-downloaded)
    /// model and fails to load it.
    pub fn set_conversation_model(&self, id: Id, model_id: String) -> Result<(), PocketError> {
        let now = self.platform.now_ms();
        pocket_store::ConversationsRepo::new(&self.store)
            .update_model(&id, &model_id, now)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    /// Resolves a single conversation by ID (F17). Callers must not page-scan
    /// `conversations()` to find one — older chats fall outside the default
    /// 50-row window and lose their model/system-prompt/title that way.
    pub fn get_conversation(&self, id: Id) -> Option<Conversation> {
        pocket_store::ConversationsRepo::new(&self.store)
            .get(&id)
            .ok()
            .flatten()
            .map(|row| Conversation {
                id: row.id,
                title: row.title,
                project_id: row.project_id,
                model_id: row.model_id,
                system_prompt: row.system_prompt,
                created_at: row.created_at,
                updated_at: row.updated_at,
                archived: row.archived,
            })
    }

    // ---- projects + RAG ----

    pub fn create_project(
        &self,
        name: String,
        instructions: Option<String>,
    ) -> Result<Project, PocketError> {
        let id = new_ulid();
        let created_at = self.platform.now_ms();
        let row = ProjectRow {
            id: id.clone(),
            name: name.clone(),
            instructions: instructions.clone(),
            created_at,
        };
        ProjectsRepo::new(&self.store)
            .create(&row)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;
        Ok(Project {
            id,
            name,
            instructions,
            created_at,
        })
    }

    pub fn list_projects(&self) -> Vec<Project> {
        ProjectsRepo::new(&self.store)
            .list_all()
            .unwrap_or_default()
            .into_iter()
            .map(|p| Project {
                id: p.id,
                name: p.name,
                instructions: p.instructions,
                created_at: p.created_at,
            })
            .collect()
    }

    pub fn rename_project(&self, id: Id, name: String) -> Result<(), PocketError> {
        ProjectsRepo::new(&self.store)
            .update_name(&id, &name)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    /// Sets the instructions prepended to every chat in this project. An
    /// empty string clears them, so the UI's "delete the text and save" does
    /// what it looks like rather than storing a blank line.
    pub fn set_project_instructions(
        &self,
        id: Id,
        instructions: Option<String>,
    ) -> Result<(), PocketError> {
        let trimmed = instructions
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        ProjectsRepo::new(&self.store)
            .update_instructions(&id, trimmed.as_deref())
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    pub fn delete_project(&self, id: Id) -> Result<(), PocketError> {
        ProjectsRepo::new(&self.store)
            .delete(&id)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    pub fn add_document(
        &self,
        project_id: Id,
        name: String,
        mime: String,
        pages: Vec<DocumentPage>,
    ) -> Result<Document, PocketError> {
        let text_chars = pages.iter().map(|p| p.text.chars().count() as u32).sum();
        let bytes = pages.iter().map(|p| p.text.len() as u64).sum();
        // no on-disk file for Project documents; text is already extracted and stored as chunks
        let path = String::new();
        let id = new_ulid();
        let created_at = self.platform.now_ms();
        let page_texts: Vec<PageText> = pages
            .into_iter()
            .map(|p| PageText {
                page: p.page,
                text: p.text,
            })
            .collect();
        let chunks = chunk_document(&page_texts, &ChunkerConfig::default());

        // D45: batch-embed all chunks in one engine call and persist the
        // vectors to SQLite. If no embedding model is installed (or embedding
        // fails), we persist empty vectors and fall back to lexical search -
        // honest instruments: never store a fabricated embedding.
        let chunk_texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embed_model = self.default_embedding_model_id();
        let embeddings: Vec<Vec<f32>> = match &embed_model {
            Some(model) if !chunk_texts.is_empty() => {
                self.embed(model, &chunk_texts).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        // Only record the embed model if every chunk actually got a vector.
        let persisted_embed_model = embed_model.filter(|_| embeddings.len() == chunk_texts.len());

        let row = DocumentRow {
            id: id.clone(),
            project_id: project_id.clone(),
            name: name.clone(),
            mime: mime.clone(),
            bytes,
            path: path.clone(),
            text_chars,
            embed_model: persisted_embed_model.clone(),
            state: "ready".to_string(),
            created_at,
        };
        DocumentsRepo::new(&self.store)
            .create(&row)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;

        let chunks_repo = ChunksRepo::new(&self.store);
        for (ord, chunk) in chunks.into_iter().enumerate() {
            let embedding = embeddings
                .get(ord)
                .map(|v| embedding_to_bytes(v))
                .unwrap_or_default();
            let chunk_row = ChunkRow {
                id: new_ulid(),
                document_id: id.clone(),
                ord: ord as u32,
                text: chunk.text,
                page: Some(chunk.page),
                embedding,
            };
            chunks_repo
                .create(&chunk_row)
                .map_err(|e| PocketError::Storage {
                    message: e.to_string(),
                })?;
        }

        Ok(Document {
            id,
            project_id,
            name,
            mime,
            bytes,
            path,
            text_chars,
            embed_model: persisted_embed_model,
            state: "ready".to_string(),
            created_at,
        })
    }

    pub fn list_documents(&self, project_id: Id) -> Vec<Document> {
        DocumentsRepo::new(&self.store)
            .list_for_project(&project_id)
            .unwrap_or_default()
            .into_iter()
            .map(|d| Document {
                id: d.id,
                project_id: d.project_id,
                name: d.name,
                mime: d.mime,
                bytes: d.bytes,
                path: d.path,
                text_chars: d.text_chars,
                embed_model: d.embed_model,
                state: d.state,
                created_at: d.created_at,
            })
            .collect()
    }

    pub fn delete_document(&self, id: Id) -> Result<(), PocketError> {
        ChunksRepo::new(&self.store)
            .delete_for_document(&id)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;
        DocumentsRepo::new(&self.store)
            .delete(&id)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;
        Ok(())
    }

    /// Hybrid BM25 + cosine search over all chunks of all documents in the
    /// project, combined by reciprocal rank fusion (D45). Falls back to
    /// lexical-only BM25 when no chunk has a persisted embedding or the query
    /// can't be embedded (no embedding model installed) - honest instruments:
    /// no fabricated vectors, just the lexical signal we actually have.
    pub fn search_project(&self, project_id: Id, query: String, k: u32) -> Vec<ChunkHit> {
        let docs = DocumentsRepo::new(&self.store)
            .list_for_project(&project_id)
            .unwrap_or_default();
        let chunks_repo = ChunksRepo::new(&self.store);
        let rows: Vec<ChunkRow> = docs
            .into_iter()
            .flat_map(|doc| chunks_repo.list_for_document(&doc.id).unwrap_or_default())
            .collect();
        let records: Vec<ChunkRecord> = rows.iter().map(chunk_row_to_record).collect();

        let bm25 = bm25_rank(&query, &records);
        // Try semantic only if some chunk carries an embedding; then embed the
        // query with the same model and fuse. Any failure degrades to lexical.
        let have_embeddings = records.iter().any(|r| !r.embedding.is_empty());
        let query_embedding = if have_embeddings {
            self.default_embedding_model_id()
                .and_then(|model| self.embed(&model, &[query.clone()]).ok())
                .and_then(|mut v| v.pop())
                .filter(|e| !e.is_empty())
        } else {
            None
        };

        let ranked: Vec<(usize, f32)> = match query_embedding {
            Some(qe) => {
                let bm25_top: Vec<usize> =
                    bm25.iter().take(50).map(|(i, _)| *i).collect();
                let cosine_top: Vec<usize> = cosine_rank(&qe, &records)
                    .into_iter()
                    .take(50)
                    .map(|(i, _)| i)
                    .collect();
                reciprocal_rank_fusion(&[bm25_top, cosine_top], 60.0)
            }
            None => bm25,
        };
        // Zero means default limit of 20
        let limit = if k == 0 { 20 } else { k as usize };
        ranked
            .into_iter()
            .take(limit)
            .map(|(index, score)| {
                let row = &rows[index];
                ChunkHit {
                    document_id: row.document_id.clone(),
                    ord: row.ord,
                    text: row.text.clone(),
                    page: row.page,
                    score,
                }
            })
            .collect()
    }

    // ---- server ----

    /// Starts the Mac host's axum server (NET-01). Idempotent: if already
    /// running, returns the current status rather than starting a second
    /// listener. No-op-shaped on iOS at the FFI boundary too, since LAN
    /// serving is gated on Pro + the Mac entitlement check NET-01 enforces.
    pub fn server_start(&self, cfg: ServerConfig) -> Result<ServerStatus, PocketError> {
        let (pro, handle_opt) = {
            let inner = self.inner.lock().unwrap();
            (
                merged_entitlements(&inner).pro,
                inner
                    .server_handle
                    .as_ref()
                    .map(|h| (h.lan_enabled, h.port)),
            )
        };
        if let Some((lan_enabled, port)) = handle_opt {
            return Ok(ServerStatus {
                running: true,
                lan_enabled,
                port,
                connected_devices: 0,
            });
        }
        let tls_identity = self.host_tls_identity();
        let server_cfg = pocket_server::ServerConfig {
            lan_enabled: cfg.lan_enabled,
            pro,
        };
        let models_provider = Arc::new(CoreInstalledModels(self.self_weak.clone()));
        let embeddings_provider = Arc::new(CoreEmbeddingProvider(self.self_weak.clone()));
        // NET-14: the device/API surface requires a valid device bearer
        // token; the pairing endpoints and `/pocket/v1/hello` are pre-token
        // and stay open (added outside this layered sub-router).
        let auth_store: Arc<dyn pocket_server::AuthStore> =
            Arc::new(CoreAuthStore(self.self_weak.clone()));
        let protected = crate::control_routes::router(self.self_weak.clone())
            .merge(crate::chat_routes::router(self.self_weak.clone()))
            .merge(crate::media_routes::router(self.self_weak.clone()))
            .merge(pocket_server::routes::models::router(models_provider))
            .merge(pocket_server::routes::embeddings::router(
                embeddings_provider,
            ))
            .layer(axum::middleware::from_fn_with_state(
                auth_store,
                pocket_server::require_auth,
            ));
        let routes = crate::pairing_routes::router(Arc::clone(&self.pairing_state)).merge(protected);
        let handle = runtime()
            .block_on(pocket_server::start_with_routes(
                server_cfg,
                "pocketcore".to_string(),
                routes,
                tls_identity.cert_der,
                tls_identity.key_der,
            ))
            .map_err(|e| match e {
                pocket_types::PocketError::NotEntitled { feature } => {
                    PocketError::NotEntitled { feature }
                }
                other => PocketError::Network {
                    message: other.to_string(),
                },
            })?;
        let status = ServerStatus {
            running: true,
            lan_enabled: handle.lan_enabled,
            port: handle.port,
            connected_devices: 0,
        };
        self.inner.lock().unwrap().server_handle = Some(handle);
        Ok(status)
    }

    pub fn server_stop(&self) -> Result<(), PocketError> {
        let handle = self.inner.lock().unwrap().server_handle.take();
        if let Some(handle) = handle {
            runtime().block_on(handle.stop());
        }
        Ok(())
    }

    pub fn server_status(&self) -> ServerStatus {
        match &self.inner.lock().unwrap().server_handle {
            Some(handle) => ServerStatus {
                running: true,
                lan_enabled: handle.lan_enabled,
                port: handle.port,
                connected_devices: 0,
            },
            None => ServerStatus {
                running: false,
                lan_enabled: false,
                port: 0,
                connected_devices: 0,
            },
        }
    }

    /// MAC-06: generates a new local API key, stores only its hash (via
    /// CORE-12's `TypedSettings`, previously never wired into the FFI
    /// layer at all), and returns the plaintext once - the Mac's Server
    /// screen is the only place that ever sees it unmasked. NET-14's
    /// `require_auth` middleware enforces device bearer tokens on the API
    /// surface; the loopback-only local-API-key path is stored/rotated here
    /// but is not yet admitted in-band (needs loopback ConnectInfo plumbing).
    pub fn generate_local_api_key(&self) -> String {
        let key = random_local_key();
        let hash = sha256_hex(&key);
        pocket_store::TypedSettings::new(&self.store).set_server_local_api_key_hash(&hash);
        key
    }

    pub fn has_local_api_key(&self) -> bool {
        pocket_store::TypedSettings::new(&self.store)
            .server_local_api_key_hash()
            .is_some()
    }

    /// MAC-06: The host's self-signed cert in PEM format (safe to share; public cert, not key).
    pub fn host_cert_pem(&self) -> String {
        pocket_trust::der_to_pem(&self.host_tls_identity().cert_der)
    }

    // ---- trust ----

    /// Generates a QR offer per A§7 (Pro-gated, 300s TTL, single-use
    /// secret). Does **not** yet turn on the LAN listener for the offer's
    /// duration or track pending-pairing state - that needs the `POST
    /// /pocket/v1/pair` route (NET-10's remaining, route-layer scope).
    /// Host identity for Bonjour advertising: same host_id/name/fingerprint a
    /// pairing offer carries, WITHOUT rotating the single-use pairing secret
    /// and without the Pro gate (it reveals no secret).
    pub fn pairing_identity(&self) -> PairingIdentity {
        PairingIdentity {
            host_id: self.config.app_support_dir.clone(),
            host_name: self.host_display_name(),
            cert_fingerprint: self.host_cert_fingerprint(),
        }
    }

    /// UI-24: the user-facing name this host advertises in pairing payloads —
    /// the computer's name from the platform layer, falling back to the
    /// hardware model identifier (e.g. "Mac15,6") when unset.
    fn host_display_name(&self) -> String {
        let profile = self.platform.device_profile();
        if profile.device_name.is_empty() {
            profile.model_identifier
        } else {
            profile.device_name
        }
    }

    pub fn pairing_offer(&self) -> Result<PairingOffer, PocketError> {
        if !merged_entitlements(&self.inner.lock().unwrap()).pro {
            return Err(PocketError::NotEntitled {
                feature: "pairing".into(),
            });
        }
        let cert_fingerprint = self.host_cert_fingerprint();
        let host_id = self.config.app_support_dir.clone();
        let now = self.platform.now_ms();
        let secret = pocket_trust::generate_secret();
        let expires_at_ms = now + 300_000;
        self.pairing_state
            .set_current_secret(secret.clone(), expires_at_ms);
        let addrs = if let Some(v4) = primary_lan_ipv4() {
            vec![v4.to_string()]
        } else {
            vec![system_bonjour_hostname()]
        };
        Ok(PairingOffer {
            host_id: host_id.clone(),
            host_name: self.host_display_name(),
            addrs,
            port: self.server_status().port,
            cert_fingerprint,
            secret,
            expires_at_ms,
        })
    }

    /// MAC-07: the exact `pocketai://pair?...` string the Mac's QR code
    /// should encode, built from a fresh `pairing_offer()` via
    /// `pocket_trust`'s own URI builder so Swift never has to replicate
    /// that format itself.
    pub fn pairing_offer_qr_string(&self) -> Result<String, PocketError> {
        let offer = self.pairing_offer()?;
        Ok(pocket_trust::build_pairing_uri(&pocket_trust::PairingUri {
            host_id: offer.host_id,
            host_name: offer.host_name,
            addrs: offer.addrs,
            port: offer.port,
            cert_fingerprint: offer.cert_fingerprint,
            secret: offer.secret,
        }))
    }

    /// MAC-07: pending incoming pairing requests awaiting an Allow/Deny
    /// decision on this Mac.
    pub fn pending_pairings(&self) -> Vec<PairingRequest> {
        self.pairing_state
            .pending_requests()
            .into_iter()
            .map(|(id, device_name, platform)| PairingRequest {
                id,
                device_name,
                platform,
            })
            .collect()
    }

    pub fn parse_pairing_qr(&self, qr: String) -> Result<PairingOffer, PocketError> {
        let parsed =
            pocket_trust::parse_pairing_uri(&qr).map_err(|e| PocketError::InvalidInput {
                message: e.to_string(),
            })?;
        Ok(PairingOffer {
            host_id: parsed.host_id,
            host_name: parsed.host_name,
            addrs: parsed.addrs,
            port: parsed.port,
            cert_fingerprint: parsed.cert_fingerprint,
            secret: parsed.secret,
            expires_at_ms: 0,
        })
    }

    pub fn save_peer(&self, approved: ApprovedPairing) -> Result<Peer, PocketError> {
        let now = self.platform.now_ms();
        let row = pocket_store::PeerRow {
            id: approved.device_id.clone(),
            // UI-24: display the host's name, not its app-support path. Empty
            // host_name (pre-UI-24 caller) keeps the old host_id behavior.
            name: if approved.host_name.is_empty() {
                approved.host_id.clone()
            } else {
                approved.host_name.clone()
            },
            cert_sha256: approved.cert_fingerprint.clone().into_bytes(),
            addrs_json: serde_json::to_string(&[&approved.addr]).unwrap_or_default(),
            port: approved.port,
            token_ref: approved.token_keychain_ref.clone(),
            paired_at: now,
            last_seen_at: None,
        };
        pocket_store::PeersRepo::new(&self.store)
            .create(&row)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;
        Ok(Peer {
            id: row.id,
            name: row.name,
            cert_sha256: approved.cert_fingerprint,
            addrs: vec![approved.addr],
            port: row.port,
            token_ref: row.token_ref,
            paired_at: now,
            last_seen_at: None,
        })
    }

    pub fn approve_pairing(&self, id: Id) -> Result<(), PocketError> {
        self.pairing_state
            .approve(&id)
            .map_err(|e| PocketError::NotFound {
                what: e.to_string(),
            })
    }

    pub fn deny_pairing(&self, id: Id) -> Result<(), PocketError> {
        self.pairing_state
            .deny(&id)
            .map_err(|e| PocketError::NotFound {
                what: e.to_string(),
            })
    }

    pub fn devices(&self) -> Vec<PairedDevice> {
        DevicesRepo::new(&self.store)
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|d| PairedDevice {
                id: d.id,
                name: d.name,
                platform: if d.platform == "ios" {
                    Platform::Ios
                } else {
                    Platform::MacOs
                },
                role: d.role,
                paired_at: d.paired_at,
                last_seen_at: d.last_seen_at,
                last_addr: d.last_addr,
                revoked_at: d.revoked_at,
            })
            .collect()
    }

    /// Clears the token hash (so it can never match again) and marks the
    /// device revoked. Closing any live SSE stream for the device is the
    /// Mac server's job (NET-11, `pocket-server::DeviceService`) once this
    /// core is wired into one; this method only owns the storage side.
    pub fn revoke_device(&self, id: Id) -> Result<(), PocketError> {
        let repo = DevicesRepo::new(&self.store);
        let now = self.platform.now_ms();
        repo.revoke(&id, now).map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?;
        self.store
            .writer()
            .execute("UPDATE devices SET token_hash = x'' WHERE id = ?1", [&id])
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;
        Ok(())
    }

    pub fn peers(&self) -> Vec<Peer> {
        pocket_store::PeersRepo::new(&self.store)
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|p| Peer {
                id: p.id,
                // UI-24 migration: peers saved before host_name existed carry
                // the Mac's app-support path as their name; never surface a
                // filesystem path in UI. Re-pairing stores a real name.
                name: if p.name.starts_with('/') {
                    "Mac".into()
                } else {
                    p.name
                },
                cert_sha256: String::from_utf8_lossy(&p.cert_sha256).into_owned(),
                addrs: serde_json::from_str(&p.addrs_json).unwrap_or_default(),
                port: p.port,
                token_ref: p.token_ref,
                paired_at: p.paired_at,
                last_seen_at: p.last_seen_at,
            })
            .collect()
    }

    /// UI-12: "forget this Mac" on the iPhone's Devices tab.
    pub fn revoke_peer(&self, id: Id) -> Result<(), PocketError> {
        pocket_store::PeersRepo::new(&self.store)
            .delete(&id)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    pub fn stop_everything(&self) {
        let _ = self.server_stop();
    }

    pub fn register_download_coordinator(&self, coordinator: Arc<dyn DownloadCoordinator>) {
        self.inner.lock().unwrap().download_coordinator = Some(coordinator);
    }

    // ---- entitlements ----

    /// Verifies a Keygen `ED25519_SIGN` license key fully offline (LIC-02)
    /// against the real Keygen account public key (LIC-01).
    pub fn activate_license(&self, key: String) -> Result<Entitlements, PocketError> {
        let public_key =
            pocket_trust::keygen_verifying_key().map_err(|e| PocketError::Internal {
                message: e.to_string(),
            })?;
        let now_ms = self.platform.now_ms();
        let payload =
            pocket_trust::verify_license_key(&key, &public_key, now_ms, &[]).map_err(|e| {
                PocketError::InvalidInput {
                    message: e.to_string(),
                }
            })?;

        let expires_at_ms = payload
            .license
            .expiry
            .as_deref()
            .and_then(pocket_trust::parse_iso8601_utc_ms);

        let license_entitlement = Entitlements {
            pro: true,
            source: format!("license:{}", payload.license.id),
            expires_at_ms,
        };
        let mut inner = self.inner.lock().unwrap();
        inner.license_entitlement = Some(license_entitlement);
        Ok(merged_entitlements(&inner))
    }

    pub fn set_store_entitlement(&self, e: StoreEntitlement) {
        self.inner.lock().unwrap().store_entitlement = Some(e);
    }

    /// LIC-03 / D103: Keygen is the sole licensing system for externally
    /// purchased licenses (no in-app Pro tier / no App Store IAP path).
    /// Store entitlement is retained as a legacy/API-stability source;
    /// `merged_entitlements` evaluates active license and any legacy entitlement.
    pub fn entitlements(&self) -> Entitlements {
        merged_entitlements(&self.inner.lock().unwrap())
    }

    // ---- privacy ----

    pub fn set_offline_mode(&self, on: bool) {
        self.inner.lock().unwrap().offline_mode = on;
        self.http.set_offline_mode(on);
    }

    /// APL-04: `Net` (Swift's only allowed internet client) checks this
    /// before every request, enforcing the same offline policy client-side
    /// instead of relying solely on `pocket_trust::PocketHttp`'s own gate.
    pub fn is_offline(&self) -> bool {
        self.inner.lock().unwrap().offline_mode
    }

    pub fn egress_log(&self, limit: u32) -> Vec<EgressEntry> {
        let inner = self.inner.lock().unwrap();
        inner
            .egress
            .iter()
            .rev()
            .take(limit as usize)
            .cloned()
            .collect()
    }

    pub fn record_egress(&self, e: EgressEntry) {
        self.inner.lock().unwrap().egress.push(e);
    }

    pub fn audit_log(&self, q: AuditQuery) -> Vec<AuditEntry> {
        let limit = if q.limit == 0 { 100 } else { q.limit };
        pocket_store::AuditRepo::new(&self.store)
            .list(limit)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| q.since_ts.is_none_or(|since| r.ts >= since))
            .map(|r| AuditEntry {
                id: r.id,
                ts: r.ts,
                actor: r.actor,
                action: r.action,
                target: r.target,
                detail_json: r.detail_json,
            })
            .collect()
    }

    pub fn export_audit(&self, path: String) -> Result<(), PocketError> {
        pocket_audit::AuditLog::new(&self.store)
            .export_jsonl(&path)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    // ---- events ----

    pub fn subscribe(&self, _sink: Arc<dyn EventSink>) -> Subscription {
        Subscription {
            id: "unimplemented".into(),
        }
    }
}

/// Sentinel id for Apple Intelligence's built-in on-device model. Must match
/// `AppleFoundationModelsEngine.modelId` on the Swift side (F15).
pub(crate) const APPLE_INTELLIGENCE_MODEL_ID: &str = "apple-intelligence@on-device";

/// Plain (non-FFI-exported) helpers - a `#[uniffi::export] impl` block
/// exports every method in it, so these live in a separate `impl`.
impl PocketCore {
    pub(crate) fn pick_engine(
        &self,
        manifest: &ModelManifest,
    ) -> Result<Arc<dyn Engine>, PocketError> {
        let inner = self.inner.lock().unwrap();
        if let Some(engine) = &inner.registered_engine {
            if engine.supports(manifest.clone()) {
                return Ok(Arc::clone(engine));
            }
        }
        if inner.llama_engine.supports(manifest.clone()) {
            return Ok(Arc::clone(&inner.llama_engine));
        }
        let engine = match manifest.format {
            ModelFormat::Gguf => EngineKind::LlamaCpp,
            _ => EngineKind::Mlx,
        };
        Err(PocketError::EngineFailed {
            engine,
            message: format!("no engine supports model {}", manifest.id),
        })
    }

    #[cfg(test)]
    pub(crate) fn store_for_test(&self) -> &Arc<Store> {
        &self.store
    }

    /// Records a benchmark on the model's row so it survives relaunch and
    /// `catalog()` can report `Confidence::Measured`. Best-effort: a model with
    /// no row (never downloaded through this core) just isn't recorded - the
    /// measurement is still returned to the caller that asked for it.
    fn persist_benchmark(&self, id: &ModelId, benchmark: &Benchmark) {
        let repo = pocket_store::ModelsRepo::new(&self.store);
        if let Ok(Some(mut row)) = repo.get(id) {
            row.benchmark_json = serde_json::to_string(&pocket_types::Benchmark {
                tps: benchmark.tps,
                ttft_ms: benchmark.ttft_ms,
                peak_bytes: benchmark.peak_bytes,
            })
            .ok();
            let _ = repo.upsert(&row);
        }
    }

    /// Records a freshly loaded engine handle against its model id so a later
    /// `unload(id)` can free it (D35).
    pub(crate) fn track_loaded(&self, model_id: &str, engine: &Arc<dyn Engine>, handle: &str) {
        self.inner
            .lock()
            .unwrap()
            .loaded_handles
            .entry(model_id.to_string())
            .or_default()
            .push((Arc::clone(engine), handle.to_string()));
    }

    /// Reuses the already-loaded handle for `model_id` on `engine` when one is
    /// tracked; only loads (and tracks) when absent. Without this every
    /// send/regenerate loaded a fresh multi-GB copy of the weights per message
    /// and `loaded_handles` accumulated them until `unload`.
    /// ponytail: a concurrent first-load race can still load twice; both get
    /// tracked and freed by unload, so it's benign. Serialize loads if it hurts.
    pub(crate) fn load_or_reuse(
        &self,
        model_id: &str,
        engine: &Arc<dyn Engine>,
        manifest: ModelManifest,
        opts: LoadOptions,
    ) -> Result<String, PocketError> {
        if let Some(handles) = self.inner.lock().unwrap().loaded_handles.get(model_id) {
            if let Some((_, handle)) = handles.iter().find(|(e, _)| Arc::ptr_eq(e, engine)) {
                return Ok(handle.clone());
            }
        }
        let loaded = engine.load(manifest, opts)?;
        self.track_loaded(model_id, engine, &loaded.handle);
        Ok(loaded.handle)
    }

    /// True when `engine` is the built-in llama.cpp adapter (vs. a registered
    /// engine like MLX, whose behavior must stay unchanged).
    fn is_builtin_llama(&self, engine: &Arc<dyn Engine>) -> bool {
        Arc::ptr_eq(engine, &self.inner.lock().unwrap().llama_engine)
    }

    /// The one effective context used for both `LoadOptions` and history
    /// truncation: the catalog context capped by the platform policy
    /// (iPhone 4096, Mac 8192), matching what the llama engine will actually
    /// allocate — a 131k catalog value must not drive truncation past the cap.
    fn effective_context_length(&self, catalog_ctx: u32) -> u32 {
        let cap = match self.platform.device_profile().platform {
            Platform::Ios => 4096,
            Platform::MacOs => 8192,
        };
        if catalog_ctx > 0 {
            catalog_ctx.min(cap)
        } else {
            cap
        }
    }

    /// Whether a model id can actually serve a turn right now: a remote Mac
    /// route, Apple's built-in model, or a locally-installed model. A model that
    /// is merely selected/known but not installed is NOT runnable.
    fn is_model_runnable(&self, id: &str) -> bool {
        if id.starts_with("mac:") || id == APPLE_INTELLIGENCE_MODEL_ID {
            return true;
        }
        pocket_store::ModelsRepo::new(&self.store)
            .get(id)
            .ok()
            .flatten()
            .map(|r| r.state == "installed")
            .unwrap_or(false)
    }

    /// Actionable message when no runnable model is found — names the real cause
    /// (still downloading / failed / paused) instead of a blanket "no model",
    /// which was confusing when a model looked present but wasn't yet installed.
    fn no_runnable_model_message(&self) -> String {
        let rows = pocket_store::ModelsRepo::new(&self.store).list().unwrap_or_default();
        let has = |p: &str| rows.iter().any(|r| r.state.starts_with(p));
        if has("downloading") || has("verifying") || has("queued") {
            "Your model is still downloading. Open Models to check its progress.".into()
        } else if has("failed") {
            "A model download failed. Open Models to retry it.".into()
        } else if has("paused") {
            "A model download is paused. Open Models to resume it.".into()
        } else {
            "No model is installed. Download a model in Models, or pair a Mac and pick one under \"On your Mac\".".into()
        }
    }

    pub(crate) fn find_manifest(&self, id: &ModelId) -> Option<pocket_models::ModelManifest> {
        // Apple Intelligence is a system-managed, built-in on-device model (F15,
        // §9.1): it has no Hugging Face repo and is never downloaded, but it must
        // still resolve to a manifest so selection/send can route to its engine.
        if id == APPLE_INTELLIGENCE_MODEL_ID {
            return Some(pocket_models::ModelManifest {
                id: id.clone(),
                display_name: "Apple Intelligence".into(),
                family: "apple".into(),
                params_b: Some(0.0),
                quant: "".into(),
                format: "apple-foundation".into(),
                modalities: vec!["text".into()],
                context_length: 3500,
                files: vec![],
                license: pocket_models::LicenseInfo {
                    spdx: "".into(),
                    commercial_ok: true,
                    url: "".into(),
                },
                hf_repo: "".into(),
                hf_revision: "".into(),
                min_ram_bytes: 0,
                est_runtime_bytes: 0,
                tags: vec!["built-in".into()],
                curated: true,
            });
        }
        if let Some(remote_suffix) = id.strip_prefix("mac:") {
            let local_id = remote_suffix
                .split_once('/')
                .map(|(_, r)| r)
                .unwrap_or(remote_suffix);
            let mut manifest = self.find_manifest(&local_id.to_string()).unwrap_or_else(|| {
                pocket_models::ModelManifest {
                    id: local_id.to_string(),
                    display_name: local_id.to_string(),
                    family: "remote".into(),
                    params_b: Some(0.0),
                    quant: "".into(),
                    format: "mlx-safetensors".into(),
                    modalities: vec!["text".into()],
                    context_length: 4096,
                    files: vec![],
                    license: pocket_models::LicenseInfo {
                        spdx: "".into(),
                        commercial_ok: true,
                        url: "".into(),
                    },
                    hf_repo: local_id.to_string(),
                    hf_revision: "".into(),
                    min_ram_bytes: 0,
                    est_runtime_bytes: 0,
                    tags: vec![],
                    curated: false,
                }
            });
            manifest.id = id.clone();
            return Some(manifest);
        }
        if let Ok(Some(row)) = pocket_store::ModelsRepo::new(&self.store).get(id) {
            if let Ok(m) = serde_json::from_str::<pocket_models::ModelManifest>(&row.manifest_json)
            {
                return Some(m);
            }
        }
        if let Some(m) = pocket_models::load()
            .entries
            .into_iter()
            .find(|m| &m.id == id)
        {
            return Some(m);
        }
        let prefix = id.split('@').next().unwrap_or(id);
        pocket_models::load()
            .entries
            .into_iter()
            .find(|m| m.id.starts_with(prefix))
    }

    pub(crate) fn embed(
        &self,
        model_id: &str,
        inputs: &[String],
    ) -> Result<Vec<Vec<f32>>, PocketError> {
        let manifest =
            self.find_manifest(&model_id.to_string())
                .ok_or_else(|| PocketError::NotFound {
                    what: model_id.to_string(),
                })?;
        let ffi_manifest = manifest_from_models(&manifest);
        let engine = self.pick_engine(&ffi_manifest)?;
        let handle = self.load_or_reuse(model_id, &engine, ffi_manifest, LoadOptions::default())?;
        engine.embed(handle, inputs.to_vec())
    }

    fn installed_openai_models(&self) -> Vec<pocket_server::OpenAiModel> {
        let rows = pocket_store::ModelsRepo::new(&self.store)
            .list()
            .unwrap_or_default();
        let mut list: Vec<pocket_server::OpenAiModel> = rows
            .into_iter()
            .filter(|r| r.state == "installed")
            .map(|r| pocket_server::OpenAiModel {
                id: r.id.clone(),
                object: "model",
                created: r.installed_at.unwrap_or(1760400000000) / 1000,
                owned_by: "pocketai",
                pocket: pocket_server::PocketFitExtension {
                    loaded: false,
                    engine: r.pinned_engine.unwrap_or_else(|| "mlx".into()),
                    fit: "runs_well".into(),
                },
            })
            .collect();

        if list.is_empty() {
            for m in pocket_models::load().entries.iter().take(3) {
                list.push(pocket_server::OpenAiModel {
                    id: m.id.clone(),
                    object: "model",
                    created: 1760400000,
                    owned_by: "pocketai",
                    pocket: pocket_server::PocketFitExtension {
                        loaded: false,
                        engine: "mlx".into(),
                        fit: "runs_well".into(),
                    },
                });
            }
        }
        list
    }

    /// ENG-08: after a conversation's first exchange, ask the same model
    /// for a short title (8 tokens max) and persist it. Best-effort - a
    /// failure here must never fail the chat turn itself, so errors are
    /// just dropped.
    fn generate_title(
        &self,
        conversation_id: &Id,
        handle: &str,
        engine: &Arc<dyn Engine>,
        chat: &ChatService,
        reply_text: &str,
    ) {
        let req = GenerateRequest {
            request_id: new_ulid(),
            messages: vec![ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text {
                    text: format!(
                        "Reply with only a short (<=6 word) title for this exchange, no punctuation or quotes:\n{reply_text}"
                    ),
                }],
            }],
            tools: vec![],
            params: SamplingParams {
                temperature: 0.3,
                top_p: 0.95,
                top_k: 40,
                min_p: 0.0,
                max_tokens: 8,
                stop: vec![],
                seed: None,
                repetition_penalty: 1.1,
            },
            reasoning: ReasoningMode::Off,
        };
        let sink = Arc::new(AccumulatingSink::new(Arc::new(NullChatSink)));
        if engine
            .generate(handle.to_string(), req, sink.clone() as Arc<dyn TokenSink>)
            .is_ok()
        {
            let title = sink.text().trim().trim_matches('"').to_string();
            if !title.is_empty() {
                let _ = chat.set_title(conversation_id, &title, self.platform.now_ms());
            }
        }
    }

    pub(crate) fn stt_engine(&self) -> Result<Arc<dyn SpeechToTextEngine>, PocketError> {
        self.inner
            .lock()
            .unwrap()
            .registered_stt
            .clone()
            .ok_or_else(|| PocketError::Internal {
                message: "no SpeechToTextEngine registered".into(),
            })
    }

    pub(crate) fn tts_engine(&self) -> Result<Arc<dyn TextToSpeechEngine>, PocketError> {
        self.inner
            .lock()
            .unwrap()
            .registered_tts
            .clone()
            .ok_or_else(|| PocketError::Internal {
                message: "no TextToSpeechEngine registered".into(),
            })
    }

    pub(crate) fn image_engine(&self) -> Result<Arc<dyn ImageEngine>, PocketError> {
        self.inner
            .lock()
            .unwrap()
            .registered_image
            .clone()
            .ok_or_else(|| PocketError::Internal {
                message: "no ImageEngine registered".into(),
            })
    }

    fn models_dir(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.config.app_support_dir).join("Models")
    }

    /// First `recommended` text model in the bundled catalog - used when a
    /// conversation has no `model_id` set yet.
    /// First installed model with the `embedding` modality, used to embed RAG
    /// chunks and queries (D45). `None` when nothing embedding-capable is
    /// installed, which routes search back to lexical-only.
    fn default_embedding_model_id(&self) -> Option<String> {
        let installed: std::collections::HashSet<String> =
            pocket_store::ModelsRepo::new(&self.store)
                .list()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.state == "installed")
                .map(|r| r.id)
                .collect();
        if installed.is_empty() {
            return None;
        }
        pocket_models::load()
            .entries
            .into_iter()
            .find(|m| m.modalities.iter().any(|mm| mm == "embedding") && installed.contains(&m.id))
            .map(|m| m.id)
    }

    fn default_recommended_model_id(&self) -> Option<String> {
        // Prefer an already-INSTALLED model so a fresh chat never tries to load
        // weights that were never downloaded (which fails with a cryptic MLX
        // "config.json: no such file" error). Only if nothing is installed do we
        // return None, so the caller can surface a clear "download a model or
        // pair a Mac" message instead.
        let installed: std::collections::HashSet<String> =
            pocket_store::ModelsRepo::new(&self.store)
                .list()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.state == "installed")
                .map(|r| r.id)
                .collect();
        if installed.is_empty() {
            return None;
        }
        pocket_models::load()
            .entries
            .into_iter()
            .find(|m| {
                m.modalities.iter().any(|mm| mm == "text") && installed.contains(&m.id)
            })
            .map(|m| m.id)
            // an installed text model wasn't in the curated catalog; fall back to
            // any installed model id.
            .or_else(|| installed.into_iter().next())
    }

    pub(crate) fn host_tls_identity(&self) -> pocket_trust::TlsIdentity {
        let mut inner = self.inner.lock().unwrap();
        if let Some(id) = &inner.tls_identity {
            return id.clone();
        }
        let mut ips = Vec::new();
        if let Some(v4) = primary_lan_ipv4() {
            ips.push(std::net::IpAddr::V4(v4));
        }
        ips.push(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));

        let hostname = system_bonjour_hostname();
        let identity = pocket_trust::generate_identity(&hostname, &ips);
        inner.tls_identity = Some(identity.clone());
        identity
    }

    fn host_cert_fingerprint(&self) -> String {
        self.host_tls_identity().fingerprint_b64url
    }

    /// NET-12: the control API calls `plan_download` then hands the plan to
    /// the registered Swift `DownloadCoordinator` (APL-05 implements it with
    /// a real background `URLSession`; no coordinator registered yet means
    /// this errors rather than silently no-op-ing).
    pub(crate) fn start_download_via_coordinator(
        &self,
        plan: DownloadPlan,
    ) -> Result<(), PocketError> {
        let coordinator = self
            .inner
            .lock()
            .unwrap()
            .download_coordinator
            .clone()
            .ok_or_else(|| PocketError::Internal {
                message: "no DownloadCoordinator registered".into(),
            })?;
        coordinator.start(plan)
    }
}

/// LIC-03: Pro if either the license or the store entitlement grants it.
fn merged_entitlements(inner: &Inner) -> Entitlements {
    let license_pro = inner.license_entitlement.as_ref().is_some_and(|e| e.pro);
    let store_pro = inner.store_entitlement.as_ref().is_some_and(|e| e.pro);
    let source = match (license_pro, store_pro) {
        (true, true) => format!(
            "{}+store",
            inner.license_entitlement.as_ref().unwrap().source
        ),
        (true, false) => inner.license_entitlement.as_ref().unwrap().source.clone(),
        (false, true) => "store".into(),
        (false, false) => "none".into(),
    };
    let expires_at_ms = if store_pro {
        None
    } else {
        inner
            .license_entitlement
            .as_ref()
            .and_then(|e| e.expires_at_ms)
    };
    Entitlements {
        pro: license_pro || store_pro,
        source,
        expires_at_ms,
    }
}

// ponytail: ~4 chars/token English estimate, not a real tokenizer count.
// Upgrade path: add `Engine::count_tokens` once an engine exposes its
// tokenizer and swap this out everywhere truncation is computed.
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4 + 1
}

/// ENG-08: drops the oldest turns (a leading `system` message is never
/// dropped) until the estimated token total fits `context_length -
/// max_tokens`, so the model always has room to actually reply.
fn truncate_to_fit(messages: &mut Vec<ChatMessage>, context_length: usize, max_tokens: usize) {
    let budget = context_length.saturating_sub(max_tokens);
    let text_of = |m: &ChatMessage| -> usize {
        m.parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { text } => estimate_tokens(text),
                ContentPart::ImageRef { .. } => 0,
            })
            .sum::<usize>()
    };
    let has_system = messages.first().is_some_and(|m| m.role == "system");
    let keep_from = if has_system { 1 } else { 0 };

    loop {
        let total: usize = messages.iter().map(text_of).sum();
        if total <= budget || messages.len() <= keep_from + 1 {
            break;
        }
        messages.remove(keep_from);
    }
}

fn chunk_row_to_record(row: &ChunkRow) -> ChunkRecord {
    ChunkRecord {
        document: row.document_id.clone(),
        page: row.page.unwrap_or(0),
        text: row.text.clone(),
        embedding: embedding_from_bytes(&row.embedding),
    }
}

/// A model row's persisted benchmark (ENG-09), or `None` when it was never
/// measured or the column holds something unreadable.
fn benchmark_from_row(row: &pocket_store::ModelRow) -> Option<pocket_types::Benchmark> {
    serde_json::from_str(row.benchmark_json.as_deref()?).ok()
}

/// Same, shaped for `pocket_models::fit`'s `measured` argument.
fn measured_benchmark(row: &pocket_store::ModelRow) -> Option<pocket_models::Benchmark> {
    benchmark_from_row(row).map(|b| pocket_models::Benchmark {
        tps: b.tps,
        ttft_ms: b.ttft_ms,
        peak_bytes: b.peak_bytes,
    })
}

fn new_ulid() -> String {
    ulid::Ulid::new().to_string()
}

fn random_local_key() -> String {
    format!("pk-{}{}", new_ulid(), new_ulid()).to_lowercase()
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn extract_text_field(content_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content_json)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_default()
}

fn extract_user_text(input: &UserInput) -> String {
    input
        .parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::ImageRef { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Serializes full message parts (text + image refs) to content_json (F13),
/// keeping a flat `"text"` field so old readers and title generation still work.
fn parts_to_content_json(parts: &[ContentPart]) -> String {
    let flat = parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::ImageRef { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let arr: Vec<serde_json::Value> = parts
        .iter()
        .map(|p| match p {
            ContentPart::Text { text } => serde_json::json!({ "type": "text", "text": text }),
            ContentPart::ImageRef { path } => serde_json::json!({ "type": "imageRef", "path": path }),
        })
        .collect();
    serde_json::json!({ "text": flat, "parts": arr }).to_string()
}

/// Reconstructs message parts from content_json (F13). Backward-compatible:
/// old text-only rows (`{"text": "..."}` with no `parts`) decode to one Text.
fn extract_parts(content_json: &str) -> Vec<ContentPart> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(content_json) {
        if let Some(arr) = v.get("parts").and_then(|p| p.as_array()) {
            let parts: Vec<ContentPart> = arr
                .iter()
                .filter_map(|e| match e.get("type").and_then(|t| t.as_str()) {
                    Some("text") => e
                        .get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| ContentPart::Text { text: s.to_string() }),
                    Some("imageRef") => e
                        .get("path")
                        .and_then(|t| t.as_str())
                        .map(|s| ContentPart::ImageRef { path: s.to_string() }),
                    _ => None,
                })
                .collect();
            if !parts.is_empty() {
                return parts;
            }
        }
    }
    vec![ContentPart::Text {
        text: extract_text_field(content_json),
    }]
}

/// Discards every callback - used where a `ChatSink` is required by the
/// type signature but nothing downstream needs to observe deltas (e.g.
/// `generate_title`'s throwaway request).
struct NullChatSink;

impl ChatSink for NullChatSink {
    fn on_delta(&self, _delta: GenerateDelta) {}
    fn on_done(&self, _stats: GenerateStats) {}
    fn on_error(&self, _err: PocketError) {}
    fn on_message_saved(&self, _message: Message) {}
}

/// Forwards every delta/done/error to the real (Swift-implemented) `ChatSink`
/// while accumulating the generated text so `send()` can persist and return
/// the final `Message` once the engine finishes.
///
/// ENG-06: raw model output is split here — `<think>…</think>` content goes
/// to `GenerateDelta.reasoning` (and the persisted `reasoning` column), never
/// to the visible text. Engines may stream raw tags; the UI never sees them.
struct AccumulatingSink {
    inner: Arc<dyn ChatSink>,
    text: Mutex<String>,
    reasoning: Mutex<String>,
    splitter: Mutex<pocket_engine::ReasoningSplitter>,
}

impl AccumulatingSink {
    fn new(inner: Arc<dyn ChatSink>) -> Self {
        Self {
            inner,
            text: Mutex::new(String::new()),
            reasoning: Mutex::new(String::new()),
            splitter: Mutex::new(pocket_engine::ReasoningSplitter::new()),
        }
    }

    fn text(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    fn reasoning(&self) -> String {
        self.reasoning.lock().unwrap().clone()
    }

    fn forward(&self, split: pocket_engine::Split, tool_calls: Vec<crate::types::ToolCallDelta>) {
        self.text.lock().unwrap().push_str(&split.text);
        self.reasoning.lock().unwrap().push_str(&split.reasoning);
        if !split.text.is_empty() || !split.reasoning.is_empty() || !tool_calls.is_empty() {
            self.inner.on_delta(GenerateDelta {
                text: split.text,
                reasoning: split.reasoning,
                tool_calls,
            });
        }
    }
}

impl TokenSink for AccumulatingSink {
    fn on_delta(&self, delta: GenerateDelta) {
        // An engine that already separates reasoning keeps its split; raw
        // text goes through the tag splitter.
        if !delta.reasoning.is_empty() {
            self.reasoning.lock().unwrap().push_str(&delta.reasoning);
            self.text.lock().unwrap().push_str(&delta.text);
            self.inner.on_delta(delta);
            return;
        }
        let split = self.splitter.lock().unwrap().push(&delta.text);
        self.forward(split, delta.tool_calls);
    }
    fn on_done(&self, stats: GenerateStats) {
        // A partial tag never completed is plain content — emit it.
        let split = self.splitter.lock().unwrap().flush();
        self.forward(split, Vec::new());
        self.inner.on_done(stats);
    }
    fn on_error(&self, err: PocketError) {
        self.inner.on_error(err);
    }
}

/// NET-14: resolves device bearer tokens against stored `devices` rows and
/// supplies the Mac's live Pro state to `pocket_server::require_auth`. Uses
/// the same SHA-256 scheme pairing used to write `token_hash`.
struct CoreAuthStore(std::sync::Weak<PocketCore>);

impl pocket_server::AuthStore for CoreAuthStore {
    fn device_id_for_token_hash(&self, hash: &str) -> Option<String> {
        let core = self.0.upgrade()?;
        // `approve` stores sha256(token) as the raw 32-byte digest; hex it
        // back to compare against the hex hash `authenticate` computed. A
        // revoked device's hash is cleared (empty), so it never matches.
        DevicesRepo::new(&core.store)
            .list()
            .ok()?
            .into_iter()
            .find(|d| d.revoked_at.is_none() && constant_time_eq(&hex_encode(&d.token_hash), hash))
            .map(|d| d.id)
    }

    fn local_api_key_hash(&self) -> Option<String> {
        let core = self.0.upgrade()?;
        pocket_store::TypedSettings::new(&core.store).server_local_api_key_hash()
    }

    fn mac_is_pro(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|core| merged_entitlements(&core.inner.lock().unwrap()).pro)
    }

    fn record_seen(&self, device_id: &str) {
        if let Some(core) = self.0.upgrade() {
            let now = core.platform.now_ms();
            let _ = DevicesRepo::new(&core.store).mark_seen(device_id, now, "");
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time compare of the two (equal-length) hex hashes, so token
/// lookup doesn't leak timing. Mirrors `pairing_routes::constant_time_eq`.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

struct CoreInstalledModels(std::sync::Weak<PocketCore>);

impl pocket_server::InstalledModels for CoreInstalledModels {
    fn list(&self) -> Vec<pocket_server::OpenAiModel> {
        let Some(core) = self.0.upgrade() else {
            return Vec::new();
        };
        core.installed_openai_models()
    }
}

struct CoreEmbeddingProvider(std::sync::Weak<PocketCore>);

impl pocket_server::EmbeddingProvider for CoreEmbeddingProvider {
    fn embed(&self, model: &str, inputs: &[String]) -> Vec<Vec<f32>> {
        let Some(core) = self.0.upgrade() else {
            return Vec::new();
        };
        let model = model.to_string();
        let inputs = inputs.to_vec();
        tokio::task::block_in_place(|| core.embed(&model, &inputs).unwrap_or_default())
    }
}

/// NET-10: returns the Mac's primary LAN IPv4 via a UDP socket connect trick
/// without sending any packets or needing any external dependencies.
fn primary_lan_ipv4() -> Option<std::net::Ipv4Addr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?; // no packet sent; just sets the route
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}

/// Discovers the local system hostname without external dependencies.
fn system_hostname() -> Option<String> {
    #[cfg(unix)]
    {
        extern "C" {
            fn gethostname(name: *mut std::ffi::c_char, len: usize) -> std::ffi::c_int;
        }
        let mut buf = [0u8; 256];
        let res = unsafe { gethostname(buf.as_mut_ptr() as *mut std::ffi::c_char, buf.len()) };
        if res == 0 {
            if let Some(pos) = buf.iter().position(|&b| b == 0) {
                if let Ok(name) = std::str::from_utf8(&buf[..pos]) {
                    let trimmed = name.trim().trim_end_matches(".local").trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .map(|s| s.trim().trim_end_matches(".local").trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Cleans a hostname label so it is a valid DNS label (RFC 1123).
fn sanitize_dns_label(s: &str) -> Option<String> {
    let label = s.split('.').next().unwrap_or(s);
    let sanitized: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Fallback Bonjour hostname ("<hostname>.local" or "pocketai.local").
fn system_bonjour_hostname() -> String {
    system_hostname()
        .as_deref()
        .and_then(sanitize_dns_label)
        .map(|h| format!("{h}.local"))
        .unwrap_or_else(|| "pocketai.local".to_string())
}
