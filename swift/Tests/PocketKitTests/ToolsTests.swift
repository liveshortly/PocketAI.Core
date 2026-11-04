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
        XCTAssertEqual(e.availability(), ToolAvailability(web: true, javascript: false, python: false, mac: false))
        s.allowTools = true
        XCTAssertEqual(e.availability(), ToolAvailability(web: true, javascript: true, python: true, mac: false))
        s.set(.runPython, false)
        XCTAssertEqual(e.availability(), ToolAvailability(web: true, javascript: true, python: false, mac: false), "Python has its own toggle")
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

    func testRunOnMacIsOfferedOnlyWithAllSwitchesAndAConnectedMac() {
        struct Runner: MacCodeRunning {
            let connected: Bool
            var isMacConnected: Bool { connected }
            func run(language: String, code: String, timeoutMs: UInt32) async -> ToolRunResult {
                ToolRunResult(ok: true, output: "\(language):\(code)", error: "", durationMs: 1, truncated: false)
            }
        }
        let center = MainActor.assumeIsolated { ToolApprovalCenter { _, _, _ in } }
        let s = settings()
        let none = PocketToolExecutor(settings: s, approvals: center)
        let off = PocketToolExecutor(settings: s, macRunner: Runner(connected: false), approvals: center)
        let on = PocketToolExecutor(settings: s, macRunner: Runner(connected: true), approvals: center)
        XCTAssertFalse(on.availability().mac, "master switch off")
        s.allowTools = true
        XCTAssertTrue(on.availability().mac)
        XCTAssertFalse(off.availability().mac, "no connected Mac")
        XCTAssertFalse(none.availability().mac, "no runner (the Mac app)")
        s.set(.runOnMac, false)
        XCTAssertFalse(on.availability().mac, "its own toggle")
        s.set(.runOnMac, true)
        // Routed to the runner, with the "mac:" prefix stripped.
        let r = on.runCode(language: "mac:shell", code: "uname", timeoutMs: 10_000)
        XCTAssertEqual(r.output, "shell:uname")
        // Turned off while the card was open: nothing is sent.
        s.set(.runOnMac, false)
        XCTAssertFalse(on.runCode(language: "mac:shell", code: "uname", timeoutMs: 10_000).ok)
    }

    func testMacRefusalsAreWordedForThePhoneWithoutLicenceTalk() {
        func err(_ status: Int, _ code: String, _ message: String = "") -> String {
            let body = #"{"error":{"code":"\#(code)","message":"\#(message)"}}"#
            let r = MacRunReply.result(status: status, body: Data(body.utf8))
            XCTAssertFalse(r.ok)
            return r.error
        }
        XCTAssertEqual(err(403, "run_disabled"), MacRunReply.settingOff)
        XCTAssertEqual(err(403, "not_entitled", "needs Pro"), "This isn't available with your Mac yet.")
        XCTAssertEqual(err(501, "interpreter_missing", "Python isn't installed on this Mac."), "Python isn't installed on this Mac.")
        for code in ["run_disabled", "not_entitled", "unauthorized", "disconnected_by_host", "busy", "rate_limited", "code_too_large", "weird"] {
            let e = err(403, code, "needs Pro licence").lowercased()
            XCTAssertFalse(e.contains("pro") || e.contains("licen"), code)
        }
        let ok = MacRunReply.result(status: 200, body: Data(#"{"ok":true,"output":"42\n","error":null,"duration_ms":7,"truncated":false,"exit_code":0}"#.utf8))
        XCTAssertEqual(ok, ToolRunResult(ok: true, output: "42\n", error: "", durationMs: 7, truncated: false))
        let bad = MacRunReply.result(status: 200, body: Data(#"{"ok":false,"output":"x","error":"Timed out after 10 s.","duration_ms":10000,"truncated":false,"exit_code":null}"#.utf8))
        XCTAssertEqual(bad.error, "Timed out after 10 s.")
    }

    func testMacRequestsNeverOfferAlwaysAllowAndAreLabelled() {
        let mac = ApprovalRequest(id: "m", conversationId: "c", tool: "run_on_mac", summary: "Run Shell on your Mac", code: "ls")
        XCTAssertTrue(mac.runsOnMac)
        XCTAssertFalse(mac.offersAlwaysAllow)
        XCTAssertEqual(mac.languageName, "Shell")
        let step = ToolStep(kind: "ran", label: "mac:python", detail: #"{"code":"1","target":"mac"}"#)
        XCTAssertTrue(step.ranOnMac)
        XCTAssertEqual(step.codeLanguageName, "Python")
        XCTAssertTrue(RanCodeDetail(step: step).onMac)
        XCTAssertTrue(ToolsSettings.Tool.runOnMac.needsPairedMac)
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
