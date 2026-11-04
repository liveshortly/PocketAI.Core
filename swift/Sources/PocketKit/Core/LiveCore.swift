import Foundation

/// Wraps the generated UniFFI `PocketCore`. The simple data-fetch methods
/// (catalog, conversations, messages, devices, entitlements, serverStatus)
/// are fast, local-only calls and run synchronously on the caller; `send`
/// blocks at the FFI boundary (D21) and is bridged to `AsyncThrowingStream`
/// via `Task.detached`, same pattern as `mockGenerate`.
public final class LiveCore: CoreClient {
    private let core: PocketCore
    private let downloadCoordinator: ModelDownloadCoordinator

    /// Exposed for call sites that need the concrete `PocketCore` rather
    /// than the `CoreClient` abstraction - e.g. `PairingCoordinator`,
    /// which drives a real network handshake (`/pocket/v1/pair`) that
    /// has no equivalent on `PreviewCore`.
    public var rawCore: PocketCore { core }

    public init(config: CoreConfig, platform: PlatformHooks) throws {
        core = try PocketCore.open(config: config, platform: platform)
        downloadCoordinator = ModelDownloadCoordinator(core: core)
        // Recover downloads stranded by a relaunch (in-memory plans and queue
        // die with the process) before any user-driven download action.
        downloadCoordinator.rehydrate()
    }

    /// Wires application(_:handleEventsForBackgroundURLSession:) through to the
    /// download coordinator. Returns false (handler not kept) for an unknown
    /// session identifier.
    public func handleBackgroundSessionEvents(identifier: String, completionHandler: @escaping () -> Void) -> Bool {
        guard identifier == ModelDownloadCoordinator.sessionIdentifier else { return false }
        downloadCoordinator.backgroundEventsCompletionHandler = completionHandler
        return true
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

    public func searchHub(query: String, format: ModelFormat) throws -> [CatalogEntry] {
        try core.searchHub(query: query, format: format)
    }

    public func startDownload(modelId: String) throws {
        let plan = try core.planDownload(id: modelId)
        try downloadCoordinator.start(plan: plan)
    }

    public func pauseDownload(modelId: String) throws {
        try downloadCoordinator.pause(id: modelId)
    }

    public func resumeDownload(modelId: String) throws {
        try downloadCoordinator.resume(id: modelId)
    }

    public func cancelDownload(modelId: String) throws {
        try downloadCoordinator.cancel(id: modelId)
    }

    public func deleteModel(modelId: String) throws {
        try core.deleteModel(id: modelId)
    }

    public func unloadModel(id: String) throws {
        try core.unload(id: id)
    }

    public func onMemoryWarning() {
        core.onMemoryWarning()
    }

    public func benchmarkModel(id: String) throws -> Benchmark {
        try core.benchmarkModel(id: id)
    }

    public func storedBenchmark(id: String) -> Benchmark? {
        core.storedBenchmark(id: id)
    }

    public func conversations(query: ConversationQuery) -> [ConversationSummary] {
        core.conversations(q: query)
    }

    public func conversation(id: String) -> Conversation? {
        core.getConversation(id: id)
    }

    public func createConversation(_ input: NewConversation) throws -> Conversation {
        defer { Self.didWrite() }
        return try core.createConversation(input: input)
    }

    public func messages(conversationId: String) -> [Message] {
        core.messages(conversationId: conversationId)
    }

    public func deleteConversation(id: String) throws {
        try core.deleteConversation(id: id)
        Self.didWrite()
    }

    public func archiveConversation(id: String, archived: Bool) throws {
        try core.archiveConversation(id: id, archived: archived)
        Self.didWrite()
    }

    public func renameConversation(id: String, title: String) throws {
        try core.renameConversation(id: id, title: title)
        Self.didWrite()
    }

    public func setConversationModel(id: String, modelId: String) throws {
        try core.setConversationModel(id: id, modelId: modelId)
        Self.didWrite()
    }

    public func conversationTools(id: String) -> Bool {
        core.conversationTools(id: id)
    }

    public func setConversationTools(id: String, on: Bool) throws {
        try core.setConversationTools(id: id, on: on)
    }

    public func sendMessage(conversationId: String, input: UserInput, reasoning: ReasoningMode) -> AsyncThrowingStream<ChatStreamEvent, Error> {
        return AsyncThrowingStream { continuation in
            let sink = ContinuationChatSink(continuation: continuation)
            let task = Task.detached { [core] in
                do {
                    _ = try core.sendWithReasoning(conversationId: conversationId, input: input, reasoning: reasoning, sink: sink)
                } catch {
                    continuation.finish(throwing: error)
                }
                LiveCore.didWrite()
            }
            continuation.onTermination = { [core] _ in
                // Reach the engine's cancel so a synchronous generate actually
                // stops (F11), then tear down the Swift task.
                core.cancelConversation(conversationId: conversationId)
                task.cancel()
            }
        }
    }

    public func cancelConversation(id: String) {
        core.cancelConversation(conversationId: id)
    }

    public func regenerate(conversationId: String) -> AsyncThrowingStream<ChatStreamEvent, Error> {
        AsyncThrowingStream { continuation in
            let sink = ContinuationChatSink(continuation: continuation)
            let task = Task.detached { [core] in
                do {
                    _ = try core.regenerate(conversationId: conversationId, sink: sink)
                } catch {
                    continuation.finish(throwing: error)
                }
                LiveCore.didWrite()
            }
            continuation.onTermination = { [core] _ in
                core.cancelConversation(conversationId: conversationId)
                task.cancel()
            }
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

    public func setDeviceDisconnected(id: String, disconnected: Bool) throws {
        try core.setDeviceDisconnected(id: id, disconnected: disconnected)
    }

    public func pairingOffer() throws -> PairingOffer {
        try core.pairingOffer()
    }

    public func pairingIdentity() -> PairingIdentity {
        core.pairingIdentity()
    }

    public func parsePairingQr(_ qr: String) throws -> PairingOffer {
        try core.parsePairingQr(qr: qr)
    }

    public func savePeer(_ approved: ApprovedPairing) throws -> Peer {
        try core.savePeer(approved: approved)
    }

    public func peers() -> [Peer] {
        core.peers()
    }

    public func revokePeer(id: String) throws {
        try core.revokePeer(id: id)
    }

    public func pairingOfferQrString() throws -> String {
        try core.pairingOfferQrString()
    }

    public func pendingPairings() -> [PairingRequest] {
        core.pendingPairings()
    }

    public func approvePairing(id: String) throws {
        try core.approvePairing(id: id)
    }

    public func denyPairing(id: String) throws {
        try core.denyPairing(id: id)
    }

    public func entitlements() -> Entitlements {
        core.entitlements()
    }

    public func activateLicense(key: String) throws -> Entitlements {
        try core.activateLicense(key: key)
    }

    public func setStoreEntitlement(_ e: StoreEntitlement) {
        core.setStoreEntitlement(e: e)
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

    public func generateLocalApiKey() -> String {
        core.generateLocalApiKey()
    }

    public func hasLocalApiKey() -> Bool {
        core.hasLocalApiKey()
    }

    public func serverStats() -> ServerStats {
        core.serverStats()
    }

    public func serverRequestLog(limit: UInt32) -> [RequestLogEntry] {
        core.serverRequestLog(limit: limit)
    }

    public func hostCertPem() -> String {
        core.hostCertPem()
    }

    public func stopEverything() {
        core.stopEverything()
    }

    public func isOffline() -> Bool {
        core.isOffline()
    }

    public func setOfflineMode(on: Bool) {
        core.setOfflineMode(on: on)
    }

    public func egressLog(limit: UInt32) -> [EgressEntry] {
        core.egressLog(limit: limit)
    }

    public func auditLog(query: AuditQuery) -> [AuditEntry] {
        core.auditLog(q: query)
    }

    public func recordEgress(_ e: EgressEntry) {
        core.recordEgress(e: e)
    }

    // MARK: - Chat sync (NET-22)

    public func syncNow(peerId: String, transport: SyncTransport) throws -> SyncReport {
        try core.syncNow(peerId: peerId, transport: transport)
    }

    public func syncStatus(peerId: String) -> SyncStatus {
        core.syncStatus(peerId: peerId)
    }

    public func syncNudge() {
        core.syncNudge()
    }

    public func syncEnabledForDevice(deviceId: String) -> Bool {
        core.syncEnabledForDevice(deviceId: deviceId)
    }

    public func setSyncEnabledForDevice(deviceId: String, enabled: Bool) throws {
        try core.setSyncEnabledForDevice(deviceId: deviceId, enabled: enabled)
    }

    @discardableResult public func removeSyncedChats(peerId: String) throws -> UInt32 {
        try core.removeSyncedChats(peerId: peerId)
    }

    public func subscribe(_ sink: EventSink) {
        _ = core.subscribe(sink: sink)
    }

    private static func didWrite() {
        NotificationCenter.default.post(name: .pocketLocalChatWrite, object: nil)
    }

    // MARK: - Projects & documents

    public func createProject(name: String, instructions: String?) throws -> Project {
        try core.createProject(name: name, instructions: instructions)
    }

    public func listProjects() -> [Project] {
        core.listProjects()
    }

    public func renameProject(id: String, name: String) throws {
        try core.renameProject(id: id, name: name)
    }

    public func setProjectInstructions(id: String, instructions: String?) throws {
        try core.setProjectInstructions(id: id, instructions: instructions)
    }

    public func deleteProject(id: String) throws {
        try core.deleteProject(id: id)
    }

    public func addDocument(projectId: String, name: String, mime: String, pages: [DocumentPage]) throws -> Document {
        try core.addDocument(projectId: projectId, name: name, mime: mime, pages: pages)
    }

    public func listDocuments(projectId: String) -> [Document] {
        core.listDocuments(projectId: projectId)
    }

    public func deleteDocument(id: String) throws {
        try core.deleteDocument(id: id)
    }

    public func searchProject(projectId: String, query: String, k: UInt32) -> [ChunkHit] {
        core.searchProject(projectId: projectId, query: query, k: k)
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

    func onToolStep(step: ToolStep) {
        continuation.yield(.toolStep(step))
    }
}

public enum LiveCoreError: Error {
    case generationFailed(String)
}
