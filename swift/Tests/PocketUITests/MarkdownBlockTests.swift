import XCTest
import SwiftUI
@testable import PocketUI

/// `AttributedString(markdown:)` drops every newline that produced a block
/// boundary, so an assistant answer used to render as one glued run of words.
/// These assert the blocks come back out.
final class MarkdownBlockTests: XCTestCase {
    private func plain(_ blocks: [MessageView.ProseBlock]) -> [String] {
        blocks.map { String($0.text.characters) }
    }

    func testParagraphsStaySeparate() {
        let blocks = MessageView.blocks("One sentence.\n\n**Why it fails:**\nBecause of this.")
        XCTAssertEqual(plain(blocks), ["One sentence.", "Why it fails: Because of this."])
        XCTAssertNil(blocks[0].marker)
    }

    func testUnorderedListItemsGetBulletsAndSeparate() {
        let blocks = MessageView.blocks("- alpha\n- beta")
        XCTAssertEqual(plain(blocks), ["alpha", "beta"])
        XCTAssertEqual(blocks.map(\.marker), ["•", "•"])
    }

    func testOrderedListKeepsItsNumbering() {
        let blocks = MessageView.blocks("1. first\n2. second")
        XCTAssertEqual(plain(blocks), ["first", "second"])
        XCTAssertEqual(blocks.map(\.marker), ["1.", "2."])
    }

    func testNestedListIndents() {
        let blocks = MessageView.blocks("- outer\n    - inner")
        XCTAssertEqual(plain(blocks), ["outer", "inner"])
        XCTAssertEqual(blocks[0].indent, 0)
        XCTAssertGreaterThan(blocks[1].indent, 0)
    }

    func testHeadingIsFlagged() {
        let blocks = MessageView.blocks("## Setup\n\nThen do this.")
        XCTAssertEqual(plain(blocks), ["Setup", "Then do this."])
        XCTAssertTrue(blocks[0].isHeading)
        XCTAssertFalse(blocks[1].isHeading)
    }

    func testBlankInputProducesNoBlocks() {
        XCTAssertTrue(MessageView.blocks("   \n\n  ").isEmpty)
    }
}
