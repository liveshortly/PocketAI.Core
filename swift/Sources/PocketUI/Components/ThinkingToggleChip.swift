import SwiftUI

/// UI-30: per-chat thinking switch whose state is readable from a screenshot.
/// On = green tint + green border; off = clear fill, secondary text. Callers
/// hide it entirely for models without reasoning — never a dead control.
/// VoiceOver: "Thinking, on, toggle button".
public struct ThinkingToggleChip: View {
    @Binding var isOn: Bool

    public init(isOn: Binding<Bool>) {
        _isOn = isOn
    }

    public var body: some View {
        Button { isOn.toggle() } label: {
            HStack(spacing: PocketSpacing.s4) {
                Image(systemName: "brain")
                Text(String(localized: "Think", bundle: .module))
                    .fixedSize()
            }
            .font(.pocket(.caption))
            .fontWeight(.semibold)
            .foregroundStyle(isOn ? PocketColor.statusOk : PocketColor.textSecondary)
            .padding(.horizontal, PocketSpacing.s8)
            .padding(.vertical, PocketSpacing.s4)
            .background(Capsule().fill(isOn ? PocketColor.statusOk.opacity(0.16) : Color.clear))
            .overlay(Capsule().strokeBorder(isOn ? PocketColor.statusOk : PocketColor.lineStrong))
            #if os(iOS)
            .frame(minHeight: 44)
            #endif
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Thinking", bundle: .module))
        .accessibilityValue(isOn ? Text("On", bundle: .module) : Text("Off", bundle: .module))
        .accessibilityAddTraits(.isToggle)
        .accessibilityHint(Text("Reason before answering in this chat", bundle: .module))
    }
}

#Preview("Light") {
    HStack {
        ThinkingToggleChip(isOn: .constant(true))
        ThinkingToggleChip(isOn: .constant(false))
    }
    .padding()
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    HStack {
        ThinkingToggleChip(isOn: .constant(true))
        ThinkingToggleChip(isOn: .constant(false))
    }
    .padding()
    .background(PocketColor.bgCanvas)
    .preferredColorScheme(.dark)
}

#Preview("AX3") {
    HStack {
        ThinkingToggleChip(isOn: .constant(true))
        ThinkingToggleChip(isOn: .constant(false))
    }
    .padding()
    .background(PocketColor.bgCanvas)
    .environment(\.dynamicTypeSize, .accessibility3)
}
