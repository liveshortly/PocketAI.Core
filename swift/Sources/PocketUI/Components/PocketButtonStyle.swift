import SwiftUI

/// PocketUI button styles for primary and secondary call-to-actions,
/// ensuring consistent green accent styling, high WCAG contrast, and robust cross-platform rendering.
public struct PocketPrimaryButtonStyle: ButtonStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        PrimaryBody(configuration: configuration)
    }

    private struct PrimaryBody: View {
        let configuration: Configuration
        @Environment(\.isEnabled) private var isEnabled

        var body: some View {
        configuration.label
            .font(.pocket(.headline))
            // White failed contrast on the dark-mode green (UI-55 fixed one
            // button by hand); a disabled button looked enabled.
            .foregroundStyle(PocketColor.accentOnAccent)
            .padding(.horizontal, PocketSpacing.s16)
            .padding(.vertical, PocketSpacing.s8)
            .background(configuration.isPressed ? PocketColor.accent.opacity(0.85) : PocketColor.accent)
            .clipShape(RoundedRectangle(cornerRadius: PocketRadius.chip, style: .continuous))
            .shadow(color: PocketColor.accent.opacity(0.2), radius: configuration.isPressed ? 1 : 2, y: 1)
            .opacity(isEnabled ? 1 : 0.4)
            .contentShape(Rectangle())
        }
    }
}

public struct PocketSecondaryButtonStyle: ButtonStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        SecondaryBody(configuration: configuration)
    }

    private struct SecondaryBody: View {
        let configuration: Configuration
        @Environment(\.isEnabled) private var isEnabled

        var body: some View {
        configuration.label
            .font(.pocket(.body))
            .foregroundStyle(PocketColor.textPrimary)
            .padding(.horizontal, PocketSpacing.s16)
            .padding(.vertical, PocketSpacing.s8)
            .background(configuration.isPressed ? PocketColor.bgSunken : PocketColor.bgSurface)
            .clipShape(RoundedRectangle(cornerRadius: PocketRadius.chip, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: PocketRadius.chip, style: .continuous)
                    .stroke(PocketColor.lineHairline, lineWidth: 1)
            )
            .opacity(isEnabled ? 1 : 0.4)
            .contentShape(Rectangle())
        }
    }
}

extension ButtonStyle where Self == PocketPrimaryButtonStyle {
    public static var pocketPrimary: PocketPrimaryButtonStyle { PocketPrimaryButtonStyle() }
}

extension ButtonStyle where Self == PocketSecondaryButtonStyle {
    public static var pocketSecondary: PocketSecondaryButtonStyle { PocketSecondaryButtonStyle() }
}
