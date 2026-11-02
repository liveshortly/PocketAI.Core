import SwiftUI

/// C03: an assistant message - full width, no bubble, markdown body with
/// fenced code blocks rendered via `CodeBlock`, a collapsible reasoning
/// disclosure, and a footer (copy/retry/speak/stats) shown on hover/
/// long-press.
///
/// Markdown is split on fenced code blocks (```lang\n...\n```), then each
/// prose run is split again on GFM pipe tables. Code blocks render via
/// `CodeBlock` (with `SyntaxHighlighter`), tables via `MarkdownTable` (a real
/// grid, since `AttributedString` can't represent them), and the remaining
/// prose via `AttributedString(markdown:)` with `.full` interpreted syntax
/// (bold, italic, links, inline code, basic lists).
public struct MessageView: View {
    private let text: String
    private let reasoning: String?
    /// True while this message's reasoning is still streaming in: the pane
    /// header animates ("Thinking…" + spinner) and can be expanded live.
    private let isThinking: Bool
    private let statsText: String?
    private let onCopy: () -> Void
    private let onRetry: () -> Void
    private let onSpeak: () -> Void

    @State private var reasoningExpanded = false
    @State private var isHovering = false

    public init(
        text: String,
        reasoning: String? = nil,
        isThinking: Bool = false,
        statsText: String? = nil,
        onCopy: @escaping () -> Void = {},
        onRetry: @escaping () -> Void = {},
        onSpeak: @escaping () -> Void = {}
    ) {
        self.text = text
        self.reasoning = reasoning
        self.isThinking = isThinking
        self.statsText = statsText
        self.onCopy = onCopy
        self.onRetry = onRetry
        self.onSpeak = onSpeak
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s12) {
            if let reasoning, !reasoning.isEmpty {
                DisclosureGroup(isExpanded: $reasoningExpanded) {
                    ScrollView {
                        Text(reasoning)
                            .font(.pocket(.footnote))
                            .foregroundStyle(PocketColor.textSecondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 180)
                    .padding(PocketSpacing.s8)
                    .background(PocketColor.bgSunken, in: RoundedRectangle(cornerRadius: PocketRadius.chip))
                } label: {
                    HStack(spacing: PocketSpacing.s8) {
                        if isThinking {
                            PocketMarkView(state: .loading, size: 16)
                            Text("Thinking…", bundle: .module)
                                .font(.pocket(.footnote))
                                .foregroundStyle(PocketColor.textSecondary)
                        } else {
                            Image(systemName: "brain")
                                .font(.caption)
                                .foregroundStyle(PocketColor.textTertiary)
                            Text("Thought for a moment", bundle: .module)
                                .font(.pocket(.footnote))
                                .foregroundStyle(PocketColor.textSecondary)
                        }
                    }
                }
                .tint(PocketColor.textSecondary)
            } else if isThinking, text.isEmpty {
                // No reasoning stream (non-thinking model loading/prefilling):
                // the animated PocketAI mark, centered — same loading treatment
                // as the LiveShortly screens (LSI-13), never a dead empty bubble.
                VStack(spacing: PocketSpacing.s8) {
                    PocketMarkView(state: .loading, size: 40)
                    Text("Thinking…", bundle: .module)
                        .font(.pocket(.footnote))
                        .foregroundStyle(PocketColor.textSecondary)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, PocketSpacing.s8)
            }

            ForEach(Array(segments.enumerated()), id: \.offset) { _, segment in
                switch segment {
                case .code(let code, let language):
                    CodeBlock(code: code, language: language)
                case .markdown(let body):
                    MarkdownProse(body, font: .pocket(.body))
                case .table(let rows, let hasHeader):
                    MarkdownTable(rows: rows, hasHeader: hasHeader)
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
        case table(rows: [[String]], hasHeader: Bool)
    }

    private var segments: [Segment] {
        // First split on fenced code, then split each prose run on GFM tables.
        Self.parseSegments(text).flatMap { seg -> [Segment] in
            if case .markdown(let body) = seg { return Self.splitTables(body) }
            return [seg]
        }
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


    /// Splits a prose run into `.markdown` and `.table` sub-segments by
    /// detecting GFM pipe tables (a row with `|` followed by a `---|---`
    /// separator). `AttributedString(markdown:)` can't render tables at all,
    /// so these are pulled out and drawn as a real grid.
    private static func splitTables(_ markdown: String) -> [Segment] {
        let lines = markdown.components(separatedBy: "\n")
        var result: [Segment] = []
        var prose: [String] = []
        func flushProse() {
            let joined = prose.joined(separator: "\n")
            if !joined.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { result.append(.markdown(joined)) }
            prose.removeAll()
        }
        var i = 0
        while i < lines.count {
            if i + 1 < lines.count, lines[i].contains("|"), isSeparatorRow(lines[i + 1]) {
                flushProse()
                var rows: [[String]] = [parseRow(lines[i])]
                i += 2
                while i < lines.count, lines[i].contains("|"), !lines[i].trimmingCharacters(in: .whitespaces).isEmpty {
                    rows.append(parseRow(lines[i]))
                    i += 1
                }
                result.append(.table(rows: rows, hasHeader: true))
                continue
            }
            prose.append(lines[i])
            i += 1
        }
        flushProse()
        return result.isEmpty ? [.markdown(markdown)] : result
    }

    private static func isSeparatorRow(_ line: String) -> Bool {
        let t = line.trimmingCharacters(in: .whitespaces)
        guard t.contains("-"), t.contains("|") else { return false }
        let allowed = Set("|-: ")
        return t.allSatisfy { allowed.contains($0) }
    }

    private static func parseRow(_ line: String) -> [String] {
        var s = line.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("|") { s.removeFirst() }
        if s.hasSuffix("|") { s.removeLast() }
        return s.components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }
}

/// Renders a GFM pipe table as a bordered grid (AttributedString can't).
private struct MarkdownTable: View {
    let rows: [[String]]
    let hasHeader: Bool

    var body: some View {
        let colCount = rows.map(\.count).max() ?? 0
        Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.offset) { rowIndex, row in
                GridRow {
                    ForEach(0..<colCount, id: \.self) { col in
                        let isHeader = hasHeader && rowIndex == 0
                        Text(MarkdownBlocks.attributed(col < row.count ? row[col] : ""))
                            .font(.pocket(.footnote))
                            .fontWeight(isHeader ? .semibold : .regular)
                            .foregroundStyle(isHeader ? PocketColor.textPrimary : PocketColor.textSecondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, PocketSpacing.s12)
                            .padding(.vertical, PocketSpacing.s8)
                            .background(isHeader ? PocketColor.bgSunken : Color.clear)
                    }
                }
                if rowIndex < rows.count - 1 {
                    Divider().overlay(PocketColor.lineHairline)
                }
            }
        }
        .overlay(RoundedRectangle(cornerRadius: PocketSpacing.s8).strokeBorder(PocketColor.lineHairline))
        .clipShape(RoundedRectangle(cornerRadius: PocketSpacing.s8))
    }

    private static func attributed(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s)) ?? AttributedString(s)
    }
}

#Preview("Light") {
    ScrollView {
        MessageView(
            text: "Here's a **short** summary with a [link](https://pocket.liveshortly.com) and some code:\n\n```swift\nprint(\"hello\")\n```\n\nThat's it.",
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

#Preview("Thinking") {
    VStack(spacing: PocketSpacing.s24) {
        MessageView(text: "", isThinking: true)
        MessageView(text: "", reasoning: "Weighing the options…", isThinking: true)
    }
    .padding()
    .background(PocketColor.bgCanvas)
}
