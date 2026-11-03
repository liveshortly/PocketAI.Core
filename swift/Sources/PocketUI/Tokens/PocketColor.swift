import SwiftUI

/// Color tokens backed by `Resources/Colors.xcassets`. No hex literals outside
/// this file — CI greps feature code for `Color(red:` and `#colorLiteral`.
public enum PocketColor {
    public static let bgCanvas = Color("bg.canvas", bundle: .module)
    public static let bgSurface = Color("bg.surface", bundle: .module)
    public static let bgRaised = Color("bg.raised", bundle: .module)
    public static let bgSunken = Color("bg.sunken", bundle: .module)

    public static let lineHairline = Color("line.hairline", bundle: .module)
    public static let lineStrong = Color("line.strong", bundle: .module)

    public static let textPrimary = Color("text.primary", bundle: .module)
    public static let textSecondary = Color("text.secondary", bundle: .module)
    public static let textTertiary = Color("text.tertiary", bundle: .module)

    public static let accent = Color("accent", bundle: .module)
    public static let accentSoft = Color("accent.soft", bundle: .module)
    public static let accentOnAccent = Color("accent.onAccent", bundle: .module)

    /// The PocketAI mark's green (UI-34): #1F7A4D light / #4ADE9A dark.
    /// Mark code reads this, never `accent`.
    public static let markGreen = Color("mark.green", bundle: .module)

    public static let statusOk = Color("status.ok", bundle: .module)
    public static let statusWarn = Color("status.warn", bundle: .module)
    public static let statusBad = Color("status.bad", bundle: .module)
    public static let statusInfo = Color("status.info", bundle: .module)

    public static let privacyLocal = Color("privacy.local", bundle: .module)
}
