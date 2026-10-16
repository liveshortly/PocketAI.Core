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
        // catalog() is wired to the real bundled catalog now (CORE-10/L0-06
        // follow-on), not a stub - at least the 26 curated entries come back.
        XCTAssertGreaterThanOrEqual(
            core.catalog(filter: CatalogFilter(modality: nil, curatedOnly: false, installedOnly: false)).count,
            20
        )

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

    func testPreviewCoreProjectsAndDocuments() throws {
        let core: CoreClient = PreviewCore()
        let initialProjects = core.listProjects()
        XCTAssertFalse(initialProjects.isEmpty)

        let newProj = try core.createProject(name: "Test Project", instructions: "Test instructions")
        XCTAssertEqual(newProj.name, "Test Project")
        XCTAssertEqual(newProj.instructions, "Test instructions")

        try core.renameProject(id: newProj.id, name: "Renamed Project")
        let updated = core.listProjects().first { $0.id == newProj.id }
        XCTAssertEqual(updated?.name, "Renamed Project")

        let doc = try core.addDocument(
            projectId: newProj.id,
            name: "test.txt",
            mime: "text/plain",
            pages: [DocumentPage(page: 1, text: "Sample text content for retrieval")]
        )
        XCTAssertEqual(doc.name, "test.txt")
        XCTAssertEqual(core.listDocuments(projectId: newProj.id).count, 1)

        let hits = core.searchProject(projectId: newProj.id, query: "retrieval", k: 3)
        XCTAssertFalse(hits.isEmpty)

        try core.deleteDocument(id: doc.id)
        XCTAssertTrue(core.listDocuments(projectId: newProj.id).isEmpty)

        try core.deleteProject(id: newProj.id)
        XCTAssertNil(core.listProjects().first { $0.id == newProj.id })
    }

    func testLiveCoreProjectsAndDocuments() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = CoreConfig(appSupportDir: tempDir.path, platform: .macOs)
        let core: CoreClient = try LiveCore(config: config, platform: StubPlatformHooks())

        let proj = try core.createProject(name: "Live Project", instructions: "Live instructions")
        XCTAssertEqual(proj.name, "Live Project")
        XCTAssertTrue(core.listProjects().contains { $0.id == proj.id })

        try core.renameProject(id: proj.id, name: "Live Project Renamed")
        XCTAssertEqual(core.listProjects().first { $0.id == proj.id }?.name, "Live Project Renamed")

        let doc = try core.addDocument(
            projectId: proj.id,
            name: "live.txt",
            mime: "text/plain",
            pages: [DocumentPage(page: 1, text: "The swift brown fox jumps over the lazy dog in retrieval test.")]
        )
        XCTAssertEqual(doc.name, "live.txt")
        XCTAssertEqual(core.listDocuments(projectId: proj.id).count, 1)

        let hits = core.searchProject(projectId: proj.id, query: "fox", k: 5)
        XCTAssertFalse(hits.isEmpty)

        try core.deleteDocument(id: doc.id)
        XCTAssertEqual(core.listDocuments(projectId: proj.id).count, 0)

        try core.deleteProject(id: proj.id)
        XCTAssertFalse(core.listProjects().contains { $0.id == proj.id })
    }

    func testLiveCoreSetStoreEntitlement() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = CoreConfig(appSupportDir: tempDir.path, platform: .macOs)
        let core: CoreClient = try LiveCore(config: config, platform: StubPlatformHooks())
        XCTAssertFalse(core.entitlements().pro)

        core.setStoreEntitlement(StoreEntitlement(pro: true, transactionId: "txn_test_123"))
        let ent = core.entitlements()
        XCTAssertTrue(ent.pro)
        XCTAssertEqual(ent.source, "store")

        core.setStoreEntitlement(StoreEntitlement(pro: false, transactionId: nil))
        XCTAssertFalse(core.entitlements().pro)
    }

    func testPreviewCoreSetStoreEntitlement() throws {
        let core: CoreClient = PreviewCore()
        core.setStoreEntitlement(StoreEntitlement(pro: true, transactionId: "preview_txn"))
        XCTAssertTrue(core.entitlements().pro)
    }

    func testLiveCoreActivateLicenseInvalidKeyThrows() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = CoreConfig(appSupportDir: tempDir.path, platform: .macOs)
        let core: CoreClient = try LiveCore(config: config, platform: StubPlatformHooks())
        XCTAssertThrowsError(try core.activateLicense(key: "invalid_license_key"))
    }

    func testLiveCoreActivateLicenseValidKeySucceeds() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = CoreConfig(appSupportDir: tempDir.path, platform: .macOs)
        let core: CoreClient = try LiveCore(config: config, platform: StubPlatformHooks())
        XCTAssertFalse(core.entitlements().pro)

        let validKey = "key/eyJhY2NvdW50Ijp7ImlkIjoiNDBmZTcyNjAtZDc5Zi00NmFkLTljMGMtOTcyMzMxZjRmZmMwIn0sInByb2R1Y3QiOnsiaWQiOiIwYjc5MDQ1YS1kOTlkLTQwMjMtOWI4Ni02NjAwODA3YTliOGUifSwicG9saWN5Ijp7ImlkIjoiNDVhMGZmMGUtNzYzNy00M2FjLWIwNjMtODEzOTZlZDQ3MGU4IiwiZHVyYXRpb24iOm51bGx9LCJ1c2VyIjpudWxsLCJsaWNlbnNlIjp7ImlkIjoiMjNkNGFiNWItZTQzYy00YjAyLWE4ZWItYWRjNDcwNjgzOWY2IiwiY3JlYXRlZCI6IjIwMjYtMTAtMTVUMDY6NTg6MzcuMjAzWiIsImV4cGlyeSI6IjIwMjgtMDMtMzFUMTg6MzA6MDAuMDAwWiJ9fQ==.fhC7LYG5-OOt4G4mG34XETvcNXnRYNuMvx9kg-lktpuq9N7NACJndb1FVPYBYCRq-0CU-N8-eOi44w4gkkyLBA=="
        let ent = try core.activateLicense(key: validKey)
        XCTAssertTrue(ent.pro)
        XCTAssertTrue(ent.source.hasPrefix("license:"))
        XCTAssertTrue(core.entitlements().pro)
    }

    func testPreviewCoreActivateLicense() throws {
        let core: CoreClient = PreviewCore()
        let ent = try core.activateLicense(key: "any_key")
        XCTAssertTrue(ent.pro)
    }

    @MainActor
    func testStoreEntitlementManagerInAppPurchasesDisabled() async throws {
        let manager = StoreEntitlementManager()
        await manager.start()
        XCTAssertFalse(manager.isPro)
        do {
            _ = try await manager.purchase()
            XCTFail("expected inAppPurchasesDisabled error")
        } catch let error as StoreError {
            XCTAssertEqual(error, StoreError.inAppPurchasesDisabled)
        }
        try await manager.restorePurchases()
        XCTAssertFalse(manager.isPro)
        manager.stop()
    }
}
