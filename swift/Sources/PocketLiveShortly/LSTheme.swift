import SwiftUI
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// LiveShortly brand palette. LS surfaces read LiveShortly green (the web
/// HUD's live color) instead of the host app's Ember accent, so the
/// integration is visually its own thing inside PocketAI.
public enum LSColor {
    /// Web token green `#1f7a4d`, lifted for dark backgrounds.
    public static let accent = dynamic(
        light: Color(red: 0x1f / 255, green: 0x7a / 255, blue: 0x4d / 255),
        dark: Color(red: 0x3d / 255, green: 0xb2 / 255, blue: 0x7e / 255))
    public static let accentSoft = accent.opacity(0.12)
    /// Text/glyph on an accent-filled control (green is dark enough in both modes).
    public static let onAccent = Color.white

    private static func dynamic(light: Color, dark: Color) -> Color {
        #if canImport(UIKit)
        return Color(UIColor { traits in
            traits.userInterfaceStyle == .dark ? UIColor(dark) : UIColor(light)
        })
        #elseif canImport(AppKit)
        return Color(NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                ? NSColor(dark) : NSColor(light)
        })
        #else
        return light
        #endif
    }
}

/// Last-seen event counts per session, for unread badges on list rows.
/// ponytail: plain UserDefaults dict, no pruning — entries for deleted
/// sessions linger as a few bytes; add pruning if the dict ever matters.
public enum LSSeen {
    private static let key = "ls.seenEvents"

    public static func count(_ sessionID: String) -> Int {
        (UserDefaults.standard.dictionary(forKey: key) as? [String: Int])?[sessionID] ?? 0
    }

    public static func mark(_ sessionID: String, _ count: Int) {
        var d = (UserDefaults.standard.dictionary(forKey: key) as? [String: Int]) ?? [:]
        guard count > (d[sessionID] ?? 0) else { return }
        d[sessionID] = count
        UserDefaults.standard.set(d, forKey: key)
    }
}
