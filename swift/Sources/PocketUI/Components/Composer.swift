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
    let onSend: () -> Void
    let onStop: () -> Void

    public enum AttachmentKind: Sendable {
        case photo, camera, file, projectDocument
    }

    public init(
        text: Binding<String>,
        modelChipLabel: String?,
        isFocused: Bool,
        isGenerating: Bool,
        onAttach: @escaping (AttachmentKind) -> Void,
        onMic: @escaping () -> Void,
        onSend: @escaping () -> Void,
        onStop: @escaping () -> Void
    ) {
        _text = text
        self.modelChipLabel = modelChipLabel
        self.isFocused = isFocused
        self.isGenerating = isGenerating
        self.onAttach = onAttach
        self.onMic = onMic
        self.onSend = onSend
        self.onStop = onStop
    }

    private var state: ComposerState {
        if isGenerating { return .generating }
        return text.isEmpty ? .idle : .hasText
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
                    Button("Photo") { onAttach(.photo) }
                    Button("Camera") { onAttach(.camera) }
                    Button("File") { onAttach(.file) }
                    Button("Project doc") { onAttach(.projectDocument) }
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 16, weight: .medium))
                        .foregroundStyle(PocketColor.textSecondary)
                }
                .menuStyle(.button)
                .buttonStyle(.plain)

                TextField("Message", text: $text, axis: .vertical)
                    .font(.pocket(.body))
                    .lineLimit(1...6)
                    .textFieldStyle(.plain)

                trailingControl
            }
            .padding(.horizontal, PocketSpacing.s16)
            .padding(.vertical, PocketSpacing.s12)
            .background(.regularMaterial, in: Capsule())
        }
    }

    @ViewBuilder
    private var trailingControl: some View {
        switch state {
        case .generating:
            Button(action: onStop) {
                Image(systemName: "stop.fill")
                    .foregroundStyle(.white)
                    .frame(width: 28, height: 28)
                    .background(PocketColor.textPrimary, in: Circle())
            }
            .buttonStyle(.plain)
        case .hasText:
            Button(action: onSend) {
                Image(systemName: "arrow.up")
                    .foregroundStyle(PocketColor.accentOnAccent)
                    .frame(width: 28, height: 28)
                    .background(PocketColor.accent, in: Circle())
            }
            .buttonStyle(.plain)
        case .idle:
            Button(action: onMic) {
                Image(systemName: "mic")
                    .foregroundStyle(PocketColor.textSecondary)
            }
            .buttonStyle(.plain)
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
