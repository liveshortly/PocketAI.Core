import Foundation
import Observation
import SwiftUI

/// The account's session list, shared by every surface that shows it (Mac
/// sidebar, Tools › LiveShortly, iPhone Chats section) — LSI-17.
///
/// Before this each view fetched once and kept its own copy behind a 15 s
/// guard, so a session started from the Mac app, the web or `live claude`
/// never appeared until a manual refresh. Now there is one list, refreshed
/// on a poll (the server has no user-level event stream, only per-session
/// SSE) and right after a create.
@Observable
@MainActor
public final class LSSessionFeed {
    public private(set) var sessions: [LSSession] = []
    /// At least one fetch has finished (successfully or not).
    public private(set) var loaded = false
    public private(set) var isLoading = false
    /// The last fetch failed. `sessions` keeps what it had — an error never
    /// empties the list.
    public private(set) var loadError: String?
    /// Set by the hosting view from `@Environment(\.scenePhase)`.
    public var phase: ScenePhase = .active

    @ObservationIgnored weak var account: LSAccountModel?
    @ObservationIgnored private var lastAttempt: Date?
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var running = false

    init() {}

    /// Live first, newest first within each group — mirrors the web HUD.
    public var ordered: [LSSession] {
        sessions.sorted {
            if $0.isLive != $1.isLive { return $0.isLive }
            return $0.createdAt > $1.createdAt
        }
    }

    /// Seconds between polls: fast while something is live and the window is
    /// in front, slow otherwise, never while hidden.
    nonisolated public static func pollInterval(phase: ScenePhase, anyLive: Bool) -> TimeInterval? {
        switch phase {
        case .background: return nil
        case .active: return anyLive ? 10 : 60
        default: return 60
        }
    }

    /// Fetch now. Overlapping calls are fine: only the newest one's answer
    /// lands, so a slow poll can't overwrite a fresher post-create fetch.
    public func reload() async {
        guard let client = account?.client else { return }
        generation += 1
        let mine = generation
        lastAttempt = Date()
        isLoading = true
        do {
            let fresh = try await client.sessions(limit: 30)
            guard mine == generation else { return }
            sessions = fresh
            loadError = nil
        } catch {
            guard mine == generation else { return }
            loadError = error.localizedDescription
        }
        loaded = true
        isLoading = false
    }

    /// The poll loop. Call from a view's `.task`; a second caller returns at
    /// once, so any number of views can host it.
    public func run() async {
        guard !running else { return }
        running = true
        defer { running = false }
        while !Task.isCancelled {
            if account?.client != nil,
               let every = Self.pollInterval(phase: phase, anyLive: sessions.contains(where: \.isLive)),
               lastAttempt.map({ Date().timeIntervalSince($0) >= every }) ?? true {
                await reload()
            }
            try? await Task.sleep(for: .seconds(1))
        }
    }

    /// Drop a row the server already deleted.
    public func remove(_ id: String) {
        sessions.removeAll { $0.id == id }
    }

    func reset() {
        generation += 1
        sessions = []
        loaded = false
        isLoading = false
        loadError = nil
        lastAttempt = nil
    }
}
