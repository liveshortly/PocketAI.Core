#if os(macOS)
import XCTest
@testable import PocketKit

/// MAC-19: moving a sandboxed install's data out of its container.
final class SandboxMigrationTests: XCTestCase {
    private let fm = FileManager.default
    private let id = "test.pocketai.\(UUID().uuidString)"
    private var root: URL!
    private var container: URL { root.appendingPathComponent("Containers/\(id)/Data") }
    private var home: URL { root.appendingPathComponent("home") }

    override func setUpWithError() throws {
        root = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try fm.createDirectory(at: home, withIntermediateDirectories: true)
    }

    override func tearDown() {
        try? fm.removeItem(at: root)
        UserDefaults.standard.removePersistentDomain(forName: id)
    }

    private func write(_ text: String, _ url: URL) throws {
        try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(text.utf8).write(to: url)
    }

    func testMovesStoreAndCacheIntoEmptyHomeAndMergesPrefs() throws {
        let support = "Library/Application Support/\(id)"
        try write("db", container.appendingPathComponent("\(support)/pocket.sqlite"))
        try write("w", container.appendingPathComponent("\(support)/Models/m/weights"))
        try write("{}", container.appendingPathComponent("Library/Application Support/liveshortly-cache/s.json"))
        // The app's appSupportDir() may already have made an empty skeleton.
        try fm.createDirectory(at: home.appendingPathComponent("\(support)/Models"), withIntermediateDirectories: true)
        let prefs = container.appendingPathComponent("Library/Preferences/\(id).plist")
        try fm.createDirectory(at: prefs.deletingLastPathComponent(), withIntermediateDirectories: true)
        XCTAssertTrue((["old": "container", "kept": "container"] as NSDictionary).write(to: prefs, atomically: true))
        UserDefaults.standard.setPersistentDomain(["kept": "home"], forName: id)

        let plan = planSandboxMigration(container: container, home: home, bundleID: id)
        XCTAssertEqual(plan.moves.count, 2)
        XCTAssertEqual(plan.skipped, [])
        XCTAssertEqual(plan.preferences, prefs)

        let (_, ok) = runSandboxMigration(plan, bundleID: id, defaults: .standard)
        XCTAssertTrue(ok)
        XCTAssertEqual(try String(contentsOf: home.appendingPathComponent("\(support)/pocket.sqlite"), encoding: .utf8), "db")
        XCTAssertTrue(fm.fileExists(atPath: home.appendingPathComponent("\(support)/Models/m/weights").path))
        XCTAssertTrue(fm.fileExists(atPath: home.appendingPathComponent("Library/Application Support/liveshortly-cache/s.json").path))
        XCTAssertFalse(fm.fileExists(atPath: container.appendingPathComponent(support).path))
        let domain = UserDefaults.standard.persistentDomain(forName: id)
        XCTAssertEqual(domain?["old"] as? String, "container")
        XCTAssertEqual(domain?["kept"] as? String, "home") // existing value wins

        // Second run: nothing left to move.
        XCTAssertEqual(planSandboxMigration(container: container, home: home, bundleID: id).moves, [])
    }

    func testNeverOverwritesExistingHomeData() throws {
        let support = "Library/Application Support/\(id)"
        try write("container", container.appendingPathComponent("\(support)/pocket.sqlite"))
        try write("home", home.appendingPathComponent("\(support)/pocket.sqlite"))

        let plan = planSandboxMigration(container: container, home: home, bundleID: id)
        XCTAssertEqual(plan.moves, [])
        XCTAssertEqual(plan.skipped.map(\.lastPathComponent), [id])
        XCTAssertNil(plan.preferences)

        XCTAssertTrue(runSandboxMigration(plan, bundleID: id, defaults: .standard).ok)
        XCTAssertEqual(try String(contentsOf: home.appendingPathComponent("\(support)/pocket.sqlite"), encoding: .utf8), "home")
        XCTAssertTrue(fm.fileExists(atPath: container.appendingPathComponent("\(support)/pocket.sqlite").path))
    }

    func testNoContainerIsEmptyPlan() {
        XCTAssertEqual(planSandboxMigration(container: container, home: home, bundleID: id), SandboxMigrationPlan())
    }
}
#endif
