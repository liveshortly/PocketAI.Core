import SwiftUI
import PocketKit

/// UI-146: the one calm status row for a reply that is being produced:
/// "Loading Qwen3…", "Thinking…", "Searching the web: <query>", "Waiting for
/// your OK to run Python", "Running Python…", "Writing the answer…".
///
/// Shared by the iPhone and Mac chats. Feed it the latest `ChatPhase` from
/// `ChatStreamEvent.status` while the reply is generating and remove it when
/// the reply ends (`phase == nil` also hides it).
///
/// - Appears only after 300 ms in a phase, so quick transitions never flash.
/// - Adds "· 12 s" once a phase has lasted 3 s.
/// - Shows a Stop button when `onStop` is given (44 pt target).
/// - VoiceOver: each new visible phase is announced politely (low priority);
///   the elapsed seconds are not re-announced.
public struct ReplyStatusLine: View {
    private let phase: ChatPhase?
    private let modelName: String?
    private let onStop: (() -> Void)?

    @State private var since = Date()
    @State private var visible = false

    /// - Parameters:
    ///   - phase: the latest phase, or nil for "nothing to show".
    ///   - modelName: used by the Loading phase ("Loading Qwen3 1.7B…").
    ///   - onStop: stops the reply; nil hides the button.
    public init(phase: ChatPhase?, modelName: String? = nil, onStop: (() -> Void)? = nil) {
        self.phase = phase
        self.modelName = modelName
        self.onStop = onStop
    }

    /// Seconds a phase must last before the row appears.
    public static let appearDelay: TimeInterval = 0.3
    /// Seconds after which the elapsed time is appended.
    public static let elapsedAfter: TimeInterval = 3

    public var body: some View {
        if let phase {
            TimelineView(.periodic(from: since, by: 1)) { context in
                row(phase, elapsed: context.date.timeIntervalSince(since))
            }
            .opacity(visible ? 1 : 0)
            .frame(minHeight: visible ? 44 : 0)
            .task(id: phase) {
                since = Date()
                visible = false
                try? await Task.sleep(for: .seconds(Self.appearDelay))
                guard !Task.isCancelled else { return }
                visible = true
                var text = AttributedString(Self.label(for: phase, modelName: modelName))
                text.accessibilitySpeechAnnouncementPriority = .low
                AccessibilityNotification.Announcement(text).post()
            }
        }
    }

    private func row(_ phase: ChatPhase, elapsed: TimeInterval) -> some View {
        HStack(spacing: PocketSpacing.s8) {
            let label = Self.label(for: phase, modelName: modelName)
            Text(Self.display(label, elapsed: elapsed))
                .font(.pocket(.caption))
                .foregroundStyle(PocketColor.textTertiary)
                .lineLimit(2)
                .accessibilityLabel(label)
            Spacer(minLength: 0)
            if let onStop {
                Button(action: onStop) {
                    Text("Stop", bundle: .module)
                        .font(.pocket(.caption))
                        .frame(minWidth: 44, minHeight: 44)
                }
                .buttonStyle(.plain)
                .foregroundStyle(PocketColor.accent)
            }
        }
        .accessibilityElement(children: .contain)
    }

    /// "Thinking… · 12 s" once `elapsed` reaches `elapsedAfter`.
    public static func display(_ label: String, elapsed: TimeInterval) -> String {
        elapsed >= elapsedAfter ? "\(label) · \(Int(elapsed)) s" : label
    }

    /// The sentence for a phase (localised). Public so hosts and tests can
    /// show or check the same wording.
    public static func label(for phase: ChatPhase, modelName: String?) -> String {
        switch phase {
        case .loading:
            if let modelName, !modelName.isEmpty {
                return String(format: String(localized: "Loading %@…", bundle: .module), modelName)
            }
            return String(localized: "Loading the model…", bundle: .module)
        case .thinking:
            return String(localized: "Thinking…", bundle: .module)
        case .writing:
            return String(localized: "Writing the answer…", bundle: .module)
        case .tool(let name, let arg):
            switch name {
            case "web_search":
                return String(format: String(localized: "Searching the web: %@", bundle: .module), arg)
            case "fetch_url":
                return String(format: String(localized: "Reading %@", bundle: .module), arg)
            case "calculator":
                return String(localized: "Calculating…", bundle: .module)
            case "datetime":
                return String(localized: "Checking the time…", bundle: .module)
            default:
                return String(localized: "Working…", bundle: .module)
            }
        case .awaitingApproval(let tool):
            return String(format: String(localized: "Waiting for your OK to run %@", bundle: .module), place(tool))
        case .running(let tool):
            return String(format: String(localized: "Running %@…", bundle: .module), place(tool))
        }
    }

    private static func place(_ tool: String) -> String {
        switch tool {
        case "run_python": String(localized: "Python", bundle: .module)
        case "run_js": String(localized: "JavaScript", bundle: .module)
        case "run_on_mac": String(localized: "code on your Mac", bundle: .module)
        default: String(localized: "code", bundle: .module)
        }
    }
}

#Preview("Light") {
    VStack(alignment: .leading) {
        ReplyStatusLine(phase: .tool(name: "web_search", arg: "latest macOS"), onStop: {})
        ReplyStatusLine(phase: .running(tool: "run_python"))
    }
    .padding()
    .background(PocketColor.bgCanvas)
}
