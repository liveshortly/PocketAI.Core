import XCTest
@testable import PocketKit

/// APL-08, APL-11, APL-16: Unit tests for engine traits, Bonjour advertiser constants, and remote engine manifest routing.
final class EngineFoundationTests: XCTestCase {
    func testAdvertiserServiceType() {
        XCTAssertEqual(Advertiser.serviceType, "_pocketai._tcp")
    }

    @MainActor
    func testBrowserInitialState() {
        let browser = Browser()
        XCTAssertEqual(browser.state, .idle)
        XCTAssertTrue(browser.peers.isEmpty)
        browser.stop()
    }

    func testAppleFoundationModelsEngineMetadata() {
        let engine = AppleFoundationModelsEngine()
        XCTAssertEqual(AppleFoundationModelsEngine.modelId, "apple-intelligence@on-device")
        XCTAssertEqual(engine.kind(), .mlx)
    }

    private func makeManifest(id: String) -> ModelManifest {
        ModelManifest(
            id: id,
            displayName: "Test Model",
            family: "qwen",
            paramsB: 7.0,
            quant: "Q4_K_M",
            format: .gguf,
            modalities: [.text],
            contextLength: 4096,
            files: [],
            license: LicenseInfo(spdx: "Apache-2.0", commercialOk: true, url: "https://example.com"),
            hfRepo: "test/repo",
            hfRevision: "main",
            minRamBytes: 8_000_000_000,
            estRuntimeBytes: 6_000_000_000,
            tags: [],
            curated: true
        )
    }

    func testRemoteEngineKindAndPrefixSupport() {
        let endpoint = PeerEndpoint(host: "192.168.1.10", port: 7480, fingerprint: "test_fp")
        let client = PeerClient(endpoint: endpoint, token: "test_token")
        let remote = RemoteEngine(client: client)

        XCTAssertEqual(remote.kind(), .remote)

        let macManifest = makeManifest(id: "mac:macbook-pro/qwen3-7b")
        XCTAssertTrue(remote.supports(model: macManifest))

        let localManifest = makeManifest(id: "local/qwen3-7b")
        XCTAssertFalse(remote.supports(model: localManifest))
    }

    func testLazyRemoteEngine() throws {
        final class ClientBox: @unchecked Sendable {
            var client: PeerClient?
        }
        let box = ClientBox()
        let lazy = LazyRemoteEngine { box.client }

        XCTAssertEqual(lazy.kind(), .remote)

        let macManifest = makeManifest(id: "mac:macbook-pro/qwen3-7b")
        let localManifest = makeManifest(id: "local/qwen3-7b")
        let opts = LoadOptions(contextLength: nil, pinnedEngine: nil)

        // No peer client available yet
        XCTAssertFalse(lazy.supports(model: macManifest))
        XCTAssertFalse(lazy.supports(model: localManifest))
        XCTAssertThrowsError(try lazy.load(model: macManifest, opts: opts))

        // Provide peer client
        let endpoint = PeerEndpoint(host: "192.168.1.10", port: 7480, fingerprint: "test_fp")
        box.client = PeerClient(endpoint: endpoint, token: "test_token")

        XCTAssertTrue(lazy.supports(model: macManifest))
        XCTAssertFalse(lazy.supports(model: localManifest))

        let loaded = try lazy.load(model: macManifest, opts: opts)
        XCTAssertEqual(loaded.handle, "qwen3-7b")
    }
}
