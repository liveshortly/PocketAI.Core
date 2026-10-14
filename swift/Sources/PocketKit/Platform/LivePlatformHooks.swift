import Foundation
import Metal
#if os(iOS)
import UIKit
#endif

/// APL-03: the real `PlatformHooks` conformance Rust's `PocketCore.open`
/// is constructed with - every value backed by an actual system API, no
/// placeholders. `PreviewCore`/tests use a fake instead (see
/// `PocketCoreTests`'s `FakeHooks`-equivalent), since most of these calls
/// need a real process/filesystem/Keychain to mean anything.
public final class LivePlatformHooks: PlatformHooks, @unchecked Sendable {
    private static let keychainService = "app.getpocketai"

    public init() {}

    // MARK: - Keychain

    public func keychainGet(key: String) -> Data? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.keychainService,
            kSecAttrAccount as String: key,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: AnyObject?
        let status = withUnsafeMutablePointer(to: &result) { ptr in
            SecItemCopyMatching(query as CFDictionary, ptr)
        }
        query.removeValue(forKey: kSecReturnData as String)
        query.removeValue(forKey: kSecMatchLimit as String)
        guard status == errSecSuccess else { return nil }
        return result as? Data
    }

    public func keychainSet(key: String, value: Data) -> Bool {
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.keychainService,
            kSecAttrAccount as String: key,
        ]
        // Keychain items can't be updated with SecItemAdd once they exist;
        // delete-then-add is simplest and correct for a single-value secret.
        SecItemDelete(base as CFDictionary)
        var toAdd = base
        toAdd[kSecValueData as String] = value
        toAdd[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(toAdd as CFDictionary, nil) == errSecSuccess
    }

    // MARK: - App support directory

    public func appSupportDir() -> String {
        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(Bundle.main.bundleIdentifier ?? "app.getpocketai", isDirectory: true)
        try? fm.createDirectory(at: base, withIntermediateDirectories: true)

        let modelsDir = base.appendingPathComponent("Models", isDirectory: true)
        try? fm.createDirectory(at: modelsDir, withIntermediateDirectories: true)
        var excluded = URL(fileURLWithPath: modelsDir.path)
        var excludeFromBackup = URLResourceValues()
        excludeFromBackup.isExcludedFromBackup = true
        try? excluded.setResourceValues(excludeFromBackup)

        #if os(iOS)
        try? fm.setAttributes(
            [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
            ofItemAtPath: modelsDir.path
        )
        #endif

        return base.path
    }

    // MARK: - Clock

    public func nowMs() -> Int64 {
        Int64((Date().timeIntervalSince1970 * 1000).rounded())
    }

    // MARK: - Device profile

    public func deviceProfile() -> DeviceProfile {
        let freeDisk = Self.freeDiskBytes()
        let totalRam = ProcessInfo.processInfo.physicalMemory

        #if os(iOS)
        let platform = Platform.ios
        let appMemoryLimit = Self.availableAppMemoryBytes() ?? UInt64(Double(totalRam) * 0.6)
        let lowPowerMode = ProcessInfo.processInfo.isLowPowerModeEnabled
        let thermal = Self.thermalState(from: ProcessInfo.processInfo.thermalState)
        let hasIncreasedMemoryEntitlement = Bundle.main.object(forInfoDictionaryKey: "PocketAIHasIncreasedMemoryEntitlement") as? Bool ?? false
        #else
        let platform = Platform.macOs
        let appMemoryLimit = totalRam
        let lowPowerMode = false
        let thermal = Self.thermalState(from: ProcessInfo.processInfo.thermalState)
        let hasIncreasedMemoryEntitlement = true
        #endif

        return DeviceProfile(
            platform: platform,
            modelIdentifier: Self.sysctlString("hw.model") ?? "unknown",
            chip: Self.chipDescription(),
            totalRamBytes: UInt64(totalRam),
            appMemoryLimitBytes: appMemoryLimit,
            gpuFamily: Self.metalFamilyDescription(),
            freeDiskBytes: freeDisk,
            thermalState: thermal,
            lowPowerMode: lowPowerMode,
            hasIncreasedMemoryEntitlement: hasIncreasedMemoryEntitlement
        )
    }

    // MARK: - Device profile helpers

    private static func sysctlString(_ name: String) -> String? {
        var size = 0
        guard sysctlbyname(name, nil, &size, nil, 0) == 0, size > 0 else { return nil }
        var buffer = [CChar](repeating: 0, count: size)
        guard sysctlbyname(name, &buffer, &size, nil, 0) == 0 else { return nil }
        return buffer.withUnsafeBufferPointer { ptr in
            let bytes = ptr.map { UInt8(bitPattern: $0) }.prefix { $0 != 0 }
            return String(decoding: bytes, as: UTF8.self)
        }
    }

    private static func chipDescription() -> String {
        // "hw.machine" (e.g. "arm64") doesn't name the chip; Apple doesn't
        // expose a direct chip-name sysctl, so this reports the machine
        // identifier (e.g. "iPhone17,1", "Mac15,6") and leaves friendly
        // chip-name mapping (A18 Pro, M3, ...) to the catalog/fit lookup
        // that already has that table (CORE-07's chips.json).
        sysctlString("hw.model") ?? sysctlString("hw.machine") ?? "unknown"
    }

    private static func metalFamilyDescription() -> String {
        guard let device = MTLCreateSystemDefaultDevice() else { return "none" }
        let families: [(MTLGPUFamily, String)] = [
            (.metal3, "metal3"),
            (.apple9, "apple9"),
            (.apple8, "apple8"),
            (.apple7, "apple7"),
            (.mac2, "mac2"),
        ]
        for (family, name) in families where device.supportsFamily(family) {
            return name
        }
        return "unknown"
    }

    private static func freeDiskBytes() -> UInt64 {
        let url = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        guard
            let values = try? url.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey]),
            let bytes = values.volumeAvailableCapacityForImportantUsage
        else { return 0 }
        return UInt64(bytes)
    }

    private static func thermalState(from state: ProcessInfo.ThermalState) -> ThermalState {
        switch state {
        case .nominal: return .nominal
        case .fair: return .fair
        case .serious: return .serious
        case .critical: return .critical
        @unknown default: return .nominal
        }
    }

    #if os(iOS)
    private static func availableAppMemoryBytes() -> UInt64? {
        let bytes = os_proc_available_memory()
        return bytes > 0 ? UInt64(bytes) : nil
    }
    #endif
}
