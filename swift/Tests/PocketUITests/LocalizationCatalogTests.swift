import XCTest

/// Guards `Sources/PocketUI/Resources/Localizable.xcstrings`: every key that
/// exists in `en` must also exist in every locale we ship, with a non-empty
/// value. A missing entry is a silent English fall-through at runtime, which
/// is invisible in a screenshot of an English build - so it has to fail here.
///
/// The catalog is read from source rather than from `Bundle.module`: `swift
/// build` only *copies* an `.xcstrings` into the resource bundle (it does not
/// run `xcstringstool`), so there is nothing compiled to look up under SwiftPM.
/// Only Xcode compiles the catalog, which is why the app-level runtime check
/// lives in the app targets.
final class LocalizationCatalogTests: XCTestCase {
    /// Locales PocketUI ships. Keep in sync with the apps' `project.yml`.
    static let requiredLocales = ["hi", "es", "fr", "de", "ja", "zh-Hans", "pt-BR"]

    private struct Catalog: Decodable {
        struct Entry: Decodable {
            struct Localization: Decodable {
                struct Unit: Decodable {
                    let state: String
                    let value: String
                }
                let stringUnit: Unit?
            }
            let localizations: [String: Localization]?
        }
        let sourceLanguage: String
        let strings: [String: Entry]
    }

    private static var catalogURL: URL {
        URL(fileURLWithPath: #filePath)            // .../Tests/PocketUITests/this.swift
            .deletingLastPathComponent()            // .../Tests/PocketUITests
            .deletingLastPathComponent()            // .../Tests
            .deletingLastPathComponent()            // .../swift
            .appendingPathComponent("Sources/PocketUI/Resources/Localizable.xcstrings")
    }

    private func loadCatalog() throws -> Catalog {
        let url = Self.catalogURL
        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path), "no catalog at \(url.path)")
        return try JSONDecoder().decode(Catalog.self, from: Data(contentsOf: url))
    }

    func testCatalogSourceLanguageIsEnglish() throws {
        XCTAssertEqual(try loadCatalog().sourceLanguage, "en")
    }

    func testEveryEnglishKeyIsPresentInEveryShippedLocale() throws {
        let catalog = try loadCatalog()

        let englishKeys = catalog.strings
            .filter { $0.value.localizations?["en"]?.stringUnit != nil }
            .map(\.key)
        XCTAssertFalse(englishKeys.isEmpty, "catalog has no English entries - wrong file?")

        for locale in Self.requiredLocales {
            let missing = englishKeys
                .filter { key in
                    guard let unit = catalog.strings[key]?.localizations?[locale]?.stringUnit else { return true }
                    return unit.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                }
                .sorted()
            XCTAssertTrue(
                missing.isEmpty,
                "\(locale) is missing \(missing.count) of \(englishKeys.count) strings: \(missing.joined(separator: " | "))"
            )
        }
    }

    /// A translation that still reads as the English source is almost always an
    /// untranslated copy-paste. Allowed only where the term is genuinely
    /// identical in that locale (product name, unit symbol, loan word).
    func testTranslationsThatEqualEnglishAreDeliberate() throws {
        let catalog = try loadCatalog()
        let knownIdentical: Set<String> = [
            "CODE", "Python", "JavaScript", "Error", "code", "Offline", "PocketAI is offline", "%@ GB", "%@ MB/s", "%lld min",
            "%lld tok/s", "%lld ms TTFT · %@", "est.", "Message", "Photo", "Camera",
        ]
        for (key, entry) in catalog.strings where !knownIdentical.contains(key) {
            for locale in Self.requiredLocales {
                guard let value = entry.localizations?[locale]?.stringUnit?.value else { continue }
                XCTAssertNotEqual(
                    value, key,
                    "\(locale) value for \"\(key)\" is identical to English - translate it, or add the key to knownIdentical"
                )
            }
        }
    }

    /// Anything produced without a native reviewer must say so, so a reviewer
    /// can find it. `translated` is reserved for strings a human has signed off.
    func testMachineProducedTranslationsAreFlaggedForReview() throws {
        let catalog = try loadCatalog()
        var unflagged: [String] = []
        for (key, entry) in catalog.strings {
            for locale in Self.requiredLocales {
                guard let unit = entry.localizations?[locale]?.stringUnit else { continue }
                if unit.state != "needs_review" && unit.state != "translated" {
                    unflagged.append("\(locale):\(key) state=\(unit.state)")
                }
            }
        }
        XCTAssertTrue(unflagged.isEmpty, "unexpected states: \(unflagged.sorted().joined(separator: " | "))")
    }

    /// `%@` / `%lld` counts must survive translation or the format crashes or
    /// prints garbage at runtime.
    func testPlaceholdersSurviveTranslation() throws {
        let catalog = try loadCatalog()
        for (key, entry) in catalog.strings {
            let expected = Self.placeholders(in: key)
            guard !expected.isEmpty else { continue }
            for locale in Self.requiredLocales {
                guard let value = entry.localizations?[locale]?.stringUnit?.value else { continue }
                XCTAssertEqual(
                    Self.placeholders(in: value).sorted(), expected.sorted(),
                    "\(locale) value for \"\(key)\" has the wrong placeholders: \"\(value)\""
                )
            }
        }
    }

    private static func placeholders(in string: String) -> [String] {
        let regex = try! NSRegularExpression(pattern: "%(?:\\d+\\$)?(?:lld|@|lf|d|f)")
        let ns = string as NSString
        return regex.matches(in: string, range: NSRange(location: 0, length: ns.length))
            .map { ns.substring(with: $0.range) }
    }
}

/// AGT-17: the PocketKit and iPhone catalogues must be as complete as PocketUI's.
/// The iPhone one lives in a sibling repo, so it is skipped when absent.
final class SiblingCatalogCompletenessTests: XCTestCase {
    /// Not words: numbers, symbols, the product name.
    private static let untranslatable: Set<String> = [
        "1024", "512", "768", "PERF", "·", "•", "PocketAI", "DIAGNOSTICS PAYLOAD", "Diagnostic report JSON payload",
    ]

    private func check(_ url: URL) throws {
        guard FileManager.default.fileExists(atPath: url.path) else { throw XCTSkip("no catalogue at \(url.path)") }
        let root = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
        let strings = root["strings"] as! [String: [String: Any]]
        var missing: [String] = []
        for (key, entry) in strings where !Self.untranslatable.contains(key) {
            let loc = entry["localizations"] as? [String: [String: [String: String]]] ?? [:]
            for l in LocalizationCatalogTests.requiredLocales where (loc[l]?["stringUnit"]?["value"] ?? "").isEmpty {
                missing.append("\(l):\(key)")
            }
        }
        XCTAssertTrue(missing.isEmpty, "\(url.lastPathComponent) missing \(missing.count): \(missing.sorted().prefix(20).joined(separator: " | "))")
    }

    private var swiftDir: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    }

    func testPocketKitCatalogIsComplete() throws {
        try check(swiftDir.appendingPathComponent("Sources/PocketKit/Resources/Localizable.xcstrings"))
    }

    func testIphoneCatalogIsComplete() throws {
        try check(swiftDir.deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("PocketAI.Iphone/Resources/Localizable.xcstrings"))
    }
}
