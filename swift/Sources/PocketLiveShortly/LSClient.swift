import Foundation
import PocketKit

public struct LSAPIError: Error, LocalizedError, Sendable {
    public var status: Int
    public var message: String
    public var errorDescription: String? { message }
}

/// Async HTTP client for a LiveShortly server. Holds the token pair and
/// refreshes the access token once on a 401, mirroring the CLI's behavior
/// (AUTH.md). An actor so refresh can't race itself.
public actor LSClient {
    public let baseURL: URL
    private var accessToken: String?
    private var refreshToken: String?
    /// Called after a successful refresh so the owner can persist the pair.
    private var onTokensUpdated: (@Sendable (String, String?) -> Void)?

    public init(baseURL: URL, accessToken: String? = nil, refreshToken: String? = nil) {
        self.baseURL = baseURL
        self.accessToken = accessToken
        self.refreshToken = refreshToken
    }

    public func setTokens(access: String?, refresh: String?) {
        accessToken = access
        refreshToken = refresh
    }

    public func setOnTokensUpdated(_ cb: @escaping @Sendable (String, String?) -> Void) {
        onTokensUpdated = cb
    }

    // MARK: - Device flow (no auth)

    public func deviceStart() async throws -> LSDeviceStart {
        try await send("POST", "/auth/device/start", authed: false)
    }

    /// One poll tick. Returns nil while pending, tokens once approved.
    public func devicePoll(deviceCode: String) async throws -> LSDevicePoll? {
        let r: LSDevicePoll = try await send(
            "POST", "/auth/device/poll", authed: false,
            body: ["device_code": deviceCode])
        return r.accessToken == nil ? nil : r
    }

    // MARK: - API

    public func me() async throws -> LSUser {
        struct Me: Codable { var email: String?; var name: String? }
        let m: Me = try await send("GET", "/api/me")
        return LSUser(email: m.email ?? "", name: m.name ?? "")
    }

    public func sessions(scope: String = "mine", limit: Int = 100) async throws -> [LSSession] {
        struct List: Codable { var results: [LSSession]? }
        let l: List = try await send("GET", "/api/sessions?scope=\(scope)&limit=\(limit)")
        return l.results ?? []
    }

    public func session(_ id: String) async throws -> LSSessionDetail {
        try await send("GET", "/api/sessions/\(id)")
    }

    public func hosts() async throws -> [LSHost] {
        struct Hosts: Codable { var hosts: [LSHost]? }
        let h: Hosts = try await send("GET", "/api/hosts")
        return h.hosts ?? []
    }

    public func createSession(
        hostID: String, agent: String, cwd: String,
        model: String?, bypassPermissions: Bool
    ) async throws -> LSCreatedSession {
        var body: [String: Any] = ["host_id": hostID, "agent": agent, "cwd": cwd]
        if let model, !model.isEmpty { body["model"] = model }
        if bypassPermissions { body["bypass_permissions"] = true }
        return try await send("POST", "/api/sessions", body: body)
    }

    public func comment(_ id: String, message: String) async throws {
        try await sendVoid("POST", "/api/sessions/\(id)/comments", body: ["message": message])
    }

    /// Ephemeral "viewer is typing" presence ping — best-effort, never throws.
    public func typing(_ id: String) async {
        try? await sendVoid("POST", "/api/sessions/\(id)/typing")
    }

    /// Answer a pending permission probe: "allow" | "deny" ("always" is
    /// expressed by the daemon side; the API takes allow/deny).
    public func decide(_ id: String, decision: String) async throws {
        try await sendVoid("POST", "/api/sessions/\(id)/decision", body: ["decision": decision])
    }

    public func setModel(_ id: String, model: String) async throws {
        try await sendVoid("POST", "/api/sessions/\(id)/model", body: ["model": model])
    }

    public func stop(_ id: String) async throws {
        try await sendVoid("POST", "/api/sessions/\(id)/stop")
    }

    public func rename(_ id: String, title: String) async throws {
        try await sendVoid("PATCH", "/api/sessions/\(id)", body: ["title": title])
    }

    // MARK: - Owner controls (web parity)

    /// PATCH visibility ("private" | "link" | "public" | "open") and/or the
    /// public-link role ("viewer" | "commenter").
    public func setVisibility(_ id: String, visibility: String? = nil, linkRole: String? = nil) async throws {
        var body: [String: Any] = [:]
        if let visibility { body["visibility"] = visibility }
        if let linkRole { body["link_role"] = linkRole }
        try await sendVoid("PATCH", "/api/sessions/\(id)", body: body)
    }

    public func deleteSession(_ id: String) async throws {
        try await sendVoid("DELETE", "/api/sessions/\(id)")
    }

    /// Publish to / remove from the public feed.
    public func publish(_ id: String) async throws {
        try await sendVoid("POST", "/api/sessions/\(id)/publish")
    }

    public func unpublish(_ id: String) async throws {
        try await sendVoid("POST", "/api/sessions/\(id)/unpublish")
    }

    public func shares(_ id: String) async throws -> [LSShare] {
        struct List: Codable { var results: [LSShare]? }
        let l: List = try await send("GET", "/api/sessions/\(id)/shares")
        return l.results ?? []
    }

    public func createShare(_ id: String, email: String, role: String) async throws -> LSShare {
        try await send("POST", "/api/sessions/\(id)/shares", body: ["email": email, "role": role])
    }

    public func deleteShare(_ id: String, shareID: String) async throws {
        try await sendVoid("DELETE", "/api/sessions/\(id)/shares/\(shareID)")
    }

    /// The web URL for a session — for copy-link / share sheets.
    public nonisolated func sessionLink(_ id: String) -> URL {
        baseURL.appendingPathComponent("session/\(id)")
    }

    // MARK: - SSE stream

    /// Live frames for a session: replay, then live, until ended/cancelled.
    /// `lastEventID` (the highest seq already held) is sent as `Last-Event-ID`
    /// so the server resumes from there instead of replaying its whole buffer
    /// (LSI-10).
    public func stream(_ id: String, lastEventID: String? = nil) -> AsyncThrowingStream<LSStreamFrame, Error> {
        let url = baseURL.appendingPathComponent("api/sessions/\(id)/stream")
        let token = accessToken
        return AsyncThrowingStream { continuation in
            let task = Task {
                var req = URLRequest(url: url)
                req.timeoutInterval = 3600
                req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                if let lastEventID { req.setValue(lastEventID, forHTTPHeaderField: "Last-Event-ID") }
                if let token { req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
                do {
                    let (bytes, resp) = try await URLSession.shared.bytes(for: req)
                    guard (resp as? HTTPURLResponse)?.statusCode == 200 else {
                        throw LSAPIError(
                            status: (resp as? HTTPURLResponse)?.statusCode ?? 0,
                            message: "stream rejected")
                    }
                    // Feed raw bytes: AsyncBytes.lines drops the blank lines
                    // that terminate SSE events, so it can't drive the parser.
                    var parser = SSEParser()
                    var buf: [UInt8] = []
                    buf.reserveCapacity(1024)
                    loop: for try await b in bytes {
                        guard !Task.isCancelled else { break }
                        buf.append(b)
                        guard b == 0x0A else { continue }
                        for ev in parser.feed(bytes: buf) {
                            if let frame = Self.frame(from: ev.data) {
                                continuation.yield(frame)
                                if case .sessionEnded = frame { break loop }
                            }
                        }
                        buf.removeAll(keepingCapacity: true)
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: Task.isCancelled ? CancellationError() : error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private static func frame(from data: String) -> LSStreamFrame? {
        guard let d = data.data(using: .utf8) else { return nil }
        struct Head: Codable {
            var type: String?; var status: String?; var count: Int?
            var who: String?; var until: Double?
        }
        guard let head = try? JSONDecoder().decode(Head.self, from: d) else { return nil }
        switch head.type {
        case "connected": return .connected(status: head.status ?? "live")
        case "watchers": return .watchers(head.count ?? 0)
        case "typing":
            return .typing(
                who: head.who ?? "someone",
                until: Date(timeIntervalSince1970: (head.until ?? 0) / 1000))
        case "session_ended": return .sessionEnded
        case "event", nil:
            // event frames carry the Event object itself (CONTRACT.md).
            if let ev = try? JSONDecoder.liveShortly.decode(LSEvent.self, from: d) {
                return .event(ev)
            }
            return head.type.map { .other(type: $0) }
        case let t?: return .other(type: t)
        }
    }

    // MARK: - Plumbing

    private func send<T: Decodable>(
        _ method: String, _ path: String, authed: Bool = true,
        body: [String: Any]? = nil, retried: Bool = false
    ) async throws -> T {
        let data = try await raw(method, path, authed: authed, body: body, retried: retried)
        return try JSONDecoder.liveShortly.decode(T.self, from: data)
    }

    private func sendVoid(
        _ method: String, _ path: String, body: [String: Any]? = nil
    ) async throws {
        _ = try await raw(method, path, authed: true, body: body, retried: false)
    }

    private func raw(
        _ method: String, _ path: String, authed: Bool,
        body: [String: Any]?, retried: Bool
    ) async throws -> Data {
        var req = URLRequest(url: URL(string: baseURL.absoluteString + path)!)
        req.httpMethod = method
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        if authed, let accessToken {
            req.setValue("Bearer \(accessToken)", forHTTPHeaderField: "Authorization")
        }
        if let body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        let (data, resp) = try await URLSession.shared.data(for: req)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        if status == 401, authed, !retried, refreshToken != nil {
            try await refresh()
            return try await raw(method, path, authed: authed, body: body, retried: true)
        }
        guard (200..<300).contains(status) else {
            struct Err: Codable { var error: String? }
            let msg = (try? JSONDecoder().decode(Err.self, from: data))?.error
            throw LSAPIError(status: status, message: msg ?? "HTTP \(status)")
        }
        return data.isEmpty ? Data("{}".utf8) : data
    }

    private func refresh() async throws {
        guard let refreshToken else { throw LSAPIError(status: 401, message: "signed out") }
        let r: LSTokenRefresh = try await send(
            "POST", "/auth/token", authed: false,
            body: ["grant_type": "refresh_token", "refresh_token": refreshToken],
            retried: true)
        accessToken = r.accessToken
        if let rot = r.refreshToken { self.refreshToken = rot }
        onTokensUpdated?(r.accessToken, r.refreshToken ?? refreshToken)
    }
}
