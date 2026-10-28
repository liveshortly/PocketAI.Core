import Foundation
import SwiftUI

/// Splits a run of markdown prose back into the blocks it was written as.
///
/// `AttributedString(markdown:)` records block structure in
/// `presentationIntent` but drops every newline that produced it, and `Text`
/// renders only the characters. A two-paragraph answer with a list therefore
/// came out as one wall of glued words — "simultaneously.**Why the code
/// fails:** When you…", and "alphabeta" for `- alpha\n- beta`. Runs that
/// share an intent belong to the same block (the intent carries a per-block
/// identity), so grouping on it puts the breaks back.
///
/// This lives apart from any view because both apps render assistant replies
/// and they render them through different rows; the one thing they must agree
/// on is what a reply looks like. It is also deliberately not `@MainActor`:
/// it reads no state and splitting text is not view work.
public enum MarkdownBlocks {
    /// One block of prose — a paragraph, a heading, or a single list item.
    public struct Block: Identifiable, Sendable {
        public let id: Int
        /// A bullet or ordinal when this block is a list item.
        public var marker: String?
        /// Leading inset, one step per level of list nesting.
        public var indent: CGFloat
        public var isHeading: Bool
        public var text: AttributedString
    }

    public static func split(_ markdown: String) -> [Block] {
        let attributed = attributed(markdown)
        var result: [Block] = []
        var pending: AttributedString?
        var pendingIntent: PresentationIntent?

        func flush() {
            guard let block = pending else { return }
            let plain = String(block.characters).trimmingCharacters(in: .whitespacesAndNewlines)
            if !plain.isEmpty {
                let style = decoration(for: pendingIntent)
                result.append(
                    Block(
                        id: result.count,
                        marker: style.marker,
                        indent: style.indent,
                        isHeading: style.isHeading,
                        text: block
                    )
                )
            }
            pending = nil
        }

        for run in attributed.runs {
            let intent = run.presentationIntent
            if pending != nil, intent != pendingIntent { flush() }
            pendingIntent = intent
            if pending == nil { pending = AttributedString() }
            pending?.append(attributed[run.range])
        }
        flush()
        return result
    }

    /// `.full` is what records the block intents this type reads; the
    /// inline-only modes keep the newlines but leave `## ` in the text.
    public static func attributed(_ markdown: String) -> AttributedString {
        (try? AttributedString(markdown: markdown, options: .init(interpretedSyntax: .full)))
            ?? AttributedString(markdown)
    }

    /// Marker, indent and heading flag for a block's presentation intent.
    /// Components come innermost-first, e.g. [paragraph, listItem 2,
    /// unorderedList], so the first list kind seen after a list item is that
    /// item's own list.
    private static func decoration(for intent: PresentationIntent?) -> (marker: String?, indent: CGFloat, isHeading: Bool) {
        guard let intent else { return (nil, 0, false) }
        var ordinal: Int?
        var isOrdered: Bool?
        var listDepth = 0
        var isHeading = false

        for component in intent.components {
            switch component.kind {
            case .listItem(let n):
                if ordinal == nil { ordinal = n }
            case .orderedList:
                if ordinal != nil, isOrdered == nil { isOrdered = true }
                listDepth += 1
            case .unorderedList:
                if ordinal != nil, isOrdered == nil { isOrdered = false }
                listDepth += 1
            case .header:
                isHeading = true
            default:
                break
            }
        }

        let marker: String? = ordinal.map { isOrdered == true ? "\($0)." : "•" }
        return (marker, CGFloat(max(0, listDepth - 1)) * PocketSpacing.s16, isHeading)
    }
}

/// Renders split markdown prose: one `Text` per block, with list markers and
/// heading weight. Shared so the two apps cannot drift apart on this.
public struct MarkdownProse: View {
    private let blocks: [MarkdownBlocks.Block]
    private let font: Font

    public init(_ markdown: String, font: Font) {
        self.blocks = MarkdownBlocks.split(markdown)
        self.font = font
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s8) {
            ForEach(blocks) { block in
                HStack(alignment: .firstTextBaseline, spacing: PocketSpacing.s8) {
                    if let marker = block.marker {
                        Text(marker)
                            .font(font)
                            .foregroundStyle(PocketColor.textSecondary)
                            .frame(minWidth: 18, alignment: .trailing)
                    }
                    Text(block.text)
                        .font(block.isHeading ? .pocket(.headline) : font)
                        .foregroundStyle(PocketColor.textPrimary)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.leading, block.indent)
            }
        }
    }
}
