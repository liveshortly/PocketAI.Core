import XCTest
@testable import PocketUI

final class UnranOutputTests: XCTestCase {
    func testCodePlusInventedOutputIsFound() {
        let reply = "Here:\n```python\nprint(1+1)\n```\nOutput:\n```\n2\n```"
        XCTAssertEqual(UnranOutput.find(in: reply), .init(language: "python", code: "print(1+1)\n"))
    }

    func testCodeAloneOrNonRunnableLanguageIsNot() {
        XCTAssertNil(UnranOutput.find(in: "```python\nprint(1)\n```\nThat prints one."))
        XCTAssertNil(UnranOutput.find(in: "```rust\nfn main(){}\n```\nOutput: nothing"))
    }
}
