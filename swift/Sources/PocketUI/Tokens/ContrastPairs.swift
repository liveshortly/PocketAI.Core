import Foundation

/// Raw sRGB + alpha values mirroring `Resources/Colors.xcassets`, kept here as
/// the single source of truth for the automated contrast test (UI-01). Alpha
/// is flattened against its usual opaque surface before computing contrast.
public struct PocketRawColor: Sendable {
    public let hex: String
    public let alpha: Double

    public init(_ hex: String, alpha: Double = 1.0) {
        self.hex = hex
        self.alpha = alpha
    }
}

public enum PocketContrastTokens {
    public static let lightBgCanvas = PocketRawColor("F7F6F3")
    public static let lightBgSurface = PocketRawColor("FFFFFF")
    public static let lightTextPrimary = PocketRawColor("15161A")
    public static let lightTextSecondary = PocketRawColor("15161A", alpha: 0.62)

    public static let darkBgCanvas = PocketRawColor("0E0F12")
    public static let darkBgSurface = PocketRawColor("16181C")
    public static let darkTextPrimary = PocketRawColor("F2F1EE")
    public static let darkTextSecondary = PocketRawColor("F2F1EE", alpha: 0.64)

    /// (foreground, background, minimum ratio) pairs the app actually renders.
    public static let pairs: [(fg: PocketRawColor, bg: PocketRawColor, minRatio: Double)] = [
        (lightTextPrimary, lightBgCanvas, 7.0),
        (lightTextPrimary, lightBgSurface, 7.0),
        (lightTextSecondary, lightBgCanvas, 4.5),
        (lightTextSecondary, lightBgSurface, 4.5),
        (darkTextPrimary, darkBgCanvas, 7.0),
        (darkTextPrimary, darkBgSurface, 7.0),
        (darkTextSecondary, darkBgCanvas, 4.5),
        (darkTextSecondary, darkBgSurface, 4.5),
    ]
}

public enum PocketContrast {
    /// WCAG 2.x contrast ratio between two opaque colors (0...1 sRGB components).
    public static func ratio(_ a: (r: Double, g: Double, b: Double), _ b: (r: Double, g: Double, b: Double)) -> Double {
        let la = relativeLuminance(a)
        let lb = relativeLuminance(b)
        let (hi, lo) = la > lb ? (la, lb) : (lb, la)
        return (hi + 0.05) / (lo + 0.05)
    }

    public static func rgb(hex: String) -> (r: Double, g: Double, b: Double) {
        var h = hex
        if h.hasPrefix("#") { h.removeFirst() }
        let value = UInt32(h, radix: 16) ?? 0
        let r = Double((value >> 16) & 0xFF) / 255.0
        let g = Double((value >> 8) & 0xFF) / 255.0
        let b = Double(value & 0xFF) / 255.0
        return (r, g, b)
    }

    /// Flattens `fg` (with alpha) over an opaque `bg`, then returns the
    /// contrast ratio between the flattened color and `bg`.
    public static func flattenedRatio(fg: PocketRawColor, bg: PocketRawColor) -> Double {
        let fgRgb = rgb(hex: fg.hex)
        let bgRgb = rgb(hex: bg.hex)
        let a = fg.alpha
        let flattened = (
            r: fgRgb.r * a + bgRgb.r * (1 - a),
            g: fgRgb.g * a + bgRgb.g * (1 - a),
            b: fgRgb.b * a + bgRgb.b * (1 - a)
        )
        return ratio(flattened, bgRgb)
    }

    private static func relativeLuminance(_ c: (r: Double, g: Double, b: Double)) -> Double {
        func channel(_ v: Double) -> Double {
            v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    }
}
