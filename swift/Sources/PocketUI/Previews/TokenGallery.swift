import SwiftUI

/// Shows every token side by side, per UI-01's "Done when".
public struct TokenGallery: View {
    public init() {}

    private let colorSwatches: [(String, Color)] = [
        ("bg.canvas", PocketColor.bgCanvas),
        ("bg.surface", PocketColor.bgSurface),
        ("bg.raised", PocketColor.bgRaised),
        ("bg.sunken", PocketColor.bgSunken),
        ("text.primary", PocketColor.textPrimary),
        ("text.secondary", PocketColor.textSecondary),
        ("text.tertiary", PocketColor.textTertiary),
        ("accent", PocketColor.accent),
        ("accent.soft", PocketColor.accentSoft),
        ("status.ok", PocketColor.statusOk),
        ("status.warn", PocketColor.statusWarn),
        ("status.bad", PocketColor.statusBad),
        ("status.info", PocketColor.statusInfo),
        ("privacy.local", PocketColor.privacyLocal),
    ]

    private let textStyles: [(String, PocketTextStyle)] = [
        ("display", .display), ("title", .title), ("headline", .headline),
        ("body", .body), ("callout", .callout), ("footnote", .footnote),
        ("caption", .caption), ("mono", .mono), ("metric", .metric), ("reading", .reading),
    ]

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: PocketSpacing.s24) {
                Group {
                    Text("Colors").font(.pocket(.title))
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 140))], spacing: PocketSpacing.s12) {
                        ForEach(colorSwatches, id: \.0) { name, color in
                            VStack(alignment: .leading, spacing: PocketSpacing.s4) {
                                RoundedRectangle(cornerRadius: PocketRadius.chip, style: .continuous)
                                    .fill(color)
                                    .frame(height: 48)
                                Text(name).font(.pocket(.caption))
                            }
                        }
                    }
                }
                Group {
                    Text("Typography").font(.pocket(.title))
                    ForEach(textStyles, id: \.0) { name, style in
                        Text("\(name) — The quick brown fox").font(.pocket(style))
                    }
                }
            }
            .padding(PocketSpacing.gutterIphone)
        }
        .background(PocketColor.bgCanvas)
    }
}

#Preview("Light") {
    TokenGallery().preferredColorScheme(.light)
}

#Preview("Dark") {
    TokenGallery().preferredColorScheme(.dark)
}
