import Foundation
import StoreKit

public enum StoreProduct {
    /// Product ID for legacy reference; in-app purchases are disabled; Pro comes from our licence keys.
    public static let proUnlockId = "com.getpocketai.app.pro"
}

public enum StoreError: LocalizedError {
    case productNotFound
    case verificationFailed
    case inAppPurchasesDisabled

    public var errorDescription: String? {
        switch self {
        case .productNotFound:
            return "PocketAI Pro product not found."
        case .verificationFailed:
            return "StoreKit transaction verification failed."
        case .inAppPurchasesDisabled:
            return "In-app purchases are disabled. PocketAI Pro is sold outside the App Store with a licence key."
        }
    }
}

/// StoreEntitlementManager: there is NO in-app Pro tier and NO App Store IAP path. Pro comes
/// only from licence keys issued by our licence server (COM-05 v2), verified in pocket-trust.
/// This manager is retained for interface stability and testing, but IAP transactions and
/// background StoreKit listeners are disabled.
@MainActor
public final class StoreEntitlementManager: ObservableObject {
    @Published public private(set) var isPro = false

    private let keychain: PlatformHooks?
    private let onEntitlementChanged: (@Sendable (StoreEntitlement) -> Void)?

    public init(keychain: PlatformHooks? = nil, onEntitlementChanged: (@Sendable (StoreEntitlement) -> Void)? = nil) {
        self.keychain = keychain
        self.onEntitlementChanged = onEntitlementChanged
    }

    /// No-op: no StoreKit transactions or listeners are registered.
    public func start() async {
        // Disabled: licence keys are the sole licensing system; no StoreKit polling or updates listener.
    }

    public func stop() {
        // No-op
    }

    public func purchase() async throws -> Bool {
        throw StoreError.inAppPurchasesDisabled
    }

    public func restorePurchases() async throws {
        // No-op: No StoreKit IAP path.
    }
}
