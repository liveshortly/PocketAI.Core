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

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
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
    store: Store,
    inner: Mutex<Inner>,
}

#[uniffi::export]
impl PocketCore {
    #[uniffi::constructor]
    pub fn open(config: CoreConfig, platform: Arc<dyn PlatformHooks>) -> Result<Arc<Self>, PocketError> {
        std::fs::create_dir_all(&config.app_support_dir).map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?;
        let db_path = std::path::Path::new(&config.app_support_dir).join("pocket.sqlite");
        let store = Store::open(&db_path).map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?;
        Ok(Arc::new(Self {
            config,
            platform,
            store,
            inner: Mutex::new(Inner::default()),
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

    pub fn search_hub(&self, _query: String, _format: ModelFormat) -> Result<Vec<CatalogEntry>, PocketError> {
        Err(unimplemented("search_hub"))
    }

    pub fn plan_download(&self, _id: ModelId) -> Result<DownloadPlan, PocketError> {
        Err(unimplemented("plan_download"))
    }

    pub fn record_download_progress(&self, _id: ModelId, _file: String, _bytes_done: u64) {}

    pub fn verify_and_commit(&self, _id: ModelId) -> Result<(), PocketError> {
        Err(unimplemented("verify_and_commit"))
    }

    pub fn delete_model(&self, _id: ModelId) -> Result<(), PocketError> {
        Err(unimplemented("delete_model"))
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
        let handle = runtime()
            .block_on(pocket_server::start(server_cfg, "pocketcore".to_string()))
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

    pub fn pairing_offer(&self) -> Result<PairingOffer, PocketError> {
        Err(unimplemented("pairing_offer"))
    }

    pub fn parse_pairing_qr(&self, _qr: String) -> Result<PairingOffer, PocketError> {
        Err(unimplemented("parse_pairing_qr"))
    }

    pub fn save_peer(&self, _approved: ApprovedPairing) -> Result<Peer, PocketError> {
        Err(unimplemented("save_peer"))
    }

    pub fn approve_pairing(&self, _id: Id) -> Result<(), PocketError> {
        Err(unimplemented("approve_pairing"))
    }

    pub fn deny_pairing(&self, _id: Id) -> Result<(), PocketError> {
        Err(unimplemented("deny_pairing"))
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
        Vec::new()
    }

    pub fn stop_everything(&self) {
        let _ = self.server_stop();
    }

    pub fn register_download_coordinator(&self, _coordinator: Arc<dyn DownloadCoordinator>) {}

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

fn new_ulid() -> String {
    ulid::Ulid::new().to_string()
}

fn extract_text_field(content_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content_json)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_default()
}
