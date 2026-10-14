import XCTest
@testable import PocketUI

final class ContrastTests: XCTestCase {
    func testEveryDeclaredPairMeetsItsThreshold() {
        for pair in PocketContrastTokens.pairs {
            let ratio = PocketContrast.flattenedRatio(fg: pair.fg, bg: pair.bg)
            XCTAssertGreaterThanOrEqual(
                ratio, pair.minRatio,
                "\(pair.fg.hex)@\(pair.fg.alpha) on \(pair.bg.hex): ratio \(ratio) < \(pair.minRatio)"
            )
        }
    }
}
