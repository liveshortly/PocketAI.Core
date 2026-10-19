import XCTest
@testable import PocketKit

/// APL-09: Unit tests for WHATWG Server-Sent Events parsing.
final class SSEParserTests: XCTestCase {
    func testSingleEventDispatchOnBlankLine() {
        var parser = SSEParser()
        XCTAssertNil(parser.feed(line: "data: hello world"))
        let event = parser.feed(line: "")
        XCTAssertEqual(event, SSEEvent(id: nil, event: nil, data: "hello world"))
    }

    func testEventWithCustomNameAndId() {
        var parser = SSEParser()
        XCTAssertNil(parser.feed(line: "id: 101"))
        XCTAssertNil(parser.feed(line: "event: delta"))
        XCTAssertNil(parser.feed(line: "data: payload"))
        let event = parser.feed(line: "")
        XCTAssertEqual(event, SSEEvent(id: "101", event: "delta", data: "payload"))
        XCTAssertEqual(parser.lastEventId, "101")
    }

    func testMultiLineDataJoinedWithNewline() {
        var parser = SSEParser()
        XCTAssertNil(parser.feed(line: "data: line 1"))
        XCTAssertNil(parser.feed(line: "data: line 2"))
        XCTAssertNil(parser.feed(line: "data: line 3"))
        let event = parser.feed(line: "")
        XCTAssertEqual(event, SSEEvent(id: nil, event: nil, data: "line 1\nline 2\nline 3"))
    }

    func testCommentLinesIgnored() {
        var parser = SSEParser()
        XCTAssertNil(parser.feed(line: ": this is a comment / keep-alive"))
        XCTAssertNil(parser.feed(line: "data: actual data"))
        XCTAssertNil(parser.feed(line: ": another comment"))
        let event = parser.feed(line: "")
        XCTAssertEqual(event, SSEEvent(id: nil, event: nil, data: "actual data"))
    }

    func testIdPersistsAcrossSubsequentEventsUntilOverridden() {
        var parser = SSEParser()
        XCTAssertNil(parser.feed(line: "id: first-id"))
        XCTAssertNil(parser.feed(line: "data: first"))
        let first = parser.feed(line: "")
        XCTAssertEqual(first?.id, "first-id")
        XCTAssertEqual(parser.lastEventId, "first-id")

        // Second event without an explicit id inherits lastEventId per spec
        XCTAssertNil(parser.feed(line: "data: second"))
        let second = parser.feed(line: "")
        XCTAssertEqual(second?.id, "first-id")
        XCTAssertEqual(parser.lastEventId, "first-id")

        // Third event overrides id
        XCTAssertNil(parser.feed(line: "id: second-id"))
        XCTAssertNil(parser.feed(line: "data: third"))
        let third = parser.feed(line: "")
        XCTAssertEqual(third?.id, "second-id")
        XCTAssertEqual(parser.lastEventId, "second-id")
    }

    func testConsecutiveBlankLinesDoNotDispatchEmptyEvents() {
        var parser = SSEParser()
        XCTAssertNil(parser.feed(line: ""))
        XCTAssertNil(parser.feed(line: ""))
    }

    func testPayloadWithBlankLineYieldsEvent() {
        var parser = SSEParser()
        let events = parser.feed(chunk: "data: {\"delta\":\"hi\"}\n\n")
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events.first, SSEEvent(id: nil, event: nil, data: "{\"delta\":\"hi\"}"))
    }

    func testSplitAcrossReadsYieldsEvent() {
        var parser = SSEParser()
        let chunk1 = "data: {\"del"
        let chunk2 = "ta\":\"hi\"}\n\n"

        let events1 = parser.feed(chunk: chunk1)
        XCTAssertTrue(events1.isEmpty)

        let events2 = parser.feed(chunk: chunk2)
        XCTAssertEqual(events2.count, 1)
        XCTAssertEqual(events2.first, SSEEvent(id: nil, event: nil, data: "{\"delta\":\"hi\"}"))
    }

    func testSplitAtNewlineBoundary() {
        var parser = SSEParser()
        let events1 = parser.feed(chunk: "data: {\"delta\":\"hi\"}\n")
        XCTAssertTrue(events1.isEmpty)

        let events2 = parser.feed(chunk: "\n")
        XCTAssertEqual(events2.count, 1)
        XCTAssertEqual(events2.first, SSEEvent(id: nil, event: nil, data: "{\"delta\":\"hi\"}"))
    }

    func testPayloadWithCRLFAndSplitReads() {
        var parser = SSEParser()
        let events1 = parser.feed(chunk: "data: {\"delta\":\"hi\"}\r")
        XCTAssertTrue(events1.isEmpty)

        let events2 = parser.feed(chunk: "\n\r\n")
        XCTAssertEqual(events2.count, 1)
        XCTAssertEqual(events2.first, SSEEvent(id: nil, event: nil, data: "{\"delta\":\"hi\"}"))
    }
}
