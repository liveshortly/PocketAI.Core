import Foundation

/// First-chat suggestion chips (UI-78, C17). `prompt` is a complete,
/// self-contained message: a chip must never send a fragment that needs
/// content the user hasn't supplied (an email, a draft).
public struct FirstChatSuggestion: Equatable, Sendable {
    public let title: String
    public let icon: String
    public let prompt: String

    public static let all: [FirstChatSuggestion] = [
        .init(title: "Explain a concept simply", icon: "lightbulb",
              prompt: "Explain how vaccines work in simple terms, as if I were new to the topic."),
        .init(title: "Plan my day", icon: "calendar",
              prompt: "Help me plan a productive day. Ask me what I need to get done, then suggest a schedule."),
        .init(title: "Write a polite reply", icon: "bubble.left.and.text.bubble.right",
              prompt: "Write a short, polite message declining an invitation to a meeting, and offer a new time."),
    ]
}
