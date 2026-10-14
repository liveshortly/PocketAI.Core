import Foundation
import StoreKit

public enum StoreProduct {
    /// Required by App Store Review Guideline 3.1.1: a license key can't be
    /// the only unlock, so the same Pro unlock must also be sold as this
    /// non-consumable IAP.
    public static let proUnlockId = "app.getpocketai.pro"
}

/// APL-18's StoreKit 2 side. Feeds `core.set_store_entitlement(...)`
/// (already merged with the license-key path by LIC-03, D46) - this type
/// owns nothing about the license key itself, only the IAP source.
@MainActor
public final class StoreEntitlementManager: ObservableObject {
    @Published public private(set) var isPro = false

    private let keychain: PlatformHooks
    private let onEntitlementChanged: @Sendable (StoreEntitlement) -> Void
    private var updatesTask: Task<Void, Never>?

    private static let lastVerifiedKeychainKey = "store.pro.lastVerifiedAt"
    private static let offlineGraceDays = 30

    public init(keychain: PlatformHooks, onEntitlementChanged: @escaping @Sendable (StoreEntitlement) -> Void) {
        self.keychain = keychain
        self.onEntitlementChanged = onEntitlementChanged
    }

    /// Call once at launch: checks `Transaction.currentEntitlements` now,
    /// then keeps listening for `Transaction.updates` (new purchases,
    /// refunds/revocations, renewal of a verification) for the rest of
    /// the process lifetime.
    public func start() async {
        await refreshCurrentEntitlements()
        updatesTask = Task { [weak self] in
            for await update in Transaction.updates {
                await self?.handle(update)
            }
        }
    }

    public func stop() {
        updatesTask?.cancel()
        updatesTask = nil
    }

    public func restorePurchases() async throws {
        try await AppStore.sync()
        await refreshCurrentEntitlements()
    }

    private func refreshCurrentEntitlements() async {
        var activeProTransaction: Transaction?
        for await result in Transaction.currentEntitlements {
            guard
                case .verified(let transaction) = result,
                transaction.productID == StoreProduct.proUnlockId,
                transaction.revocationDate == nil
            else { continue }
            activeProTransaction = transaction
        }

        if let transaction = activeProTransaction {
            apply(transaction: transaction, pro: true)
        } else if let lastVerified = cachedLastVerifiedAt(),
            Date().timeIntervalSince(lastVerified) < Double(Self.offlineGraceDays * 86_400)
        {
            // Offline grace: `currentEntitlements` needs network the first
            // time; keep reporting Pro from the last successful online
            // verification rather than revoking on a launch with no
            // connectivity.
            isPro = true
        } else {
            isPro = false
            onEntitlementChanged(StoreEntitlement(pro: false, transactionId: nil))
        }
    }

    private func handle(_ update: VerificationResult<Transaction>) async {
        guard case .verified(let transaction) = update, transaction.productID == StoreProduct.proUnlockId else { return }
        if transaction.revocationDate != nil {
            isPro = false
            onEntitlementChanged(StoreEntitlement(pro: false, transactionId: String(transaction.id)))
        } else {
            apply(transaction: transaction, pro: true)
        }
        await transaction.finish()
    }

    private func apply(transaction: Transaction, pro: Bool) {
        isPro = pro
        setCachedLastVerifiedAt(Date())
        onEntitlementChanged(StoreEntitlement(pro: pro, transactionId: String(transaction.id)))
    }

    private func cachedLastVerifiedAt() -> Date? {
        guard
            let data = keychain.keychainGet(key: Self.lastVerifiedKeychainKey),
            let string = String(data: data, encoding: .utf8),
            let timestamp = Double(string)
        else { return nil }
        return Date(timeIntervalSince1970: timestamp)
    }

    private func setCachedLastVerifiedAt(_ date: Date) {
        _ = keychain.keychainSet(key: Self.lastVerifiedKeychainKey, value: Data(String(date.timeIntervalSince1970).utf8))
    }
}
