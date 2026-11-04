import SwiftUI

/// C04: a user message - trailing-aligned, `accent.soft` fill, radius 20,
/// max width 80%, attachment thumbnails above the text.
public struct UserBubble: View {
    private let text: String
    private let attachmentThumbnails: [Image]

    public init(text: String, attachmentThumbnails: [Image] = []) {
        self.text = text
        self.attachmentThumbnails = attachmentThumbnails
    }

    public var body: some View {
        HStack {
            Spacer(minLength: PocketSpacing.s40)
            VStack(alignment: .trailing, spacing: PocketSpacing.s8) {
                if !attachmentThumbnails.isEmpty {
                    HStack(spacing: PocketSpacing.s8) {
                        ForEach(Array(attachmentThumbnails.enumerated()), id: \.offset) { _, thumbnail in
                            thumbnail
                                .resizable()
                                .aspectRatio(contentMode: .fill)
                                .frame(width: 64, height: 64)
                                .clipShape(RoundedRectangle(cornerRadius: PocketSpacing.s8))
                                .accessibilityLabel(Text("Attached image", bundle: .module))
                        }
                    }
                }
                if !text.isEmpty {
                    UserMessageText(text)
                        .foregroundStyle(PocketColor.textPrimary)
                        .textSelection(.enabled)
                        .padding(.horizontal, PocketSpacing.s16)
                        .padding(.vertical, PocketSpacing.s12)
                        .background(PocketColor.accentSoft, in: RoundedRectangle(cornerRadius: PocketRadius.bubble))
                }
            }
        }
    }
}

/// A user message's text: plain `Text`, except fenced code blocks, which render
/// as `CodeBlock` like they do in the assistant's reply (UI-143). Colour
/// is inherited from the bubble; no padding or fill - the caller supplies the bubble.
public struct UserMessageText: View {
    private let text: String

    public init(_ text: String) { self.text = text }

    public var body: some View {
        if text.contains("```") {
            VStack(alignment: .leading, spacing: PocketSpacing.s8) {
                ForEach(Array(MessageView.parseSegments(text).enumerated()), id: \.offset) { _, segment in
                    switch segment {
                    case .code(let code, let language):
                        CodeBlock(code: code.trimmingCharacters(in: .newlines), language: language)
                    case .markdown(let body):
                        let prose = body.trimmingCharacters(in: .whitespacesAndNewlines)
                        if !prose.isEmpty { Text(prose).font(.pocket(.body)) }
                    case .table:
                        EmptyView()
                    }
                }
            }
        } else {
            Text(text).font(.pocket(.body))
        }
    }
}

#Preview("Light") {
    VStack(spacing: PocketSpacing.s12) {
        UserBubble(text: "Can you summarize this for me?")
        UserBubble(text: "What's the capital of France? I need a long example here to see how the bubble wraps across several lines at 80% max width.")
    }
    .padding()
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    UserBubble(text: "Can you summarize this for me?")
        .padding()
        .background(PocketColor.bgCanvas)
        .preferredColorScheme(.dark)
}
