import Foundation
import Network
#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// The one reconnecting SSE loop for an open session, shared by the iPhone
/// `LSSessionView` and the Mac `LSSessionModel` (LSI-18) so the two can't
/// drift again.
///
/// - Retries on any drop, error or clean end, for as long as it runs:
///   1, 2, 4, 8, 16, 30, 30… s, reset by the first frame of a connection.
/// - Each connect sends `Last-Event-ID` = `lastEventID()`.
/// - Event frames already delivered (by id) are dropped here: the server
///   ignores `Last-Event-ID` today and replays its whole buffer on every
///   connect, so dedupe is the client's job.
/// - Every 6th failed attempt, and straight away on 401/403/404, it calls
///   `refetch` — that refreshes an expired access token (the stream itself
///   never refreshes) and catches a session that ended while away. Only a
///   refetch that is itself 401/403/404 stops the loop as `.unreachable`.
/// - Stops on `session_ended`, on `refetch` reporting not-live, or `stop()`.
/// - Restarts at once on wake from sleep / app foreground and when the
///   network comes back.
@MainActor
public final class LSStreamLoop {
    public enum Status: Equatable, Sendable {
        case connected
        case reconnecting(attempt: Int)
        case unreachable(String)
    }

    private let connect: @MainActor (_ lastEventID: String?) async -> AsyncThrowingStream<LSStreamFrame, Error>
    private let lastEventID: @MainActor () -> String?
    /// Re-fetch the session; returns whether it is still live.
    private let refetch: @MainActor () async throws -> Bool
    private let onFrame: @MainActor (LSStreamFrame) -> Void
    private let onStatus: @MainActor (Status) -> Void
    private let watchesSystem: Bool
    /// Injectable so tests run the backoff without waiting.
    var sleep: @MainActor (Double) async throws -> Void = { try await Task.sleep(for: .seconds($0)) }

    private(set) var task: Task<Void, Never>?
    private var seen = Set<String>()
    private var wake: (NotificationCenter, NSObjectProtocol)?
    private var monitor: NWPathMonitor?
    private var pathOK: Bool?

    public init(
        watchSystem: Bool = true,
        connect: @escaping @MainActor (_ lastEventID: String?) async -> AsyncThrowingStream<LSStreamFrame, Error>,
        lastEventID: @escaping @MainActor () -> String?,
        refetch: @escaping @MainActor () async throws -> Bool,
        onFrame: @escaping @MainActor (LSStreamFrame) -> Void,
        onStatus: @escaping @MainActor (Status) -> Void
    ) {
        self.watchesSystem = watchSystem
        self.connect = connect
        self.lastEventID = lastEventID
        self.refetch = refetch
        self.onFrame = onFrame
        self.onStatus = onStatus
    }

    nonisolated static func backoff(_ attempt: Int) -> Double {
        min(30, pow(2, Double(max(attempt, 1) - 1)))
    }

    /// (Re)start streaming. `known` are the event ids already held, so a
    /// replay of them is dropped before it reaches the owner.
    public func start(known: some Sequence<String>) {
        seen = Set(known)
        restart()
        watchSystem()
    }

    public func stop() {
        task?.cancel()
        task = nil
        if let (center, token) = wake { center.removeObserver(token) }
        wake = nil
        monitor?.cancel()
        monitor = nil
        pathOK = nil
    }

    /// Reconnect now with the backoff reset — wake, foreground, network back.
    public func kick() {
        guard task != nil else { return }
        restart()
    }

    private func restart() {
        task?.cancel()
        task = Task { [weak self] in await self?.run() }
    }

    private func run() async {
        var attempt = 0
        while !Task.isCancelled {
            var failure: Error?
            do {
                var fresh = true
                for try await frame in await connect(lastEventID()) {
                    if Task.isCancelled { return }
                    if fresh {
                        fresh = false
                        attempt = 0
                        onStatus(.connected)
                    }
                    if case .event(let e) = frame, !seen.insert(e.id).inserted { continue }
                    onFrame(frame)
                    if case .sessionEnded = frame { return }
                }
            } catch is CancellationError {
                return
            } catch {
                failure = error
            }
            if Task.isCancelled { return }
            attempt += 1
            onStatus(.reconnecting(attempt: attempt))
            let gone: Set<Int> = [401, 403, 404]
            if attempt % 6 == 0 || gone.contains((failure as? LSAPIError)?.status ?? 0) {
                do {
                    let live = try await refetch()
                    if Task.isCancelled { return }
                    if !live { return }
                } catch let e as LSAPIError where gone.contains(e.status) {
                    if !Task.isCancelled { onStatus(.unreachable(e.message)) }
                    return
                } catch {}
            }
            try? await sleep(Self.backoff(attempt))
        }
    }

    private func watchSystem() {
        guard watchesSystem, wake == nil else { return }
        #if os(iOS)
        let center = NotificationCenter.default
        let name = UIApplication.willEnterForegroundNotification
        #else
        let center = NSWorkspace.shared.notificationCenter
        let name = NSWorkspace.didWakeNotification
        #endif
        let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.kick() }
        }
        wake = (center, token)

        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { [weak self] path in
            let ok = path.status == .satisfied
            Task { @MainActor in self?.pathChanged(ok) }
        }
        monitor.start(queue: .main)
        self.monitor = monitor
    }

    private func pathChanged(_ ok: Bool) {
        defer { pathOK = ok }
        if ok, pathOK == false { kick() }
    }
}
