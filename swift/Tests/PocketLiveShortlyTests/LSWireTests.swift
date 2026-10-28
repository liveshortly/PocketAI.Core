import XCTest
@testable import PocketLiveShortly

final class LSWireTests: XCTestCase {
    func testSessionDecodesFromServerJSON() throws {
        let json = """
        {"results":[{"id":"abc","title":"Fix race","owner_handle":"rohit",
        "model":"claude-sonnet","framework":"claude-code","status":"live",
        "tags":[],"event_count":12,"view_count":3,
        "created_at":"2026-10-24T05:00:00Z","ended_at":null}],"total":1}
        """
        struct List: Codable { var results: [LSSession] }
        let l = try JSONDecoder.liveShortly.decode(List.self, from: Data(json.utf8))
        XCTAssertEqual(l.results[0].id, "abc")
        XCTAssertTrue(l.results[0].isLive)
        XCTAssertEqual(l.results[0].eventCount, 12)
    }

    func testDatesParseBothRFC3339Forms() {
        XCTAssertNotNil(LSDates.parse("2026-10-24T05:00:00Z"))
        XCTAssertNotNil(LSDates.parse("2026-10-24T05:00:00.123456Z"))
        XCTAssertNil(LSDates.parse("yesterday"))
    }

    func testEventPayloadLookup() throws {
        let json = """
        {"id":"e1","session_id":"abc","seq":4,"actor":"viewer",
        "event_type":"viewer_comment",
        "payload":{"message":"run ls","username":"marcus"},
        "ts":"2026-10-24T05:01:00Z"}
        """
        let e = try JSONDecoder.liveShortly.decode(LSEvent.self, from: Data(json.utf8))
        XCTAssertEqual(e.payload["message"]?.stringValue, "run ls")
        XCTAssertEqual(lsBody(of: e), "run ls")
        XCTAssertEqual(handle(of: e, fallback: "x"), "@marcus")
    }
}
