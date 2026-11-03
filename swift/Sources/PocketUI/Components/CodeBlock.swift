import SwiftUI

/// C05: a fenced code block - `bg.sunken`, mono, language label + copy
/// button, horizontal scroll, no wrapping by default.
public struct CodeBlock: View {
    private let code: String
    private let language: String?
    @State private var copied = false

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
        }
        .background(PocketColor.bgSunken, in: RoundedRectangle(cornerRadius: PocketRadius.chip, style: .continuous))
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
