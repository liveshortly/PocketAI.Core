import SwiftUI

/// PocketUI button styles for primary and secondary call-to-actions,
/// ensuring consistent Ember styling, high WCAG contrast, and robust cross-platform rendering.
public struct PocketPrimaryButtonStyle: ButtonStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.pocket(.headline))
            .foregroundStyle(Color.white)
            .padding(.horizontal, PocketSpacing.s16)
            .padding(.vertical, PocketSpacing.s8)
            .background(configuration.isPressed ? PocketColor.accent.opacity(0.85) : PocketColor.accent)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .shadow(color: PocketColor.accent.opacity(0.2), radius: configuration.isPressed ? 1 : 2, y: 1)
            .contentShape(Rectangle())
    }
}

public struct PocketSecondaryButtonStyle: ButtonStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.pocket(.body))
            .foregroundStyle(PocketColor.textPrimary)
            .padding(.horizontal, PocketSpacing.s16)
            .padding(.vertical, PocketSpacing.s8)
            .background(configuration.isPressed ? PocketColor.bgSunken : PocketColor.bgSurface)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(PocketColor.lineHairline, lineWidth: 1)
            )
            .contentShape(Rectangle())
    }
}

extension ButtonStyle where Self == PocketPrimaryButtonStyle {
    public static var pocketPrimary: PocketPrimaryButtonStyle { PocketPrimaryButtonStyle() }
}

extension ButtonStyle where Self == PocketSecondaryButtonStyle {
    public static var pocketSecondary: PocketSecondaryButtonStyle { PocketSecondaryButtonStyle() }
}
