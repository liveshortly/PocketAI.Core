import SwiftUI

/// UI-26: a small capsule chip marking a model that can emit reasoning
/// ("thinking") tokens. Non-thinking models show nothing — there is no
/// negative badge. Visuals match `FitBadge` (caption text in a sunken capsule).
public struct ThinkingBadge: View {
    public init() {}

    public var body: some View {
        HStack(spacing: PocketSpacing.s4) {
            Image(systemName: "brain")
                .font(.system(size: 8, weight: .semibold))
                .foregroundStyle(PocketColor.accent)
            Text(String(localized: "Thinking", bundle: .module))
                .font(.pocket(.caption))
                .foregroundStyle(PocketColor.textSecondary)
                .fixedSize()
        }
        .padding(.horizontal, PocketSpacing.s8)
        .padding(.vertical, PocketSpacing.s4)
        .background(Capsule().fill(PocketColor.bgSunken))
        .fixedSize()
        .accessibilityLabel("Supports thinking")
    }
}

#Preview("Light") {
    ThinkingBadge()
        .padding()
        .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    ThinkingBadge()
        .padding()
        .background(PocketColor.bgCanvas)
        .preferredColorScheme(.dark)
}

#Preview("AX3") {
    ThinkingBadge()
        .padding()
        .background(PocketColor.bgCanvas)
        .environment(\.dynamicTypeSize, .accessibility3)
}
