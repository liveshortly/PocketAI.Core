import SwiftUI

/// UI-145: what a "Run" tap on a code block gets back.
public struct CodeRunOutcome: Equatable, Sendable {
    public var ok: Bool
    public var output: String
    public var error: String
    public init(ok: Bool, output: String, error: String) {
        self.ok = ok; self.output = output; self.error = error
    }
}

public typealias CodeRunner = @MainActor @Sendable (_ language: String, _ code: String) async -> CodeRunOutcome

private struct CodeRunnerKey: EnvironmentKey {
    static let defaultValue: CodeRunner? = nil
}

public extension EnvironmentValues {
    /// Set by a chat that allows running code: Python and JavaScript blocks
    /// then get a Run button (it asks first, then runs in the sandbox).
    var codeRunner: CodeRunner? {
        get { self[CodeRunnerKey.self] }
        set { self[CodeRunnerKey.self] = newValue }
    }
}

/// C05: a fenced code block - `bg.sunken`, mono, language label + copy
/// button, horizontal scroll, no wrapping by default.
public struct CodeBlock: View {
    private let code: String
    private let language: String?
    @State private var copied = false
    @Environment(\.codeRunner) private var runner
    @State private var confirmRun = false
    @State private var running = false
    @State private var outcome: CodeRunOutcome?

    /// "python" / "javascript" for the languages the sandbox runs, else nil.
    private var runnableLanguage: String? {
        switch language?.lowercased() {
        case "python", "py", "python3": "python"
        case "javascript", "js": "javascript"
        default: nil
        }
    }

    public init(code: String, language: String? = nil) {
        self.code = code
        self.language = language
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(language?.uppercased() ?? String(localized: "CODE", bundle: .module))
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
                Spacer()
                if runner != nil, runnableLanguage != nil {
                    Button {
                        confirmRun = true
                    } label: {
                        Label(running ? String(localized: "Running…", bundle: .module) : String(localized: "Run", bundle: .module),
                              systemImage: "play.fill")
                            .font(.pocket(.caption))
                            .labelStyle(.titleAndIcon)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(PocketColor.accent)
                    .disabled(running)
                    .padding(.trailing, PocketSpacing.s8)
                }
                Button {
                    #if os(iOS)
                    UIPasteboard.general.string = code
                    #elseif os(macOS)
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(code, forType: .string)
                    #endif
                    copied = true
                    AccessibilityNotification.Announcement(String(localized: "Copied", bundle: .module)).post()
                    Task {
                        try? await Task.sleep(for: .seconds(1.5))
                        copied = false
                    }
                } label: {
                    Label(copied ? String(localized: "Copied", bundle: .module) : String(localized: "Copy", bundle: .module),
                          systemImage: copied ? "checkmark" : "doc.on.doc")
                        .font(.pocket(.caption))
                        .labelStyle(.titleAndIcon)
                }
                .buttonStyle(.plain)
                .foregroundStyle(PocketColor.textSecondary)
            }
            .padding(.horizontal, PocketSpacing.s12)
            .padding(.vertical, PocketSpacing.s8)

            ScrollView(.horizontal, showsIndicators: false) {
                Text(SyntaxHighlighter.highlight(code, language: language))
                    .font(.pocket(.mono))
                    .padding(.horizontal, PocketSpacing.s12)
                    .padding(.bottom, PocketSpacing.s12)
                    .fixedSize(horizontal: true, vertical: false)
                    .textSelection(.enabled)
            }

            if let outcome {
                VStack(alignment: .leading, spacing: PocketSpacing.s4) {
                    Text(outcome.ok ? String(localized: "Output", bundle: .module) : String(localized: "Error", bundle: .module))
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                    Text(outcome.ok ? (outcome.output.isEmpty ? String(localized: "(no output)", bundle: .module) : outcome.output) : outcome.error)
                        .font(.pocket(.mono))
                        .foregroundStyle(outcome.ok ? PocketColor.textPrimary : PocketColor.textSecondary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(PocketSpacing.s12)
                .overlay(alignment: .top) { Rectangle().fill(PocketColor.lineHairline).frame(height: 1) }
            }
        }
        .background(PocketColor.bgSunken, in: RoundedRectangle(cornerRadius: PocketRadius.chip, style: .continuous))
        .confirmationDialog(String(localized: "Run this code?", bundle: .module), isPresented: $confirmRun, titleVisibility: .visible) {
            Button(String(localized: "Run", bundle: .module)) {
                guard let runner, let lang = runnableLanguage else { return }
                running = true
                Task {
                    let result = await runner(lang, code)
                    outcome = result
                    running = false
                }
            }
            Button(String(localized: "Cancel", bundle: .module), role: .cancel) {}
        } message: {
            Text("It runs in a sandbox on this device with no network and no access to your files.", bundle: .module)
        }
    }
}

#Preview("Light") {
    CodeBlock(code: "func fit(model: ModelManifest, profile: DeviceProfile) -> FitReport {\n    // ...\n}", language: "swift")
        .padding()
        .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    CodeBlock(code: "print(\"hello world\")", language: "swift")
        .padding()
        .background(PocketColor.bgCanvas)
        .preferredColorScheme(.dark)
}
