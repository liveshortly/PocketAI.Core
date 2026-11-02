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

    func testTranscriptCacheRoundTripCapsAndSkipsEchoes() throws {
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("ls-cache-test-\(UUID().uuidString)", isDirectory: true)
        LSTranscriptCache.baseDirectory = tmp
        defer { try? FileManager.default.removeItem(at: tmp) }

        let detail = LSSessionDetail(
            id: "s1", title: "t", ownerHandle: "o", model: nil, framework: nil,
            status: "live", eventCount: 3,
            createdAt: Date(timeIntervalSince1970: 1_700_000_000), endedAt: nil,
            watcherCount: nil, canComment: nil, visibility: nil, linkRole: nil,
            publishedAt: nil, isOwner: nil, agentConnected: nil, forkedFrom: nil,
            events: [LSEvent(
                id: "dup", seq: 1, actor: "agent", eventType: "output",
                payload: .null, ts: Date(timeIntervalSince1970: 1_700_000_000))])

        // maxEvents + 2 real events, plus one local echo that must not persist.
        var events = (1...(LSTranscriptCache.maxEvents + 2)).map {
            LSEvent(id: "e\($0)", seq: $0, actor: "agent", eventType: "output",
                    payload: .object(["text": .string("x")]),
                    ts: Date(timeIntervalSince1970: 1_700_000_000))
        }
        events.append(LSEvent(
            id: "local-1", seq: 9_999, actor: "viewer", eventType: "viewer_comment",
            payload: .object([:]), ts: Date(timeIntervalSince1970: 1_700_000_000)))

        LSTranscriptCache.save("s1", detail: detail, events: events)
        let cached = try XCTUnwrap(LSTranscriptCache.load("s1"))
        XCTAssertTrue(cached.truncated)
        XCTAssertEqual(cached.events.count, LSTranscriptCache.maxEvents)
        // Echo dropped before capping: the newest kept seq is the real tail.
        XCTAssertEqual(cached.events.last?.seq, LSTranscriptCache.maxEvents + 2)
        XCTAssertEqual(cached.events.first?.seq, 3)
        XCTAssertEqual(cached.events.first?.payload["text"]?.stringValue, "x")
        // Transcript is stored once, not duplicated inside detail.
        XCTAssertTrue(cached.detail.events.isEmpty)
        XCTAssertEqual(cached.detail.status, "live")
        XCTAssertNil(LSTranscriptCache.load("missing"))

        LSTranscriptCache.remove("s1")
        XCTAssertNil(LSTranscriptCache.load("s1"))
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
