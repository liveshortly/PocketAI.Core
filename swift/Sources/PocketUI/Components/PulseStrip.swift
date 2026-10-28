import SwiftUI

/// C02 §4.2: one hardware-like instrument - collapsed capsule always
/// visible, tap opens `PulseDetailSheet`. Not wired to a live
/// `CoreClient.events` stream (that surface doesn't exist yet - `CoreClient`
/// only exposes `mockGenerate`, D22/D52); `PulseState` is the data contract
/// whoever does wire it up needs to produce.
public enum PulseState: Sendable, Equatable {
    case idle
    case generating(modelName: String, location: String, tokPerSec: Double)
    case loading(modelName: String, detail: String)
    case offlineMode
    case remote(modelName: String)
}

public struct PulseStrip: View {
    private let state: PulseState
    private let onTap: () -> Void

    public init(state: PulseState, onTap: @escaping () -> Void) {
        self.state = state
        self.onTap = onTap
    }

    public var body: some View {
        Button(action: onTap) {
            HStack(spacing: PocketSpacing.s8) {
                mark
                Text(label)
                    .font(.pocket(.footnote))
                    .foregroundStyle(PocketColor.textPrimary)
                    .lineLimit(1)
                    .accessibilityHidden(true)
                Spacer(minLength: 0)
                if case .generating(_, _, let tokPerSec) = state {
                    Text(String(localized: "\(Int(tokPerSec.rounded())) tok/s", bundle: .module))
                        .font(.pocket(.metric))
                        .foregroundStyle(PocketColor.textSecondary)
                }
            }
            .padding(.horizontal, PocketSpacing.s16)
            .frame(height: 44)
            .background(PocketColor.bgRaised, in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
        .accessibilityHint(Text("Double-tap for details.", bundle: .module))
        .accessibilityAddTraits(.isButton)
    }

    @ViewBuilder
    private var mark: some View {
        switch state {
        case .idle:
            PocketMarkView(state: .idle, size: 18)
        case .generating(_, _, let tokPerSec):
            PocketMarkView(state: .working(tokPerSec: tokPerSec), size: 18)
        case .loading:
            PocketMarkView(state: .loading, size: 18)
        case .offlineMode:
            PocketMarkView(state: .offline, size: 18).opacity(0.4)
        case .remote:
            Image(systemName: "laptopcomputer")
                .font(.system(size: 16))
                .foregroundStyle(PocketColor.statusInfo)
        }
    }

    private var label: String {
        switch state {
        case .idle: String(localized: "Ready", bundle: .module)
        // Both parts are caller-supplied text; nothing here to translate.
        case .generating(let model, let location, _): "\(model) · \(location)"
        case .loading(let model, let detail): "\(detail) \(model)"
        case .offlineMode: String(localized: "Offline", bundle: .module)
        case .remote(let model): String(localized: "On \(model)", bundle: .module)
        }
    }

    private var accessibilityLabel: String {
        switch state {
        case .generating(let model, let location, let tokPerSec):
            return String(localized: "\(model), running \(location), \(Int(tokPerSec)) tokens per second.", bundle: .module)
        default:
            return label
        }
    }
}

#Preview("Light") {
    VStack(spacing: PocketSpacing.s12) {
        PulseStrip(state: .idle, onTap: {})
        PulseStrip(state: .generating(modelName: "Qwen3 4B", location: "On this iPhone", tokPerSec: 27), onTap: {})
        PulseStrip(state: .loading(modelName: "2.3 GB", detail: "Loading"), onTap: {})
        PulseStrip(state: .offlineMode, onTap: {})
        PulseStrip(state: .remote(modelName: "Rohit's Mac"), onTap: {})
    }
    .padding()
    .background(PocketColor.bgCanvas)
}

#Preview("Dark") {
    PulseStrip(state: .generating(modelName: "Qwen3 4B", location: "On this iPhone", tokPerSec: 27), onTap: {})
        .padding()
        .background(PocketColor.bgCanvas)
        .preferredColorScheme(.dark)
}
