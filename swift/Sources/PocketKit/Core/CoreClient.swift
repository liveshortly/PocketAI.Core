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
    /// AGT-12: the per-chat Tools switch (web search). Off by default.
    func conversationTools(id: String) -> Bool
    func setConversationTools(id: String, on: Bool) throws

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
    /// UI-68: replaces the sent message `messageId` and everything after it,
    /// then answers `input`. Only a user message can be edited.
    func editMessage(conversationId: String, messageId: String, input: UserInput, reasoning: ReasoningMode) -> AsyncThrowingStream<ChatStreamEvent, Error>

    func registerEngine(_ engine: Engine)
    /// AGT-13: the code sandbox + "Run this?" UI. Without one, no code tool is offered.
    func registerToolExecutor(_ executor: ToolExecutor)
    /// AGT-13: the user's answer to a "Run this?" card (only the card calls this).
    func resolveApproval(id: String, decision: ApprovalDecision, alwaysAllow: Bool)

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
    /// NET-23: the "Enter code manually" fallback (8-char one-time code + address).
    func pairingManualCode() throws -> ManualPairingCode
    func pendingPairings() -> [PairingRequest]
    func approvePairing(id: String) throws
    func denyPairing(id: String) throws

    // MARK: - Entitlements

    func entitlements() -> Entitlements
    func activateLicense(key: String) throws -> Entitlements
    /// COM-08 testing phase: a free 90-day test key from the licence server.
    /// Blocks on the network; call off the main thread.
    func requestFreeLicense(email: String) throws -> String
    func setStoreEntitlement(_ e: StoreEntitlement)

    // MARK: - Server

    func serverStart(cfg: ServerConfig) throws -> ServerStatus
    func serverStop() throws
    func serverStatus() -> ServerStatus
    func generateLocalApiKey() -> String
    func hasLocalApiKey() -> Bool
    /// MAC-23: today's request totals and the recent request log.
    func serverStats() -> ServerStats
    func serverRequestLog(limit: UInt32) -> [RequestLogEntry]
    func hostCertPem() -> String
    func stopEverything()

    // MARK: - Privacy

    func isOffline() -> Bool
    func setOfflineMode(on: Bool)
    func egressLog(limit: UInt32) -> [EgressEntry]
    func auditLog(query: AuditQuery) -> [AuditEntry]
    func recordEgress(_ e: EgressEntry)

    // MARK: - Chat sync (NET-22)

    /// iPhone: one full sync with the paired Mac. Blocking - off the main thread.
    func syncNow(peerId: String, transport: SyncTransport) throws -> SyncReport
    /// Last sync time and outbox for `peerId` (peer id on iPhone, device id on Mac).
    func syncStatus(peerId: String) -> SyncStatus
    /// Mac: menu-bar "Sync now" - wakes a phone waiting on `/sync/wait`.
    func syncNudge()
    /// Mac: Devices › phone "Sync chats" (on unless turned off).
    func syncEnabledForDevice(deviceId: String) -> Bool
    func setSyncEnabledForDevice(deviceId: String, enabled: Bool) throws
    /// Mac: "Let my paired iPhone run code on this Mac" (AGT-13). Default off;
    /// while off the Mac refuses every run request.
    func runCodeEnabled() -> Bool
    func setRunCodeEnabled(_ enabled: Bool)
    /// Forget's "Remove chats that came from <device>" - local only.
    @discardableResult func removeSyncedChats(peerId: String) throws -> UInt32
    /// Core events (NET-22: `sync.applied` on the Mac). Called off the main thread.
    func subscribe(_ sink: EventSink)

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
    /// AGT-12/13: a tool ran ("Searched" / "Read" / "Ran" code), before the answer.
    case toolStep(ToolStep)
    /// UI-146: the reply moved to a new phase; drive `ReplyStatusLine` with it.
    case status(ChatPhase)
    /// UI-145: a non-content diagnostic line for the app's log. Lines that
    /// start with `ChatStreamEvent.contentLinePrefix` carry model output:
    /// show them in "Test tools", never log them.
    case diagnostic(String)
}

public extension ChatStreamEvent {
    static let contentLinePrefix = "content: "
}

public extension CoreClient {
    /// Fakes and previews cannot edit a sent message.
    func editMessage(conversationId: String, messageId: String, input: UserInput, reasoning: ReasoningMode) -> AsyncThrowingStream<ChatStreamEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: PocketError.NotFound(what: "edit")) }
    }

    /// Fakes and previews run no code.
    func registerToolExecutor(_ executor: ToolExecutor) {}
    func resolveApproval(id: String, decision: ApprovalDecision, alwaysAllow: Bool) {}

    /// Fakes and previews have no web tools.
    func conversationTools(id: String) -> Bool { false }
    func setConversationTools(id: String, on: Bool) throws {}

    /// Fakes and previews have no live sessions to drop.
    func setDeviceDisconnected(id: String, disconnected: Bool) throws {}

    /// Fakes and previews serve no requests.
    func serverStats() -> ServerStats {
        ServerStats(requestsToday: 0, tokensOutToday: 0, avgTokensPerSec: 0)
    }
    func serverRequestLog(limit: UInt32) -> [RequestLogEntry] { [] }

    /// Fakes and previews neither log egress nor sync.
    func recordEgress(_ e: EgressEntry) {}
    func syncNow(peerId: String, transport: SyncTransport) throws -> SyncReport {
        throw PocketError.NotFound(what: "sync")
    }
    func syncStatus(peerId: String) -> SyncStatus {
        SyncStatus(lastSyncedAt: nil, pendingOps: 0, pendingConversationIds: [])
    }
    func syncNudge() {}
    func syncEnabledForDevice(deviceId: String) -> Bool { true }
    func setSyncEnabledForDevice(deviceId: String, enabled: Bool) throws {}
    func runCodeEnabled() -> Bool { false }
    func setRunCodeEnabled(_ enabled: Bool) {}
    @discardableResult func removeSyncedChats(peerId: String) throws -> UInt32 { 0 }
    func subscribe(_ sink: EventSink) {}
}

public extension Notification.Name {
    /// NET-22: posted after a local chat write (create, send, rename, delete...)
    /// so the iPhone can schedule a debounced sync with its Mac.
    static let pocketLocalChatWrite = Notification.Name("app.getpocketai.localChatWrite")
}
