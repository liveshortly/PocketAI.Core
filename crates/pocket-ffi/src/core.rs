use std::sync::{Arc, Mutex};

use crate::convert::*;
use crate::error::{unimplemented, PocketError};
use crate::traits::*;
use crate::types::*;
use pocket_store::repos::devices::DevicesRepo;
use pocket_store::services::chat::{ChatService, ConversationQuery as StoreConversationQuery};
use pocket_store::repos::conversations::ConversationRow as StoreConversationRow;
use pocket_store::Store;

/// Placeholder test keypair's public half ([7u8; 32], matching pocket-trust's
/// license tests) - NOT the real Keygen account key. See activate_license.
const PLACEHOLDER_KEYGEN_PUBLIC_KEY: [u8; 32] = [
    234, 74, 108, 99, 226, 156, 82, 10, 190, 245, 80, 123, 19, 46, 197, 249, 149, 71, 118, 174,
    190, 190, 123, 146, 66, 30, 234, 105, 20, 70, 210, 44,
];

/// Multi-thread, not current-thread: `server_start` spawns the axum server
/// as a background task and returns immediately. A current-thread runtime
/// only polls spawned tasks while something is actively inside `block_on`
/// on that same runtime - once `server_start`'s own `block_on` call
/// returned, the spawned server task would never be polled again and the
/// "running" server would silently stop accepting connections. A
/// multi-thread runtime has worker threads that keep driving it regardless.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build pocket-ffi server runtime")
    })
}

#[derive(Default)]
struct Inner {
    offline_mode: bool,
    entitlements: Entitlements,
    store_entitlement: Option<StoreEntitlement>,
    registered_engine: Option<Arc<dyn Engine>>,
    egress: Vec<EgressEntry>,
    server_handle: Option<pocket_server::ServerHandle>,
    download_plans: std::collections::HashMap<String, pocket_models::DownloadPlan>,
    tls_fingerprint: Option<String>,
    download_coordinator: Option<Arc<dyn DownloadCoordinator>>,
}

/// The public surface for Swift (A§4.5). Every method here is a stub per
/// L0-06: returns `PocketError::Internal{"unimplemented: <name>"}` or an
/// empty collection. Real behavior lands incrementally as each owning lane's
/// cards complete; `register_engine`/`loaded_models` and
/// `subscribe`/`on_memory_warning` are wired just enough to prove the
/// register-then-invoke round trip this card's "Done when" asks for.
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
    pub fn open(config: CoreConfig, platform: Arc<dyn PlatformHooks>) -> Result<Arc<Self>, PocketError> {
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
        Ok(Arc::new_cyclic(|self_weak| Self {
            config,
            platform,
            store,
            http: pocket_trust::PocketHttp::new(),
            pairing_state,
            inner: Mutex::new(Inner::default()),
            self_weak: self_weak.clone(),
        }))
    }

    // ---- device + fit ----

    pub fn device_profile(&self) -> DeviceProfile {
        self.platform.device_profile()
    }

    pub fn on_memory_warning(&self) {
        // Real behavior: cancel background work, unload idle models (ENG-04).
    }

    pub fn on_thermal_change(&self, _state: ThermalState) {
        // Real behavior: cap generation, emit thermal.changed (ENG-04).
    }

    // ---- engines ----

    pub fn register_engine(&self, engine: Arc<dyn Engine>) {
        self.inner.lock().unwrap().registered_engine = Some(engine);
    }

    pub fn register_stt(&self, _e: Arc<dyn SpeechToTextEngine>) {}
    pub fn register_tts(&self, _e: Arc<dyn TextToSpeechEngine>) {}
    pub fn register_image(&self, _e: Arc<dyn ImageEngine>) {}

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
        catalog
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
                if filter.installed_only && row.as_ref().map(|r| r.state.as_str()) != Some("installed") {
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
                    None,
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
            .collect()
    }

    /// User-initiated Hugging Face search (CORE-10), egress-recorded and
    /// refused in offline mode via `PocketHttp`. Results are uncurated with
    /// a placeholder fit report - real fit needs `config.json`, fetched
    /// lazily when the user opens a result (not implemented yet).
    pub fn search_hub(&self, query: String, format: ModelFormat) -> Result<Vec<CatalogEntry>, PocketError> {
        let format_str = match format {
            ModelFormat::Gguf => "gguf",
            ModelFormat::CoreMlPackage => "coreml",
            ModelFormat::WhisperKit => "whisperkit",
            ModelFormat::MlxSafetensors => "mlx",
        };
        let manifests = runtime()
            .block_on(pocket_models::search_hub(&self.http, "https://huggingface.co", &query, format_str))
            .map_err(|e| match e {
                pocket_types::PocketError::Network { message } => PocketError::Network { message },
                other => PocketError::Internal { message: other.to_string() },
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
        let manifest = self.find_manifest(&id).ok_or_else(|| PocketError::NotFound { what: id.clone() })?;
        let profile = self.platform.device_profile();
        let plan = pocket_models::plan_download(&manifest, &self.models_dir(), profile.free_disk_bytes).map_err(|e| {
            match e {
                pocket_models::DownloadError::DiskFull { needed_bytes, available_bytes } => PocketError::WillNotFit {
                    needed_bytes,
                    available_bytes,
                    advice: "Free up disk space before downloading this model".into(),
                },
                other => PocketError::Storage { message: other.to_string() },
            }
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
            .map_err(|e| PocketError::Storage { message: e.to_string() })?;

        let ffi_plan = DownloadPlan {
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

    pub fn record_download_progress(&self, id: ModelId, _file: String, bytes_done: u64) {
        if let Ok(Some(mut row)) = pocket_store::ModelsRepo::new(&self.store).get(&id) {
            row.bytes_done = bytes_done;
            let _ = pocket_store::ModelsRepo::new(&self.store).upsert(&row);
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
                Err(PocketError::Storage { message: e.to_string() })
            }
        }
    }

    pub fn delete_model(&self, id: ModelId) -> Result<(), PocketError> {
        if let Some(manifest) = self.find_manifest(&id) {
            let dir = self.models_dir().join(&manifest.hf_repo).join(&manifest.hf_revision);
            let _ = std::fs::remove_dir_all(dir);
        }
        pocket_store::ModelsRepo::new(&self.store)
            .delete(&id)
            .map_err(|e| PocketError::Storage { message: e.to_string() })?;
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

    pub fn unload(&self, _id: ModelId) -> Result<(), PocketError> {
        Err(unimplemented("unload"))
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
            .map_err(|e| PocketError::Storage { message: e.to_string() })?;
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
                parts: vec![ContentPart::Text {
                    text: extract_text_field(&m.content_json),
                }],
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

    pub fn send(&self, _conversation_id: Id, _input: UserInput, _sink: Arc<dyn ChatSink>) -> Result<Message, PocketError> {
        Err(unimplemented("send"))
    }

    pub fn stop(&self, _conversation_id: Id) {}

    pub fn delete_conversation(&self, id: Id) -> Result<(), PocketError> {
        pocket_store::ConversationsRepo::new(&self.store)
            .delete(&id)
            .map_err(|e| PocketError::Storage { message: e.to_string() })
    }

    // ---- projects + RAG ----

    pub fn add_document(&self, _project_id: Id, _path: String) -> Result<Document, PocketError> {
        Err(unimplemented("add_document"))
    }

    pub fn search_project(&self, _project_id: Id, _query: String, _k: u32) -> Result<Vec<ChunkHit>, PocketError> {
        Err(unimplemented("search_project"))
    }

    // ---- server ----

    /// Starts the Mac host's axum server (NET-01). Idempotent: if already
    /// running, returns the current status rather than starting a second
    /// listener. No-op-shaped on iOS at the FFI boundary too, since LAN
    /// serving is gated on Pro + the Mac entitlement check NET-01 enforces.
    pub fn server_start(&self, cfg: ServerConfig) -> Result<ServerStatus, PocketError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(handle) = &inner.server_handle {
            return Ok(ServerStatus {
                running: true,
                lan_enabled: handle.lan_enabled,
                port: handle.port,
                connected_devices: 0,
            });
        }
        let pro = inner.entitlements.pro;
        let server_cfg = pocket_server::ServerConfig {
            lan_enabled: cfg.lan_enabled,
            pro,
        };
        let routes = crate::pairing_routes::router(Arc::clone(&self.pairing_state))
            .merge(crate::control_routes::router(self.self_weak.clone()));
        let handle = runtime()
            .block_on(pocket_server::start_with_routes(server_cfg, "pocketcore".to_string(), routes))
            .map_err(|e| match e {
                pocket_types::PocketError::NotEntitled { feature } => PocketError::NotEntitled { feature },
                other => PocketError::Network { message: other.to_string() },
            })?;
        let status = ServerStatus {
            running: true,
            lan_enabled: handle.lan_enabled,
            port: handle.port,
            connected_devices: 0,
        };
        inner.server_handle = Some(handle);
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

    // ---- trust ----

    /// Generates a QR offer per A§7 (Pro-gated, 300s TTL, single-use
    /// secret). Does **not** yet turn on the LAN listener for the offer's
    /// duration or track pending-pairing state - that needs the `POST
    /// /pocket/v1/pair` route (NET-10's remaining, route-layer scope).
    pub fn pairing_offer(&self) -> Result<PairingOffer, PocketError> {
        if !self.inner.lock().unwrap().entitlements.pro {
            return Err(PocketError::NotEntitled { feature: "pairing".into() });
        }
        let cert_fingerprint = self.host_cert_fingerprint();
        let host_id = self.config.app_support_dir.clone();
        let now = self.platform.now_ms();
        let secret = pocket_trust::generate_secret();
        let expires_at_ms = now + 300_000;
        self.pairing_state.set_current_secret(secret.clone(), expires_at_ms);
        Ok(PairingOffer {
            host_id: host_id.clone(),
            host_name: self.platform.device_profile().model_identifier,
            addrs: vec![format!("{host_id}.local")],
            port: self.server_status().port,
            cert_fingerprint,
            secret,
            expires_at_ms,
        })
    }

    pub fn parse_pairing_qr(&self, qr: String) -> Result<PairingOffer, PocketError> {
        let parsed = pocket_trust::parse_pairing_uri(&qr).map_err(|e| PocketError::InvalidInput {
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
            name: approved.host_id.clone(),
            cert_sha256: approved.cert_fingerprint.clone().into_bytes(),
            addrs_json: serde_json::to_string(&[&approved.addr]).unwrap_or_default(),
            port: approved.port,
            token_ref: approved.token_keychain_ref.clone(),
            paired_at: now,
            last_seen_at: None,
        };
        pocket_store::PeersRepo::new(&self.store)
            .create(&row)
            .map_err(|e| PocketError::Storage { message: e.to_string() })?;
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
        self.pairing_state.approve(&id).map_err(|e| PocketError::NotFound { what: e.to_string() })
    }

    pub fn deny_pairing(&self, id: Id) -> Result<(), PocketError> {
        self.pairing_state.deny(&id).map_err(|e| PocketError::NotFound { what: e.to_string() })
    }

    pub fn devices(&self) -> Vec<PairedDevice> {
        DevicesRepo::new(&self.store)
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|d| PairedDevice {
                id: d.id,
                name: d.name,
                platform: if d.platform == "ios" { Platform::Ios } else { Platform::MacOs },
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
        repo.revoke(&id, now).map_err(|e| PocketError::Storage { message: e.to_string() })?;
        self.store
            .writer()
            .execute("UPDATE devices SET token_hash = x'' WHERE id = ?1", [&id])
            .map_err(|e| PocketError::Storage { message: e.to_string() })?;
        Ok(())
    }

    pub fn peers(&self) -> Vec<Peer> {
        pocket_store::PeersRepo::new(&self.store)
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|p| Peer {
                id: p.id,
                name: p.name,
                cert_sha256: String::from_utf8_lossy(&p.cert_sha256).into_owned(),
                addrs: serde_json::from_str(&p.addrs_json).unwrap_or_default(),
                port: p.port,
                token_ref: p.token_ref,
                paired_at: p.paired_at,
                last_seen_at: p.last_seen_at,
            })
            .collect()
    }

    pub fn stop_everything(&self) {
        let _ = self.server_stop();
    }

    pub fn register_download_coordinator(&self, coordinator: Arc<dyn DownloadCoordinator>) {
        self.inner.lock().unwrap().download_coordinator = Some(coordinator);
    }

    // ---- entitlements ----

    /// Verifies a Keygen `ED25519_SIGN` license key fully offline (LIC-02)
    /// against a **placeholder test public key** - LIC-01 (the real Keygen
    /// account) is an external gate (DECISIONS.md D26); swap
    /// `PLACEHOLDER_KEYGEN_PUBLIC_KEY` for the real one before shipping.
    pub fn activate_license(&self, key: String) -> Result<Entitlements, PocketError> {
        let public_key = ed25519_dalek::VerifyingKey::from_bytes(&PLACEHOLDER_KEYGEN_PUBLIC_KEY)
            .map_err(|e| PocketError::Internal { message: e.to_string() })?;
        let payload = pocket_trust::verify_license_key(&key, &public_key, 1, &[])
            .map_err(|e| PocketError::InvalidInput { message: e.to_string() })?;

        let entitlements = Entitlements {
            pro: true,
            source: format!("license:{}", payload.serial),
            expires_at_ms: None,
        };
        self.inner.lock().unwrap().entitlements = entitlements.clone();
        Ok(entitlements)
    }

    pub fn set_store_entitlement(&self, e: StoreEntitlement) {
        self.inner.lock().unwrap().store_entitlement = Some(e);
    }

    pub fn entitlements(&self) -> Entitlements {
        self.inner.lock().unwrap().entitlements.clone()
    }

    // ---- privacy ----

    pub fn set_offline_mode(&self, on: bool) {
        self.inner.lock().unwrap().offline_mode = on;
        self.http.set_offline_mode(on);
    }

    pub fn egress_log(&self, limit: u32) -> Vec<EgressEntry> {
        let inner = self.inner.lock().unwrap();
        inner.egress.iter().rev().take(limit as usize).cloned().collect()
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
            .map_err(|e| PocketError::Storage { message: e.to_string() })
    }

    // ---- events ----

    pub fn subscribe(&self, _sink: Arc<dyn EventSink>) -> Subscription {
        Subscription {
            id: "unimplemented".into(),
        }
    }
}

/// Plain (non-FFI-exported) helpers - a `#[uniffi::export] impl` block
/// exports every method in it, so these live in a separate `impl`.
impl PocketCore {
    fn find_manifest(&self, id: &ModelId) -> Option<pocket_models::ModelManifest> {
        pocket_models::load().entries.into_iter().find(|m| &m.id == id)
    }

    fn models_dir(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.config.app_support_dir).join("Models")
    }

    /// Generates this host's TLS identity once and caches its fingerprint;
    /// the private key itself should move to Keychain via `PlatformHooks`
    /// once NET-02's TLS identity is wired into the actual server listener
    /// (currently `pocket_server::start` generates its own ad hoc, separate
    /// from this fingerprint - unifying them is part of finishing NET-10).
    fn host_cert_fingerprint(&self) -> String {
        let mut inner = self.inner.lock().unwrap();
        if let Some(fp) = &inner.tls_fingerprint {
            return fp.clone();
        }
        let identity = pocket_trust::generate_identity(&format!("{}.local", self.config.app_support_dir), &[]);
        inner.tls_fingerprint = Some(identity.fingerprint_b64url.clone());
        identity.fingerprint_b64url
    }

    /// NET-12: the control API calls `plan_download` then hands the plan to
    /// the registered Swift `DownloadCoordinator` (APL-05 implements it with
    /// a real background `URLSession`; no coordinator registered yet means
    /// this errors rather than silently no-op-ing).
    pub(crate) fn start_download_via_coordinator(&self, plan: DownloadPlan) -> Result<(), PocketError> {
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

fn new_ulid() -> String {
    ulid::Ulid::new().to_string()
}

fn extract_text_field(content_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content_json)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_default()
}
