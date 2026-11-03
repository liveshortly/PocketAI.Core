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

    // MARK: - LSI-19 sessions shared with me

    private func session(_ id: String, role: String? = nil) throws -> LSSession {
        let r = role.map { "\"shared_role\":\"\($0)\"," } ?? ""
        let json = """
        {"id":"\(id)","title":"","owner_handle":"ana","status":"ended",\(r)
        "agent":"codex","event_count":0,"view_count":0,"created_at":"2026-11-03T05:00:00Z"}
        """
        return try JSONDecoder.liveShortly.decode(LSSession.self, from: Data(json.utf8))
    }

    func testRoleToControls() {
        let owner = LSRole(isOwner: true, canComment: true)
        let commenter = LSRole(isOwner: false, canComment: true)
        let viewer = LSRole(isOwner: false, canComment: false)
        XCTAssertEqual([owner, commenter, viewer], [.owner, .commenter, .viewer])
        XCTAssertEqual(LSRole(isOwner: nil, canComment: nil), .viewer) // unknown = least access
        XCTAssertEqual([owner, commenter, viewer].map(\.canComment), [true, true, false])
        XCTAssertEqual([owner, commenter, viewer].map(\.canControl), [true, false, false])
    }

    func testMineAndSharedMergeDedupesByID() throws {
        let mine = [try session("a"), try session("b")]
        let shared = [try session("b", role: "viewer"), try session("c", role: "commenter"),
                      try session("c", role: "commenter"), try session("d", role: "viewer")]
        XCTAssertEqual(LSSessionFeed.sharedOnly(shared, mine: mine).map(\.id), ["c", "d"])
        XCTAssertEqual(LSSessionFeed.sharedOnly([], mine: mine), [])
    }

    func testSharedRowLine() throws {
        XCTAssertEqual(try session("c", role: "viewer").sharedRole, "viewer")
        XCTAssertEqual(try session("c", role: "viewer").sharedLine, "@ana · codex · Ended")
        XCTAssertNil(try session("a").sharedLine)
    }

    func testSessionLinks() {
        let server = URL(string: "https://ls.example.dev")!
        func id(_ s: String) -> String? { LSSessionLink.sessionID(in: URL(string: s)!, server: server) }
        XCTAssertEqual(id("https://liveshortly.com/session/abc"), "abc")
        XCTAssertEqual(id("https://ls.example.dev/session/abc"), "abc")
        XCTAssertEqual(id("pocketai://session/abc"), "abc")
        XCTAssertNil(id("https://evil.com/session/abc"))
        XCTAssertNil(id("https://liveshortly.com/feed"))
        XCTAssertNil(id("pocketai://chat/abc"))
    }
}
