import SwiftUI

public enum PocketMarkState: Equatable, Sendable {
    case idle
    case working(tokPerSec: Double?)
    case loading
    case offline
}

/// The Spark core chip (D13): working and loading = signals fire outward
/// along traces (owner, 2026-11-02: loading shows the chip, not the thread
/// line); idle/offline are static. Pauses
/// off-screen via `TimelineView(.animation(paused:))` and falls back to a
/// cross-fade under Reduce Motion.
public struct PocketMarkView: View {
    private let state: PocketMarkState
    private let size: CGFloat

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(state: PocketMarkState, size: CGFloat) {
        self.state = state
        self.size = size
    }

    private var isAnimating: Bool {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-PocketPerfDiagnostics") {
            return false
        }
        #endif
        switch state {
        case .working, .loading: return !reduceMotion
        case .idle, .offline: return false
        }
    }

    private var baseColor: Color {
        switch state {
        case .offline: return PocketColor.textTertiary
        default: return PocketColor.accent
        }
    }

    public var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60.0, paused: !isAnimating)) { context in
            Canvas { gfx, canvasSize in
                let rect = CGRect(origin: .zero, size: canvasSize)
                let phase = isAnimating
                    ? context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1.4) / 1.4
                    : 0

                gfx.stroke(PocketChipShape().path(in: rect), with: .color(baseColor), lineWidth: PocketMarkGeometry.chipStrokeWidth * scale(for: rect))
                gfx.fill(PocketSparkleShape().path(in: rect), with: .color(baseColor))

                for index in 0..<PocketMarkGeometry.allTraces.count {
                    let trace = PocketTraceShape(index: index).path(in: rect)
                    let pad = PocketTracePadShape(index: index).path(in: rect)
                    let stagger = Double(index) / Double(PocketMarkGeometry.allTraces.count)
                    let local = isAnimating ? fireProgress(phase: phase, stagger: stagger) : 1.0
                    gfx.stroke(
                        trace.trimmedPath(from: 0, to: local),
                        with: .color(baseColor),
                        style: StrokeStyle(lineWidth: PocketMarkGeometry.traceStrokeWidth * scale(for: rect), lineCap: .round, lineJoin: .round)
                    )
                    gfx.stroke(pad, with: .color(baseColor), lineWidth: PocketMarkGeometry.tracePadStrokeWidth * scale(for: rect))
                }
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel(accessibilityLabel)
    }

    private func scale(for rect: CGRect) -> CGFloat {
        min(rect.width, rect.height) / PocketMarkGeometry.viewBoxSize
    }

    /// A signal fires from the chip to its pad over a staggered window,
    /// looping continuously while `.working` or `.loading`.
    private func fireProgress(phase: Double, stagger: Double) -> CGFloat {
        let windowed = (phase + stagger).truncatingRemainder(dividingBy: 1.0)
        return CGFloat(min(1.0, windowed * 2.2))
    }

    private var accessibilityLabel: String {
        switch state {
        // "PocketAI" is the product name and is deliberately not localized.
        case .idle: return "PocketAI"
        case .working(let tokPerSec):
            if let tokPerSec {
                return String(localized: "PocketAI is generating, \(Int(tokPerSec)) tokens per second", bundle: .module)
            }
            return String(localized: "PocketAI is generating", bundle: .module)
        case .loading: return String(localized: "PocketAI is loading a model", bundle: .module)
        case .offline: return String(localized: "PocketAI is offline", bundle: .module)
        }
    }
}

#Preview("States") {
    HStack(spacing: PocketSpacing.s24) {
        PocketMarkView(state: .idle, size: 64)
        PocketMarkView(state: .working(tokPerSec: 42), size: 64)
        PocketMarkView(state: .loading, size: 64)
        PocketMarkView(state: .offline, size: 64)
    }
    .padding()
    .background(PocketColor.bgCanvas)
}
