import CoreGraphics

/// Pure geometry for the chip mark (UI-34), from `strategy/brand/chip-mark-v2.html`
/// on a 100x100 viewBox: a rounded chip body, 8 pins (2 per side) and the
/// LiveShortly glyph as its core. Kept separate from the SwiftUI `Shape`s so
/// the numbers are unit-testable without a view.
public enum PocketMarkGeometry {
    public static let viewBoxSize: CGFloat = 100
    public static let center = CGPoint(x: 50, y: 50)

    public static let bodyRect = CGRect(x: 22, y: 22, width: 56, height: 56)
    public static let bodyCornerRadius: CGFloat = 12
    public static let bodyStrokeWidth: CGFloat = 7

    public static let pinStrokeWidth: CGFloat = 6

    /// Top pins, outer end first so the Live beam flows inward.
    public static let pinTemplates: [(outer: CGPoint, inner: CGPoint)] = [
        (CGPoint(x: 40, y: 8), CGPoint(x: 40, y: 20)),
        (CGPoint(x: 60, y: 8), CGPoint(x: 60, y: 20)),
    ]

    /// All 8 pins clockwise from top-left (the Thinking stagger order):
    /// the top pair rotated 0/90/180/270 degrees around the center.
    public static let pins: [(outer: CGPoint, inner: CGPoint)] = [0.0, 90, 180, 270].flatMap { angle in
        pinTemplates.map { (rotate($0.outer, by: angle, around: center), rotate($0.inner, by: angle, around: center)) }
    }

    /// Core: the LiveShortly glyph — a filled bar and an outlined panel.
    public static let barRect = CGRect(x: 35, y: 38, width: 10.5, height: 24)
    public static let panelRect = CGRect(x: 51.2, y: 39.7, width: 13.6, height: 20.6)
    public static let coreCornerRadius: CGFloat = 2.4
    public static let panelStrokeWidth: CGFloat = 3.4

    /// Loading: a 56/168 dash orbiting the body (one pattern = 224 units).
    public static let loadingDash: [CGFloat] = [56, 168]
    /// Live: 4/14 dashes flowing down each pin (one pattern = 18 units).
    public static let liveDash: [CGFloat] = [4, 14]

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
}
