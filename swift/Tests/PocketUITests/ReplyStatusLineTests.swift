import XCTest
import PocketKit
@testable import PocketUI

final class ReplyStatusLineTests: XCTestCase {
    func testLabelsNameTheArgument() {
        XCTAssertTrue(ReplyStatusLine.label(for: .tool(name: "web_search", arg: "macOS"), modelName: nil).contains("macOS"))
        XCTAssertTrue(ReplyStatusLine.label(for: .tool(name: "fetch_url", arg: "apple.com"), modelName: nil).contains("apple.com"))
        XCTAssertTrue(ReplyStatusLine.label(for: .loading, modelName: "Qwen3").contains("Qwen3"))
        XCTAssertTrue(ReplyStatusLine.label(for: .awaitingApproval(tool: "run_python"), modelName: nil).contains("Python"))
        XCTAssertTrue(ReplyStatusLine.label(for: .running(tool: "run_on_mac"), modelName: nil).contains("Mac"))
    }

    func testElapsedAppearsOnlyAfterThreeSeconds() {
        XCTAssertEqual(ReplyStatusLine.display("Thinking…", elapsed: 2.9), "Thinking…")
        XCTAssertEqual(ReplyStatusLine.display("Thinking…", elapsed: 4.2), "Thinking… · 4 s")
    }
}
