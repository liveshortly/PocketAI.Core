#if os(macOS) // MAC-19 is Mac-only; iOS was never unsandboxed.
import Foundation
import os

/// MAC-19 (D132): the Mac app used to run in the App Sandbox, so everything it
/// wrote lives under `~/Library/Containers/<bundle id>/Data/Library/…`.
/// Unsandboxed, the same APIs resolve to `~/Library/…`. This moves the data
/// across once, before the store opens, so nobody loses models, chats or
/// settings. Never overwrites data already at the destination.
public struct SandboxMigrationPlan: Equatable {
    public struct Move: Equatable {
        public let from: URL
        public let to: URL
    }

    /// Container directories to move; each destination is missing or holds no files.
    public var moves: [Move] = []
    /// Destinations that already hold data, left alone.
    public var skipped: [URL] = []
    /// The container's preferences plist, to merge into the standard domain.
    public var preferences: URL?
}

/// Pure: looks, never touches. `container` is `~/Library/Containers/<id>/Data`.
/// Only Application Support moves — Caches is `URLCache` and regenerates.
public func planSandboxMigration(
    container: URL,
    home: URL,
    bundleID: String = "com.getpocketai.app",
    fileManager fm: FileManager = .default
) -> SandboxMigrationPlan {
    var plan = SandboxMigrationPlan()
    // LivePlatformHooks.appSupportDir (store DB + Models) and LSTranscriptCache.
    for rel in ["Library/Application Support/\(bundleID)", "Library/Application Support/liveshortly-cache"] {
        let from = container.appendingPathComponent(rel, isDirectory: true)
        let to = home.appendingPathComponent(rel, isDirectory: true)
        guard fm.fileExists(atPath: from.path) else { continue }
        if holdsNoFiles(to, fm) {
            plan.moves.append(.init(from: from, to: to))
        } else {
            plan.skipped.append(to)
        }
    }
    let prefs = container.appendingPathComponent("Library/Preferences/\(bundleID).plist")
    if fm.fileExists(atPath: prefs.path) { plan.preferences = prefs }
    return plan
}

/// Carries out `plan`. Returns a log line per step and whether every step
/// succeeded; a failed step leaves its container directory where it was.
public func runSandboxMigration(
    _ plan: SandboxMigrationPlan,
    bundleID: String,
    defaults: UserDefaults,
    fileManager fm: FileManager = .default
) -> (log: [String], ok: Bool) {
    var log: [String] = []
    var ok = true
    for move in plan.moves {
        do {
            try relocate(move.from, to: move.to, fm)
            log.append("moved \(move.from.path) -> \(move.to.path)")
        } catch {
            ok = false
            log.append("FAILED \(move.from.path): \(error.localizedDescription); container left in place")
        }
    }
    for dest in plan.skipped {
        log.append("kept existing \(dest.path)")
    }
    if let url = plan.preferences {
        if let old = NSDictionary(contentsOf: url) as? [String: Any] {
            // Anything already set unsandboxed wins.
            let merged = old.merging(defaults.persistentDomain(forName: bundleID) ?? [:]) { _, new in new }
            defaults.setPersistentDomain(merged, forName: bundleID)
            log.append("imported \(old.count) preference(s) from \(url.path)")
        } else {
            ok = false
            log.append("FAILED reading \(url.path)")
        }
    }
    return (log, ok)
}

/// App-launch entry point: no-op once the marker is set, or while still sandboxed.
public func migrateFromSandboxIfNeeded(defaults: UserDefaults = .standard) {
    let markerKey = "migratedFromSandbox"
    guard
        !defaults.bool(forKey: markerKey),
        ProcessInfo.processInfo.environment["APP_SANDBOX_CONTAINER_ID"] == nil,
        let bundleID = Bundle.main.bundleIdentifier
    else { return }
    let logger = Logger(subsystem: bundleID, category: "migration")
    let home = FileManager.default.homeDirectoryForCurrentUser
    let container = home.appendingPathComponent("Library/Containers/\(bundleID)/Data", isDirectory: true)
    let plan = planSandboxMigration(container: container, home: home, bundleID: bundleID)
    let (lines, ok) = runSandboxMigration(plan, bundleID: bundleID, defaults: defaults)
    for line in lines { logger.notice("\(line, privacy: .public)") }
    // A failure retries next launch; successful moves are gone from the plan by then.
    if ok { defaults.set(true, forKey: markerKey) }
}

private func holdsNoFiles(_ url: URL, _ fm: FileManager) -> Bool {
    var isDir: ObjCBool = false
    guard fm.fileExists(atPath: url.path, isDirectory: &isDir) else { return true }
    guard isDir.boolValue, let walk = fm.enumerator(at: url, includingPropertiesForKeys: [.isDirectoryKey]) else {
        return false
    }
    for case let item as URL in walk where (try? item.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) != true {
        return false
    }
    return true
}

/// Same volume: one atomic rename. Otherwise copy and keep the container as the backup.
private func relocate(_ from: URL, to: URL, _ fm: FileManager) throws {
    try fm.createDirectory(at: to.deletingLastPathComponent(), withIntermediateDirectories: true)
    if fm.fileExists(atPath: to.path) { try fm.removeItem(at: to) } // planner checked: no files
    if rename(from.path, to.path) == 0 { return }
    let err = errno
    guard err == EXDEV else { throw POSIXError(POSIXErrorCode(rawValue: err) ?? .EIO) }
    let staging = to.appendingPathExtension("migrating")
    try? fm.removeItem(at: staging)
    do {
        try fm.copyItem(at: from, to: staging)
        try fm.moveItem(at: staging, to: to)
    } catch {
        try? fm.removeItem(at: staging)
        throw error
    }
}
#endif
