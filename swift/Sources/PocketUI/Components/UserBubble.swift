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
                        }
                    }
                }
                Text(text)
                    .font(.pocket(.body))
                    .foregroundStyle(PocketColor.textPrimary)
                    .padding(.horizontal, PocketSpacing.s16)
                    .padding(.vertical, PocketSpacing.s12)
                    .background(PocketColor.accentSoft, in: RoundedRectangle(cornerRadius: PocketRadius.bubble))
            }
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
