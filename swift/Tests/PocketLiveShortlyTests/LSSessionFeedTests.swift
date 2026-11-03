import SwiftUI
import XCTest
@testable import PocketLiveShortly

/// LSI-17: the shared list polls on a schedule and tolerates in-between states.
final class LSSessionFeedTests: XCTestCase {
    func testPollIntervalFastWhileLiveSlowOtherwisePausedHidden() {
        XCTAssertEqual(LSSessionFeed.pollInterval(phase: .active, anyLive: true), 10)
        XCTAssertEqual(LSSessionFeed.pollInterval(phase: .active, anyLive: false), 60)
        XCTAssertEqual(LSSessionFeed.pollInterval(phase: .inactive, anyLive: true), 60)
        XCTAssertNil(LSSessionFeed.pollInterval(phase: .background, anyLive: true))
    }

    func testStatusesAndRepoName() throws {
        func row(_ status: String, remote: String? = nil) throws -> LSSession {
            let r = remote.map { "\"git_remote\":\"\($0)\"," } ?? ""
            let json = """
            {"id":"a","title":"","owner_handle":"o","status":"\(status)",\(r)
            "event_count":0,"view_count":0,"created_at":"2026-11-03T05:00:00Z"}
            """
            return try JSONDecoder.liveShortly.decode(LSSession.self, from: Data(json.utf8))
        }
        XCTAssertTrue(try row("live").isLive)
        XCTAssertFalse(try row("live").isStarting)
        XCTAssertTrue(try row("starting").isLive)
        XCTAssertTrue(try row("pending").isStarting)
        XCTAssertFalse(try row("ended").isLive)
        XCTAssertEqual(try row("live", remote: "git@github.com:org/pocket.git").repoName, "pocket")
        XCTAssertEqual(try row("live", remote: "https://github.com/org/web").repoName, "web")
        XCTAssertNil(try row("live").repoName)
    }
}
