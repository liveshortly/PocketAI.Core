import SwiftUI

/// C17: shown in an empty chat - the mark (idle), a one-line title, and
/// 3-4 suggestion chips tailored to the loaded model's tags.
public struct EmptyState: View {
    private let title: String
    private let suggestions: [String]
    private let onTapSuggestion: (String) -> Void

    public init(title: String, suggestions: [String], onTapSuggestion: @escaping (String) -> Void) {
        self.title = title
        self.suggestions = suggestions
        self.onTapSuggestion = onTapSuggestion
    }

    public var body: some View {
        VStack(spacing: PocketSpacing.s24) {
            PocketMarkView(state: .idle, size: 64)

            Text(title)
                .font(.pocket(.title))
                .foregroundStyle(PocketColor.textPrimary)
                .multilineTextAlignment(.center)

            FlowChips(items: suggestions, onTap: onTapSuggestion)
        }
        .padding(PocketSpacing.s24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// A simple wrapping chip row - full flow-layout isn't needed at 3-4 items
/// in a centered column, so this just stacks them instead of reaching for
/// a custom `Layout`.
private struct FlowChips: View {
    let items: [String]
    let onTap: (String) -> Void

    var body: some View {
        VStack(spacing: PocketSpacing.s8) {
            ForEach(items, id: \.self) { item in
                Button {
                    onTap(item)
                } label: {
                    Text(item)
                        .font(.pocket(.callout))
                        .foregroundStyle(PocketColor.textPrimary)
                        .padding(.horizontal, PocketSpacing.s16)
                        .padding(.vertical, PocketSpacing.s8)
                        .background(Capsule().fill(PocketColor.bgSurface))
                        .overlay(Capsule().strokeBorder(PocketColor.lineHairline))
                }
                .buttonStyle(.plain)
            }
        }
    }
}

#Preview("Light") {
    EmptyState(
        title: "Ask me anything.",
        suggestions: ["Summarize a document", "Help me write an email", "Explain a concept"],
        onTapSuggestion: { _ in }
    )
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    EmptyState(
        title: "Ask me anything.",
        suggestions: ["Summarize a document", "Help me write an email"],
        onTapSuggestion: { _ in }
    )
    .background(PocketColor.bgCanvas)
    .preferredColorScheme(.dark)
}
