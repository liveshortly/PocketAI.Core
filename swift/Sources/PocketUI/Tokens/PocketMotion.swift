import SwiftUI
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// Motion tokens (spec §3.4). Every spring in the apps goes through here so
/// they feel the same, and Reduce Motion turns movement into a cross-fade
/// once instead of at each call site (UI-129).
@MainActor
public enum PocketMotion {
    public static var reduceMotion: Bool {
        #if canImport(UIKit)
        UIAccessibility.isReduceMotionEnabled
        #elseif canImport(AppKit)
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        #else
        false
        #endif
    }

    /// Default spring: response 0.32, damping 0.86. A short ease under Reduce Motion.
    public static var standard: Animation {
        reduceMotion ? .easeInOut(duration: 0.2) : .spring(response: 0.32, dampingFraction: 0.86)
    }

    /// Quick spring: response 0.22.
    public static var quick: Animation {
        reduceMotion ? .easeInOut(duration: 0.15) : .spring(response: 0.22, dampingFraction: 0.86)
    }

    /// Slide in from `edge`, or fade only under Reduce Motion.
    public static func slide(from edge: Edge) -> AnyTransition {
        reduceMotion ? .opacity : .move(edge: edge).combined(with: .opacity)
    }
}
