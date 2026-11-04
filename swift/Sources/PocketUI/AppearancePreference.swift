import SwiftUI

/// User's light/dark choice (UI-118). Stored as its raw string in
/// `@AppStorage`; anything unknown (or "system") follows the system.
public enum AppearancePreference: String, CaseIterable, Sendable {
    case system, light, dark

    public static let storageKey = "app.getpocketai.appearance"

    /// nil = follow the system, which is what `.preferredColorScheme(nil)` does.
    public static func colorScheme(forStored raw: String) -> ColorScheme? {
        switch AppearancePreference(rawValue: raw) {
        case .light: .light
        case .dark: .dark
        case .system, .none: nil
        }
    }
}
