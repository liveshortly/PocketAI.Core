import XCTest
@testable import PocketUI

final class FirstChatSuggestionsTests: XCTestCase {
    /// A chip sends its prompt immediately, so it must be a full sentence that
    /// doesn't depend on content the user hasn't pasted.
    func testPromptsAreCompleteAndDistinct() {
        let all = FirstChatSuggestion.all
        XCTAssertEqual(Set(all.map(\.title)).count, all.count)
        for s in all {
            XCTAssertGreaterThan(s.prompt.split(separator: " ").count, 6, s.title)
            XCTAssertNotEqual(s.prompt, s.title)
            XCTAssertTrue(s.prompt.hasSuffix(".") || s.prompt.hasSuffix("?"), s.title)
        }
    }
}
