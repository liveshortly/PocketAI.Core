import Foundation

/// Engine that resolves the paired Mac peer lazily on each operation (UI-13).
///
/// Pairing is dynamic (peers can be paired, un-paired, or change IP/port),
/// while `RoutingEngine` is registered once at app startup. `LazyRemoteEngine`
/// delegates each engine call to a `RemoteEngine(client:)` built from the current
/// peer returned by `clientProvider`.
public final class LazyRemoteEngine: Engine, @unchecked Sendable {
    /// Resolves the peer client for a given remote model id (`mac:<peer>/<model>`).
    /// The id lets the provider pick the *specific* paired Mac (F16) instead of
    /// `peers().first`; a nil id means "the current/active peer".
    private let clientProvider: @Sendable (_ modelId: String?) -> PeerClient?
    private let engineLock = NSLock()
    private var cachedClient: PeerClient?
    private var cachedEngine: RemoteEngine?

    public init(clientProvider: @escaping @Sendable (_ modelId: String?) -> PeerClient?) {
        self.clientProvider = clientProvider
    }

    public func kind() -> EngineKind { .remote }

    /// A remote model id is prefixed `mac:<device>/` and requires its peer.
    public func supports(model: ModelManifest) -> Bool {
        model.id.hasPrefix("mac:") && clientProvider(model.id) != nil
    }

    /// Builds (or reuses) the engine for a specific model id's peer. `load`
    /// resolves the right Mac here; subsequent generate/unload reuse the cached
    /// engine for the active turn (one foreground generation at a time, §9.2).
    private func engine(forModelId modelId: String?) throws -> RemoteEngine {
        guard let client = clientProvider(modelId) else {
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

    /// The engine already resolved at load time; falls back to the current peer.
    private func activeEngine() throws -> RemoteEngine {
        engineLock.lock()
        let existing = cachedEngine
        engineLock.unlock()
        if let existing { return existing }
        return try engine(forModelId: nil)
    }

    public func load(model: ModelManifest, opts: LoadOptions) throws -> LoadedModel {
        try engine(forModelId: model.id).load(model: model, opts: opts)
    }

    public func unload(handle: String) throws {
        engineLock.lock()
        let existing = cachedEngine
        engineLock.unlock()
        try existing?.unload(handle: handle)
    }

    public func generate(handle: String, req: GenerateRequest, sink: TokenSink) throws -> GenerateStats {
        try activeEngine().generate(handle: handle, req: req, sink: sink)
    }

    public func embed(handle: String, inputs: [String]) throws -> [[Float]] {
        try activeEngine().embed(handle: handle, inputs: inputs)
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
