import Foundation
import PDFKit

public struct ExtractedPage: Equatable {
    public let page: Int
    public let text: String
}

public enum DocumentExtractionError: Error, Equatable {
    case fileTooLarge
    case tooManyCharacters
    case unsupportedFormat
    case readFailed
}

/// APL-17: PDF (PDFKit, page-separated, page numbers kept), plain
/// text/Markdown/code (UTF-8 with encoding detection), DOCX (macOS only,
/// via `NSAttributedString` document reading - `NSAttributedString`'s
/// Office Open XML reader isn't available on iOS, so that path throws
/// `.unsupportedFormat` there rather than silently returning nothing).
public enum DocumentTextExtractor {
    public static let maxFileBytes = 20 * 1024 * 1024
    public static let maxChars = 2_000_000

    private static let plainTextExtensions: Set<String> = [
        "txt", "md", "markdown", "swift", "py", "js", "ts", "rs", "json",
        "yaml", "yml", "c", "cpp", "h", "hpp", "java", "go", "rb", "sh", "toml",
    ]

    public static func extract(fileURL: URL) throws -> [ExtractedPage] {
        let attributes = try FileManager.default.attributesOfItem(atPath: fileURL.path)
        let size = (attributes[.size] as? Int) ?? 0
        guard size <= maxFileBytes else { throw DocumentExtractionError.fileTooLarge }

        let ext = fileURL.pathExtension.lowercased()
        let pages: [ExtractedPage]
        if ext == "pdf" {
            pages = try extractPDF(fileURL)
        } else if ext == "docx" {
            pages = try extractDocx(fileURL)
        } else if plainTextExtensions.contains(ext) {
            pages = [ExtractedPage(page: 1, text: try decodeText(fileURL))]
        } else {
            throw DocumentExtractionError.unsupportedFormat
        }

        let totalChars = pages.reduce(0) { $0 + $1.text.count }
        guard totalChars <= maxChars else { throw DocumentExtractionError.tooManyCharacters }
        return pages
    }

    private static func extractPDF(_ url: URL) throws -> [ExtractedPage] {
        guard let document = PDFDocument(url: url) else { throw DocumentExtractionError.readFailed }
        var pages: [ExtractedPage] = []
        for i in 0..<document.pageCount {
            guard let page = document.page(at: i) else { continue }
            pages.append(ExtractedPage(page: i + 1, text: page.string ?? ""))
        }
        return pages
    }

    private static func decodeText(_ url: URL) throws -> String {
        let data = try Data(contentsOf: url)
        if let utf8 = String(data: data, encoding: .utf8) {
            return utf8
        }
        var convertedString: NSString?
        let encoding = NSString.stringEncoding(
            for: data,
            encodingOptions: nil,
            convertedString: &convertedString,
            usedLossyConversion: nil
        )
        guard encoding != 0, let convertedString else {
            throw DocumentExtractionError.readFailed
        }
        return convertedString as String
    }

    private static func extractDocx(_ url: URL) throws -> [ExtractedPage] {
        #if os(macOS)
        let options: [NSAttributedString.DocumentReadingOptionKey: Any] = [
            .documentType: NSAttributedString.DocumentType.officeOpenXML
        ]
        guard let attributed = try? NSAttributedString(url: url, options: options, documentAttributes: nil) else {
            throw DocumentExtractionError.readFailed
        }
        return [ExtractedPage(page: 1, text: attributed.string)]
        #else
        throw DocumentExtractionError.unsupportedFormat
        #endif
    }
}
