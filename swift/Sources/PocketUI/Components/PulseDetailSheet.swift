import SwiftUI

public struct LoadedModelSummary: Identifiable, Sendable {
    public let id: String
    public let name: String
    public let residentBytes: Int64
    public let onEject: @Sendable () -> Void

    public init(id: String, name: String, residentBytes: Int64, onEject: @escaping @Sendable () -> Void) {
        self.id = id
        self.name = name
        self.residentBytes = residentBytes
        self.onEject = onEject
    }
}

public struct LastReplyStats: Sendable, Equatable {
    public let ttftMs: Int
    public let tokens: Int
    public let tokPerSec: Double
    public let isMeasured: Bool

    public init(ttftMs: Int, tokens: Int, tokPerSec: Double, isMeasured: Bool) {
        self.ttftMs = ttftMs
        self.tokens = tokens
        self.tokPerSec = tokPerSec
        self.isMeasured = isMeasured
    }
}

/// C02's expanded sheet (detents medium/large): RAM gauge, thermal state,
/// loaded models with eject, last reply stats, Network Proof summary.
public struct PulseDetailSheet: View {
    private let ramUsedBytes: Int64
    private let ramBudgetBytes: Int64
    private let ramTotalBytes: Int64
    private let thermalLabel: String
    private let loadedModels: [LoadedModelSummary]
    private let lastReply: LastReplyStats?
    private let egressCountToday: Int

    public init(
        ramUsedBytes: Int64,
        ramBudgetBytes: Int64,
        ramTotalBytes: Int64,
        thermalLabel: String,
        loadedModels: [LoadedModelSummary],
        lastReply: LastReplyStats?,
        egressCountToday: Int
    ) {
        self.ramUsedBytes = ramUsedBytes
        self.ramBudgetBytes = ramBudgetBytes
        self.ramTotalBytes = ramTotalBytes
        self.thermalLabel = thermalLabel
        self.loadedModels = loadedModels
        self.lastReply = lastReply
        self.egressCountToday = egressCountToday
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: PocketSpacing.s24) {
                ramGauge
                HStack {
                    StatTile(label: "Thermal state", value: thermalLabel)
                    if let lastReply {
                        StatTile(
                            label: "Last reply",
                            value: String(format: "%.0f tok/s", lastReply.tokPerSec),
                            hint: "\(lastReply.ttftMs) ms TTFT · \(lastReply.isMeasured ? "measured" : "est.")"
                        )
                    }
                }

                if !loadedModels.isEmpty {
                    VStack(alignment: .leading, spacing: PocketSpacing.s8) {
                        Text("Loaded models").font(.pocket(.caption)).foregroundStyle(PocketColor.textTertiary)
                        ForEach(loadedModels) { model in
                            HStack {
                                Text(model.name).font(.pocket(.body))
                                Spacer()
                                Text(Self.gb(model.residentBytes)).font(.pocket(.caption)).foregroundStyle(PocketColor.textSecondary)
                                Button("Eject", action: model.onEject)
                                    .font(.pocket(.caption))
                                    .foregroundStyle(PocketColor.statusBad)
                            }
                        }
                    }
                }

                StatTile(label: "Network Proof", value: "\(egressCountToday) connections today")
            }
            .padding()
        }
        .background(PocketColor.bgCanvas)
    }

    private var ramGauge: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s8) {
            Text("Memory").font(.pocket(.caption)).foregroundStyle(PocketColor.textTertiary)
            GeometryReader { geometry in
                ZStack(alignment: .leading) {
                    Capsule().fill(PocketColor.bgSunken)
                    Capsule()
                        .fill(PocketColor.accent)
                        .frame(width: geometry.size.width * usedFraction)
                    Capsule()
                        .stroke(PocketColor.lineStrong, lineWidth: 1)
                        .frame(width: geometry.size.width * budgetFraction)
                }
            }
            .frame(height: 10)
            Text("\(Self.gb(ramUsedBytes)) used by models · budget \(Self.gb(ramBudgetBytes)) · \(Self.gb(ramTotalBytes)) total")
                .font(.pocket(.caption))
                .foregroundStyle(PocketColor.textSecondary)
        }
    }

    private var usedFraction: Double {
        ramTotalBytes > 0 ? min(Double(ramUsedBytes) / Double(ramTotalBytes), 1) : 0
    }

    private var budgetFraction: Double {
        ramTotalBytes > 0 ? min(Double(ramBudgetBytes) / Double(ramTotalBytes), 1) : 0
    }

    private static func gb(_ bytes: Int64) -> String {
        String(format: "%.1f GB", Double(bytes) / 1_000_000_000)
    }
}

#Preview("Light") {
    PulseDetailSheet(
        ramUsedBytes: 2_300_000_000,
        ramBudgetBytes: 4_600_000_000,
        ramTotalBytes: 8_000_000_000,
        thermalLabel: "Nominal",
        loadedModels: [LoadedModelSummary(id: "1", name: "Qwen3 4B", residentBytes: 2_300_000_000, onEject: {})],
        lastReply: LastReplyStats(ttftMs: 320, tokens: 84, tokPerSec: 27, isMeasured: true),
        egressCountToday: 0
    )
}

#Preview("Dark") {
    PulseDetailSheet(
        ramUsedBytes: 2_300_000_000,
        ramBudgetBytes: 4_600_000_000,
        ramTotalBytes: 8_000_000_000,
        thermalLabel: "Nominal",
        loadedModels: [],
        lastReply: nil,
        egressCountToday: 3
    )
    .preferredColorScheme(.dark)
}
