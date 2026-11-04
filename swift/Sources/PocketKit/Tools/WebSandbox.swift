import Foundation
import WebKit

/// AGT-14 / AGT-13 stage 3: runs model-written JavaScript and Python in a
/// hidden `WKWebView`, using only public API.
///
/// - **JavaScript** runs in a Web Worker made from a Blob, one per run.
/// - **Python** runs in a long-lived Pyodide (CPython on WebAssembly) worker,
///   bundled in the app: nothing is downloaded, ever (App Review 2.5.2).
/// - The page host races each run against its timeout and calls
///   `worker.terminate()`, which stops the script wherever it is (a `finally`
///   block does not get to run). Swift keeps its own, later deadline in case
///   the page itself is stuck, and Stop (`cancel`) takes the same route.
/// - A memory bomb dies in WebKit's separate content process, not in the app:
///   when it terminates we report an error and build a fresh web view for the
///   next run.
/// - Nothing loads from the network. The page and workers are served from the
///   app bundle by a custom `pocket-sandbox://` scheme (a `file://` page
///   cannot `fetch` its own wasm), the navigation delegate refuses every other
///   URL, and the CSP sent with each resource closes the rest.
///
/// The web view is main-actor bound (WebKit requires it) but nothing here
/// blocks the main thread: the scripts run in workers. The interpreter and web
/// view are dropped after `idleTimeout` without a run, or by `shutdown()`.
@MainActor
public final class WebSandbox {
    public nonisolated static let defaultTimeout: TimeInterval = 10
    public nonisolated static let defaultOutputLimit = 20 * 1024
    /// First Python run: unpacking the interpreter takes a moment.
    public nonisolated static let pythonLoadTimeout: TimeInterval = 30

    private let outputLimit: Int
    private let idleTimeout: TimeInterval
    private let pythonLoadTimeout: TimeInterval

    private var webView: WKWebView?
    private var delegate: Delegate?
    private var run: Run?
    private var idleTask: Task<Void, Never>?
    /// How many web views this sandbox has created (tests: recovery proof).
    public private(set) var webViewsCreated = 0

    private final class Run {
        let id = UUID()
        let started = Date()
        var continuation: CheckedContinuation<ToolRunResult, Never>?
        var hardDeadline: Task<Void, Never>?
    }

    public nonisolated init(outputLimit: Int = WebSandbox.defaultOutputLimit,
                            idleTimeout: TimeInterval = 120,
                            pythonLoadTimeout: TimeInterval = WebSandbox.pythonLoadTimeout) {
        self.outputLimit = outputLimit
        self.idleTimeout = idleTimeout
        self.pythonLoadTimeout = pythonLoadTimeout
    }

    /// True while a web view (and so possibly a warm interpreter) exists.
    public var isWarm: Bool { webView != nil }

    // MARK: - Running

    /// Runs `code` ("javascript" or "python") and always returns, never throws.
    public func run(language: String, code: String,
                    timeout: TimeInterval = WebSandbox.defaultTimeout) async -> ToolRunResult {
        guard language == "javascript" || language == "python" else {
            return Self.failure("\(language) can't run on this device.")
        }
        guard run == nil else { return Self.failure("Another run is in progress.") }
        idleTask?.cancel()
        let python = language == "python"
        let run = Run()
        self.run = run

        let result: ToolRunResult = await withCheckedContinuation { continuation in
            run.continuation = continuation
            // Swift's own deadline, after the page's: only fires if the page is stuck.
            let allowance = timeout + (python ? pythonLoadTimeout : 0) + 3
            run.hardDeadline = Task { [weak self] in
                try? await Task.sleep(for: .seconds(allowance))
                guard !Task.isCancelled else { return }
                self?.abandon(run, error: "ran too long (\(Self.format(timeout)) s)")
            }
            Task { [weak self] in
                guard let self else { return }
                do {
                    let view = try await self.ensureWebView()
                    guard self.run === run else { return }
                    let value = try await view.callAsyncJavaScript(
                        "return await sandbox.run(language, code, timeoutMs, cap, loadMs);",
                        arguments: ["language": language, "code": code, "timeoutMs": Int(timeout * 1000),
                                    "cap": self.outputLimit, "loadMs": Int(self.pythonLoadTimeout * 1000)],
                        in: nil, contentWorld: .page)
                    self.finish(run, Self.parse(value, fallbackMs: run.started))
                } catch {
                    // The content process died (memory bomb) or the page broke.
                    self.abandon(run, error: self.webView == nil
                                 ? "The sandbox stopped (it probably ran out of memory)."
                                 : "The sandbox failed: \(error.localizedDescription)")
                }
            }
        }
        scheduleIdleTeardown()
        return result
    }

    /// Stop: ends the run in progress with "Stopped." and kills its worker.
    public func cancel() {
        guard let run else { return }
        webView?.evaluateJavaScript("sandbox.cancel()", completionHandler: nil)
        finish(run, ToolRunResult(ok: false, output: "", error: "Stopped.",
                                  durationMs: Self.ms(since: run.started), truncated: false))
    }

    /// Drops the web view, and with it the Python interpreter.
    public func shutdown() {
        idleTask?.cancel()
        tearDown()
        if let run { abandon(run, error: "The sandbox was shut down.") }
    }

    // MARK: - Completion

    private func finish(_ run: Run, _ result: ToolRunResult) {
        guard self.run === run, let continuation = run.continuation else { return }
        run.hardDeadline?.cancel()
        run.continuation = nil
        self.run = nil
        continuation.resume(returning: result)
    }

    /// The page can't be trusted any more: report, then start over next time.
    private func abandon(_ run: Run, error: String) {
        guard self.run === run else { return }
        tearDown()
        finish(run, ToolRunResult(ok: false, output: "", error: error,
                                  durationMs: Self.ms(since: run.started), truncated: false))
    }

    private func tearDown() {
        webView?.stopLoading()
        webView?.navigationDelegate = nil
        webView = nil
        delegate = nil
    }

    private func scheduleIdleTeardown() {
        idleTask?.cancel()
        idleTask = Task { [weak self, idleTimeout] in
            try? await Task.sleep(for: .seconds(idleTimeout))
            guard !Task.isCancelled, let self, self.run == nil else { return }
            self.tearDown()
        }
    }

    private static func parse(_ value: Any?, fallbackMs started: Date) -> ToolRunResult {
        guard let dict = value as? [String: Any] else { return failure("The sandbox gave no answer.") }
        let output = dict["output"] as? String ?? ""
        return ToolRunResult(ok: dict["ok"] as? Bool ?? false,
                             output: output,
                             error: dict["error"] as? String ?? "",
                             durationMs: (dict["durationMs"] as? NSNumber)?.uint64Value ?? ms(since: started),
                             truncated: dict["truncated"] as? Bool ?? false)
    }

    private static func failure(_ message: String) -> ToolRunResult {
        ToolRunResult(ok: false, output: "", error: message, durationMs: 0, truncated: false)
    }

    private static func ms(since date: Date) -> UInt64 { UInt64(max(0, Date().timeIntervalSince(date) * 1000)) }

    private static func format(_ seconds: TimeInterval) -> String {
        seconds == seconds.rounded() ? String(Int(seconds)) : String(format: "%.1f", seconds)
    }

    // MARK: - The web view

    private func ensureWebView() async throws -> WKWebView {
        if let webView { return webView }
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.setURLSchemeHandler(SchemeHandler(), forURLScheme: SchemeHandler.scheme)
        // Detached from any window; this actor keeps it alive.
        let view = WKWebView(frame: .zero, configuration: configuration)
        let delegate = Delegate { [weak self] in
            // The content process died: fail the run now, rebuild on the next one.
            guard let self else { return }
            self.tearDown()
            if let run = self.run {
                self.finish(run, ToolRunResult(ok: false, output: "", error: "The sandbox stopped (it probably ran out of memory).",
                                               durationMs: Self.ms(since: run.started), truncated: false))
            }
        }
        view.navigationDelegate = delegate
        webView = view
        self.delegate = delegate
        webViewsCreated += 1
        try await delegate.load(SchemeHandler.pageURL, in: view)
        return view
    }

    /// Refuses every navigation except the bundled page; reports content
    /// process death and load completion.
    private final class Delegate: NSObject, WKNavigationDelegate {
        private let onTerminate: () -> Void
        private var loading: CheckedContinuation<Void, Error>?

        init(onTerminate: @escaping () -> Void) { self.onTerminate = onTerminate }

        func load(_ url: URL, in view: WKWebView) async throws {
            try await withCheckedThrowingContinuation { continuation in
                loading = continuation
                view.load(URLRequest(url: url))
            }
        }

        private func loaded(_ error: Error?) {
            guard let continuation = loading else { return }
            loading = nil
            if let error { continuation.resume(throwing: error) } else { continuation.resume() }
        }

        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                     decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
            let url = navigationAction.request.url
            decisionHandler(url?.scheme == SchemeHandler.scheme && url?.host == SchemeHandler.host ? .allow : .cancel)
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { loaded(nil) }
        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { loaded(error) }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { loaded(error) }
        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
            loaded(CocoaError(.userCancelled))
            onTerminate()
        }
    }

    // MARK: - Serving the bundle

    /// `pocket-sandbox://app/<path>` -> the app bundle's `websandbox/` and
    /// `pyodide/` folders, nothing else, with a CSP on every response.
    private final class SchemeHandler: NSObject, WKURLSchemeHandler {
        static let scheme = "pocket-sandbox"
        static let host = "app"
        static let pageURL = URL(string: "\(scheme)://\(host)/sandbox.html")!

        /// The JavaScript page. `unsafe-eval` is how the Blob worker evaluates
        /// model code and returns its last expression; the worker has no
        /// network (`connect-src 'none'`), no DOM and nothing to steal.
        private static let pageCSP = "default-src 'none'; script-src 'self' blob: 'wasm-unsafe-eval' 'unsafe-eval'; connect-src 'none'"
        /// The Pyodide worker and its files: `connect-src 'self'` only so the
        /// interpreter can read its own bundled wasm and stdlib. Remote hosts
        /// stay blocked.
        private static let pythonCSP = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'"

        func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
            guard let url = task.request.url, url.host == Self.host,
                  let (file, mime) = Self.locate(url.path)
            else {
                task.didFailWithError(URLError(.fileDoesNotExist))
                return
            }
            guard let data = try? Data(contentsOf: file, options: .mappedIfSafe) else {
                task.didFailWithError(URLError(.cannotOpenFile))
                return
            }
            let python = url.path.hasPrefix("/pyodide/") || url.path == "/worker-py.js"
            let response = HTTPURLResponse(
                url: url, statusCode: 200, httpVersion: "HTTP/1.1",
                headerFields: ["Content-Type": mime, "Content-Length": String(data.count),
                               "Content-Security-Policy": python ? Self.pythonCSP : Self.pageCSP,
                               "Cache-Control": "no-store"])!
            task.didReceive(response)
            task.didReceive(data)
            task.didFinish()
        }

        func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}

        /// Only files that live in the two bundled folders, no `..`.
        private static func locate(_ path: String) -> (URL, String)? {
            let parts = path.split(separator: "/").map(String.init)
            guard !parts.isEmpty, !parts.contains(".."), !parts.contains(".") else { return nil }
            let relative: String
            if parts.count == 1 { relative = "websandbox/" + parts[0] }
            else if parts[0] == "pyodide" { relative = parts.joined(separator: "/") }
            else { return nil }
            let file = Bundle.module.resourceURL!.appendingPathComponent(relative)
            guard FileManager.default.fileExists(atPath: file.path) else { return nil }
            let mime: String
            switch file.pathExtension {
            case "html": mime = "text/html; charset=utf-8"
            case "js": mime = "text/javascript; charset=utf-8"
            case "wasm": mime = "application/wasm"
            case "json": mime = "application/json"
            case "zip": mime = "application/zip"
            default: mime = "application/octet-stream"
            }
            return (file, mime)
        }
    }
}
