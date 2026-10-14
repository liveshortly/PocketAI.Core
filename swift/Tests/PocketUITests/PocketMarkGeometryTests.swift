import XCTest
@testable import PocketUI

final class PocketMarkGeometryTests: XCTestCase {
    func testTwelveTracesFromThreeTemplatesAndFourRotations() {
        XCTAssertEqual(PocketMarkGeometry.allTraces.count, 12)
    }

    func test90DegreeRotationMapsCenterTraceOntoItself() {
        // Template 0 (the straight-up trace) passes through (50, 32)-(50, 12.5).
        // Rotating the whole mark 90 degrees around the (50,50) center should
        // land that same template on the horizontal axis.
        let rotated = PocketMarkGeometry.rotate(CGPoint(x: 50, y: 12.5), by: 90, around: PocketMarkGeometry.center)
        XCTAssertEqual(rotated.x, 87.5, accuracy: 0.001)
        XCTAssertEqual(rotated.y, 50, accuracy: 0.001)
    }

    func testFullRotationIsIdentity() {
        let original = CGPoint(x: 42, y: 18)
        let rotated = PocketMarkGeometry.rotate(original, by: 360, around: PocketMarkGeometry.center)
        XCTAssertEqual(rotated.x, original.x, accuracy: 0.001)
        XCTAssertEqual(rotated.y, original.y, accuracy: 0.001)
    }

    func testTracePadIsTheLastPointOfEachTrace() {
        for i in 0..<PocketMarkGeometry.allTraces.count {
            let pad = PocketMarkGeometry.tracePad(for: i)
            XCTAssertEqual(pad, PocketMarkGeometry.allTraces[i].last)
        }
    }
}
