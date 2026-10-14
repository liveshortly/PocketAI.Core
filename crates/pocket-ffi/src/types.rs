//! FFI-facing records/enums mirroring `01-architecture.md` §4.1-4.6.
//!
//! These are hand-written, not derived from `pocket-types`, because sharing
//! `#[derive(uniffi::Record)]` types across crates needs per-crate scaffolding
//! that doesn't compose the way the architecture doc assumed (DECISIONS.md
//! D19). `ModelId`/`Id` are plain `String` here, matching the architecture
//! doc's own `pub type Id = String` convention, to avoid a redundant wrapper
//! at the boundary where nothing yet parses it.

pub type Id = String;
pub type ModelId = String;

// ---- device ----

#[derive(uniffi::Enum)]
pub enum Platform {
    Ios,
    MacOs,
}

#[derive(uniffi::Enum)]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
}

#[derive(uniffi::Record)]
pub struct DeviceProfile {
    pub platform: Platform,
    pub model_identifier: String,
    pub chip: String,
    pub total_ram_bytes: u64,
    pub app_memory_limit_bytes: u64,
    pub gpu_family: String,
    pub free_disk_bytes: u64,
    pub thermal_state: ThermalState,
    pub low_power_mode: bool,
    pub has_increased_memory_entitlement: bool,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq)]
pub enum Modality {
    Text,
    Vision,
    Embedding,
    SpeechToText,
    TextToSpeech,
    Image,
}

#[derive(uniffi::Enum, Clone, Copy, Debug)]
pub enum EngineKind {
    Mlx,
    LlamaCpp,
    CoreMl,
    WhisperKit,
    AppleSpeech,
    AppleFoundation,
    Remote,
}

// ---- engine / chat ----

#[derive(uniffi::Enum)]
pub enum ContentPart {
    Text { text: String },
    ImageRef { path: String },
}

#[derive(uniffi::Record)]
pub struct ChatMessage {
    pub role: String,
    pub parts: Vec<ContentPart>,
}

#[derive(uniffi::Record)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters_json_schema: String,
}

#[derive(uniffi::Record)]
pub struct ToolCallDelta {
    pub id: String,
    pub name: String,
    pub arguments_json_fragment: String,
}

#[derive(uniffi::Enum)]
pub enum ReasoningMode {
    Off,
    On,
    Auto,
}

#[derive(uniffi::Record)]
pub struct SamplingParams {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub min_p: f32,
    pub max_tokens: u32,
    pub stop: Vec<String>,
    pub seed: Option<i64>,
    pub repetition_penalty: f32,
}

#[derive(uniffi::Record)]
pub struct GenerateRequest {
    pub request_id: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub params: SamplingParams,
    pub reasoning: ReasoningMode,
}

#[derive(uniffi::Record)]
pub struct GenerateDelta {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCallDelta>,
}

#[derive(uniffi::Enum)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Cancelled,
    Error,
}

#[derive(uniffi::Record)]
pub struct GenerateStats {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub ttft_ms: u32,
    pub decode_tps: f32,
    pub finish: FinishReason,
}

#[derive(uniffi::Record)]
pub struct UserInput {
    pub parts: Vec<ContentPart>,
}

#[derive(uniffi::Enum)]
pub enum MessageStatus {
    Streaming,
    Done,
    Error,
    Cancelled,
}

#[derive(uniffi::Record)]
pub struct Message {
    pub id: Id,
    pub conversation_id: Id,
    pub role: String,
    pub parts: Vec<ContentPart>,
    pub reasoning: Option<String>,
    pub stats: Option<GenerateStats>,
    pub model_id: Option<String>,
    pub created_at: i64,
    pub status: MessageStatus,
}

#[derive(uniffi::Record)]
pub struct Conversation {
    pub id: Id,
    pub title: Option<String>,
    pub project_id: Option<Id>,
    pub model_id: Option<String>,
    pub system_prompt: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
}

#[derive(uniffi::Record)]
pub struct ConversationSummary {
    pub id: Id,
    pub title: Option<String>,
    pub updated_at: i64,
    pub archived: bool,
}

#[derive(uniffi::Record)]
pub struct ConversationQuery {
    pub search: Option<String>,
    pub archived: Option<bool>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(uniffi::Record)]
pub struct NewConversation {
    pub title: Option<String>,
    pub project_id: Option<Id>,
    pub model_id: Option<String>,
}

// ---- models / catalog / fit ----

#[derive(uniffi::Enum, Clone, Copy)]
pub enum ModelFormat {
    MlxSafetensors,
    Gguf,
    CoreMlPackage,
    WhisperKit,
}

#[derive(uniffi::Record)]
pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
}

#[derive(uniffi::Record)]
pub struct LicenseInfo {
    pub spdx: String,
    pub commercial_ok: bool,
    pub url: String,
}

#[derive(uniffi::Record)]
pub struct ModelManifest {
    pub id: ModelId,
    pub display_name: String,
    pub family: String,
    pub params_b: f32,
    pub quant: String,
    pub format: ModelFormat,
    pub modalities: Vec<Modality>,
    pub context_length: u32,
    pub files: Vec<ModelFile>,
    pub license: LicenseInfo,
    pub hf_repo: String,
    pub hf_revision: String,
    pub min_ram_bytes: u64,
    pub est_runtime_bytes: u64,
    pub tags: Vec<String>,
    pub curated: bool,
}

#[derive(uniffi::Enum)]
pub enum FitVerdict {
    RunsWell,
    RunsSlow,
    WillNotFit,
    NotSupported,
}

#[derive(uniffi::Enum)]
pub enum Confidence {
    Estimated,
    Measured,
}

#[derive(uniffi::Record)]
pub struct Benchmark {
    pub tps: f32,
    pub ttft_ms: u32,
    pub peak_bytes: u64,
}

#[derive(uniffi::Record)]
pub struct FitReport {
    pub verdict: FitVerdict,
    pub reason: String,
    pub est_tokens_per_sec: Option<f32>,
    pub measured_tokens_per_sec: Option<f32>,
    pub est_runtime_bytes: u64,
    pub disk_bytes: u64,
    pub confidence: Confidence,
}

#[derive(uniffi::Record, Default)]
pub struct CatalogFilter {
    pub modality: Option<Modality>,
    pub curated_only: bool,
    pub installed_only: bool,
}

#[derive(uniffi::Enum)]
pub enum ModelState {
    Available,
    Queued,
    Downloading,
    Paused,
    Verifying,
    Installed,
    Failed { reason: String },
}

#[derive(uniffi::Record)]
pub struct CatalogEntry {
    pub manifest: ModelManifest,
    pub fit: FitReport,
    pub state: ModelState,
    pub bytes_done: u64,
}

#[derive(uniffi::Record)]
pub struct LoadedModelInfo {
    pub handle: String,
    pub id: ModelId,
    pub engine: EngineKind,
    pub resident_bytes: u64,
}

#[derive(uniffi::Record, Default)]
pub struct LoadOptions {
    pub context_length: Option<u32>,
    pub pinned_engine: Option<EngineKind>,
}

#[derive(uniffi::Record)]
pub struct LoadedModel {
    pub handle: String,
}

#[derive(uniffi::Record)]
pub struct PlannedFile {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub dest_part: String,
    pub dest_final: String,
}

#[derive(uniffi::Record)]
pub struct DownloadPlan {
    pub files: Vec<PlannedFile>,
    pub total_bytes: u64,
    pub dest_dir: String,
}

// ---- projects / RAG ----

#[derive(uniffi::Record)]
pub struct Document {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    pub mime: String,
    pub bytes: u64,
    pub path: String,
    pub text_chars: u32,
    pub embed_model: Option<String>,
    pub state: String,
    pub created_at: i64,
}

#[derive(uniffi::Record)]
pub struct ChunkHit {
    pub document_id: Id,
    pub ord: u32,
    pub text: String,
    pub page: Option<u32>,
    pub score: f32,
}

// ---- trust / pairing / licensing ----

#[derive(uniffi::Record)]
pub struct PairingOffer {
    pub host_id: String,
    pub host_name: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub cert_fingerprint: String,
    pub secret: String,
    pub expires_at_ms: i64,
}

#[derive(uniffi::Record)]
pub struct ApprovedPairing {
    pub host_id: String,
    pub cert_fingerprint: String,
    pub addr: String,
    pub port: u16,
    pub device_id: Id,
    pub token_keychain_ref: String,
}

#[derive(uniffi::Record)]
pub struct Peer {
    pub id: Id,
    pub name: String,
    pub cert_sha256: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub token_ref: String,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
}

#[derive(uniffi::Record)]
pub struct PairedDevice {
    pub id: Id,
    pub name: String,
    pub platform: Platform,
    pub role: String,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
    pub last_addr: Option<String>,
    pub revoked_at: Option<i64>,
}

#[derive(uniffi::Record, Clone)]
pub struct Entitlements {
    pub pro: bool,
    pub source: String,
    pub expires_at_ms: Option<i64>,
}

impl Default for Entitlements {
    fn default() -> Self {
        Self {
            pro: false,
            source: "none".into(),
            expires_at_ms: None,
        }
    }
}

#[derive(uniffi::Record)]
pub struct StoreEntitlement {
    pub pro: bool,
    pub transaction_id: Option<String>,
}

#[derive(uniffi::Record, Default)]
pub struct AuditQuery {
    pub limit: u32,
    pub since_ts: Option<i64>,
}

#[derive(uniffi::Record)]
pub struct AuditEntry {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail_json: String,
}

#[derive(uniffi::Record, Clone)]
pub struct EgressEntry {
    pub ts: i64,
    pub host: String,
    pub purpose: String,
    pub bytes_out: u64,
    pub bytes_in: u64,
    pub initiated_by: String,
}

// ---- server / events ----

#[derive(uniffi::Record)]
pub struct ServerConfig {
    pub lan_enabled: bool,
    pub port: u16,
}

#[derive(uniffi::Record, Debug)]
pub struct ServerStatus {
    pub running: bool,
    pub lan_enabled: bool,
    pub port: u16,
    pub connected_devices: u32,
}

#[derive(uniffi::Record)]
pub struct Event {
    pub v: u32,
    pub id: String,
    pub seq: u64,
    pub ts: i64,
    pub source: String,
    pub event_type: String,
    pub data_json: String,
}

#[derive(uniffi::Record)]
pub struct Subscription {
    pub id: String,
}

#[derive(uniffi::Record)]
pub struct CoreConfig {
    pub app_support_dir: String,
    pub platform: Platform,
}
