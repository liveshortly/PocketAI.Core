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
    /// My own sessions (`scope=mine`).
    public private(set) var sessions: [LSSession] = []
    /// Sessions other people shared with me (`scope=shared`), minus any that
    /// are also mine — LSI-19. Same poll, same ordering.
    public private(set) var shared: [LSSession] = []
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
    public var ordered: [LSSession] { Self.order(sessions) }
    public var orderedShared: [LSSession] { Self.order(shared) }

    /// Shared rows never opened on this device — the quiet "new" badge on
    /// the Shared with me section.
    public var newSharedCount: Int { shared.filter { LSSeen.count($0.id) == 0 }.count }

    nonisolated static func order(_ list: [LSSession]) -> [LSSession] {
        list.sorted {
            if $0.isLive != $1.isLive { return $0.isLive }
            return $0.createdAt > $1.createdAt
        }
    }

    /// Shared rows with anything already in `mine` dropped, and duplicate ids
    /// collapsed to the first — a session is listed once, under mine first.
    nonisolated static func sharedOnly(_ shared: [LSSession], mine: [LSSession]) -> [LSSession] {
        var seen = Set(mine.map(\.id))
        return shared.filter { seen.insert($0.id).inserted }
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
        // Shared is best-effort: its failure keeps the last shared list and
        // never blocks my own sessions.
        async let sharedFetch = try? client.sessions(scope: "shared", limit: 30)
        do {
            let fresh = try await client.sessions(limit: 30)
            let freshShared = await sharedFetch
            guard mine == generation else { return }
            sessions = fresh
            if let freshShared { shared = Self.sharedOnly(freshShared, mine: fresh) }
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
               let every = Self.pollInterval(
                   phase: phase, anyLive: (sessions + shared).contains(where: \.isLive)),
               lastAttempt.map({ Date().timeIntervalSince($0) >= every }) ?? true {
                await reload()
            }
            try? await Task.sleep(for: .seconds(1))
        }
    }

    /// Drop a row the server already deleted.
    public func remove(_ id: String) {
        sessions.removeAll { $0.id == id }
        shared.removeAll { $0.id == id }
    }

    func reset() {
        generation += 1
        sessions = []
        shared = []
        loaded = false
        isLoading = false
        loadError = nil
        lastAttempt = nil
    }
}
