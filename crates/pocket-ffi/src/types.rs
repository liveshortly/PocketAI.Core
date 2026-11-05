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

#[derive(uniffi::Enum, Debug)]
pub enum Platform {
    Ios,
    MacOs,
}

#[derive(uniffi::Enum, Debug)]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
}

#[derive(uniffi::Record, Debug)]
pub struct DeviceProfile {
    pub platform: Platform,
    pub model_identifier: String,
    /// UI-24: user-facing device name (e.g. "Rohit's MacBook Pro"), from
    /// `Host.current().localizedName` / `UIDevice.current.name`. Empty when
    /// the platform layer has none; consumers fall back to `model_identifier`.
    pub device_name: String,
    pub chip: String,
    pub total_ram_bytes: u64,
    pub app_memory_limit_bytes: u64,
    pub gpu_family: String,
    pub free_disk_bytes: u64,
    pub thermal_state: ThermalState,
    pub low_power_mode: bool,
    pub has_increased_memory_entitlement: bool,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub enum Modality {
    Text,
    Vision,
    Embedding,
    SpeechToText,
    TextToSpeech,
    Image,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(uniffi::Enum, Debug, Clone)]
pub enum ContentPart {
    Text { text: String },
    ImageRef { path: String },
}

#[derive(uniffi::Record, Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub parts: Vec<ContentPart>,
}

#[derive(uniffi::Record, Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters_json_schema: String,
}

#[derive(uniffi::Record, Debug)]
pub struct ToolCallDelta {
    pub id: String,
    pub name: String,
    pub arguments_json_fragment: String,
}

#[derive(uniffi::Enum, Debug, Clone)]
pub enum ReasoningMode {
    Off,
    On,
    Auto,
}

#[derive(uniffi::Record, Debug, Clone)]
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

#[derive(uniffi::Record, Debug, Clone)]
pub struct GenerateRequest {
    pub request_id: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub params: SamplingParams,
    pub reasoning: ReasoningMode,
}

#[derive(uniffi::Record, Debug)]
pub struct GenerateDelta {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCallDelta>,
}

#[derive(uniffi::Enum, Debug, Clone)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Cancelled,
    Error,
}

#[derive(uniffi::Record, Debug, Clone)]
pub struct GenerateStats {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub ttft_ms: u32,
    pub decode_tps: f32,
    pub finish: FinishReason,
}

#[derive(uniffi::Record, Debug)]
pub struct UserInput {
    pub parts: Vec<ContentPart>,
}

#[derive(uniffi::Enum, Debug, Clone)]
pub enum MessageStatus {
    Streaming,
    Done,
    Error,
    Cancelled,
}

#[derive(uniffi::Record, Debug, Clone)]
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
    /// AGT-12: web tool steps run for this reply ("Searched" / "Read" rows).
    #[uniffi(default)]
    pub tool_steps: Vec<ToolStep>,
    /// AGT-12: URLs of the pages read for this reply, cited under the answer.
    #[uniffi(default)]
    pub sources: Vec<String>,
}

/// AGT-12: one tool run during a reply. `kind` is "searched" (label = the
/// query) or "read" (label = the site's host); `detail` is what the
/// expanded row shows (result titles and URLs, or the page URL).
#[derive(uniffi::Record, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolStep {
    pub kind: String,
    pub label: String,
    pub detail: String,
}

/// UI-146: what a reply is doing right now, for the live status line. Sent
/// at each transition through `ChatSink::on_status`; the UI adds elapsed time.
#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum ChatPhase {
    /// The model is being loaded (or reused) before the first token.
    Loading,
    /// Prompt prefill / reasoning stream, nothing visible yet.
    Thinking,
    /// The first visible answer text is streaming.
    Writing,
    /// A tool is running: `name` is the tool, `arg` its main argument
    /// (the query, the URL's host, the expression) or empty.
    Tool { name: String, arg: String },
    /// A "Run this?" card is waiting for the user (`tool` is the tool name).
    AwaitingApproval { tool: String },
    /// Approved code is running in the sandbox (`tool` is the tool name).
    Running { tool: String },
}

/// AGT-13: which tool families the user allows in Settings -> Tools, asked of
/// the registered `ToolExecutor` at the start of each reply. Both false (or no
/// executor) means nothing but the chat's own Tools switch decides.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ToolAvailability {
    /// Web search + read page.
    pub web: bool,
    /// "Run code on this iPhone": `run_js`. The Swift side folds the master
    /// "Allow tools" switch into this.
    pub javascript: bool,
    /// "Run Python on this iPhone": `run_python` (Pyodide in a web view).
    /// Same master switch, its own toggle; always asks, never "always allow".
    pub python: bool,
    /// "Run on my Mac": `run_on_mac`. Swift sets it only when the master
    /// switch, this toggle, and a paired *and currently connected* Mac all
    /// hold (the chat's Tools chip is checked by Core). Always asks.
    pub mac: bool,
}

/// AGT-13: what a sandbox run produced. Swift caps `output` at 20 KB
/// (`truncated`) and enforces the timeout (`ok` false, `error` says so).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ToolRunResult {
    pub ok: bool,
    pub output: String,
    pub error: String,
    pub duration_ms: u64,
    pub truncated: bool,
}

/// AGT-13: a "Run this?" card for the UI. `id` is minted by Core and is the
/// only handle that can answer it (`PocketCore::resolve_approval`).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ApprovalRequest {
    pub id: String,
    pub conversation_id: String,
    pub tool: String,
    pub summary: String,
    pub code: String,
}

#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum ApprovalDecision {
    Run,
    RunEdited { code: String },
    Deny,
}

#[derive(uniffi::Record, Debug)]
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

#[derive(uniffi::Record, Debug)]
pub struct ConversationSummary {
    pub id: Id,
    pub title: Option<String>,
    pub updated_at: i64,
    pub archived: bool,
}

#[derive(uniffi::Record, Debug)]
pub struct ConversationQuery {
    pub search: Option<String>,
    pub archived: Option<bool>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(uniffi::Record, Debug)]
pub struct NewConversation {
    pub title: Option<String>,
    pub project_id: Option<Id>,
    pub model_id: Option<String>,
}

// ---- models / catalog / fit ----

#[derive(uniffi::Enum, Clone, Copy, Debug, serde::Serialize)]
pub enum ModelFormat {
    MlxSafetensors,
    Gguf,
    CoreMlPackage,
    WhisperKit,
}

#[derive(uniffi::Record, Clone, Debug, serde::Serialize)]
pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
}

#[derive(uniffi::Record, Clone, Debug, serde::Serialize)]
pub struct LicenseInfo {
    pub spdx: String,
    pub commercial_ok: bool,
    pub url: String,
}

#[derive(uniffi::Record, Clone, Debug, serde::Serialize)]
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
    /// CORE-14: true when the model emits thinking/reasoning tokens. Curated
    /// catalog entries carry it from catalog.json; uncurated ids derive it
    /// from `pocket_models::supports_reasoning`.
    pub reasoning: bool,
}

#[derive(uniffi::Enum, Debug, serde::Serialize)]
pub enum FitVerdict {
    RunsWell,
    RunsSlow,
    WillNotFit,
    NotSupported,
}

#[derive(uniffi::Enum, Debug, serde::Serialize)]
pub enum Confidence {
    Estimated,
    Measured,
}

#[derive(uniffi::Record, Debug)]
pub struct Benchmark {
    pub tps: f32,
    pub ttft_ms: u32,
    pub peak_bytes: u64,
}

#[derive(uniffi::Record, Debug, serde::Serialize)]
pub struct FitReport {
    pub verdict: FitVerdict,
    pub reason: String,
    pub est_tokens_per_sec: Option<f32>,
    pub measured_tokens_per_sec: Option<f32>,
    pub est_runtime_bytes: u64,
    pub disk_bytes: u64,
    pub confidence: Confidence,
}

#[derive(uniffi::Record, Default, Debug)]
pub struct CatalogFilter {
    pub modality: Option<Modality>,
    pub curated_only: bool,
    pub installed_only: bool,
}

#[derive(uniffi::Enum, Debug, serde::Serialize)]
pub enum ModelState {
    Available,
    Queued,
    Downloading,
    Paused,
    Verifying,
    Installed,
    Failed { reason: String },
}

#[derive(uniffi::Record, Debug, serde::Serialize)]
pub struct CatalogEntry {
    pub manifest: ModelManifest,
    pub fit: FitReport,
    pub state: ModelState,
    pub bytes_done: u64,
}

#[derive(uniffi::Record, Debug)]
pub struct LoadedModelInfo {
    pub handle: String,
    pub id: ModelId,
    pub engine: EngineKind,
    pub resident_bytes: u64,
}

#[derive(uniffi::Record, Default, Debug)]
pub struct LoadOptions {
    pub context_length: Option<u32>,
    pub pinned_engine: Option<EngineKind>,
}

#[derive(uniffi::Record, Debug)]
pub struct LoadedModel {
    pub handle: String,
}

#[derive(uniffi::Record, Debug)]
pub struct PlannedFile {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub dest_part: String,
    pub dest_final: String,
}

#[derive(uniffi::Record, Debug)]
pub struct DownloadPlan {
    pub model_id: String,
    pub files: Vec<PlannedFile>,
    pub total_bytes: u64,
    pub dest_dir: String,
}

// ---- projects / RAG ----

#[derive(uniffi::Record, Debug)]
pub struct Project {
    pub id: Id,
    pub name: String,
    pub instructions: Option<String>,
    pub created_at: i64,
}

#[derive(uniffi::Record, Debug)]
pub struct DocumentPage {
    pub page: u32,
    pub text: String,
}

#[derive(uniffi::Record, Debug)]
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

#[derive(uniffi::Record, Debug)]
pub struct ChunkHit {
    pub document_id: Id,
    pub ord: u32,
    pub text: String,
    pub page: Option<u32>,
    pub score: f32,
}

// ---- trust / pairing / licensing ----

/// Read-only host identity for Bonjour advertising (APL-08): the subset of
/// `PairingOffer` that identifies this host, with no secret. Reading it never
/// rotates the single-use pairing secret and is not Pro-gated.
#[derive(uniffi::Record, Debug)]
pub struct PairingIdentity {
    pub host_id: String,
    pub host_name: String,
    pub cert_fingerprint: String,
}

#[derive(uniffi::Record, Debug)]
pub struct PairingOffer {
    pub host_id: String,
    pub host_name: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub cert_fingerprint: String,
    pub secret: String,
    pub expires_at_ms: i64,
}

#[derive(uniffi::Record, Debug)]
pub struct ApprovedPairing {
    pub host_id: String,
    /// UI-24: user-facing host name from the pairing offer. Empty means the
    /// caller predates host_name; `save_peer` then falls back to `host_id`.
    pub host_name: String,
    pub cert_fingerprint: String,
    pub addr: String,
    pub port: u16,
    pub device_id: Id,
    pub token_keychain_ref: String,
}

#[derive(uniffi::Record, Debug)]
pub struct PairingRequest {
    pub id: String,
    pub device_name: String,
    pub platform: String,
    /// NET-23: for a manual-code request, the 6-digit SAS the user must see
    /// on the iPhone too before tapping Allow. `None` for QR pairing.
    pub sas: Option<String>,
}

/// NET-23: what the Mac shows under "Enter code manually".
#[derive(uniffi::Record, Debug)]
pub struct ManualPairingCode {
    /// 8 characters from `23456789ABCDEFGHJKLMNPQRSTUVWXYZ`.
    pub code: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub expires_at_ms: i64,
}

#[derive(uniffi::Record, Debug)]
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

#[derive(uniffi::Record, Debug)]
pub struct PairedDevice {
    pub id: Id,
    pub name: String,
    pub platform: Platform,
    pub role: String,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
    pub last_addr: Option<String>,
    pub revoked_at: Option<i64>,
    /// NET-17/18: the device's live session as this Mac sees it.
    pub link_state: DeviceLinkState,
}

/// NET-18 lease: Connected while the phone was heard from in the last 45 s,
/// Idle after that (or right after a background goodbye), Disconnected after
/// 5 min or when the phone tapped Disconnect. `DisconnectedByHost` is the
/// Mac's own per-device Disconnect (NET-17), held until the phone reconnects.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceLinkState {
    Connected,
    Idle,
    Disconnected,
    DisconnectedByHost,
}

#[derive(uniffi::Record, Clone, Debug)]
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

#[derive(uniffi::Record, Debug)]
pub struct StoreEntitlement {
    pub pro: bool,
    pub transaction_id: Option<String>,
}

#[derive(uniffi::Record, Default, Debug)]
pub struct AuditQuery {
    pub limit: u32,
    pub since_ts: Option<i64>,
}

#[derive(uniffi::Record, Debug)]
pub struct AuditEntry {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail_json: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct EgressEntry {
    pub ts: i64,
    pub host: String,
    pub purpose: String,
    pub bytes_out: u64,
    pub bytes_in: u64,
    pub initiated_by: String,
}

// ---- server / events ----

#[derive(uniffi::Record, Debug)]
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

/// MAC-23: today's totals for the Server page (local day, in memory).
#[derive(uniffi::Record, Debug)]
pub struct ServerStats {
    pub requests_today: u64,
    pub tokens_out_today: u64,
    pub avg_tokens_per_sec: f64,
}

/// MAC-23: one completed API request. `client` is a paired device's name,
/// "API key" (loopback caller with the local API key) or "local".
#[derive(uniffi::Record, Debug)]
pub struct RequestLogEntry {
    pub ts_ms: i64,
    pub client: String,
    pub endpoint: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub duration_ms: u32,
    pub status: u16,
}

#[derive(uniffi::Record, Debug)]
pub struct Event {
    pub v: u32,
    pub id: String,
    pub seq: u64,
    pub ts: i64,
    pub source: String,
    pub event_type: String,
    pub data_json: String,
}

#[derive(uniffi::Record, Debug)]
pub struct Subscription {
    pub id: String,
}

#[derive(uniffi::Record, Debug)]
pub struct CoreConfig {
    pub app_support_dir: String,
    pub platform: Platform,
}

/// NET-22: what one `sync_now` moved.
#[derive(uniffi::Record, Debug, Clone)]
pub struct SyncReport {
    pub pulled: u32,
    pub pushed: u32,
    pub blobs_fetched: u32,
    pub blobs_sent: u32,
    pub synced_at: i64,
}

/// NET-22: "Not synced · last synced 12:40" and the per-chat pending dot.
#[derive(uniffi::Record, Debug, Clone)]
pub struct SyncStatus {
    pub last_synced_at: Option<i64>,
    /// This device's ops the peer hasn't received (the outbox).
    pub pending_ops: u64,
    pub pending_conversation_ids: Vec<Id>,
}
