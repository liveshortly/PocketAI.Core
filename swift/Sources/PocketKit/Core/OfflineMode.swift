import Foundation

/// The offline switch, as one observable value both apps share.
///
/// `CoreClient.isOffline()` is a plain call on a protocol that is `AnyObject,
/// Sendable` rather than `@Observable`, so nothing invalidates a view that
/// reads it. Every screen that offered the switch therefore copied the flag
/// into its own `@State` and the copies drifted: on the Mac, Settings kept
/// showing the value it read when the tab was built, including after the
/// Privacy screen had changed the same setting.
///
/// One instance of this lives on the app model and every screen binds to it,
/// so there is a single copy to be wrong.
@MainActor
@Observable
public final class OfflineMode {
    public private(set) var isOn: Bool

    private let core: CoreClient

    public init(core: CoreClient) {
        self.core = core
        self.isOn = core.isOffline()
    }

    /// Writes through to the core and re-reads it, rather than trusting the
    /// value we just sent — the core is free to refuse or normalise it.
    public func set(_ on: Bool) {
        core.setOfflineMode(on: on)
        isOn = core.isOffline()
    }

    /// Re-reads the core. Needed where something other than this type can
    /// change the flag, such as a control-API request from a paired device.
    public func refresh() {
        isOn = core.isOffline()
    }
}
