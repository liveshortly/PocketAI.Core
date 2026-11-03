import SwiftUI

/// Maps viewBox units into `rect` (square-fit, top-left anchored like the SVG).
private struct MarkSpace {
    let rect: CGRect
    var scale: CGFloat { min(rect.width, rect.height) / PocketMarkGeometry.viewBoxSize }
    func point(_ p: CGPoint) -> CGPoint { CGPoint(x: rect.minX + p.x * scale, y: rect.minY + p.y * scale) }
    func rect(_ r: CGRect) -> CGRect {
        CGRect(origin: point(r.origin), size: CGSize(width: r.width * scale, height: r.height * scale))
    }
}

/// The chip body. Built by hand (not `addRoundedRect`) so the path starts at
/// the top edge and runs clockwise like an SVG `<rect>` — the Loading dash
/// orbit depends on that start point and direction.
public struct PocketChipBodyShape: Shape {
    public init() {}

    public func path(in rect: CGRect) -> Path {
        let space = MarkSpace(rect: rect)
        let b = space.rect(PocketMarkGeometry.bodyRect)
        let r = PocketMarkGeometry.bodyCornerRadius * space.scale
        var path = Path()
        path.move(to: CGPoint(x: b.minX + r, y: b.minY))
        path.addArc(tangent1End: CGPoint(x: b.maxX, y: b.minY), tangent2End: CGPoint(x: b.maxX, y: b.maxY), radius: r)
        path.addArc(tangent1End: CGPoint(x: b.maxX, y: b.maxY), tangent2End: CGPoint(x: b.minX, y: b.maxY), radius: r)
        path.addArc(tangent1End: CGPoint(x: b.minX, y: b.maxY), tangent2End: CGPoint(x: b.minX, y: b.minY), radius: r)
        path.addArc(tangent1End: CGPoint(x: b.minX, y: b.minY), tangent2End: CGPoint(x: b.maxX, y: b.minY), radius: r)
        path.closeSubpath()
        return path
    }
}

/// One pin, drawn outer → inner so dashes flow inward.
public struct PocketPinShape: Shape {
    public let index: Int

    public init(index: Int) {
        self.index = index
    }

    public func path(in rect: CGRect) -> Path {
        let space = MarkSpace(rect: rect)
        let pin = PocketMarkGeometry.pins[index]
        var path = Path()
        path.move(to: space.point(pin.outer))
        path.addLine(to: space.point(pin.inner))
        return path
    }
}

/// The core's filled bar.
public struct PocketCoreBarShape: Shape {
    public init() {}

    public func path(in rect: CGRect) -> Path {
        let space = MarkSpace(rect: rect)
        return Path(roundedRect: space.rect(PocketMarkGeometry.barRect),
                    cornerRadius: PocketMarkGeometry.coreCornerRadius * space.scale, style: .circular)
    }
}

/// The core's outlined panel (stroke it; fill it for Live).
public struct PocketCorePanelShape: Shape {
    public init() {}

    public func path(in rect: CGRect) -> Path {
        let space = MarkSpace(rect: rect)
        return Path(roundedRect: space.rect(PocketMarkGeometry.panelRect),
                    cornerRadius: PocketMarkGeometry.coreCornerRadius * space.scale, style: .circular)
    }
}
