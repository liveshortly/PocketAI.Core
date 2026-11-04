import XCTest
@testable import PocketKit

/// AGT-14 / AGT-13 stage 3: the hidden web view sandbox, driven for real
/// (a WKWebView, Web Workers, the bundled Pyodide).
@MainActor
final class WebSandboxTests: XCTestCase {
    private func js(_ code: String, timeout: TimeInterval = 10, limit: Int = 20 * 1024) async -> ToolRunResult {
        let sandbox = WebSandbox(outputLimit: limit)
        defer { sandbox.shutdown() }
        return await sandbox.run(language: "javascript", code: code, timeout: timeout)
    }

    // MARK: JavaScript

    func testLastExpressionAndConsoleAreCaptured() async {
        let r = await js("console.log('hi', 1, {a: 2}); print('there'); 6 * 7")
        XCTAssertTrue(r.ok, r.error)
        XCTAssertEqual(r.output, "hi 1 {\"a\":2}\nthere\n=> 42")
    }

    func testNoNetwork() async {
        let r = await js("""
        const out = [];
        try { fetch('https://example.com/') } catch (e) { out.push('fetch: ' + e.message) }
        try { new XMLHttpRequest() } catch (e) { out.push('xhr blocked') }
        try { new WebSocket('wss://example.com') } catch (e) { out.push('ws blocked') }
        try { importScripts('https://example.com/x.js') } catch (e) { out.push('import blocked') }
        out.join('\\n')
        """)
        XCTAssertTrue(r.ok, r.error)
        XCTAssertTrue(r.output.contains("fetch: fetch is not available"), r.output)
        XCTAssertTrue(r.output.contains("xhr blocked") && r.output.contains("ws blocked") && r.output.contains("import blocked"), r.output)
    }

    /// The shadowing above is a courtesy; the CSP is the wall. Reach the real
    /// `fetch` through the prototype chain and watch the policy refuse it.
    func testCSPBlocksTheRealFetch() async {
        let r = await js("""
        (function () {
          for (let o = Object.getPrototypeOf(self); o; o = Object.getPrototypeOf(o)) {
            const d = Object.getOwnPropertyDescriptor(o, 'fetch');
            if (d) return d.value.call(self, 'https://example.com/').then(() => 'NETWORK REACHED');
          }
          return 'fetch not reachable';
        })()
        """)
        XCTAssertFalse(r.output.contains("NETWORK REACHED"), r.output)
        print("CSP fetch probe -> ok=\(r.ok) output=\(r.output) error=\(r.error)")
    }

    func testEachRunStartsFresh() async {
        let sandbox = WebSandbox()
        defer { sandbox.shutdown() }
        let a = await sandbox.run(language: "javascript", code: "globalThis.leak = 1")
        XCTAssertTrue(a.ok, a.error)
        let b = await sandbox.run(language: "javascript", code: "typeof leak")
        XCTAssertEqual(b.output, "=> undefined")
    }

    func testThrownErrorsAreReadable() async {
        let thrown = await js("console.log('before'); throw new Error('boom')")
        XCTAssertFalse(thrown.ok)
        XCTAssertEqual(thrown.error, "Error: boom")
        XCTAssertEqual(thrown.output, "before")
        let ref = await js("notDefined + 1")
        XCTAssertTrue(ref.error.contains("ReferenceError"), ref.error)
        let syntax = await js("let = = 1")
        XCTAssertTrue(syntax.error.contains("SyntaxError"), syntax.error)
    }

    func testOutputIsCappedAndFlagged() async {
        let r = await js("for (let i = 0; i < 100000; i++) console.log('x'.repeat(50))", limit: 1000)
        XCTAssertTrue(r.ok, r.error)
        XCTAssertTrue(r.truncated)
        XCTAssertLessThanOrEqual(r.output.utf8.count, 1000)
        let small = await js("console.log('small')")
        XCTAssertFalse(small.truncated)
    }

    func testRunawayLoopIsKilledAndTheAppStaysResponsive() async {
        let sandbox = WebSandbox()
        defer { sandbox.shutdown() }
        var ticks = 0
        let ticker = Task { @MainActor in
            while !Task.isCancelled { try? await Task.sleep(for: .milliseconds(50)); ticks += 1 }
        }
        let started = Date()
        let r = await sandbox.run(language: "javascript", code: "while (true) {}", timeout: 1)
        let took = Date().timeIntervalSince(started)
        ticker.cancel()
        XCTAssertFalse(r.ok)
        XCTAssertEqual(r.error, "ran too long (1 s)")
        XCTAssertLessThan(took, 3, "terminated, not abandoned")
        XCTAssertGreaterThan(ticks, 10, "the main thread kept running while the script spun")
        // Usable afterwards, on the same web view.
        let after = await sandbox.run(language: "javascript", code: "1 + 1")
        XCTAssertEqual(after.output, "=> 2")
        XCTAssertEqual(sandbox.webViewsCreated, 1)
    }

    func testTryFinallyCannotSwallowTheKill() async {
        let r = await js("try { while (true) {} } catch (e) { 'survived' } finally { while (true) {} }", timeout: 1)
        XCTAssertFalse(r.ok)
        XCTAssertEqual(r.error, "ran too long (1 s)")
    }

    func testStopCancelsARunInProgress() async {
        let sandbox = WebSandbox()
        defer { sandbox.shutdown() }
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(500))
            sandbox.cancel()
        }
        let started = Date()
        let r = await sandbox.run(language: "javascript", code: "while (true) {}", timeout: 30)
        XCTAssertLessThan(Date().timeIntervalSince(started), 5)
        XCTAssertFalse(r.ok)
        XCTAssertEqual(r.error, "Stopped.")
        let after = await sandbox.run(language: "javascript", code: "'ok'")
        XCTAssertEqual(after.output, "=> ok")
    }

    func testMemoryBombIsContainedAndTheNextRunWorks() async {
        let sandbox = WebSandbox()
        defer { sandbox.shutdown() }
        let bomb = await sandbox.run(language: "javascript", code: """
        const keep = [];
        for (let i = 0; i < 1e6; i++) keep.push('a'.repeat(1e9) + i);
        keep.length
        """, timeout: 20)
        print("memory bomb -> error=\(bomb.error) webViewsCreated=\(sandbox.webViewsCreated)")
        XCTAssertFalse(bomb.ok, "the bomb must not report success: \(bomb.output)")
        let after = await sandbox.run(language: "javascript", code: "40 + 2")
        XCTAssertEqual(after.output, "=> 42", after.error)
    }

    func testHugeStringIsHandled() async {
        let r = await js("'a'.repeat(1e9)", timeout: 20)
        // Either the cap truncates it or WebKit refuses the allocation; never a crash.
        XCTAssertTrue(r.truncated || !r.ok, "\(r.output.count) chars")
    }

    func testUnknownLanguageIsRefused() async {
        let sandbox = WebSandbox()
        let r = await sandbox.run(language: "ruby", code: "1")
        XCTAssertFalse(r.ok)
        XCTAssertFalse(sandbox.isWarm, "no web view for a refused run")
    }

    // MARK: Python

    func testPythonPrintAndLastExpression() async {
        let sandbox = WebSandbox()
        defer { sandbox.shutdown() }
        let a = await sandbox.run(language: "python", code: "print(sum(range(10)))")
        XCTAssertTrue(a.ok, a.error)
        XCTAssertEqual(a.output, "45")
        let b = await sandbox.run(language: "python", code: "x = [1, 2]\nx + [3]")
        XCTAssertEqual(b.output, "=> [1, 2, 3]", b.error)
        let c = await sandbox.run(language: "python", code: "import sys\nprint('err', file=sys.stderr)\nprint('out')")
        XCTAssertEqual(Set(c.output.split(separator: "\n")), ["err", "out"])
    }

    func testPythonTracebackIsShortAndKeepsOutput() async {
        let sandbox = WebSandbox()
        defer { sandbox.shutdown() }
        let r = await sandbox.run(language: "python", code: "print('before')\ndef f():\n    return 1 / 0\nf()")
        XCTAssertFalse(r.ok)
        XCTAssertEqual(r.output, "before")
        XCTAssertTrue(r.error.contains("ZeroDivisionError"), r.error)
        XCTAssertTrue(r.error.contains("line 3"), r.error)
        XCTAssertFalse(r.error.contains("__pocket_run"), r.error)
        let syntax = await sandbox.run(language: "python", code: "def (:")
        XCTAssertTrue(syntax.error.contains("SyntaxError"), syntax.error)
    }

    func testPythonInfiniteLoopIsTerminatedAndTheNextRunRestartsIt() async {
        let sandbox = WebSandbox()
        defer { sandbox.shutdown() }
        let warm = await sandbox.run(language: "python", code: "1")  // first load, outside the timing
        XCTAssertTrue(warm.ok, warm.error)
        let started = Date()
        let r = await sandbox.run(language: "python", code: "while True: pass", timeout: 1)
        XCTAssertFalse(r.ok)
        XCTAssertEqual(r.error, "ran too long (1 s)")
        XCTAssertLessThan(Date().timeIntervalSince(started), 4)
        let after = await sandbox.run(language: "python", code: "print('alive')")
        XCTAssertEqual(after.output, "alive", after.error)
    }

    func testPythonHasNoNetworkAndNoPackageInstalls() async {
        let sandbox = WebSandbox()
        defer { sandbox.shutdown() }
        let xhr = await sandbox.run(language: "python", code: "from pyodide.http import open_url\nopen_url('https://example.com/').read()")
        XCTAssertFalse(xhr.ok, "\(xhr.output)")
        print("python open_url probe -> \(xhr.error.suffix(200))")
        let pkg = await sandbox.run(language: "python", code: "import numpy")
        XCTAssertFalse(pkg.ok)
        XCTAssertTrue(pkg.error.contains("ModuleNotFoundError"), pkg.error)
    }

    func testPythonIsReusedWhileWarmThenDropped() async {
        let sandbox = WebSandbox()
        let a = await sandbox.run(language: "python", code: "import builtins\nbuiltins.marker = 7")
        XCTAssertTrue(a.ok, a.error)
        let started = Date()
        let b = await sandbox.run(language: "python", code: "import builtins\nbuiltins.marker")
        XCTAssertEqual(b.output, "=> 7", "same interpreter: \(b.error)")
        XCTAssertLessThan(Date().timeIntervalSince(started), 2, "warm, no reload")
        // Each run's own namespace is fresh.
        let c = await sandbox.run(language: "python", code: "'leak' in dir()")
        XCTAssertEqual(c.output, "=> False")
        XCTAssertTrue(sandbox.isWarm)
        sandbox.shutdown()
        XCTAssertFalse(sandbox.isWarm)
        let d = await sandbox.run(language: "python", code: "import builtins\nhasattr(builtins, 'marker')")
        XCTAssertEqual(d.output, "=> False", "a new interpreter after teardown")
        sandbox.shutdown()
    }

    func testIdleTimeoutDropsTheWebView() async {
        let sandbox = WebSandbox(idleTimeout: 0.5)
        let r = await sandbox.run(language: "javascript", code: "1")
        XCTAssertTrue(r.ok)
        XCTAssertTrue(sandbox.isWarm)
        try? await Task.sleep(for: .seconds(1.5))
        XCTAssertFalse(sandbox.isWarm)
    }

    func testPythonOutputIsCapped() async {
        let sandbox = WebSandbox(outputLimit: 500)
        defer { sandbox.shutdown() }
        let r = await sandbox.run(language: "python", code: "for i in range(10000): print('y' * 40)")
        XCTAssertTrue(r.truncated)
        XCTAssertLessThanOrEqual(r.output.utf8.count, 500)
    }
}
