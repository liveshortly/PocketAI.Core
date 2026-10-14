import XCTest
@testable import PocketKit

final class W0SpikeTests: XCTestCase {
    func testSwiftToRustCall() {
        XCTAssertEqual(ping(), "pong")
        XCTAssertEqual(sampleModel().id, "sample")
    }

    func testRustToSwiftCallback() {
        final class Recorder: ProgressDelegate {
            var seen: [UInt32] = []
            func onProgress(percent: UInt32) { seen.append(percent) }
        }
        let recorder = Recorder()
        driveProgress(delegate: recorder)
        XCTAssertEqual(recorder.seen, [0, 50, 100])
    }
}
