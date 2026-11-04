import Foundation
import Observation

/// AGT-13: the "Run this?" cards waiting for the user. The only way an answer
/// reaches Core is `answer`, called from the card's buttons.
@MainActor
@Observable
public final class ToolApprovalCenter {
    public private(set) var pending: [ApprovalRequest] = []
    /// Chats whose approved script is running now (for the "Running code…"
    /// row with Stop). Cleared by `finishRun` when its step arrives or the
    /// reply ends.
    public private(set) var running: Set<String> = []
    @ObservationIgnored private let resolve: @Sendable (String, ApprovalDecision, Bool) -> Void

    public init(resolve: @escaping @Sendable (String, ApprovalDecision, Bool) -> Void) {
        self.resolve = resolve
    }

    /// The card to show for a chat, if any.
    public func request(for conversationId: String) -> ApprovalRequest? {
        pending.first { $0.conversationId == conversationId }
    }

    public func answer(_ id: String, _ decision: ApprovalDecision, alwaysAllow: Bool = false) {
        guard let request = pending.first(where: { $0.id == id }) else { return }
        pending.removeAll { $0.id == id }
        if decision != .deny { running.insert(request.conversationId) }
        resolve(id, decision, alwaysAllow)
    }

    public func finishRun(conversationId: String) {
        running.remove(conversationId)
    }

    func add(_ request: ApprovalRequest) { pending.append(request) }
    func remove(_ id: String) { pending.removeAll { $0.id == id } }
}

/// AGT-13: Swift's half of `ToolExecutor`: Settings for availability, the
/// web sandbox (JavaScript and Python) for runs, the approval center for cards.
public final class PocketToolExecutor: ToolExecutor, @unchecked Sendable {
    private let settings: ToolsSettings
    private let sandbox: WebSandbox
    private let macRunner: MacCodeRunning?
    public let approvals: ToolApprovalCenter
    private let macLock = NSLock()
    private var macTask: Task<Void, Never>?

    public init(settings: ToolsSettings = ToolsSettings(), sandbox: WebSandbox = WebSandbox(),
                macRunner: MacCodeRunning? = nil, approvals: ToolApprovalCenter) {
        self.settings = settings
        self.sandbox = sandbox
        self.macRunner = macRunner
        self.approvals = approvals
    }

    public func availability() -> ToolAvailability {
        ToolAvailability(web: settings.isAvailable(.webSearch),
                         javascript: settings.isAvailable(.runJavaScript),
                         python: settings.isAvailable(.runPython),
                         mac: settings.isAvailable(.runOnMac) && (macRunner?.isMacConnected ?? false))
    }

    /// Core calls this from its own worker thread and waits; never call it
    /// from the main thread (the run itself hops to the main actor).
    public func runCode(language: String, code: String, timeoutMs: UInt32) -> ToolRunResult {
        let box = ResultBox()
        let done = DispatchSemaphore(value: 0)
        // "mac:python" etc.: over the peer channel to the paired Mac. Re-checks
        // the toggles here so nothing reaches the Mac if the user turned the
        // tool off while the card was open.
        if language.hasPrefix("mac:") {
            guard settings.isAvailable(.runOnMac), let macRunner else {
                return MacRunReply.failure(String(localized: "Run on my Mac is turned off.", bundle: .module))
            }
            let task = Task {
                box.result = await macRunner.run(language: String(language.dropFirst(4)), code: code, timeoutMs: timeoutMs)
                done.signal()
            }
            macLock.withLock { macTask = task }
            done.wait()
            macLock.withLock { macTask = nil }
            return box.result
        }
        Task { @MainActor [sandbox] in
            box.result = await sandbox.run(language: language, code: code,
                                           timeout: TimeInterval(timeoutMs) / 1000)
            done.signal()
        }
        done.wait()
        return box.result
    }

    /// Drops the Python interpreter and its web view (memory). Call when the
    /// chat closes; the sandbox also does it itself after two idle minutes.
    public func releaseInterpreter() {
        Task { @MainActor [sandbox] in sandbox.shutdown() }
    }

    // Main-queue hops keep add/remove in the order Core called them.
    public func requestApproval(request: ApprovalRequest) {
        DispatchQueue.main.async { [approvals] in
            MainActor.assumeIsolated { approvals.add(request) }
        }
    }

    public func withdrawApproval(id: String) {
        DispatchQueue.main.async { [approvals] in
            MainActor.assumeIsolated { approvals.remove(id) }
        }
    }

    public func cancelRun() {
        macLock.withLock { macTask }?.cancel()
        Task { @MainActor [sandbox] in sandbox.cancel() }
    }
}

/// What the apps hold: registers the executor with the core and exposes the
/// approval center to the chat UI.
@MainActor
public final class ToolsRuntime {
    public let settings: ToolsSettings
    public let approvals: ToolApprovalCenter
    private let executor: PocketToolExecutor

    public init(core: CoreClient, settings: ToolsSettings = ToolsSettings(), macRunner: MacCodeRunning? = nil) {
        self.settings = settings
        let approvals = ToolApprovalCenter { [weak core] id, decision, always in
            core?.resolveApproval(id: id, decision: decision, alwaysAllow: always)
        }
        self.approvals = approvals
        executor = PocketToolExecutor(settings: settings, macRunner: macRunner, approvals: approvals)
        core.registerToolExecutor(executor)
    }
}

/// The detail of a `ran` tool step (`ToolStep.detail`, JSON written by Core).
public struct RanCodeDetail: Equatable, Sendable {
    public var code = ""
    public var output = ""
    public var error = ""
    public var durationMs: UInt64 = 0
    public var truncated = false
    /// The user said no (or never answered); nothing ran.
    public var denied = false
    /// Ran on the paired Mac rather than this device.
    public var onMac = false

    public init(code: String = "", output: String = "", error: String = "",
                durationMs: UInt64 = 0, truncated: Bool = false, denied: Bool = false, onMac: Bool = false) {
        self.code = code; self.output = output; self.error = error
        self.durationMs = durationMs; self.truncated = truncated; self.denied = denied
        self.onMac = onMac
    }

    public init(step: ToolStep) {
        guard let data = step.detail.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            self.init(code: step.detail)
            return
        }
        self.init(code: json["code"] as? String ?? "",
                  output: json["output"] as? String ?? "",
                  error: json["error"] as? String ?? "",
                  durationMs: (json["duration_ms"] as? NSNumber)?.uint64Value ?? 0,
                  truncated: json["truncated"] as? Bool ?? false,
                  denied: json["denied"] as? Bool ?? false,
                  onMac: json["target"] as? String == "mac")
    }
}

private final class ResultBox: @unchecked Sendable {
    var result = ToolRunResult(ok: false, output: "", error: String(localized: "The run didn't finish.", bundle: .module), durationMs: 0, truncated: false)
}

public extension ApprovalRequest {
    /// "Python" or "JavaScript", for the card and rows.
    var languageName: String {
        if tool == "run_on_mac" {
            return summary.contains("Python") ? "Python" : summary.contains("JavaScript") ? "JavaScript" : "Shell"
        }
        return tool == "run_python" ? "Python" : "JavaScript"
    }
    /// Runs on the paired Mac (the card says so).
    var runsOnMac: Bool { tool == "run_on_mac" }
    /// Only on-device JavaScript offers "Always allow in this chat"; Python
    /// and everything on the Mac always ask.
    var offersAlwaysAllow: Bool { tool != "run_python" && tool != "run_on_mac" }
}

public extension ToolStep {
    /// The language of a `ran` step (Core writes it as the step label).
    var codeLanguageName: String {
        switch label {
        case "python", "mac:python": "Python"
        case "mac:shell": "Shell"
        default: "JavaScript"
        }
    }
    /// Ran (or was declined) on the paired Mac.
    var ranOnMac: Bool { label.hasPrefix("mac") }
}
