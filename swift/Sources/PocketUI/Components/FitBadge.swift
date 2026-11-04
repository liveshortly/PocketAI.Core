import SwiftUI

/// C09: a capsule chip reporting whether a model fits this device, per
/// `FitReport.verdict` (A§4.4). Tapping shows `FitReport.reason` in a
/// popover rather than cluttering the row with it directly.
public enum FitBadgeKind: Sendable {
    case runsWell
    case runsSlow
    case willNotFit
    case notSupported
}

public struct FitBadge: View {
    private let kind: FitBadgeKind
    private let reason: String
    /// False when the badge sits inside another button (Home's Model card):
    /// it then shows the label only and the reason stays in the a11y label.
    private let interactive: Bool
    @State private var showReason = false

    public init(kind: FitBadgeKind, reason: String, interactive: Bool = true) {
        self.kind = kind
        self.reason = reason
        self.interactive = interactive
    }

    public var body: some View {
        if interactive { button } else { chip.accessibilityLabel("\(label). \(reason)") }
    }

    private var chip: some View {
        HStack(spacing: PocketSpacing.s4) {
            Circle()
                .fill(dotColor)
                .frame(width: 6, height: 6)
            Text(label)
                .font(.pocket(.caption))
                .foregroundStyle(PocketColor.textSecondary)
                .fixedSize()
        }
        .padding(.horizontal, PocketSpacing.s8)
        .padding(.vertical, PocketSpacing.s4)
        .background(Capsule().fill(PocketColor.bgSunken))
    }

    private var button: some View {
        Button {
            showReason = true
        } label: {
            chip
                #if os(iOS)
                .frame(minHeight: 44) // 44pt touch target (UI-58)
                .contentShape(Rectangle())
                #endif
        }
        .buttonStyle(.plain)
        .fixedSize()
        .accessibilityLabel("\(label). \(reason)")
        .popover(isPresented: $showReason) {
            Text(reason)
                .font(.pocket(.footnote))
                .foregroundStyle(PocketColor.textPrimary)
                .padding(PocketSpacing.s12)
                .frame(maxWidth: 260, alignment: .leading)
                .presentationCompactAdaptation(.popover)
        }
    }

    private var dotColor: Color {
        switch kind {
        case .runsWell: PocketColor.statusOk
        case .runsSlow: PocketColor.statusWarn
        case .willNotFit: PocketColor.statusBad
        case .notSupported: PocketColor.textTertiary
        }
    }

    private var label: String {
        switch kind {
        case .runsWell: String(localized: "Runs well", bundle: .module)
        case .runsSlow: String(localized: "Runs slow", bundle: .module)
        case .willNotFit: String(localized: "Won't fit", bundle: .module)
        case .notSupported: String(localized: "Not supported", bundle: .module)
        }
    }
}

#Preview("Light") {
    VStack(alignment: .leading, spacing: PocketSpacing.s12) {
        FitBadge(kind: .runsWell, reason: "Fits comfortably in available memory.")
        FitBadge(kind: .runsSlow, reason: "Fits, but expect under 5 tok/s on this device.")
        FitBadge(kind: .willNotFit, reason: "Needs 9.2 GB; only 6.0 GB is available.")
        FitBadge(kind: .notSupported, reason: "This engine isn't available on this platform.")
    }
    .padding()
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    VStack(alignment: .leading, spacing: PocketSpacing.s12) {
        FitBadge(kind: .runsWell, reason: "Fits comfortably in available memory.")
        FitBadge(kind: .willNotFit, reason: "Needs 9.2 GB; only 6.0 GB is available.")
    }
    .padding()
    .background(PocketColor.bgCanvas)
    .preferredColorScheme(.dark)
}
