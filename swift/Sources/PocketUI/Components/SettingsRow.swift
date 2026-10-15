import SwiftUI

/// C19: consistent icons in rounded-square tiles tinted by section, plus
/// one of four row kinds.
public enum SettingsRowAccessory {
    case toggle(Binding<Bool>)
    case value(String)
    case navigation
    case destructive
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

    public var body: some View {
        HStack(spacing: PocketSpacing.s12) {
            RoundedRectangle(cornerRadius: 7)
                .fill(tint)
                .frame(width: 28, height: 28)
                .overlay {
                    Image(systemName: icon)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(PocketColor.accentOnAccent)
                }
                .accessibilityHidden(true)

            Text(title)
                .font(.pocket(.body))
                .foregroundStyle(isDestructive ? PocketColor.statusBad : PocketColor.textPrimary)

            Spacer()

            trailing
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
            Toggle("", isOn: binding).labelsHidden()
        case .value(let text):
            Text(text).font(.pocket(.callout)).foregroundStyle(PocketColor.textSecondary)
        case .navigation:
            Image(systemName: "chevron.right").font(.caption).foregroundStyle(PocketColor.textTertiary)
        case .destructive:
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
