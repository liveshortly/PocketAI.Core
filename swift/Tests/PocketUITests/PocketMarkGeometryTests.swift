import XCTest
@testable import PocketUI

final class PocketMarkGeometryTests: XCTestCase {
    /// The 8 pins of `chip-mark-v2.html`, clockwise, outer → inner.
    private let expectedPins: [[CGFloat]] = [
        [40, 8, 40, 20], [60, 8, 60, 20], [92, 40, 80, 40], [92, 60, 80, 60],
        [60, 92, 60, 80], [40, 92, 40, 80], [8, 60, 20, 60], [8, 40, 20, 40],
    ]

    func testPinsMatchTheSpecClockwiseOuterToInner() {
        let pins = PocketMarkGeometry.pins
        XCTAssertEqual(pins.count, 8)
        for (pin, e) in zip(pins, expectedPins) {
            XCTAssertEqual(pin.outer.x, e[0], accuracy: 0.001)
            XCTAssertEqual(pin.outer.y, e[1], accuracy: 0.001)
            XCTAssertEqual(pin.inner.x, e[2], accuracy: 0.001)
            XCTAssertEqual(pin.inner.y, e[3], accuracy: 0.001)
        }
    }

    func testPinsTouchTheBodyStroke() {
        // Inner pin ends sit 2 units outside the body edge, inside its 7-wide stroke.
        let body = PocketMarkGeometry.bodyRect
        for pin in PocketMarkGeometry.pins {
            let gap = max(body.minX - pin.inner.x, pin.inner.x - body.maxX,
                          body.minY - pin.inner.y, pin.inner.y - body.maxY)
            XCTAssertEqual(gap, 2, accuracy: 0.001)
        }
    }

    func testBodyAndCoreNumbers() {
        XCTAssertEqual(PocketMarkGeometry.bodyRect, CGRect(x: 22, y: 22, width: 56, height: 56))
        XCTAssertEqual(PocketMarkGeometry.bodyCornerRadius, 12)
        XCTAssertEqual(PocketMarkGeometry.barRect, CGRect(x: 35, y: 38, width: 10.5, height: 24))
        XCTAssertEqual(PocketMarkGeometry.panelRect, CGRect(x: 51.2, y: 39.7, width: 13.6, height: 20.6))
    }

    func testFullRotationIsIdentity() {
        let original = CGPoint(x: 42, y: 18)
        let rotated = PocketMarkGeometry.rotate(original, by: 360, around: PocketMarkGeometry.center)
        XCTAssertEqual(rotated.x, original.x, accuracy: 0.001)
        XCTAssertEqual(rotated.y, original.y, accuracy: 0.001)
    }

    func testKeyframeCurveHitsCSSStops() {
        XCTAssertEqual(MarkRenderer.keyframe(0, peak: 0.3), 0, accuracy: 0.001)
        XCTAssertEqual(MarkRenderer.keyframe(0.3, peak: 0.3), 1, accuracy: 0.001)
        XCTAssertEqual(MarkRenderer.keyframe(0.999_999, peak: 0.3), 0, accuracy: 0.001)
        XCTAssertEqual(MarkRenderer.fraction(-0.25), 0.75, accuracy: 0.001)
    }

    func testBodyPathStartsAtTopEdgeLikeSVG() {
        let path = PocketChipBodyShape().path(in: CGRect(x: 0, y: 0, width: 100, height: 100))
        var first: CGPoint?
        path.forEach { el in
            if first == nil, case .move(let p) = el { first = p }
        }
        XCTAssertEqual(first, CGPoint(x: 34, y: 22))
    }
}
