import SwiftUI

/// C03: an assistant message - full width, no bubble, markdown body with
/// fenced code blocks rendered via `CodeBlock`, a collapsible reasoning
/// disclosure, and a footer (copy/retry/speak/stats) shown on hover/
/// long-press.
///
/// Markdown is split on fenced code blocks (```lang\n...\n```) and each
/// non-code segment is rendered via `AttributedString(markdown:)` with
/// `.full` interpreted syntax - this covers inline formatting (bold,
/// italic, links, inline code) and basic lists reliably. True block-level
/// tables aren't representable in `AttributedString` at all; rendering
/// those needs a real block parser (`swift-markdown` per the card's own
/// fallback) - not implemented here.
public struct MessageView: View {
    private let text: String
    private let reasoning: String?
    private let statsText: String?
    private let onCopy: () -> Void
    private let onRetry: () -> Void
    private let onSpeak: () -> Void

    @State private var reasoningExpanded = false
    @State private var isHovering = false

    public init(
        text: String,
        reasoning: String? = nil,
        statsText: String? = nil,
        onCopy: @escaping () -> Void = {},
        onRetry: @escaping () -> Void = {},
        onSpeak: @escaping () -> Void = {}
    ) {
        self.text = text
        self.reasoning = reasoning
        self.statsText = statsText
        self.onCopy = onCopy
        self.onRetry = onRetry
        self.onSpeak = onSpeak
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s12) {
            if let reasoning, !reasoning.isEmpty {
                DisclosureGroup(isExpanded: $reasoningExpanded) {
                    Text(reasoning)
                        .font(.pocket(.footnote))
                        .foregroundStyle(PocketColor.textSecondary)
                } label: {
                    Text("Thought for a moment")
                        .font(.pocket(.footnote))
                        .foregroundStyle(PocketColor.textSecondary)
                }
            }

            ForEach(Array(segments.enumerated()), id: \.offset) { _, segment in
                switch segment {
                case .code(let code, let language):
                    CodeBlock(code: code, language: language)
                case .markdown(let body):
                    Text(Self.attributed(body))
                        .font(.pocket(.body))
                        .foregroundStyle(PocketColor.textPrimary)
                        .textSelection(.enabled)
                }
            }

            if isHovering {
                HStack(spacing: PocketSpacing.s16) {
                    footerButton("doc.on.doc", action: onCopy)
                    footerButton("arrow.clockwise", action: onRetry)
                    footerButton("speaker.wave.2", action: onSpeak)
                    if let statsText {
                        Text(statsText)
                            .font(.pocket(.caption))
                            .foregroundStyle(PocketColor.textTertiary)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onHover { isHovering = $0 }
    }

    private func footerButton(_ systemImage: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.caption)
                .foregroundStyle(PocketColor.textTertiary)
        }
        .buttonStyle(.plain)
    }

    private enum Segment {
        case markdown(String)
        case code(String, language: String?)
    }

    private var segments: [Segment] {
        Self.parseSegments(text)
    }

    private static func parseSegments(_ text: String) -> [Segment] {
        var result: [Segment] = []
        var rest = text[...]
        while let fenceStart = rest.range(of: "```") {
            let before = String(rest[rest.startIndex..<fenceStart.lowerBound])
            if !before.isEmpty { result.append(.markdown(before)) }

            let afterFence = rest[fenceStart.upperBound...]
            let firstLineEnd = afterFence.firstIndex(of: "\n") ?? afterFence.endIndex
            let language = String(afterFence[afterFence.startIndex..<firstLineEnd]).trimmingCharacters(in: .whitespaces)
            let codeStart = firstLineEnd < afterFence.endIndex ? afterFence.index(after: firstLineEnd) : afterFence.endIndex

            if let fenceEnd = afterFence[codeStart...].range(of: "```") {
                let code = String(afterFence[codeStart..<fenceEnd.lowerBound])
                result.append(.code(code, language: language.isEmpty ? nil : language))
                rest = afterFence[fenceEnd.upperBound...]
            } else {
                result.append(.code(String(afterFence[codeStart...]), language: language.isEmpty ? nil : language))
                rest = afterFence[afterFence.endIndex...]
            }
        }
        let remainder = String(rest)
        if !remainder.isEmpty { result.append(.markdown(remainder)) }
        return result
    }

    private static func attributed(_ markdown: String) -> AttributedString {
        (try? AttributedString(markdown: markdown, options: .init(interpretedSyntax: .full))) ?? AttributedString(markdown)
    }
}

#Preview("Light") {
    ScrollView {
        MessageView(
            text: "Here's a **short** summary with a [link](https://getpocketai.app) and some code:\n\n```swift\nprint(\"hello\")\n```\n\nThat's it.",
            reasoning: "Considered a few phrasings before picking the simplest one.",
            statsText: "32 tok/s · 1.2 s"
        )
        .padding()
    }
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    MessageView(text: "A short plain reply with *emphasis*.")
        .padding()
        .background(PocketColor.bgCanvas)
        .preferredColorScheme(.dark)
}
