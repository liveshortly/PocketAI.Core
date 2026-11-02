import XCTest
import PocketKit
@testable import PocketMLX

private final class FakeEngine: Engine, @unchecked Sendable {
    let supportedModelId: String
    let handlePrefix: String
    let kindValue: EngineKind

    private let lock = NSLock()
    private(set) var loadCalls: [ModelManifest] = []
    private(set) var unloadCalls: [String] = []
    private(set) var generateCalls: [(handle: String, requestId: String)] = []
    private(set) var embedCalls: [(handle: String, inputs: [String])] = []
    private(set) var cancelCalls: [String] = []
    private(set) var residentBytesCalls: [String] = []

    var residentBytesToReturn: UInt64 = 1024

    init(supportedModelId: String, handlePrefix: String, kind: EngineKind = .mlx) {
        self.supportedModelId = supportedModelId
        self.handlePrefix = handlePrefix
        self.kindValue = kind
    }

    func kind() -> EngineKind { kindValue }

    func supports(model: ModelManifest) -> Bool {
        model.id == supportedModelId
    }

    func load(model: ModelManifest, opts: LoadOptions) throws -> LoadedModel {
        lock.lock()
        defer { lock.unlock() }
        loadCalls.append(model)
        return LoadedModel(handle: "\(handlePrefix)-\(model.id)")
    }

    func unload(handle: String) throws {
        lock.lock()
        defer { lock.unlock() }
        unloadCalls.append(handle)
    }

    func generate(handle: String, req: GenerateRequest, sink: TokenSink) throws -> GenerateStats {
        lock.lock()
        defer { lock.unlock() }
        generateCalls.append((handle: handle, requestId: req.requestId))
        return GenerateStats(
            promptTokens: 10,
            completionTokens: 20,
            ttftMs: 50,
            decodeTps: 30.0,
            finish: .stop
        )
    }

    func embed(handle: String, inputs: [String]) throws -> [[Float]] {
        lock.lock()
        defer { lock.unlock() }
        embedCalls.append((handle: handle, inputs: inputs))
        return inputs.map { _ in [0.1, 0.2, 0.3] }
    }

    func cancel(requestId: String) {
        lock.lock()
        defer { lock.unlock() }
        cancelCalls.append(requestId)
    }

    func residentBytes(handle: String) -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        residentBytesCalls.append(handle)
        return residentBytesToReturn
    }
}

private final class FakeTokenSink: TokenSink, @unchecked Sendable {
    func onDelta(delta: GenerateDelta) {}
    func onDone(stats: GenerateStats) {}
    func onError(err: PocketError) {}
}

final class RoutingEngineTests: XCTestCase {

    private func makeManifest(id: String) -> ModelManifest {
        ModelManifest(
            id: id,
            displayName: id,
            family: "test",
            paramsB: 1.0,
            quant: "4bit",
            format: .mlxSafetensors,
            modalities: [.text],
            contextLength: 2048,
            files: [],
            license: LicenseInfo(spdx: "mit", commercialOk: true, url: ""),
            hfRepo: "test/\(id)",
            hfRevision: "rev1",
            minRamBytes: 0,
            estRuntimeBytes: 1024,
            tags: [],
            curated: false,
            reasoning: false
        )
    }

    private func makeRequest(requestId: String = "req-1") -> GenerateRequest {
        GenerateRequest(
            requestId: requestId,
            messages: [],
            tools: [],
            params: SamplingParams(
                temperature: 0.7,
                topP: 0.9,
                topK: 40,
                minP: 0.05,
                maxTokens: 100,
                stop: [],
                seed: nil,
                repetitionPenalty: 1.1
            ),
            reasoning: .off
        )
    }

    func testSupportsIsOrOfChildren() {
        let fakeA = FakeEngine(supportedModelId: "model-a", handlePrefix: "a")
        let fakeB = FakeEngine(supportedModelId: "model-b", handlePrefix: "b")
        let router = RoutingEngine(engines: [fakeA, fakeB])

        XCTAssertTrue(router.supports(model: makeManifest(id: "model-a")))
        XCTAssertTrue(router.supports(model: makeManifest(id: "model-b")))
        XCTAssertFalse(router.supports(model: makeManifest(id: "model-c")))
    }

    func testLoadRoutesToFirstSupportingChildAndRemembersHandle() throws {
        let fakeA = FakeEngine(supportedModelId: "model-a", handlePrefix: "a")
        let fakeB = FakeEngine(supportedModelId: "model-b", handlePrefix: "b")
        let router = RoutingEngine(engines: [fakeA, fakeB])

        let manifestA = makeManifest(id: "model-a")
        let opts = LoadOptions(contextLength: 2048, pinnedEngine: nil)

        let loaded = try router.load(model: manifestA, opts: opts)
        XCTAssertEqual(loaded.handle, "a-model-a")
        XCTAssertEqual(fakeA.loadCalls.count, 1)
        XCTAssertEqual(fakeB.loadCalls.count, 0)

        // Generate routes to fakeA
        let sink = FakeTokenSink()
        let req = makeRequest(requestId: "req-101")
        let stats = try router.generate(handle: loaded.handle, req: req, sink: sink)
        XCTAssertEqual(stats.promptTokens, 10)
        XCTAssertEqual(fakeA.generateCalls.count, 1)
        XCTAssertEqual(fakeA.generateCalls.first?.handle, "a-model-a")
        XCTAssertEqual(fakeA.generateCalls.first?.requestId, "req-101")
        XCTAssertEqual(fakeB.generateCalls.count, 0)

        // Resident bytes routes to fakeA
        fakeA.residentBytesToReturn = 4096
        XCTAssertEqual(router.residentBytes(handle: loaded.handle), 4096)
        XCTAssertEqual(fakeA.residentBytesCalls, ["a-model-a"])
        XCTAssertEqual(fakeB.residentBytesCalls.count, 0)

        // Embed routes to fakeA
        let embeddings = try router.embed(handle: loaded.handle, inputs: ["hello", "world"])
        XCTAssertEqual(embeddings.count, 2)
        XCTAssertEqual(fakeA.embedCalls.count, 1)
        XCTAssertEqual(fakeA.embedCalls.first?.handle, "a-model-a")
        XCTAssertEqual(fakeA.embedCalls.first?.inputs, ["hello", "world"])
        XCTAssertEqual(fakeB.embedCalls.count, 0)

        // Unload routes to fakeA and removes handle
        try router.unload(handle: loaded.handle)
        XCTAssertEqual(fakeA.unloadCalls, ["a-model-a"])
        XCTAssertEqual(fakeB.unloadCalls.count, 0)

        // Second unload should throw unknown handle
        XCTAssertThrowsError(try router.unload(handle: loaded.handle)) { error in
            guard case PocketError.EngineFailed(let engine, let message) = error else {
                XCTFail("Expected PocketError.EngineFailed, got \(error)")
                return
            }
            XCTAssertEqual(engine, .mlx)
            XCTAssertTrue(message.contains(loaded.handle))
        }

        // Subsequent generate should throw unknown handle
        XCTAssertThrowsError(try router.generate(handle: loaded.handle, req: req, sink: sink)) { error in
            guard case PocketError.EngineFailed(let engine, let message) = error else {
                XCTFail("Expected PocketError.EngineFailed, got \(error)")
                return
            }
            XCTAssertEqual(engine, .mlx)
            XCTAssertTrue(message.contains(loaded.handle))
        }
    }

    func testLoadFirstMatchWinsWhenMultipleSupport() throws {
        let fake1 = FakeEngine(supportedModelId: "shared-model", handlePrefix: "first")
        let fake2 = FakeEngine(supportedModelId: "shared-model", handlePrefix: "second")
        let router = RoutingEngine(engines: [fake1, fake2])

        let manifest = makeManifest(id: "shared-model")
        let loaded = try router.load(model: manifest, opts: LoadOptions(contextLength: nil, pinnedEngine: nil))

        XCTAssertEqual(loaded.handle, "first-shared-model")
        XCTAssertEqual(fake1.loadCalls.count, 1)
        XCTAssertEqual(fake2.loadCalls.count, 0)
    }

    func testLoadUnsupportedModelThrows() {
        let fakeA = FakeEngine(supportedModelId: "model-a", handlePrefix: "a")
        let router = RoutingEngine(engines: [fakeA])

        let manifest = makeManifest(id: "unsupported-model")
        XCTAssertThrowsError(try router.load(model: manifest, opts: LoadOptions(contextLength: nil, pinnedEngine: nil))) { error in
            guard case PocketError.EngineFailed(let engine, let message) = error else {
                XCTFail("Expected PocketError.EngineFailed, got \(error)")
                return
            }
            XCTAssertEqual(engine, .mlx)
            XCTAssertEqual(message, "no engine supports model unsupported-model")
        }
    }

    func testCancelBroadcastsToAllChildren() {
        let fakeA = FakeEngine(supportedModelId: "model-a", handlePrefix: "a")
        let fakeB = FakeEngine(supportedModelId: "model-b", handlePrefix: "b")
        let router = RoutingEngine(engines: [fakeA, fakeB])

        router.cancel(requestId: "req-999")

        XCTAssertEqual(fakeA.cancelCalls, ["req-999"])
        XCTAssertEqual(fakeB.cancelCalls, ["req-999"])
    }

    func testTwoDifferentModelsRouteIndependently() throws {
        let fakeA = FakeEngine(supportedModelId: "model-a", handlePrefix: "a")
        let fakeB = FakeEngine(supportedModelId: "model-b", handlePrefix: "b")
        let router = RoutingEngine(engines: [fakeA, fakeB])

        let loadedA = try router.load(model: makeManifest(id: "model-a"), opts: LoadOptions(contextLength: nil, pinnedEngine: nil))
        let loadedB = try router.load(model: makeManifest(id: "model-b"), opts: LoadOptions(contextLength: nil, pinnedEngine: nil))

        XCTAssertEqual(loadedA.handle, "a-model-a")
        XCTAssertEqual(loadedB.handle, "b-model-b")

        let sink = FakeTokenSink()
        _ = try router.generate(handle: loadedA.handle, req: makeRequest(requestId: "req-a"), sink: sink)
        _ = try router.generate(handle: loadedB.handle, req: makeRequest(requestId: "req-b"), sink: sink)

        XCTAssertEqual(fakeA.generateCalls.map(\.requestId), ["req-a"])
        XCTAssertEqual(fakeB.generateCalls.map(\.requestId), ["req-b"])

        // Unloading A does not affect B
        try router.unload(handle: loadedA.handle)
        XCTAssertEqual(fakeA.unloadCalls, ["a-model-a"])
        XCTAssertEqual(fakeB.unloadCalls, [])

        _ = try router.generate(handle: loadedB.handle, req: makeRequest(requestId: "req-b2"), sink: sink)
        XCTAssertEqual(fakeB.generateCalls.map(\.requestId), ["req-b", "req-b2"])
    }

    func testUnknownHandleHandling() {
        let fakeA = FakeEngine(supportedModelId: "model-a", handlePrefix: "a")
        let router = RoutingEngine(engines: [fakeA])

        XCTAssertEqual(router.residentBytes(handle: "unknown"), 0)

        XCTAssertThrowsError(try router.unload(handle: "unknown")) { error in
            guard case PocketError.EngineFailed(let engine, let message) = error else {
                XCTFail("Expected PocketError.EngineFailed, got \(error)")
                return
            }
            XCTAssertEqual(engine, .mlx)
            XCTAssertTrue(message.contains("unknown"))
        }

        let sink = FakeTokenSink()
        XCTAssertThrowsError(try router.generate(handle: "unknown", req: makeRequest(), sink: sink)) { error in
            guard case PocketError.EngineFailed(let engine, let message) = error else {
                XCTFail("Expected PocketError.EngineFailed, got \(error)")
                return
            }
            XCTAssertEqual(engine, .mlx)
            XCTAssertTrue(message.contains("unknown"))
        }

        XCTAssertThrowsError(try router.embed(handle: "unknown", inputs: ["test"])) { error in
            guard case PocketError.EngineFailed(let engine, let message) = error else {
                XCTFail("Expected PocketError.EngineFailed, got \(error)")
                return
            }
            XCTAssertEqual(engine, .mlx)
            XCTAssertTrue(message.contains("unknown"))
        }
    }

    func testConvenienceInitAndKind() {
        let router = RoutingEngine()
        XCTAssertEqual(router.kind(), .mlx)

        let mlxManifest = ModelManifest(
            id: "mlx-community/Qwen3-1.7B-4bit@rev1",
            displayName: "Qwen3-1.7B-4bit",
            family: "Qwen3",
            paramsB: 1.7,
            quant: "4bit",
            format: .mlxSafetensors,
            modalities: [.text],
            contextLength: 40960,
            files: [],
            license: LicenseInfo(spdx: "apache-2.0", commercialOk: true, url: ""),
            hfRepo: "mlx-community/Qwen3-1.7B-4bit",
            hfRevision: "rev1",
            minRamBytes: 0,
            estRuntimeBytes: 1126428293,
            tags: ["recommended"],
            curated: true,
            reasoning: false
        )
        XCTAssertTrue(router.supports(model: mlxManifest))

        var unsupportedManifest = mlxManifest
        unsupportedManifest.format = .gguf
        unsupportedManifest.id = "some-gguf-model"
        XCTAssertFalse(router.supports(model: unsupportedManifest))
    }

    /// A Mac-hosted model keeps the remote model's `mlx-safetensors` format, so
    /// the local MLX engine must still decline it — otherwise it wins the
    /// routing order ahead of the remote engine and fails on missing weights.
    func testMlxEngineDeclinesRemoteMacModelIds() {
        let mlx = PocketMLXEngine()
        XCTAssertTrue(mlx.supports(model: makeManifest(id: "mlx-community/Qwen3-1.7B-4bit@rev1")))
        XCTAssertFalse(mlx.supports(model: makeManifest(id: "mac:peer-1/mlx-community/Qwen3-1.7B-4bit@rev1")))
    }
}
