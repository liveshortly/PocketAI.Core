import SwiftUI

private func scaledPoint(_ p: CGPoint, in rect: CGRect) -> CGPoint {
    let scale = min(rect.width, rect.height) / PocketMarkGeometry.viewBoxSize
    return CGPoint(x: rect.minX + p.x * scale, y: rect.minY + p.y * scale)
}

/// The chip square + 4 corner pins.
public struct PocketChipShape: Shape {
    public init() {}

    public func path(in rect: CGRect) -> Path {
        let scale = min(rect.width, rect.height) / PocketMarkGeometry.viewBoxSize
        var path = Path()
        let chip = PocketMarkGeometry.chipRect
        let scaledRect = CGRect(
            x: rect.minX + chip.minX * scale,
            y: rect.minY + chip.minY * scale,
            width: chip.width * scale,
            height: chip.height * scale
        )
        path.addRoundedRect(in: scaledRect, cornerSize: CGSize(width: PocketMarkGeometry.chipCornerRadius * scale, height: PocketMarkGeometry.chipCornerRadius * scale), style: .continuous)
        for pin in PocketMarkGeometry.cornerPins {
            let center = scaledPoint(pin, in: rect)
            path.addEllipse(in: CGRect(
                x: center.x - PocketMarkGeometry.cornerPinRadius * scale,
                y: center.y - PocketMarkGeometry.cornerPinRadius * scale,
                width: PocketMarkGeometry.cornerPinRadius * 2 * scale,
                height: PocketMarkGeometry.cornerPinRadius * 2 * scale
            ))
        }
        return path
    }
}

/// The 4-pointed sparkle at the chip's center.
public struct PocketSparkleShape: Shape {
    public init() {}

    public func path(in rect: CGRect) -> Path {
        var path = Path()
        let start = scaledPoint(PocketMarkGeometry.sparkleStart, in: rect)
        path.move(to: start)
        for (point, control) in PocketMarkGeometry.sparklePoints {
            path.addQuadCurve(to: scaledPoint(point, in: rect), control: scaledPoint(control, in: rect))
        }
        path.closeSubpath()
        return path
    }
}

/// One of the 12 traces, as its own `Shape` so `ThreadBeamView`-style views
/// can `.trim(from:to:)` it independently for the firing animation.
public struct PocketTraceShape: Shape {
    public let index: Int

    public init(index: Int) {
        self.index = index
    }

    public func path(in rect: CGRect) -> Path {
        var path = Path()
        let points = PocketMarkGeometry.allTraces[index].map { scaledPoint($0, in: rect) }
        guard let first = points.first else { return path }
        path.move(to: first)
        for point in points.dropFirst() {
            path.addLine(to: point)
        }
        return path
    }
}

/// The small open circle at the end of a trace ("signal" lands here).
public struct PocketTracePadShape: Shape {
    public let index: Int

    public init(index: Int) {
        self.index = index
    }

    public func path(in rect: CGRect) -> Path {
        let scale = min(rect.width, rect.height) / PocketMarkGeometry.viewBoxSize
        let center = scaledPoint(PocketMarkGeometry.tracePad(for: index), in: rect)
        let r = PocketMarkGeometry.tracePadRadius * scale
        return Path(ellipseIn: CGRect(x: center.x - r, y: center.y - r, width: r * 2, height: r * 2))
    }
}
