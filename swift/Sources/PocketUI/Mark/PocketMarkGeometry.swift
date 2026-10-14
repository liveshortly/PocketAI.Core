import CoreGraphics

/// Pure geometry for the Spark core mark (D13), transcribed from
/// `brand/pocketai-mark.svg`'s 100x100 viewBox: a chip square, 4 corner pins,
/// a 4-pointed sparkle core, and 12 traces (3 trace shapes x 4 rotations)
/// each firing outward from the chip to a pad circle. Kept separate from the
/// SwiftUI `Shape` types so the point math is unit-testable without a view.
public enum PocketMarkGeometry {
    public static let viewBoxSize: CGFloat = 100
    public static let center = CGPoint(x: 50, y: 50)

    public static let chipRect = CGRect(x: 32, y: 32, width: 36, height: 36)
    public static let chipCornerRadius: CGFloat = 6
    public static let chipStrokeWidth: CGFloat = 3.6

    public static let cornerPinRadius: CGFloat = 1.7
    public static let cornerPins: [CGPoint] = [
        CGPoint(x: 37.5, y: 37.5),
        CGPoint(x: 62.5, y: 37.5),
        CGPoint(x: 37.5, y: 62.5),
        CGPoint(x: 62.5, y: 62.5),
    ]

    public static let traceStrokeWidth: CGFloat = 3
    public static let tracePadRadius: CGFloat = 3
    public static let tracePadStrokeWidth: CGFloat = 2.6

    /// The 3 trace polylines at rotation 0, straight from the SVG path data.
    /// `rotate(_:)` reproduces the other 3 copies (90/180/270) exactly as the
    /// SVG's `<g transform="rotate(...)">` groups do.
    public static let traceTemplates: [[CGPoint]] = [
        [CGPoint(x: 50, y: 32), CGPoint(x: 50, y: 12.5)],
        [CGPoint(x: 42, y: 32), CGPoint(x: 42, y: 25), CGPoint(x: 35, y: 18), CGPoint(x: 35, y: 12.5)],
        [CGPoint(x: 58, y: 32), CGPoint(x: 58, y: 25), CGPoint(x: 65, y: 18), CGPoint(x: 65, y: 12.5)],
    ]

    public static let rotations: [Double] = [0, 90, 180, 270]

    /// All 12 traces in firing order (template 0/1/2 at rotation 0, then 90, …).
    public static var allTraces: [[CGPoint]] {
        rotations.flatMap { angle in
            traceTemplates.map { points in points.map { rotate($0, by: angle, around: center) } }
        }
    }

    public static func tracePad(for traceIndex: Int) -> CGPoint {
        let points = allTraces[traceIndex]
        return points[points.count - 1]
    }

    public static func rotate(_ point: CGPoint, by degrees: Double, around pivot: CGPoint) -> CGPoint {
        let radians = degrees * .pi / 180
        let dx = point.x - pivot.x
        let dy = point.y - pivot.y
        let cosA = CGFloat(cos(radians))
        let sinA = CGFloat(sin(radians))
        return CGPoint(
            x: pivot.x + dx * cosA - dy * sinA,
            y: pivot.y + dx * sinA + dy * cosA
        )
    }

    /// The 4-pointed sparkle at the chip's center, as a quad-Bezier path
    /// matching the SVG `M50,40 Q51.2,48.8 60,50 Q51.2,51.2 50,60 Q48.8,51.2
    /// 40,50 Q48.8,48.8 50,40Z`.
    public static let sparklePoints: [(point: CGPoint, control: CGPoint)] = [
        (CGPoint(x: 60, y: 50), CGPoint(x: 51.2, y: 48.8)),
        (CGPoint(x: 50, y: 60), CGPoint(x: 51.2, y: 51.2)),
        (CGPoint(x: 40, y: 50), CGPoint(x: 48.8, y: 51.2)),
        (CGPoint(x: 50, y: 40), CGPoint(x: 48.8, y: 48.8)),
    ]
    public static let sparkleStart = CGPoint(x: 50, y: 40)
}
