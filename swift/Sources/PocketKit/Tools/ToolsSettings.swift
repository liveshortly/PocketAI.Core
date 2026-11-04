import Foundation

/// AGT-13: Settings -> Tools, persisted in UserDefaults and read live by the
/// executor at the start of every reply.
///
/// - `allowTools` is the master switch for **code** tools. Off by default.
///   Web search is unaffected: it follows the chat's Tools chip (AGT-12).
/// - Each tool has its own switch. A code tool runs only if the master switch,
///   its own switch and the chat's Tools chip are all on.
/// - Python is its own tool (`runPython`), same master switch and approval
///   rule, but never "always allow" (that is JavaScript only).
/// - `runOnMac` (stage 2) is also offered only while a Mac is paired and
///   connected; that check lives in the executor, not here.
public struct ToolsSettings: @unchecked Sendable {
    public enum Tool: String, CaseIterable, Sendable {
        case webSearch
        case runJavaScript
        case runPython
        case runOnMac

        /// Code tools need the master switch; web search does not.
        public var runsCode: Bool { self != .webSearch }

        /// Only the iPhone has a paired Mac to run on; the Mac's own Settings
        /// leaves this row out.
        public var needsPairedMac: Bool { self == .runOnMac }

        /// Row copy for Settings -> Tools. `device` is "iPhone" or "Mac".
        public func title(device: String) -> String {
            switch self {
            case .webSearch: "Web search"
            case .runJavaScript: "Run JavaScript on this \(device)"
            case .runPython: "Run Python on this \(device)"
            case .runOnMac: "Run on my Mac"
            }
        }

        public var detail: String {
            switch self {
            case .webSearch: "Search the web and read pages. Turn it on per chat with the Tools button."
            case .runJavaScript: "JavaScript in a sandbox with no network or files. You approve every run."
            case .runPython: "Python in a sandbox with no network or files. You approve every run."
            case .runOnMac: "Python, JavaScript or shell on your paired Mac, in a sandbox with no network and no access to your files. You approve every run, and your Mac must allow it."
            }
        }

        public var symbol: String {
            switch self {
            case .webSearch: "globe"
            case .runJavaScript: "terminal"
            case .runPython: "chevron.left.forwardslash.chevron.right"
            case .runOnMac: "desktopcomputer"
            }
        }

        /// The `@AppStorage` key.
        public var defaultsKey: String { "tools.\(rawValue)" }
    }

    public static let allowToolsKey = "tools.allow"

    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    public var allowTools: Bool {
        get { defaults.bool(forKey: Self.allowToolsKey) }
        nonmutating set { defaults.set(newValue, forKey: Self.allowToolsKey) }
    }

    /// A tool's own switch: on unless the user turned it off. (Code tools are
    /// still held back by the master switch, which starts off.)
    public func isOn(_ tool: Tool) -> Bool {
        defaults.object(forKey: tool.defaultsKey) as? Bool ?? true
    }

    public func set(_ tool: Tool, _ on: Bool) {
        defaults.set(on, forKey: tool.defaultsKey)
    }

    /// Whether `tool` may be offered at all (the chat's Tools chip is checked
    /// separately, in Core).
    public func isAvailable(_ tool: Tool) -> Bool {
        isOn(tool) && (!tool.runsCode || allowTools)
    }
}
