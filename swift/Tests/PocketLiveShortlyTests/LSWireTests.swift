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

    func testHostDecodesAgentModelsAndFallsBackWithout() throws {
        let newJSON = """
        {"id":"h1","name":"mbp","os":"darwin","arch":"arm64","dirs":["/x"],
        "agents":["claude","codex","ollama","gemini"],"models":["llama3"],
        "agent_models":[
          {"name":"codex","models":[{"id":"gpt-5","label":"GPT-5"},{"id":"o3"}],
           "default_model":"gpt-5","model_switchable":true},
          {"name":"gemini","models":[]}]}
        """
        let h = try JSONDecoder.liveShortly.decode(LSHost.self, from: Data(newJSON.utf8))
        XCTAssertEqual(h.models(forAgent: "codex"), ["gpt-5", "o3"])
        XCTAssertEqual(h.modelLabel("gpt-5", forAgent: "codex"), "GPT-5")
        XCTAssertEqual(h.modelLabel("o3", forAgent: "codex"), "o3")
        XCTAssertEqual(h.defaultModel(forAgent: "codex"), "gpt-5")
        XCTAssertTrue(h.modelSwitchable(forAgent: "codex"))
        XCTAssertEqual(h.models(forAgent: "gemini"), [])
        XCTAssertFalse(h.modelSwitchable(forAgent: "gemini"))
        // No entry for claude/ollama: today's fallbacks.
        XCTAssertEqual(h.models(forAgent: "claude"), LS.claudeModels)
        XCTAssertEqual(h.models(forAgent: "ollama"), ["llama3"])

        let oldJSON = """
        {"id":"h1","name":"mbp","os":"darwin","arch":"arm64","dirs":["/x"],
        "agents":["claude","codex"]}
        """
        let old = try JSONDecoder.liveShortly.decode(LSHost.self, from: Data(oldJSON.utf8))
        XCTAssertNil(old.agentModels)
        XCTAssertEqual(old.models(forAgent: "claude"), LS.claudeModels)
        XCTAssertEqual(old.models(forAgent: "codex"), [])
        XCTAssertTrue(old.modelSwitchable(forAgent: "claude"))

        // Remembered pick is only reused while still on the list.
        let d = try XCTUnwrap(UserDefaults(suiteName: "ls-model-test-\(UUID().uuidString)"))
        h.rememberModel("o3", forAgent: "codex", defaults: d)
        XCTAssertEqual(h.lastModel(forAgent: "codex", defaults: d), "o3")
        h.rememberModel("gone", forAgent: "codex", defaults: d)
        XCTAssertNil(h.lastModel(forAgent: "codex", defaults: d))
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
