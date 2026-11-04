import SwiftUI

public enum ComposerState: Sendable, Equatable {
    case idle
    case hasText
    case generating
}

/// C06: the floating capsule composer - `+` attach menu, a 1-6 line text
/// field, and a trailing control that morphs between mic (empty) / send
/// (has text) / stop (generating).
public struct Composer: View {
    @Binding var text: String
    let modelChipLabel: String?
    let isFocused: Bool
    let isGenerating: Bool
    let onAttach: (AttachmentKind) -> Void
    let onMic: () -> Void
    let onMicLongPress: (() -> Void)?
    let onSend: () -> Void
    let onStop: () -> Void
    let externalFocus: FocusState<Bool>.Binding?
    let hasAttachment: Bool

    public enum AttachmentKind: Sendable {
        case photo, camera, file, projectDocument, image
    }

    public init(
        text: Binding<String>,
        modelChipLabel: String?,
        isFocused: Bool,
        isGenerating: Bool,
        hasAttachment: Bool = false,
        onAttach: @escaping (AttachmentKind) -> Void,
        onMic: @escaping () -> Void,
        onMicLongPress: (() -> Void)? = nil,
        onSend: @escaping () -> Void,
        onStop: @escaping () -> Void,
        externalFocus: FocusState<Bool>.Binding? = nil
    ) {
        _text = text
        self.modelChipLabel = modelChipLabel
        self.isFocused = isFocused
        self.isGenerating = isGenerating
        self.hasAttachment = hasAttachment
        self.onAttach = onAttach
        self.onMic = onMic
        self.onMicLongPress = onMicLongPress
        self.onSend = onSend
        self.onStop = onStop
        self.externalFocus = externalFocus
    }

    private var state: ComposerState {
        if isGenerating { return .generating }
        return (text.isEmpty && !hasAttachment) ? .idle : .hasText
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s8) {
            if isFocused, let modelChipLabel {
                HStack(spacing: PocketSpacing.s4) {
                    Text(modelChipLabel).font(.pocket(.caption))
                    Image(systemName: "chevron.down").font(.system(size: 9))
                }
                .foregroundStyle(PocketColor.textSecondary)
                .padding(.horizontal, PocketSpacing.s4)
            }

            HStack(spacing: PocketSpacing.s12) {
                Menu {
                    Button { onAttach(.photo) } label: { Text("Photo", bundle: .module) }
                    Button { onAttach(.camera) } label: { Text("Camera", bundle: .module) }
                    Button { onAttach(.image) } label: { Text("Generate image", bundle: .module) }
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 16, weight: .medium))
                        .foregroundStyle(PocketColor.textSecondary)
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                }
                .menuStyle(.button)
                .buttonStyle(.plain)
                .accessibilityLabel(Text("Attach media or generate image", bundle: .module))

                Group {
                    if let externalFocus {
                        TextField(String(localized: "Message", bundle: .module), text: $text, axis: .vertical)
                            .focused(externalFocus)
                    } else {
                        TextField(String(localized: "Message", bundle: .module), text: $text, axis: .vertical)
                    }
                }
                .accessibilityIdentifier("chat_composer")
                .font(.pocket(.body))
                .lineLimit(1...6)
                .textFieldStyle(.plain)

                trailingControl
            }
            .padding(.horizontal, PocketSpacing.s8)
            .padding(.vertical, PocketSpacing.s4)
            .background(.regularMaterial, in: Capsule())
        }
    }

    @ViewBuilder
    private var trailingControl: some View {
        switch state {
        case .generating:
            Button(action: onStop) {
                Image(systemName: "stop.fill")
                    // Inverse of the textPrimary disc: `.white` vanished on
                    // dark mode's near-white fill.
                    .foregroundStyle(PocketColor.bgCanvas)
                    .frame(width: 28, height: 28)
                    .background(PocketColor.textPrimary, in: Circle())
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("Stop generating", bundle: .module))
        case .hasText:
            Button(action: onSend) {
                Image(systemName: "arrow.up")
                    .foregroundStyle(PocketColor.accentOnAccent)
                    .frame(width: 28, height: 28)
                    .background(PocketColor.accent, in: Circle())
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("Send message", bundle: .module))
            .accessibilityIdentifier("chat_send_button")
        case .idle:
            if let onMicLongPress {
                Button(action: onMic) {
                    Image(systemName: "mic")
                        .foregroundStyle(PocketColor.textSecondary)
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .onLongPressGesture(perform: onMicLongPress)
                .accessibilityLabel(Text("Voice input", bundle: .module))
                .accessibilityHint(Text("Double-tap to dictate, or double-tap and hold for hands-free send", bundle: .module))
                .accessibilityAction(named: Text("Hands-free send", bundle: .module), onMicLongPress)
            } else {
                Button(action: onMic) {
                    Image(systemName: "mic")
                        .foregroundStyle(PocketColor.textSecondary)
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text("Dictate", bundle: .module))
                .accessibilityHint(Text("Double-tap to dictate", bundle: .module))
            }
        }
    }
}

#Preview("Light") {
    VStack {
        Spacer()
        Composer(
            text: .constant(""),
            modelChipLabel: "Qwen3 4B",
            isFocused: true,
            isGenerating: false,
            onAttach: { _ in },
            onMic: {},
            onSend: {},
            onStop: {}
        )
        .padding()
    }
    .background(PocketColor.bgCanvas)
}

#Preview("Dark - has text") {
    VStack {
        Spacer()
        Composer(
            text: .constant("What's the capital of France?"),
            modelChipLabel: "Qwen3 4B",
            isFocused: true,
            isGenerating: false,
            onAttach: { _ in },
            onMic: {},
            onSend: {},
            onStop: {}
        )
        .padding()
    }
    .background(PocketColor.bgCanvas)
    .preferredColorScheme(.dark)
}
