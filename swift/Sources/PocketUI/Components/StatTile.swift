import SwiftUI

/// C20: label + big metric value + optional delta/hint, used in Mac
/// Server and Pulse detail.
public struct StatTile: View {
    private let label: String
    private let value: String
    private let hint: String?

    public init(label: String, value: String, hint: String? = nil) {
        self.label = label
        self.value = value
        self.hint = hint
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s4) {
            Text(label)
                .font(.pocket(.caption))
                .foregroundStyle(PocketColor.textSecondary)
            Text(value)
                .font(.pocket(.metric))
                .foregroundStyle(PocketColor.textPrimary)
            if let hint {
                Text(hint)
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
            }
        }
        .padding(PocketSpacing.s12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: PocketRadius.card).fill(PocketColor.bgSurface))
    }
}

#Preview("Light") {
    HStack {
        StatTile(label: "Requests today", value: "142", hint: "+12% vs. yesterday")
        StatTile(label: "Connected devices", value: "2")
    }
    .padding()
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    StatTile(label: "Decode speed", value: "38 tok/s", hint: "measured")
        .padding()
        .background(PocketColor.bgCanvas)
        .preferredColorScheme(.dark)
}
