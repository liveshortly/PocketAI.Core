import SwiftUI

/// C13: a paired device - glyph, name, connection status dot, route
/// label, revoke action.
public struct DeviceRow: View {
    private let name: String
    private let isConnected: Bool
    private let lastSeenText: String
    private let route: String?
    private let onRevoke: () -> Void

    public init(name: String, isConnected: Bool, lastSeenText: String, route: String?, onRevoke: @escaping () -> Void) {
        self.name = name
        self.isConnected = isConnected
        self.lastSeenText = lastSeenText
        self.route = route
        self.onRevoke = onRevoke
    }

    public var body: some View {
        HStack(spacing: PocketSpacing.s12) {
            Image(systemName: "iphone")
                .font(.system(size: 20))
                .foregroundStyle(PocketColor.textSecondary)
                .frame(width: 28)

            VStack(alignment: .leading, spacing: PocketSpacing.s2) {
                HStack(spacing: PocketSpacing.s4) {
                    Circle()
                        .fill(isConnected ? PocketColor.statusOk : PocketColor.textTertiary)
                        .frame(width: 6, height: 6)
                    Text(name)
                        .font(.pocket(.body))
                        .foregroundStyle(PocketColor.textPrimary)
                }
                Text(isConnected ? (route ?? "Connected") : lastSeenText)
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
            }

            Spacer()

            Button("Revoke", role: .destructive, action: onRevoke)
                .font(.pocket(.callout))
        }
        .padding(.vertical, PocketSpacing.s4)
    }
}

#Preview("Light") {
    List {
        DeviceRow(name: "Rohit's iPhone", isConnected: true, lastSeenText: "", route: "Wi-Fi · 2 ms", onRevoke: {})
        DeviceRow(name: "Work iPad", isConnected: false, lastSeenText: "Last seen 2 h ago", route: nil, onRevoke: {})
    }
}

#Preview("Dark") {
    List {
        DeviceRow(name: "Rohit's iPhone", isConnected: true, lastSeenText: "", route: "Wi-Fi · 2 ms", onRevoke: {})
    }
    .preferredColorScheme(.dark)
}
