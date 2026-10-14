import XCTest
@testable import PocketKit

final class W0SpikeTests: XCTestCase {
    func testSwiftToRustCall() {
        XCTAssertEqual(ping(), "pong")
        XCTAssertEqual(sampleDeviceProfile().chip, "M4")
    }

    func testRustToSwiftCallback() {
        final class Recorder: ProgressDelegate, @unchecked Sendable {
            private let lock = NSLock()
            private var _seen: [UInt32] = []
            var seen: [UInt32] { lock.withLock { _seen } }
            func onProgress(percent: UInt32) { lock.withLock { _seen.append(percent) } }
        }
        let recorder = Recorder()
        driveProgress(delegate: recorder)
        XCTAssertEqual(recorder.seen, [0, 50, 100])
    }

    func testLiveCoreStreamsFromMockEngineThroughFFI() async throws {
        let hooks = LivePlatformHooks()
        let config = CoreConfig(appSupportDir: NSTemporaryDirectory(), platform: .macOs)
        let core: CoreClient = try LiveCore(config: config, platform: hooks)
        var words: [String] = []
        for try await chunk in core.mockGenerate(seed: 0, tokensPerSec: 200) {
            words.append(chunk)
        }
        XCTAssertEqual(
            words.joined(),
            "the quick brown fox jumps over lazy dog the quick brown fox jumps over lazy dog "
        )
    }

    func testPreviewCoreStreamsFixtureWords() async throws {
        let core: CoreClient = PreviewCore()
        var chunks: [String] = []
        for try await chunk in core.mockGenerate(seed: 0, tokensPerSec: 2000) {
            chunks.append(chunk)
        }
        XCTAssertFalse(chunks.isEmpty)
        XCTAssertTrue(chunks.joined().contains("Preview"))
    }
}
