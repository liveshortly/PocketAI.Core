use std::sync::{Arc, Mutex};

use crate::error::{unimplemented, PocketError};
use crate::traits::*;
use crate::types::*;

#[derive(Default)]
struct Inner {
    offline_mode: bool,
    entitlements: Entitlements,
    store_entitlement: Option<StoreEntitlement>,
    registered_engine: Option<Arc<dyn Engine>>,
    egress: Vec<EgressEntry>,
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
    inner: Mutex<Inner>,
}

#[uniffi::export]
impl PocketCore {
    #[uniffi::constructor]
    pub fn open(config: CoreConfig, platform: Arc<dyn PlatformHooks>) -> Result<Arc<Self>, PocketError> {
        Ok(Arc::new(Self {
            config,
            platform,
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

    pub fn catalog(&self, _filter: CatalogFilter) -> Vec<CatalogEntry> {
        Vec::new()
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

    pub fn conversations(&self, _q: ConversationQuery) -> Vec<ConversationSummary> {
        Vec::new()
    }

    pub fn create_conversation(&self, _input: NewConversation) -> Result<Conversation, PocketError> {
        Err(unimplemented("create_conversation"))
    }

    pub fn messages(&self, _conversation_id: Id) -> Vec<Message> {
        Vec::new()
    }

    pub fn send(&self, _conversation_id: Id, _input: UserInput, _sink: Arc<dyn ChatSink>) -> Result<Message, PocketError> {
        Err(unimplemented("send"))
    }

    pub fn stop(&self, _conversation_id: Id) {}

    pub fn delete_conversation(&self, _id: Id) -> Result<(), PocketError> {
        Err(unimplemented("delete_conversation"))
    }

    // ---- projects + RAG ----

    pub fn add_document(&self, _project_id: Id, _path: String) -> Result<Document, PocketError> {
        Err(unimplemented("add_document"))
    }

    pub fn search_project(&self, _project_id: Id, _query: String, _k: u32) -> Result<Vec<ChunkHit>, PocketError> {
        Err(unimplemented("search_project"))
    }

    // ---- server ----

    pub fn server_start(&self, _cfg: ServerConfig) -> Result<ServerStatus, PocketError> {
        Err(unimplemented("server_start"))
    }

    pub fn server_stop(&self) -> Result<(), PocketError> {
        Err(unimplemented("server_stop"))
    }

    pub fn server_status(&self) -> ServerStatus {
        ServerStatus {
            running: false,
            lan_enabled: false,
            port: 0,
            connected_devices: 0,
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
        Vec::new()
    }

    pub fn revoke_device(&self, _id: Id) -> Result<(), PocketError> {
        Err(unimplemented("revoke_device"))
    }

    pub fn peers(&self) -> Vec<Peer> {
        Vec::new()
    }

    pub fn stop_everything(&self) {}

    pub fn register_download_coordinator(&self, _coordinator: Arc<dyn DownloadCoordinator>) {}

    // ---- entitlements ----

    pub fn activate_license(&self, _key: String) -> Result<Entitlements, PocketError> {
        Err(unimplemented("activate_license"))
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

    pub fn audit_log(&self, _q: AuditQuery) -> Vec<AuditEntry> {
        Vec::new()
    }

    pub fn export_audit(&self, _path: String) -> Result<(), PocketError> {
        Err(unimplemented("export_audit"))
    }

    // ---- events ----

    pub fn subscribe(&self, _sink: Arc<dyn EventSink>) -> Subscription {
        Subscription {
            id: "unimplemented".into(),
        }
    }
}
