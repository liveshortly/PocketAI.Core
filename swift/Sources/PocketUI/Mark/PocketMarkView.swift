import SwiftUI

public enum PocketMarkState: Equatable, Sendable {
    /// The static mark at full opacity (logos, empty states).
    case still
    /// The whole mark breathes between .55 and 1 every 3 s.
    case idle
    /// Generating: pins pulse in a clockwise wave; the core stays steady.
    case thinking(tokPerSec: Double?)
    /// Loading a model: a dash orbits the body over a ghost body.
    case loading
    /// A LiveShortly session is streaming: beams flow in, the panel fills.
    case live
    /// Grey static mark (offline, ended, unavailable).
    case offline
}

/// The chip mark (UI-34): green chip holding the LiveShortly glyph, motion per
/// `strategy/brand/chip-mark-v2.html`. Animates through a paused-when-static
/// `TimelineView`; under Reduce Motion every state is static except Loading,
/// which keeps a slow opacity pulse.
public struct PocketMarkView: View {
    private let state: PocketMarkState
    private let size: CGFloat

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(state: PocketMarkState, size: CGFloat) {
        self.state = state
        self.size = size
    }

    private var motion: MarkMotion? {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-PocketPerfDiagnostics") {
            return nil
        }
        #endif
        switch state {
        case .thinking: return reduceMotion ? nil : .thinking
        case .loading: return reduceMotion ? .breathe : .loading
        case .live: return reduceMotion ? nil : .live
        case .idle: return reduceMotion ? nil : .breathe
        case .still, .offline: return nil
        }
    }

    private var color: Color {
        state == .offline ? PocketColor.textTertiary : PocketColor.markGreen
    }

    public var body: some View {
        let motion = motion
        let color = color
        TimelineView(.animation(minimumInterval: 1.0 / 60.0, paused: motion == nil)) { context in
            Canvas { gfx, canvasSize in
                MarkRenderer.draw(
                    &gfx, rect: CGRect(origin: .zero, size: canvasSize), color: color,
                    motion: motion, time: context.date.timeIntervalSinceReferenceDate)
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel(accessibilityLabel)
    }

    private var accessibilityLabel: String {
        switch state {
        // "PocketAI" is the product name and is deliberately not localized.
        case .idle, .still: return "PocketAI"
        case .thinking(let tokPerSec):
            if let tokPerSec {
                return String(localized: "PocketAI is generating, \(Int(tokPerSec)) tokens per second", bundle: .module)
            }
            return String(localized: "PocketAI is generating", bundle: .module)
        case .loading: return String(localized: "PocketAI is loading a model", bundle: .module)
        case .live: return String(localized: "PocketAI is live", bundle: .module)
        case .offline: return String(localized: "PocketAI is offline", bundle: .module)
        }
    }
}

enum MarkMotion {
    case thinking, loading, live, breathe
}

/// Draws one frame of the mark. Timings and curves mirror the CSS keyframes
/// in `chip-mark-v2.html`.
enum MarkRenderer {
    private static let easeInOut = UnitCurve.bezier(
        startControlPoint: UnitPoint(x: 0.42, y: 0), endControlPoint: UnitPoint(x: 0.58, y: 1))
    private static let orbit = UnitCurve.bezier(
        startControlPoint: UnitPoint(x: 0.3, y: 0.1), endControlPoint: UnitPoint(x: 0.3, y: 1))

    static func draw(_ gfx: inout GraphicsContext, rect: CGRect, color: Color, motion: MarkMotion?, time t: Double) {
        let s = min(rect.width, rect.height) / PocketMarkGeometry.viewBoxSize
        if motion == .breathe {
            gfx.opacity = 0.55 + 0.45 * keyframe(fraction(t / 3), peak: 0.5)
        }
        drawBody(&gfx, rect: rect, scale: s, color: color, loading: motion == .loading, time: t)
        for index in PocketMarkGeometry.pins.indices {
            drawPin(index, &gfx, rect: rect, scale: s, color: color, motion: motion, time: t)
        }
        gfx.fill(PocketCoreBarShape().path(in: rect), with: .color(color))
        let panel = PocketCorePanelShape().path(in: rect)
        if motion == .live {
            gfx.fill(panel, with: .color(color.opacity(keyframe(fraction(t / 1.6), peak: 0.5))))
        }
        gfx.stroke(panel, with: .color(color), lineWidth: PocketMarkGeometry.panelStrokeWidth * s)
    }

    private static func drawBody(
        _ gfx: inout GraphicsContext, rect: CGRect, scale s: CGFloat, color: Color, loading: Bool, time t: Double
    ) {
        let body = PocketChipBodyShape().path(in: rect)
        let width = PocketMarkGeometry.bodyStrokeWidth * s
        guard loading else {
            gfx.stroke(body, with: .color(color), lineWidth: width)
            return
        }
        gfx.stroke(body, with: .color(color.opacity(0.18)), lineWidth: width)
        // CSS: dashoffset 0 → -224 per 1.4 s; one dash pattern is 224 units.
        let progress = CGFloat(orbit.value(at: fraction(t / 1.4)))
        let dash = PocketMarkGeometry.loadingDash.map { $0 * s }
        let style = StrokeStyle(lineWidth: width, dash: dash, dashPhase: 224 * s * (1 - progress))
        gfx.stroke(body, with: .color(color), style: style)
    }

    private static func drawPin(
        _ index: Int, _ gfx: inout GraphicsContext, rect: CGRect, scale s: CGFloat,
        color: Color, motion: MarkMotion?, time t: Double
    ) {
        var style = StrokeStyle(lineWidth: PocketMarkGeometry.pinStrokeWidth * s, lineCap: .round)
        var shade = color
        if motion == .thinking {
            // .25 → 1 (at 30 %) → .25 over 1.2 s, staggered 0.15 s clockwise.
            shade = color.opacity(0.25 + 0.75 * keyframe(fraction((t - Double(index) * 0.15) / 1.2), peak: 0.3))
        } else if motion == .live {
            // CSS: dashoffset 18 → 0 per 0.8 s, so dashes travel outer → inner.
            style.dash = PocketMarkGeometry.liveDash.map { $0 * s }
            style.dashPhase = 18 * s * CGFloat(1 - fraction(t / 0.8))
        }
        gfx.stroke(PocketPinShape(index: index).path(in: rect), with: .color(shade), style: style)
    }

    /// 0 → 1 at `peak` → 0, ease-in-out on each leg (CSS per-keyframe easing).
    static func keyframe(_ p: Double, peak: Double) -> Double {
        p < peak
            ? easeInOut.value(at: p / peak)
            : 1 - easeInOut.value(at: (p - peak) / (1 - peak))
    }

    static func fraction(_ x: Double) -> Double {
        x - x.rounded(.down)
    }
}

#Preview("States") {
    HStack(spacing: PocketSpacing.s24) {
        PocketMarkView(state: .thinking(tokPerSec: 42), size: 64)
        PocketMarkView(state: .loading, size: 64)
        PocketMarkView(state: .live, size: 64)
        PocketMarkView(state: .idle, size: 64)
        PocketMarkView(state: .still, size: 64)
        PocketMarkView(state: .offline, size: 64)
    }
    .padding()
    .background(PocketColor.bgCanvas)
}
