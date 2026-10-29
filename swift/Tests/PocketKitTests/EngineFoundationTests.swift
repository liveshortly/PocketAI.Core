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

    /// Manual peer addresses (VPN / Tailscale MagicDNS, where Bonjour is dead)
    /// are typed by hand, so the parser is the trust boundary in front of
    /// `PeerClient`'s force-unwrapped `https://host:port` URL.
    func testManualPeerAddressParsing() {
        XCTAssertEqual(PeerAddressBook.parse("mac.tail1234.ts.net", defaultPort: 7480),
                       ResolvedPeerAddress(host: "mac.tail1234.ts.net", port: 7480))
        XCTAssertEqual(PeerAddressBook.parse(" https://mac.tail1234.ts.net:9000/ ", defaultPort: 7480),
                       ResolvedPeerAddress(host: "mac.tail1234.ts.net", port: 9000))
        XCTAssertEqual(PeerAddressBook.parse("100.64.0.3", defaultPort: 7480),
                       ResolvedPeerAddress(host: "100.64.0.3", port: 7480))
        XCTAssertNil(PeerAddressBook.parse("", defaultPort: 7480))
        XCTAssertNil(PeerAddressBook.parse("bad host", defaultPort: 7480))
        XCTAssertNil(PeerAddressBook.parse("mac.ts.net/path", defaultPort: 7480))
        XCTAssertNil(PeerAddressBook.parse("mac.ts.net", defaultPort: 0))
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
        let lazy = LazyRemoteEngine { _ in box.client }

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

// MARK: - Model naming

extension EngineFoundationTests {
    /// Model ids carry org, revision, quantisation and format. None of that
    /// belongs in a label, and the long form is what the user complained about.
    func testShortModelNames() {
        XCTAssertEqual(
            ModelNaming.short(
                "mlx-community/Qwen3-4B-Instruct-2507-4bit@50d427756c6b1b2fe0c0a10f67fbda1fc8e82c1b"),
            "Qwen3 4B")
        XCTAssertEqual(
            ModelNaming.short("unsloth/Qwen3-4B-Instruct-2507-GGUF@a06e946bb6b"),
            "Qwen3 4B")
        XCTAssertEqual(ModelNaming.short("hf.co/Reza2kn/Jev-Omni-Q4_K_M-GGUF:Q4_K_M"),
                       "Jev Omni")
        XCTAssertEqual(ModelNaming.short("llama3.1:70b"), "llama3.1")
        XCTAssertEqual(ModelNaming.short("phi4-mini:latest"), "phi4 mini")
        // A remote id names the model, not the machine path.
        XCTAssertEqual(
            ModelNaming.short("mac:peer-123/mlx-community/Qwen3-8B-4bit@abc"),
            "Qwen3 8B")
        XCTAssertTrue(ModelNaming.isRemote("mac:peer-123/whatever"))
        XCTAssertFalse(ModelNaming.isRemote("mlx-community/Qwen3-8B-4bit"))
        // Remote labels say which Mac, because borrowed is not local.
        XCTAssertEqual(
            ModelNaming.remote("mac:peer-123/mlx-community/Qwen3-8B-4bit@abc", machine: "mini"),
            "Qwen3 8B · mini")
        // Never empty, whatever it is handed.
        XCTAssertFalse(ModelNaming.short("weird").isEmpty)
    }
}
