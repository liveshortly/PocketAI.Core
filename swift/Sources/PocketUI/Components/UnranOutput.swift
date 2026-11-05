import SwiftUI

/// UI-145: small models write a script and then invent its output ("Output:
/// 42"). When a reply holds runnable code plus such an output block but no
/// tool ran, the chat shows `NotRunNote` so nobody mistakes the guess for a
/// result.
public enum UnranOutput {
    public struct Finding: Equatable, Sendable {
        /// "python" or "javascript".
        public let language: String
        public let code: String
    }

    /// The first Python/JavaScript fenced block that is followed by something
    /// that reads like its output, or nil.
    public static func find(in text: String) -> Finding? {
        // ponytail: regex heuristic; a reply that says "Output" after code is
        // treated as fabricated. Refine if real replies false-positive.
        guard let fence = try? NSRegularExpression(pattern: "```[ \\t]*(\\w*)[^\\n]*\\n(.*?)```", options: [.dotMatchesLineSeparators]),
              let output = try? NSRegularExpression(pattern: "(?im)^[^\\n]{0,12}\\b(output|result|results)\\b", options: [])
        else { return nil }
        let ns = text as NSString
        for m in fence.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
            let lang: String
            switch ns.substring(with: m.range(at: 1)).lowercased() {
            case "python", "py", "python3": lang = "python"
            case "javascript", "js": lang = "javascript"
            default: continue
            }
            let after = NSRange(location: m.range.upperBound, length: ns.length - m.range.upperBound)
            if output.firstMatch(in: text, range: after) != nil {
                return Finding(language: lang, code: ns.substring(with: m.range(at: 2)))
            }
        }
        return nil
    }
}

/// "Not run: this output was written by the model" with a Run button. Uses
/// the chat's `codeRunner` (so it is hidden by the host when code tools are
/// off); Run confirms first, then shows the real output.
public struct NotRunNote: View {
    private let finding: UnranOutput.Finding
    @Environment(\.codeRunner) private var runner
    @State private var confirm = false
    @State private var running = false
    @State private var outcome: CodeRunOutcome?

    public init(finding: UnranOutput.Finding) {
        self.finding = finding
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s4) {
            HStack(spacing: PocketSpacing.s8) {
                Text("Not run: this output was written by the model", bundle: .module)
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
                Spacer(minLength: 0)
                if runner != nil {
                    Button {
                        confirm = true
                    } label: {
                        Text(running ? "Running…" : "Run it", bundle: .module)
                            .font(.pocket(.caption))
                            .frame(minHeight: 44)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(PocketColor.accent)
                    .disabled(running)
                }
            }
            if let outcome {
                Text(outcome.ok ? (outcome.output.isEmpty ? "(no output)" : outcome.output) : outcome.error)
                    .font(.pocket(.mono))
                    .foregroundStyle(PocketColor.textPrimary)
                    .textSelection(.enabled)
            }
        }
        .confirmationDialog(String(localized: "Run this code?", bundle: .module), isPresented: $confirm, titleVisibility: .visible) {
            Button(String(localized: "Run", bundle: .module)) {
                guard let runner else { return }
                running = true
                Task {
                    outcome = await runner(finding.language, finding.code)
                    running = false
                }
            }
            Button(String(localized: "Cancel", bundle: .module), role: .cancel) {}
        } message: {
            Text("It runs in a sandbox on this device with no network and no access to your files.", bundle: .module)
        }
    }
}
