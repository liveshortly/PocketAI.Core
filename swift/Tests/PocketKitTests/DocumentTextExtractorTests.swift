import PDFKit
import XCTest
@testable import PocketKit

/// APL-17: Document extraction unit tests across supported text and document formats.
final class DocumentTextExtractorTests: XCTestCase {
    private var tempDir: URL!

    override func setUpWithError() throws {
        try super.setUpWithError()
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: tempDir)
        try super.tearDownWithError()
    }

    func testExtractPlainTextFormats() throws {
        let extensions = ["txt", "md", "swift", "json", "py", "rs"]
        for ext in extensions {
            let fileURL = tempDir.appendingPathComponent("sample.\(ext)")
            let content = "Hello from \(ext) document extraction test."
            try content.write(to: fileURL, atomically: true, encoding: .utf8)

            let pages = try DocumentTextExtractor.extract(fileURL: fileURL)
            XCTAssertEqual(pages.count, 1)
            XCTAssertEqual(pages[0].page, 1)
            XCTAssertEqual(pages[0].text, content)
        }
    }

    func testExtractPDFMultiPage() throws {
        let pdfURL = tempDir.appendingPathComponent("test.pdf")
        let pdfDoc = PDFDocument()
        let page1 = PDFPage()
        let page2 = PDFPage()
        pdfDoc.insert(page1, at: 0)
        pdfDoc.insert(page2, at: 1)
        XCTAssertTrue(pdfDoc.write(to: pdfURL))

        let pages = try DocumentTextExtractor.extract(fileURL: pdfURL)
        XCTAssertEqual(pages.count, 2)
        XCTAssertEqual(pages[0].page, 1)
        XCTAssertEqual(pages[1].page, 2)
    }

    func testUnsupportedFormatThrows() throws {
        let fileURL = tempDir.appendingPathComponent("test.bin")
        try Data([0x00, 0x01, 0x02, 0x03]).write(to: fileURL)

        XCTAssertThrowsError(try DocumentTextExtractor.extract(fileURL: fileURL)) { error in
            XCTAssertEqual(error as? DocumentExtractionError, DocumentExtractionError.unsupportedFormat)
        }
    }

    func testExceedingMaxCharsThrows() throws {
        let fileURL = tempDir.appendingPathComponent("large.txt")
        // Exceed 2,000,000 characters
        let largeContent = String(repeating: "A", count: DocumentTextExtractor.maxChars + 10)
        try largeContent.write(to: fileURL, atomically: true, encoding: .utf8)

        XCTAssertThrowsError(try DocumentTextExtractor.extract(fileURL: fileURL)) { error in
            XCTAssertEqual(error as? DocumentExtractionError, DocumentExtractionError.tooManyCharacters)
        }
    }
}
