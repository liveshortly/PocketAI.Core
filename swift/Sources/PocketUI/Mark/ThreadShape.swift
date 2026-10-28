import SwiftUI

/// The owner's hand-drawn loading thread, transcribed from
/// `brand/pocketai-thread.svg`'s `d` attribute (viewBox `170 160 1660 300`)
/// and rescaled to fit whatever rect it's drawn in.
public struct ThreadShape: Shape {
    private static let viewBox = CGRect(x: 170, y: 160, width: 1660, height: 300)

    public init() {}

    public func path(in rect: CGRect) -> Path {
        let sx = rect.width / Self.viewBox.width
        let sy = rect.height / Self.viewBox.height
        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
            CGPoint(x: rect.minX + (x - Self.viewBox.minX) * sx, y: rect.minY + (y - Self.viewBox.minY) * sy)
        }

        var path = Path()
        path.move(to: p(188, 342))
        path.addCurve(to: p(380, 233), control1: p(230, 275), control2: p(300, 238))
        path.addCurve(to: p(575, 295), control1: p(470, 228), control2: p(540, 255))
        path.addCurve(to: p(620, 412), control1: p(610, 330), control2: p(640, 385))
        path.addCurve(to: p(540, 410), control1: p(600, 440), control2: p(555, 445))
        path.addCurve(to: p(575, 297), control1: p(528, 380), control2: p(540, 330))
        path.addCurve(to: p(700, 264), control1: p(605, 268), control2: p(650, 265))
        path.addCurve(to: p(980, 272), control1: p(800, 262), control2: p(900, 272))
        path.addCurve(to: p(1165, 250), control1: p(1060, 272), control2: p(1130, 262))
        path.addCurve(to: p(1205, 190), control1: p(1205, 236), control2: p(1222, 210))
        path.addCurve(to: p(1153, 215), control1: p(1190, 178), control2: p(1160, 182))
        path.addCurve(to: p(1205, 375), control1: p(1146, 260), control2: p(1160, 330))
        path.addCurve(to: p(1430, 395), control1: p(1240, 405), control2: p(1330, 408))
        path.addCurve(to: p(1810, 225), control1: p(1530, 380), control2: p(1650, 280))
        return path
    }
}

/// `02` §4.1's loading beam: a faint static track with a comet made of a
/// bright head, a mid segment, and a fading tail sweeping the thread once
/// every 2.4s, ease-in-out, looping.
public struct ThreadBeamView: View {
    private let size: CGSize

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(size: CGSize) {
        self.size = size
    }

    public var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60.0, paused: reduceMotion)) { context in
            let cycle = 2.4
            let t = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: cycle) / cycle
            let eased = easeInOut(reduceMotion ? 0.5 : t)

            Canvas { gfx, canvasSize in
                let rect = CGRect(origin: .zero, size: canvasSize)
                let thread = ThreadShape().path(in: rect)

                gfx.stroke(thread, with: .color(PocketColor.accent.opacity(0.1)), style: StrokeStyle(lineWidth: 3, lineCap: .round))

                for segment in Self.segments {
                    let start = max(0, eased - segment.length)
                    gfx.stroke(
                        thread.trimmedPath(from: start, to: eased),
                        with: .color(PocketColor.accent.opacity(segment.opacity)),
                        style: StrokeStyle(lineWidth: segment.width, lineCap: .round)
                    )
                }
            }
        }
        .frame(width: size.width, height: size.height)
        .accessibilityLabel(Text("Loading", bundle: .module))
    }

    private struct Segment {
        let length: Double
        let opacity: Double
        let width: CGFloat
    }

    private static let segments: [Segment] = [
        Segment(length: 0.22, opacity: 0.18, width: 3),
        Segment(length: 0.12, opacity: 0.45, width: 3.6),
        Segment(length: 0.05, opacity: 1.0, width: 4.2),
    ]

    private func easeInOut(_ t: Double) -> CGFloat {
        CGFloat(t < 0.5 ? 4 * t * t * t : 1 - pow(-2 * t + 2, 3) / 2)
    }
}

#Preview("Thread beam") {
    ThreadBeamView(size: CGSize(width: 240, height: 48))
        .padding()
        .background(PocketColor.bgCanvas)
}
