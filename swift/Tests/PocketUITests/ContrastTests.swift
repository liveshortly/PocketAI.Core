import Foundation
import XCTest
@testable import PocketUI

/// Contrast, measured against the colors the app actually ships.
///
/// This used to assert a hand-maintained table of hex values in
/// `PocketContrastTokens`, documented as "mirroring Resources/Colors.xcassets".
/// It stopped mirroring them when the green palette landed: the table still
/// claimed canvas `F7F6F3` (shipped `F3F1E9`), surface `FFFFFF` (shipped
/// `FAF9F4`), primary `15161A` (shipped `1A1916`), and tertiary as primary at
/// 66% alpha when the shipped token is an opaque warm grey. The suite stayed
/// green throughout, which is worse than no suite — it reported that a palette
/// nobody renders meets AA.
///
/// So this reads the colorsets themselves. Resolving them through `NSColor`
/// would be neater, but under SwiftPM the catalog is not compiled and every
/// token comes back as the same fallback, making every ratio exactly 1.0 —
/// a test that fails for a reason unrelated to the palette is no better than
/// one that passes for one.
final class ContrastTests: XCTestCase {
    /// WCAG AA for body text. Tertiary text is still text — it carries row
    /// subtitles and captions throughout the HUD — so it is held to the same
    /// bar, not the 3:1 floor for incidental content.
    private let bodyMinimum = 4.5
    private let strongMinimum = 7.0

    private static let surfaces = ["bg.canvas", "bg.surface", "bg.raised", "bg.sunken"]

    func testTextOnEverySurfaceMeetsAA() throws {
        for appearance in Appearance.allCases {
            for surface in Self.surfaces {
                try assertRatio("text.primary", on: surface, atLeast: strongMinimum, in: appearance)
                try assertRatio("text.secondary", on: surface, atLeast: bodyMinimum, in: appearance)
                try assertRatio("text.tertiary", on: surface, atLeast: bodyMinimum, in: appearance)
            }
        }
    }

    func testAccentReadsAsTextWhereItIsUsedAsText() throws {
        // Accent is the colour of links and actionable words, not only of fills.
        for appearance in Appearance.allCases {
            for surface in ["bg.canvas", "bg.surface", "bg.raised"] {
                try assertRatio("accent", on: surface, atLeast: bodyMinimum, in: appearance)
            }
            try assertRatio("accent.onAccent", on: "accent", atLeast: bodyMinimum, in: appearance)
        }
    }

    /// Guards the specific drift that went unnoticed: a token file that no
    /// longer matches what the suite believes it contains.
    func testEveryTokenUnderTestDeclaresBothAppearances() throws {
        for name in Self.surfaces + ["text.primary", "text.secondary", "text.tertiary", "accent"] {
            for appearance in Appearance.allCases {
                XCTAssertNoThrow(
                    try Self.components(of: name, in: appearance),
                    "\(name) has no \(appearance) variant")
            }
        }
    }

    // MARK: - Reading the catalog

    enum Appearance: String, CaseIterable { case light, dark }

    private func assertRatio(
        _ foreground: String, on background: String, atLeast minimum: Double,
        in appearance: Appearance, file: StaticString = #filePath, line: UInt = #line
    ) throws {
        let fg = try Self.components(of: foreground, in: appearance)
        let bg = try Self.components(of: background, in: appearance)
        let ratio = PocketContrast.ratio(fg, bg)
        XCTAssertGreaterThanOrEqual(
            ratio, minimum,
            "\(foreground) on \(background) in \(appearance.rawValue): "
                + "\(String(format: "%.2f", ratio)):1 is below \(minimum):1",
            file: file, line: line)
    }

    private static var catalogURL: URL {
        // Tests/PocketUITests/ -> Sources/PocketUI/Resources/Colors.xcassets
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/PocketUI/Resources/Colors.xcassets")
    }

    static func components(
        of name: String, in appearance: Appearance
    ) throws -> (r: Double, g: Double, b: Double) {
        let url = catalogURL
            .appendingPathComponent("\(name).colorset")
            .appendingPathComponent("Contents.json")
        let data = try Data(contentsOf: url)
        let json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any])
        let colors = try XCTUnwrap(json["colors"] as? [[String: Any]])

        for entry in colors {
            guard let color = entry["color"] as? [String: Any],
                  let components = color["components"] as? [String: Any]
            else { continue }
            let appearances = entry["appearances"] as? [[String: Any]] ?? []
            let isDark = appearances.contains {
                ($0["value"] as? String)?.contains("dark") == true
            }
            guard isDark == (appearance == .dark) else { continue }
            return (try channel(components["red"]),
                    try channel(components["green"]),
                    try channel(components["blue"]))
        }
        throw Failure.missingAppearance(name, appearance)
    }

    /// Asset catalogs write channels as "0.122", "122" or "0x7A" depending on
    /// who last edited the file.
    private static func channel(_ raw: Any?) throws -> Double {
        let text = try XCTUnwrap(raw as? String).trimmingCharacters(in: .whitespaces)
        if text.hasPrefix("0x") || text.hasPrefix("0X") {
            return Double(UInt8(text.dropFirst(2), radix: 16) ?? 0) / 255.0
        }
        let value = try XCTUnwrap(Double(text))
        return text.contains(".") ? value : value / 255.0
    }

    enum Failure: Error { case missingAppearance(String, Appearance) }
}
