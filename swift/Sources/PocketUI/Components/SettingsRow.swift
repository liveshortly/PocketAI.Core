import SwiftUI

/// C19: consistent icons in rounded-square tiles tinted by section, plus
/// one of four row kinds.
public enum SettingsRowAccessory {
    case toggle(Binding<Bool>)
    case value(String)
    /// Draws its own chevron. For a row inside a `NavigationLink`, which
    /// draws one already, use `.none` instead or the row shows two.
    case navigation
    case destructive
    /// No trailing element at all.
    case none
}

public struct SettingsRow: View {
    private let icon: String
    private let tint: Color
    private let title: String
    private let accessory: SettingsRowAccessory

    public init(icon: String, tint: Color = PocketColor.accent, title: String, accessory: SettingsRowAccessory) {
        self.icon = icon
        self.tint = tint
        self.title = title
        self.accessory = accessory
    }

    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    /// The tile grows with the text, so the icon never shrinks to a dot beside
    /// accessibility-sized type.
    @ScaledMetric(relativeTo: .body) private var tileSize: CGFloat = 28
    @ScaledMetric(relativeTo: .body) private var glyphSize: CGFloat = 14

    public var body: some View {
        // At accessibility sizes a title and its value cannot share a line:
        // both get squeezed into narrow columns and hyphenate mid-word
        // ("De-fault chat mod-el" beside "Cho ose"). Stack them instead.
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: PocketSpacing.s8))
            : AnyLayout(HStackLayout(spacing: PocketSpacing.s12))

        return layout {
            HStack(spacing: PocketSpacing.s12) {
                RoundedRectangle(cornerRadius: 7)
                    .fill(tint)
                    .frame(width: tileSize, height: tileSize)
                    .overlay {
                        Image(systemName: icon)
                            .font(.system(size: glyphSize, weight: .semibold))
                            .foregroundStyle(PocketColor.accentOnAccent)
                    }
                    .accessibilityHidden(true)

                Text(title)
                    .font(.pocket(.body))
                    .foregroundStyle(
                        isDestructive ? PocketColor.statusBad : PocketColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)

                if !dynamicTypeSize.isAccessibilitySize { Spacer(minLength: PocketSpacing.s8) }
            }

            trailing
                .frame(
                    maxWidth: dynamicTypeSize.isAccessibilitySize ? .infinity : nil,
                    alignment: .leading)
        }
        .padding(.vertical, PocketSpacing.s4)
    }

    private var isDestructive: Bool {
        if case .destructive = accessory { return true }
        return false
    }

    @ViewBuilder
    private var trailing: some View {
        switch accessory {
        case .toggle(let binding):
            // Hidden visually, still read by VoiceOver: an empty label made
            // every settings switch announce as just "switch, off".
            Toggle(title, isOn: binding).labelsHidden()
        case .value(let text):
            Text(text)
                .font(.pocket(.callout))
                .foregroundStyle(PocketColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        case .navigation:
            Image(systemName: "chevron.right").font(.caption).foregroundStyle(PocketColor.textTertiary)
        case .destructive:
            EmptyView()
        case .none:
            EmptyView()
        }
    }
}

#Preview("Light") {
    List {
        SettingsRow(icon: "network", title: "Network Proof", accessory: .navigation)
        SettingsRow(icon: "airplane", tint: PocketColor.statusWarn, title: "Offline mode", accessory: .toggle(.constant(false)))
        SettingsRow(icon: "cpu", tint: PocketColor.statusInfo, title: "Memory limit", accessory: .value("70%"))
        SettingsRow(icon: "trash", tint: PocketColor.statusBad, title: "Delete all models", accessory: .destructive)
    }
}

#Preview("Dark") {
    List {
        SettingsRow(icon: "network", title: "Network Proof", accessory: .navigation)
        SettingsRow(icon: "airplane", tint: PocketColor.statusWarn, title: "Offline mode", accessory: .toggle(.constant(true)))
    }
    .preferredColorScheme(.dark)
}
