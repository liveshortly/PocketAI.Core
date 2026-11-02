import Foundation

/// The one protocol PocketAI's apps program against (A§4.6). `LiveCore` wraps
/// the generated UniFFI `PocketCore`; `PreviewCore` is deterministic fixtures
/// so UI lanes never wait for Rust.
///
/// Covers the slice of A§4.5's surface actually needed by what's built so
/// far (catalog, conversations, chat, pairing, devices, entitlements,
/// server lifecycle, engine registration). Methods are added here as more
/// of the surface gets a real caller.
public protocol CoreClient: AnyObject, Sendable {
    /// Proves the Swift-to-Rust-to-Swift streaming pattern that real chat
    /// streaming (`send`) will use: a token sink implemented in Swift, driven
    /// from Rust, turned into an `AsyncThrowingStream`.
    func mockGenerate(seed: Int64, tokensPerSec: Float) -> AsyncThrowingStream<String, Error>

    // MARK: - Catalog

    func catalog(filter: CatalogFilter) -> [CatalogEntry]
    func searchHub(query: String, format: ModelFormat) throws -> [CatalogEntry]

    /// Kicks off (or resumes) a model's download; progress shows up on
    /// subsequent `catalog()` calls via that entry's `bytesDone`/`state`.
    func startDownload(modelId: String) throws
    func pauseDownload(modelId: String) throws
    func resumeDownload(modelId: String) throws
    func cancelDownload(modelId: String) throws
    func deleteModel(modelId: String) throws

    /// Unloads a loaded model, freeing its resident weights (D35). Idempotent:
    /// unloading a model that isn't resident is a no-op success.
    func unloadModel(id: String) throws

    /// Forwards an OS memory-pressure warning to the core so it can release
    /// idle model handles. Safe to call at any time.
    func onMemoryWarning()

    /// Measures the model on this device: loads or reuses it, generates a fixed
    /// prompt, and persists tokens/sec, time-to-first-token and peak memory.
    /// Seconds of real inference behind a blocking FFI call (D21), so callers
    /// must run it off the main actor. Throws while a reply is generating — a
    /// benchmark alongside real decoding measures contention, not the model.
    func benchmarkModel(id: String) throws -> Benchmark

    /// The last benchmark run for a model, or nil if it was never measured.
    /// A local row read, so it is safe to call inline.
    func storedBenchmark(id: String) -> Benchmark?

    // MARK: - Conversations

    func conversations(query: ConversationQuery) -> [ConversationSummary]
    /// Resolves one conversation by ID (F17) — never page-scan `conversations()`
    /// to find it; older chats fall outside the default window.
    func conversation(id: String) -> Conversation?
    func createConversation(_ input: NewConversation) throws -> Conversation
    func messages(conversationId: String) -> [Message]
    func deleteConversation(id: String) throws
    func archiveConversation(id: String, archived: Bool) throws
    func renameConversation(id: String, title: String) throws
    /// Sets which model an existing conversation uses, so a selection made in
    /// the UI after the conversation was created actually takes effect.
    func setConversationModel(id: String, modelId: String) throws

    /// Streams one chat turn's deltas; the final element carries the
    /// saved assistant `Message` once generation completes. `send()`
    /// itself blocks synchronously at the FFI boundary (D21), so this
    /// runs it off the calling task via `Task.detached`.
    /// `reasoning` is the per-chat thinking choice (UI-27): `.auto` leaves the
    /// model's family default, `.off` suppresses thinking on capable models.
    func sendMessage(conversationId: String, input: UserInput, reasoning: ReasoningMode) -> AsyncThrowingStream<ChatStreamEvent, Error>
    /// Cancels the in-flight generation for a conversation, reaching the engine
    /// so a synchronous generate actually stops (F11), not just the stream.
    func cancelConversation(id: String)
    /// Regenerates the latest assistant reply (F12): drops the trailing
    /// assistant message and re-runs from existing history — no duplicated user
    /// input, never the assistant text as a prompt.
    func regenerate(conversationId: String) -> AsyncThrowingStream<ChatStreamEvent, Error>

    func registerEngine(_ engine: Engine)

    // MARK: - Pairing & devices

    func devices() -> [PairedDevice]
    func revokeDevice(id: String) throws
    /// NET-17: the Mac's per-device Disconnect (pairing kept), persisted.
    func setDeviceDisconnected(id: String, disconnected: Bool) throws
    func pairingOffer() throws -> PairingOffer
    /// Host identity (id/name/cert fingerprint) for Bonjour advertising —
    /// unlike `pairingOffer()` it never rotates the single-use pairing secret
    /// and is not Pro-gated.
    func pairingIdentity() -> PairingIdentity
    func parsePairingQr(_ qr: String) throws -> PairingOffer
    func savePeer(_ approved: ApprovedPairing) throws -> Peer
    func peers() -> [Peer]
    func revokePeer(id: String) throws
    func pairingOfferQrString() throws -> String
    func pendingPairings() -> [PairingRequest]
    func approvePairing(id: String) throws
    func denyPairing(id: String) throws

    // MARK: - Entitlements

    func entitlements() -> Entitlements
    func activateLicense(key: String) throws -> Entitlements
    func setStoreEntitlement(_ e: StoreEntitlement)

    // MARK: - Server

    func serverStart(cfg: ServerConfig) throws -> ServerStatus
    func serverStop() throws
    func serverStatus() -> ServerStatus
    func generateLocalApiKey() -> String
    func hasLocalApiKey() -> Bool
    func hostCertPem() -> String
    func stopEverything()

    // MARK: - Privacy

    func isOffline() -> Bool
    func setOfflineMode(on: Bool)
    func egressLog(limit: UInt32) -> [EgressEntry]
    func auditLog(query: AuditQuery) -> [AuditEntry]

    // MARK: - Projects & documents

    func createProject(name: String, instructions: String?) throws -> Project
    func listProjects() -> [Project]
    func renameProject(id: String, name: String) throws
    /// Sets the instructions prepended to every chat in this project.
    /// An empty or whitespace-only string clears them.
    func setProjectInstructions(id: String, instructions: String?) throws
    func deleteProject(id: String) throws
    func addDocument(projectId: String, name: String, mime: String, pages: [DocumentPage]) throws -> Document
    func listDocuments(projectId: String) -> [Document]
    func deleteDocument(id: String) throws
    func searchProject(projectId: String, query: String, k: UInt32) -> [ChunkHit]
}

public extension CoreClient {
    /// Callers without a per-chat thinking choice keep the model's default.
    func sendMessage(conversationId: String, input: UserInput) -> AsyncThrowingStream<ChatStreamEvent, Error> {
        sendMessage(conversationId: conversationId, input: input, reasoning: .auto)
    }
}

public enum ChatStreamEvent: Sendable {
    case delta(GenerateDelta)
    case done(Message)
}

public extension CoreClient {
    /// Fakes and previews have no live sessions to drop.
    func setDeviceDisconnected(id: String, disconnected: Bool) throws {}
}
