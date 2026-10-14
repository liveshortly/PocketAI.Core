import SwiftUI

public struct ModelRowData: Sendable {
    public let name: String
    public let familyParamsQuant: String
    public let fitKind: FitBadgeKind
    public let fitReason: String
    public let sizeOnDisk: String?
    public let speedText: String?
    public let speedIsMeasured: Bool
    public let licenseLabel: String

    public init(
        name: String,
        familyParamsQuant: String,
        fitKind: FitBadgeKind,
        fitReason: String,
        sizeOnDisk: String?,
        speedText: String?,
        speedIsMeasured: Bool,
        licenseLabel: String
    ) {
        self.name = name
        self.familyParamsQuant = familyParamsQuant
        self.fitKind = fitKind
        self.fitReason = fitReason
        self.sizeOnDisk = sizeOnDisk
        self.speedText = speedText
        self.speedIsMeasured = speedIsMeasured
        self.licenseLabel = licenseLabel
    }
}

/// C08: a catalog row - name, family/params/quant, `FitBadge`, size on
/// disk, speed (flagged est./measured), license chip, primary action.
public struct ModelRow: View {
    private let data: ModelRowData
    private let primaryActionLabel: String
    private let onPrimaryAction: () -> Void

    public init(data: ModelRowData, primaryActionLabel: String, onPrimaryAction: @escaping () -> Void) {
        self.data = data
        self.primaryActionLabel = primaryActionLabel
        self.onPrimaryAction = onPrimaryAction
    }

    public var body: some View {
        HStack(alignment: .top, spacing: PocketSpacing.s12) {
            VStack(alignment: .leading, spacing: PocketSpacing.s4) {
                Text(data.name)
                    .font(.pocket(.headline))
                    .foregroundStyle(PocketColor.textPrimary)
                Text(data.familyParamsQuant)
                    .font(.pocket(.footnote))
                    .foregroundStyle(PocketColor.textSecondary)

                HStack(spacing: PocketSpacing.s8) {
                    FitBadge(kind: data.fitKind, reason: data.fitReason)
                    if let sizeOnDisk {
                        Text(sizeOnDisk).font(.pocket(.caption)).foregroundStyle(PocketColor.textTertiary)
                    }
                    if let speedText {
                        Text("\(speedText) (\(data.speedIsMeasured ? "measured" : "est."))")
                            .font(.pocket(.caption))
                            .foregroundStyle(PocketColor.textTertiary)
                    }
                }

                Text(data.licenseLabel)
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
            }

            Spacer()

            Button(primaryActionLabel, action: onPrimaryAction)
                .font(.pocket(.callout))
                .buttonStyle(.bordered)
        }
        .padding(.vertical, PocketSpacing.s8)
    }

    private var sizeOnDisk: String? { data.sizeOnDisk }
    private var speedText: String? { data.speedText }
}

#Preview("Light") {
    List {
        ModelRow(
            data: ModelRowData(
                name: "Qwen3 4B",
                familyParamsQuant: "Qwen3 · 4B · 4-bit",
                fitKind: .runsWell,
                fitReason: "Fits comfortably in available memory.",
                sizeOnDisk: "2.3 GB",
                speedText: "32 tok/s",
                speedIsMeasured: true,
                licenseLabel: "Apache 2.0"
            ),
            primaryActionLabel: "Open",
            onPrimaryAction: {}
        )
        ModelRow(
            data: ModelRowData(
                name: "Llama 3.1 8B",
                familyParamsQuant: "Llama 3.1 · 8B · 4-bit",
                fitKind: .willNotFit,
                fitReason: "Needs 9.2 GB; only 6.0 GB is available.",
                sizeOnDisk: nil,
                speedText: nil,
                speedIsMeasured: false,
                licenseLabel: "Llama 3.1 Community License"
            ),
            primaryActionLabel: "Get",
            onPrimaryAction: {}
        )
    }
}

#Preview("Dark") {
    List {
        ModelRow(
            data: ModelRowData(
                name: "Qwen3 4B",
                familyParamsQuant: "Qwen3 · 4B · 4-bit",
                fitKind: .runsWell,
                fitReason: "Fits comfortably.",
                sizeOnDisk: "2.3 GB",
                speedText: "32 tok/s",
                speedIsMeasured: true,
                licenseLabel: "Apache 2.0"
            ),
            primaryActionLabel: "Open",
            onPrimaryAction: {}
        )
    }
    .preferredColorScheme(.dark)
}
