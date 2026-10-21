import Foundation

/// Engine that resolves the paired Mac peer lazily on each operation (UI-13).
///
/// Pairing is dynamic (peers can be paired, un-paired, or change IP/port),
/// while `RoutingEngine` is registered once at app startup. `LazyRemoteEngine`
/// delegates each engine call to a `RemoteEngine(client:)` built from the current
/// peer returned by `clientProvider`.
public final class LazyRemoteEngine: Engine, @unchecked Sendable {
    private let clientProvider: @Sendable () -> PeerClient?
    private let engineLock = NSLock()
    private var cachedClient: PeerClient?
    private var cachedEngine: RemoteEngine?

    public init(clientProvider: @escaping @Sendable () -> PeerClient?) {
        self.clientProvider = clientProvider
    }

    public func kind() -> EngineKind { .remote }

    /// A remote model id is prefixed `mac:<device>/` and requires an active peer.
    public func supports(model: ModelManifest) -> Bool {
        model.id.hasPrefix("mac:") && clientProvider() != nil
    }

    private func currentEngine() throws -> RemoteEngine {
        guard let client = clientProvider() else {
            throw PocketError.EngineFailed(engine: .remote, message: "No paired Mac peer available")
        }
        engineLock.lock()
        defer { engineLock.unlock() }
        if let existing = cachedEngine,
           let cached = cachedClient,
           cached.endpoint == client.endpoint && cached.token == client.token {
            return existing
        }
        let newEngine = RemoteEngine(client: client)
        cachedEngine = newEngine
        cachedClient = client
        return newEngine
    }

    public func load(model: ModelManifest, opts: LoadOptions) throws -> LoadedModel {
        try currentEngine().load(model: model, opts: opts)
    }

    public func unload(handle: String) throws {
        if clientProvider() != nil {
            try currentEngine().unload(handle: handle)
        }
    }

    public func generate(handle: String, req: GenerateRequest, sink: TokenSink) throws -> GenerateStats {
        try currentEngine().generate(handle: handle, req: req, sink: sink)
    }

    public func embed(handle: String, inputs: [String]) throws -> [[Float]] {
        try currentEngine().embed(handle: handle, inputs: inputs)
    }

    public func cancel(requestId: String) {
        engineLock.lock()
        let engine = cachedEngine
        engineLock.unlock()
        engine?.cancel(requestId: requestId)
    }

    public func residentBytes(handle: String) -> UInt64 {
        0
    }
}
