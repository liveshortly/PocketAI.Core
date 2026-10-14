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
}
