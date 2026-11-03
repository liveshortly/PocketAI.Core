import Foundation
import XCTest
@testable import PocketLiveShortly

/// LSI-18: the shared reconnect loop, driven by a scripted fake stream.
@MainActor
final class LSStreamLoopTests: XCTestCase {
    /// One scripted connection: frames, then an error (nil = clean end).
    struct Script {
        var frames: [LSStreamFrame]
        var error: Error? = URLError(.networkConnectionLost)
    }

    @MainActor
    final class Harness {
        var scripts: [Script] = []
        var lastIDs: [String?] = []
        var held: [LSEvent] = []
        var delays: [Double] = []
        var statuses: [LSStreamLoop.Status] = []
        var refetches = 0
        var refetchResult: () throws -> Bool = { true }
        var onConnect: (Int) -> Void = { _ in }
        var loop: LSStreamLoop!

        init(known: [LSEvent] = []) {
            held = known
            loop = LSStreamLoop(
                watchSystem: false,
                connect: { [unowned self] last in
                    lastIDs.append(last)
                    onConnect(lastIDs.count)
                    let script = scripts.isEmpty ? Script(frames: []) : scripts.removeFirst()
                    return AsyncThrowingStream { c in
                        for f in script.frames { c.yield(f) }
                        c.finish(throwing: script.error)
                    }
                },
                lastEventID: { [unowned self] in held.last.map { String($0.seq) } },
                refetch: { [unowned self] in
                    refetches += 1
                    return try refetchResult()
                },
                onFrame: { [unowned self] frame in
                    if case .event(let e) = frame { held.append(e) }
                },
                onStatus: { [unowned self] in statuses.append($0) })
            loop.sleep = { [unowned self] in delays.append($0) }
        }

        func run() async {
            loop.start(known: held.map(\.id))
            await loop.task?.value
        }
    }

    static func ev(_ seq: Int) -> LSStreamFrame {
        .event(LSEvent(id: "e\(seq)", seq: seq, actor: "agent", eventType: "output",
                       payload: .object([:]), ts: Date()))
    }

    func testDropMidwayResumesWithLastEventIDAndNoDuplicates() async {
        let h = Harness()
        h.scripts = [
            Script(frames: [.connected(status: "live"), Self.ev(1), Self.ev(2)]),
            // The server ignores Last-Event-ID and replays its whole buffer.
            Script(frames: [.connected(status: "live"), Self.ev(1), Self.ev(2), Self.ev(3), .sessionEnded],
                   error: nil),
        ]
        await h.run()
        XCTAssertEqual(h.lastIDs, [nil, "2"])
        XCTAssertEqual(h.held.map(\.seq), [1, 2, 3])
        XCTAssertEqual(h.delays, [1])
        XCTAssertEqual(h.statuses, [.connected, .reconnecting(attempt: 1), .connected])
    }

    func testAlreadyHeldEventsAreNotRedelivered() async {
        guard case .event(let held) = Self.ev(5) else { return XCTFail() }
        let h = Harness(known: [held])
        h.scripts = [Script(frames: [Self.ev(5), Self.ev(6), .sessionEnded], error: nil)]
        await h.run()
        XCTAssertEqual(h.lastIDs, ["5"])
        XCTAssertEqual(h.held.map(\.seq), [5, 6])
    }

    func testCleanEndWithoutSessionEndedReconnects() async {
        let h = Harness()
        h.scripts = [
            Script(frames: [Self.ev(1)], error: nil),
            Script(frames: [.sessionEnded], error: nil),
        ]
        await h.run()
        XCTAssertEqual(h.lastIDs, [nil, "1"])
    }

    func testBackoffCapsNeverGivesUpRefetchesEverySixthAndStopsOnStop() async {
        let h = Harness()
        h.onConnect = { [unowned h] n in if n == 20 { h.loop.stop() } }
        await h.run()
        XCTAssertEqual(h.lastIDs.count, 20)
        XCTAssertEqual(Array(h.delays.prefix(8)), [1, 2, 4, 8, 16, 30, 30, 30])
        XCTAssertEqual(h.delays.count, 19)
        XCTAssertEqual(h.refetches, 3) // attempts 6, 12, 18
        XCTAssertFalse(h.statuses.contains { if case .unreachable = $0 { true } else { false } })
    }

    func testFirstFrameResetsBackoff() async {
        let h = Harness()
        h.scripts = [Script(frames: []), Script(frames: [.connected(status: "live")]), Script(frames: [])]
        h.onConnect = { [unowned h] n in if n == 4 { h.loop.stop() } }
        await h.run()
        XCTAssertEqual(h.delays, [1, 1, 2])
    }

    func testExpiredTokenRefetchesAtOnceThenResumes() async {
        let h = Harness()
        h.scripts = [
            Script(frames: [], error: LSAPIError(status: 401, message: "stream rejected")),
            Script(frames: [Self.ev(1), .sessionEnded], error: nil),
        ]
        await h.run()
        XCTAssertEqual(h.refetches, 1)
        XCTAssertEqual(h.held.map(\.seq), [1])
    }

    func testGoneSessionStopsAsUnreachable() async {
        let h = Harness()
        h.scripts = [Script(frames: [], error: LSAPIError(status: 404, message: "stream rejected"))]
        h.refetchResult = { throw LSAPIError(status: 404, message: "session not found") }
        await h.run()
        XCTAssertEqual(h.lastIDs.count, 1)
        XCTAssertEqual(h.statuses.last, .unreachable("session not found"))
    }

    func testRefetchSeeingEndedSessionStops() async {
        let h = Harness()
        h.refetchResult = { false }
        await h.run()
        XCTAssertEqual(h.lastIDs.count, 6)
    }

    func testKickRestartsWithBackoffReset() async {
        let h = Harness()
        h.onConnect = { [unowned h] n in
            if n == 3 { h.loop.kick() }
            if n == 5 { h.loop.stop() }
        }
        await h.run() // returns when the first task is cancelled by kick
        await h.loop.task?.value
        // 1, 2 before the kick; the kicked task starts again from 1.
        XCTAssertEqual(h.delays, [1, 2, 1])
    }

    func testBackoffSchedule() {
        XCTAssertEqual((1...8).map(LSStreamLoop.backoff), [1, 2, 4, 8, 16, 30, 30, 30])
    }
}
