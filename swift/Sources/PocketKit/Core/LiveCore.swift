import Foundation

/// Wraps the generated UniFFI `PocketCore`. The simple data-fetch methods
/// (catalog, conversations, messages, devices, entitlements, serverStatus)
/// are fast, local-only calls and run synchronously on the caller; `send`
/// blocks at the FFI boundary (D21) and is bridged to `AsyncThrowingStream`
/// via `Task.detached`, same pattern as `mockGenerate`.
public final class LiveCore: CoreClient {
    private let core: PocketCore
    private let downloadCoordinator: ModelDownloadCoordinator

    public init(config: CoreConfig, platform: PlatformHooks) throws {
        core = try PocketCore.open(config: config, platform: platform)
        downloadCoordinator = ModelDownloadCoordinator(core: core)
    }

    public func mockGenerate(seed: Int64, tokensPerSec: Float) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let sink = ContinuationSink(continuation: continuation)
            Task.detached {
                PocketKit.mockGenerate(seed: seed, tokensPerSec: tokensPerSec, sink: sink)
            }
        }
    }

    public func catalog(filter: CatalogFilter) -> [CatalogEntry] {
        core.catalog(filter: filter)
    }

    public func startDownload(modelId: String) throws {
        let plan = try core.planDownload(id: modelId)
        try downloadCoordinator.start(plan: plan)
    }

    public func deleteModel(modelId: String) throws {
        try core.deleteModel(id: modelId)
    }

    public func conversations(query: ConversationQuery) -> [ConversationSummary] {
        core.conversations(q: query)
    }

    public func createConversation(_ input: NewConversation) throws -> Conversation {
        try core.createConversation(input: input)
    }

    public func messages(conversationId: String) -> [Message] {
        core.messages(conversationId: conversationId)
    }

    public func deleteConversation(id: String) throws {
        try core.deleteConversation(id: id)
    }

    public func archiveConversation(id: String, archived: Bool) throws {
        try core.archiveConversation(id: id, archived: archived)
    }

    public func renameConversation(id: String, title: String) throws {
        try core.renameConversation(id: id, title: title)
    }

    public func sendMessage(conversationId: String, input: UserInput) -> AsyncThrowingStream<ChatStreamEvent, Error> {
        AsyncThrowingStream { continuation in
            let sink = ContinuationChatSink(continuation: continuation)
            let task = Task.detached { [core] in
                do {
                    _ = try core.send(conversationId: conversationId, input: input, sink: sink)
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    public func registerEngine(_ engine: Engine) {
        core.registerEngine(engine: engine)
    }

    public func devices() -> [PairedDevice] {
        core.devices()
    }

    public func revokeDevice(id: String) throws {
        try core.revokeDevice(id: id)
    }

    public func pairingOffer() throws -> PairingOffer {
        try core.pairingOffer()
    }

    public func parsePairingQr(_ qr: String) throws -> PairingOffer {
        try core.parsePairingQr(qr: qr)
    }

    public func savePeer(_ approved: ApprovedPairing) throws -> Peer {
        try core.savePeer(approved: approved)
    }

    public func entitlements() -> Entitlements {
        core.entitlements()
    }

    public func activateLicense(key: String) throws -> Entitlements {
        try core.activateLicense(key: key)
    }

    public func serverStart(cfg: ServerConfig) throws -> ServerStatus {
        try core.serverStart(cfg: cfg)
    }

    public func serverStop() throws {
        try core.serverStop()
    }

    public func serverStatus() -> ServerStatus {
        core.serverStatus()
    }
}

private final class ContinuationSink: GenerateSink, @unchecked Sendable {
    private let continuation: AsyncThrowingStream<String, Error>.Continuation

    init(continuation: AsyncThrowingStream<String, Error>.Continuation) {
        self.continuation = continuation
    }

    func onDelta(text: String) {
        continuation.yield(text)
    }

    func onDone(completionTokens: UInt32) {
        continuation.finish()
    }

    func onError(message: String) {
        continuation.finish(throwing: LiveCoreError.generationFailed(message))
    }
}

private final class ContinuationChatSink: ChatSink, @unchecked Sendable {
    private let continuation: AsyncThrowingStream<ChatStreamEvent, Error>.Continuation

    init(continuation: AsyncThrowingStream<ChatStreamEvent, Error>.Continuation) {
        self.continuation = continuation
    }

    func onDelta(delta: GenerateDelta) {
        continuation.yield(.delta(delta))
    }

    func onDone(stats: GenerateStats) {}

    func onError(err: PocketError) {
        continuation.finish(throwing: err)
    }

    func onMessageSaved(message: Message) {
        continuation.yield(.done(message))
        continuation.finish()
    }
}

public enum LiveCoreError: Error {
    case generationFailed(String)
}
