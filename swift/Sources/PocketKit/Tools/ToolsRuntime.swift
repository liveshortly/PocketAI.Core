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
/// JavaScript sandbox for runs, the approval center for cards.
public final class PocketToolExecutor: ToolExecutor, @unchecked Sendable {
    private let settings: ToolsSettings
    private let sandbox: JavaScriptSandbox
    public let approvals: ToolApprovalCenter

    public init(settings: ToolsSettings = ToolsSettings(), sandbox: JavaScriptSandbox = JavaScriptSandbox(),
                approvals: ToolApprovalCenter) {
        self.settings = settings
        self.sandbox = sandbox
        self.approvals = approvals
    }

    public func availability() -> ToolAvailability {
        ToolAvailability(web: settings.isAvailable(.webSearch),
                         javascript: settings.isAvailable(.runJavaScript))
    }

    public func runCode(language: String, code: String, timeoutMs: UInt32) -> ToolRunResult {
        guard language == "javascript" else {
            return ToolRunResult(ok: false, output: "", error: "\(language) can't run on this device yet.",
                                 durationMs: 0, truncated: false)
        }
        return sandbox.run(code)
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

    public func cancelRun() { sandbox.cancel() }
}

/// What the apps hold: registers the executor with the core and exposes the
/// approval center to the chat UI.
@MainActor
public final class ToolsRuntime {
    public let settings: ToolsSettings
    public let approvals: ToolApprovalCenter
    private let executor: PocketToolExecutor

    public init(core: CoreClient, settings: ToolsSettings = ToolsSettings()) {
        self.settings = settings
        let approvals = ToolApprovalCenter { [weak core] id, decision, always in
            core?.resolveApproval(id: id, decision: decision, alwaysAllow: always)
        }
        self.approvals = approvals
        executor = PocketToolExecutor(settings: settings, approvals: approvals)
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

    public init(code: String = "", output: String = "", error: String = "",
                durationMs: UInt64 = 0, truncated: Bool = false, denied: Bool = false) {
        self.code = code; self.output = output; self.error = error
        self.durationMs = durationMs; self.truncated = truncated; self.denied = denied
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
                  denied: json["denied"] as? Bool ?? false)
    }
}
