import XCTest
@testable import PocketKit

final class PocketCoreTests: XCTestCase {
    final class StubPlatformHooks: PlatformHooks {
        func keychainGet(key: String) -> Data? { nil }
        func keychainSet(key: String, value: Data) -> Bool { true }
        func deviceProfile() -> DeviceProfile { sampleDeviceProfile() }
        func appSupportDir() -> String { NSTemporaryDirectory() }
        func nowMs() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }
    }

    final class StubEngine: PocketKit.Engine {
        func kind() -> EngineKind { .llamaCpp }
        func supports(model: ModelManifest) -> Bool { true }
        func load(model: ModelManifest, opts: LoadOptions) throws -> LoadedModel { LoadedModel(handle: "h") }
        func unload(handle: String) throws {}
        func generate(handle: String, req: GenerateRequest, sink: PocketKit.TokenSink) throws -> GenerateStats {
            GenerateStats(promptTokens: 0, completionTokens: 0, ttftMs: 0, decodeTps: 0, finish: .stop)
        }
        func embed(handle: String, inputs: [String]) throws -> [[Float]] { [] }
        func cancel(requestId: String) {}
        func residentBytes(handle: String) -> UInt64 { 0 }
    }

    func testOpenAndStubCall() throws {
        let config = CoreConfig(appSupportDir: NSTemporaryDirectory(), platform: .macOs)
        let core = try PocketCore.open(config: config, platform: StubPlatformHooks())
        XCTAssertEqual(core.deviceProfile().chip, "M4")
        XCTAssertTrue(core.catalog(filter: CatalogFilter(modality: nil, curatedOnly: false, installedOnly: false)).isEmpty)

        do {
            _ = try core.unload(id: "nonexistent@rev")
            XCTFail("expected unimplemented error")
        } catch PocketError.Internal(let message) {
            XCTAssertTrue(message.contains("unimplemented: unload"))
        }
    }

    /// Proves PocketCore itself (not just the free-function spike) stores a
    /// Swift-implemented foreign trait object and later calls back into it.
    func testRegisterEngineThenCoreInvokesItsCallback() throws {
        let config = CoreConfig(appSupportDir: NSTemporaryDirectory(), platform: .macOs)
        let core = try PocketCore.open(config: config, platform: StubPlatformHooks())
        XCTAssertTrue(core.loadedModels().isEmpty)

        core.registerEngine(engine: StubEngine())

        let loaded = core.loadedModels()
        XCTAssertEqual(loaded.count, 1)
        guard case .llamaCpp = loaded[0].engine else {
            return XCTFail("expected llamaCpp, got \(loaded[0].engine)")
        }
    }
}
