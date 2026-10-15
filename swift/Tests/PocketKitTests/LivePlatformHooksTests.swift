import XCTest
@testable import PocketKit

/// APL-03: exercises `LivePlatformHooks` against real system APIs. A debug
/// screen showing these on-device (L5 Settings -> About -> Diagnostics)
/// still needs L5's UI and a physical device for thermal/low-power-mode
/// fidelity - these tests only cover what's checkable headless, in CI, on
/// this Mac.
final class LivePlatformHooksTests: XCTestCase {
    func testKeychainRoundTripsAndOverwrites() throws {
        // The self-hosted CI runner runs as a headless launchd agent with no
        // unlocked login keychain, so Keychain Services (SecItemAdd) can't
        // succeed there. Run this interactively; skip it under CI rather than
        // mutating the host Mac's default keychain.
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["POCKET_SKIP_KEYCHAIN_TESTS"] == "1",
            "Keychain unavailable in the headless CI runner session"
        )
        let hooks = LivePlatformHooks()
        let key = "test.\(UUID().uuidString)"
        defer { _ = hooks.keychainSet(key: key, value: Data()) }

        XCTAssertNil(hooks.keychainGet(key: key))

        let first = Data("first".utf8)
        XCTAssertTrue(hooks.keychainSet(key: key, value: first))
        XCTAssertEqual(hooks.keychainGet(key: key), first)

        let second = Data("second".utf8)
        XCTAssertTrue(hooks.keychainSet(key: key, value: second))
        XCTAssertEqual(hooks.keychainGet(key: key), second, "overwriting an existing key should replace its value")
    }

    func testAppSupportDirCreatesModelsSubdirectory() {
        let hooks = LivePlatformHooks()
        let path = hooks.appSupportDir()
        var isDirectory: ObjCBool = false
        XCTAssertTrue(FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory))
        XCTAssertTrue(isDirectory.boolValue)

        let modelsPath = (path as NSString).appendingPathComponent("Models")
        XCTAssertTrue(FileManager.default.fileExists(atPath: modelsPath, isDirectory: &isDirectory))
        XCTAssertTrue(isDirectory.boolValue)
    }

    func testNowMsIsCloseToTheRealWallClock() {
        let hooks = LivePlatformHooks()
        let expected = Int64(Date().timeIntervalSince1970 * 1000)
        XCTAssertEqual(hooks.nowMs(), expected, accuracy: 1000)
    }

    func testDeviceProfileReportsPlausibleValues() {
        let hooks = LivePlatformHooks()
        let profile = hooks.deviceProfile()

        XCTAssertFalse(profile.modelIdentifier.isEmpty)
        XCTAssertFalse(profile.chip.isEmpty)
        XCTAssertGreaterThan(profile.totalRamBytes, 0)
        XCTAssertGreaterThan(profile.appMemoryLimitBytes, 0)
        XCTAssertNotEqual(profile.gpuFamily, "")

        #if os(macOS)
        XCTAssertEqual(profile.platform, .macOs)
        XCTAssertFalse(profile.lowPowerMode)
        XCTAssertTrue(profile.hasIncreasedMemoryEntitlement)
        #else
        XCTAssertEqual(profile.platform, .ios)
        #endif
    }
}
