import Foundation
import PocketKit

/// A composite `Engine` that dispatches calls to whichever sub-engine supports the model.
///
/// Implements the FFI `Engine` protocol to allow registering a single engine slot that can
/// route requests across multiple concrete engines (e.g. `PocketMLXEngine` and `AppleFoundationModelsEngine`).
public final class RoutingEngine: Engine, @unchecked Sendable {
    private let engines: [Engine]
    private let lock = NSLock()
    private var handles: [String: Engine] = [:]

    /// Creates a composite routing engine with the given sub-engines evaluated in priority order.
    public init(engines: [Engine]) {
        self.engines = engines
    }

    /// Convenience initializer configuring the default engine hierarchy:
    /// 1. `PocketMLXEngine` (.mlxSafetensors)
    /// 2. `AppleFoundationModelsEngine` (apple-intelligence@on-device)
    public convenience init() {
        self.init(engines: [
            PocketMLXEngine(),
            AppleFoundationModelsEngine(),
        ])
    }

    /// Nominal engine kind representing this composite.
    public func kind() -> EngineKind {
        .mlx
    }

    /// Returns `true` if any configured sub-engine supports the model manifest.
    public func supports(model: ModelManifest) -> Bool {
        engines.contains { $0.supports(model: model) }
    }

    /// Loads the model on the first sub-engine supporting it and records handle ownership.
    public func load(model: ModelManifest, opts: LoadOptions) throws -> LoadedModel {
        guard let engine = engines.first(where: { $0.supports(model: model) }) else {
            throw PocketError.EngineFailed(engine: .mlx, message: "no engine supports model \(model.id)")
        }
        let loaded = try engine.load(model: model, opts: opts)
        lock.lock()
        handles[loaded.handle] = engine
        lock.unlock()
        return loaded
    }

    /// Forwards unload to the owning engine and removes the handle from the map.
    public func unload(handle: String) throws {
        let engine: Engine
        lock.lock()
        if let found = handles.removeValue(forKey: handle) {
            engine = found
            lock.unlock()
        } else {
            lock.unlock()
            throw PocketError.EngineFailed(engine: .mlx, message: "unknown handle \(handle)")
        }
        try engine.unload(handle: handle)
    }

    /// Forwards generation to the owning sub-engine.
    public func generate(handle: String, req: GenerateRequest, sink: TokenSink) throws -> GenerateStats {
        let engine: Engine
        lock.lock()
        if let found = handles[handle] {
            engine = found
            lock.unlock()
        } else {
            lock.unlock()
            throw PocketError.EngineFailed(engine: .mlx, message: "unknown handle \(handle)")
        }
        return try engine.generate(handle: handle, req: req, sink: sink)
    }

    /// Forwards embedding extraction to the owning sub-engine.
    public func embed(handle: String, inputs: [String]) throws -> [[Float]] {
        let engine: Engine
        lock.lock()
        if let found = handles[handle] {
            engine = found
            lock.unlock()
        } else {
            lock.unlock()
            throw PocketError.EngineFailed(engine: .mlx, message: "unknown handle \(handle)")
        }
        return try engine.embed(handle: handle, inputs: inputs)
    }

    /// Forwards cancellation to all sub-engines.
    public func cancel(requestId: String) {
        for engine in engines {
            engine.cancel(requestId: requestId)
        }
    }

    /// Returns resident memory bytes for the model handle, or 0 if unknown.
    public func residentBytes(handle: String) -> UInt64 {
        let engine: Engine?
        lock.lock()
        engine = handles[handle]
        lock.unlock()
        return engine?.residentBytes(handle: handle) ?? 0
    }
}
