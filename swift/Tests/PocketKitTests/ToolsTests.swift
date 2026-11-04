import XCTest
@testable import PocketKit

/// AGT-13: the Tools settings and executor. (The sandbox itself: WebSandboxTests.)
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
        XCTAssertEqual(e.availability(), ToolAvailability(web: true, javascript: false, python: false))
        s.allowTools = true
        XCTAssertEqual(e.availability(), ToolAvailability(web: true, javascript: true, python: true))
        s.set(.runPython, false)
        XCTAssertEqual(e.availability(), ToolAvailability(web: true, javascript: true, python: false), "Python has its own toggle")
    }

    func testPythonRequestsNeverOfferAlwaysAllow() {
        let py = ApprovalRequest(id: "p", conversationId: "c", tool: "run_python", summary: "", code: "1")
        let js = ApprovalRequest(id: "j", conversationId: "c", tool: "run_js", summary: "", code: "1")
        XCTAssertEqual(py.languageName, "Python")
        XCTAssertFalse(py.offersAlwaysAllow)
        XCTAssertEqual(js.languageName, "JavaScript")
        XCTAssertTrue(js.offersAlwaysAllow)
        XCTAssertEqual(ToolStep(kind: "ran", label: "python", detail: "{}").codeLanguageName, "Python")
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
