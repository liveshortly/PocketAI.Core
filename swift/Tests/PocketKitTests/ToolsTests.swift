import XCTest
@testable import PocketKit

/// AGT-13: the JavaScript sandbox's safety properties, and the Tools settings.
final class JavaScriptSandboxTests: XCTestCase {
    private func run(_ code: String, timeout: TimeInterval = 10, limit: Int = 20 * 1024) -> ToolRunResult {
        JavaScriptSandbox(timeout: timeout, outputLimit: limit).run(code)
    }

    func testLastExpressionAndConsoleAreCaptured() {
        let r = run("console.log('hi', 1, {a: 2}); print('there'); 6 * 7")
        XCTAssertTrue(r.ok, r.error)
        XCTAssertEqual(r.output, "hi 1 {\"a\":2}\nthere\n=> 42")
    }

    func testNoNetworkNoFilesNoTimers() {
        let r = run("""
        [typeof fetch, typeof XMLHttpRequest, typeof WebSocket, typeof require, typeof process,
         typeof importScripts, typeof setTimeout, typeof XMLHttpRequestUpload].join(',')
        """)
        XCTAssertEqual(r.output, "=> undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined")
    }

    func testEachRunStartsFresh() {
        XCTAssertTrue(run("globalThis.leak = 1").ok)
        XCTAssertEqual(run("typeof leak").output, "=> undefined")
    }

    func testErrorsAreReadableAndDontCrash() {
        let r = run("\n\nnotDefined + 1")
        XCTAssertFalse(r.ok)
        XCTAssertTrue(r.error.contains("ReferenceError"), r.error)
        XCTAssertTrue(r.error.contains("line 3"), r.error)
        let syntax = run("let = = 1")
        XCTAssertFalse(syntax.ok)
        XCTAssertTrue(syntax.error.contains("SyntaxError"), syntax.error)
        let thrown = run("console.log('before'); throw new Error('boom')")
        XCTAssertTrue(thrown.error.contains("boom"))
        XCTAssertEqual(thrown.output, "before")
    }

    func testOutputIsCappedAndFlagged() {
        let r = run("for (let i = 0; i < 100000; i++) console.log('x'.repeat(50))", limit: 1000)
        XCTAssertTrue(r.ok, r.error)
        XCTAssertTrue(r.truncated)
        XCTAssertLessThanOrEqual(r.output.utf8.count, 1000)
        XCTAssertFalse(run("console.log('small')").truncated)
    }

    func testRunawayLoopIsKilledAtTheTimeout() {
        let sandbox = JavaScriptSandbox(timeout: 1)
        let started = Date()
        let r = sandbox.run("while (true) {}")
        let took = Date().timeIntervalSince(started)
        XCTAssertFalse(r.ok)
        XCTAssertTrue(r.error.contains("ran too long"), r.error)
        XCTAssertLessThan(took, 2.0, "killed, not merely abandoned")
        XCTAssertEqual(sandbox.abandonedRuns, 0, "the watchdog really killed the script")
        // A script can't swallow the kill.
        let sneaky = sandbox.run("try { while (true) {} } catch (e) { 'survived' } finally { while (true) {} }")
        XCTAssertFalse(sneaky.ok)
        XCTAssertEqual(sandbox.abandonedRuns, 0)
        // And the sandbox is usable afterwards.
        XCTAssertEqual(sandbox.run("1 + 1").output, "=> 2")
    }

    func testStopKillsARunInProgress() {
        let sandbox = JavaScriptSandbox(timeout: 30)
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.3) { sandbox.cancel() }
        let started = Date()
        let r = sandbox.run("while (true) {}")
        XCTAssertLessThan(Date().timeIntervalSince(started), 3)
        XCTAssertFalse(r.ok)
        XCTAssertEqual(r.error, "Stopped.")
    }

    func testDeepRecursionIsAnErrorNotACrash() {
        let r = run("function f() { return f() + 1 } f()")
        XCTAssertFalse(r.ok)
        XCTAssertTrue(r.error.contains("RangeError"), r.error)
    }
}

final class ToolsSettingsTests: XCTestCase {
    private func settings() -> ToolsSettings {
        ToolsSettings(defaults: UserDefaults(suiteName: "tools-\(UUID().uuidString)")!)
    }

    func testCodeIsOffUntilTheMasterSwitchIsOn() {
        let s = settings()
        XCTAssertFalse(s.allowTools)
        XCTAssertTrue(s.isAvailable(.webSearch), "web search keeps AGT-12's behaviour")
        XCTAssertFalse(s.isAvailable(.runJavaScript))
        s.allowTools = true
        XCTAssertTrue(s.isAvailable(.runJavaScript))
        s.set(.runJavaScript, false)
        XCTAssertFalse(s.isAvailable(.runJavaScript), "needs its own switch too")
    }

    func testSettingsPersist() {
        let suite = "tools-\(UUID().uuidString)"
        let a = ToolsSettings(defaults: UserDefaults(suiteName: suite)!)
        a.allowTools = true
        a.set(.webSearch, false)
        let b = ToolsSettings(defaults: UserDefaults(suiteName: suite)!)
        XCTAssertTrue(b.allowTools)
        XCTAssertFalse(b.isOn(.webSearch))
        XCTAssertTrue(b.isOn(.runJavaScript))
    }

    func testExecutorOffersNothingWithMasterOff() {
        let s = settings()
        let center = MainActor.assumeIsolated { ToolApprovalCenter { _, _, _ in } }
        let e = PocketToolExecutor(settings: s, approvals: center)
        XCTAssertEqual(e.availability(), ToolAvailability(web: true, javascript: false))
        s.allowTools = true
        XCTAssertEqual(e.availability(), ToolAvailability(web: true, javascript: true))
    }

    func testRanCodeDetailParsesCoreJSON() {
        let step = ToolStep(kind: "ran", label: "javascript",
                            detail: #"{"code":"1+1","output":"2","error":"","duration_ms":4,"truncated":false}"#)
        XCTAssertEqual(RanCodeDetail(step: step), RanCodeDetail(code: "1+1", output: "2", durationMs: 4))
        let no = ToolStep(kind: "ran", label: "javascript", detail: #"{"code":"x","denied":true}"#)
        XCTAssertTrue(RanCodeDetail(step: no).denied)
    }

    @MainActor func testApprovalAnswersOnceAndOnlyForKnownIds() {
        let answers = Locked<[(String, Bool)]>([])
        let center = ToolApprovalCenter { id, _, always in answers.mutate { $0.append((id, always)) } }
        center.add(ApprovalRequest(id: "a", conversationId: "c", tool: "run_js", summary: "", code: "1"))
        center.answer("zzz", .run)
        XCTAssertTrue(answers.value.isEmpty)
        center.answer("a", .run, alwaysAllow: true)
        center.answer("a", .run)
        XCTAssertEqual(answers.value.count, 1)
        XCTAssertNil(center.request(for: "c"))
    }
}

private final class Locked<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: T
    init(_ value: T) { stored = value }
    var value: T { lock.withLock { stored } }
    func mutate(_ f: (inout T) -> Void) { lock.withLock { f(&stored) } }
}
