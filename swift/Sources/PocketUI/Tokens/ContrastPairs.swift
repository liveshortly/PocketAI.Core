import Foundation

/// WCAG contrast maths, used by the design-token tests.
///
/// The hand-maintained table of hex values that used to live here is gone: it
/// claimed to mirror `Resources/Colors.xcassets` and silently stopped doing so
/// when the palette changed, leaving the contrast suite green against colors
/// nobody rendered. The tests read the colorsets directly now, so there is one
/// copy of each value instead of two.
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

    private static func relativeLuminance(_ c: (r: Double, g: Double, b: Double)) -> Double {
        func channel(_ v: Double) -> Double {
            v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    }
}
