import XCTest
import SwiftUI
@testable import PocketUI

final class AppearancePreferenceTests: XCTestCase {
    func testStoredValuesMapToSchemes() {
        XCTAssertEqual(AppearancePreference.colorScheme(forStored: "light"), .light)
        XCTAssertEqual(AppearancePreference.colorScheme(forStored: "dark"), .dark)
        XCTAssertNil(AppearancePreference.colorScheme(forStored: "system"))
        XCTAssertNil(AppearancePreference.colorScheme(forStored: ""))
        XCTAssertNil(AppearancePreference.colorScheme(forStored: "garbage"))
    }
}
