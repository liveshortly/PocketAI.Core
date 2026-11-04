import CryptoKit
import Foundation

public struct PeerEndpoint: Sendable, Equatable {
    public let host: String
    public let port: UInt16
    /// Base64 SHA-256 of the host's leaf TLS certificate DER (A§7) - the
    /// only thing `PeerClient` trusts, regardless of system CA roots.
    public let fingerprint: String

    public init(host: String, port: UInt16, fingerprint: String) {
        self.host = host
        self.port = port
        self.fingerprint = fingerprint
    }
}

public enum PeerClientError: LocalizedError, CustomStringConvertible {
    case pinningFailed
    case invalidResponse
    /// NET-17: this phone's Disconnect is on for the Mac - no traffic goes out.
    case disconnected
    /// NET-17: the Mac disconnected this phone (`disconnected_by_host`).
    case disconnectedByHost
    /// NET-22: the Mac answered 403 `not_entitled` (sync is Pro).
    case notEntitled
    /// NET-22: the Mac answered 403 `sync_disabled` (its per-phone "Sync chats" is off).
    case syncDisabled

    /// `RemoteEngine` stringifies errors into the chat, so say it plainly.
    public var description: String {
        switch self {
        case .pinningFailed: return String(localized: "This Mac's certificate didn't match the paired one.", bundle: .module)
        case .invalidResponse: return String(localized: "The Mac didn't answer. Open PocketAI on your Mac.", bundle: .module)
        case .disconnected: return String(localized: "Mac disconnected · Connect it from the Mac tab.", bundle: .module)
        case .disconnectedByHost: return String(localized: "Your Mac disconnected this iPhone · Connect it from the Mac tab.", bundle: .module)
        case .notEntitled: return String(localized: "Chat sync isn't available with this Mac yet.", bundle: .module)
        case .syncDisabled: return String(localized: "Chat sync is off for this iPhone on your Mac.", bundle: .module)
        }
    }

    public var errorDescription: String? { description }
}

public extension Notification.Name {
    /// Posted with the Mac's cert fingerprint as `object` when the user
    /// disconnects it; every `PeerClient` for that Mac cancels its requests.
    static let pocketPeerDisconnected = Notification.Name("app.getpocketai.peerDisconnected")
}

/// APL-09: talks to a paired Mac's HTTPS server, trusting only the
/// pinned certificate fingerprint from pairing (A§7) rather than the
/// system's CA roots. SSE is the only streaming transport V1 needs.
public final class PeerClient: NSObject, @unchecked Sendable {
    public let endpoint: PeerEndpoint
    public let token: String
    /// Bounded on purpose. The defaults give a resource timeout of seven days,
    /// so a wedged link — a Mac that slept, a VPN that dropped mid-stream —
    /// left the phone waiting rather than failing, with no way back except
    /// un-pairing. The idle timeout is deliberately generous because the
    /// server sends no SSE keepalive, so a stream is legitimately silent for
    /// as long as the Mac takes to load a model; it only has to be shorter
    /// than "forever".
    private lazy var session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 120
        config.timeoutIntervalForResource = 2 * 60 * 60
        return URLSession(configuration: config, delegate: self, delegateQueue: nil)
    }()

    public init(endpoint: PeerEndpoint, token: String) {
        self.endpoint = endpoint
        self.token = token
        super.init()
        NotificationCenter.default.addObserver(
            self, selector: #selector(peerDisconnected(_:)),
            name: .pocketPeerDisconnected, object: nil)
    }

    /// NET-17: cancel in-flight requests and SSE streams to this Mac.
    @objc private func peerDisconnected(_ note: Notification) {
        guard note.object as? String == endpoint.fingerprint else { return }
        session.getAllTasks { $0.forEach { $0.cancel() } }
    }

    /// No traffic to a Mac the user disconnected - except the session
    /// open/close that tells the Mac about it.
    private func ensureAllowed(path: String) throws {
        if !path.hasPrefix("/pocket/v1/session/"),
           PeerAddressBook.shared.isUserDisconnected(fingerprint: endpoint.fingerprint) {
            throw PeerClientError.disconnected
        }
    }

    private func check(_ response: URLResponse) throws {
        guard let http = response as? HTTPURLResponse else { throw PeerClientError.invalidResponse }
        if http.value(forHTTPHeaderField: "x-pocket-error") == "disconnected_by_host" {
            throw PeerClientError.disconnectedByHost
        }
        guard (200..<300).contains(http.statusCode) else { throw PeerClientError.invalidResponse }
    }

    private func request(path: String) -> URLRequest {
        var request = URLRequest(url: URL(string: "https://\(endpoint.host):\(endpoint.port)\(path)")!)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        return request
    }

    /// Streams Server-Sent Events from `path`, reconnecting the caller's
    /// concern (not this method's) - one call is one connection attempt.
    /// `body` turns the request into a POST with a JSON body (e.g.
    /// `/v1/chat/completions`'s `stream: true`); omit it for a plain GET
    /// SSE stream.
    public func events(path: String, lastEventId: String? = nil, body: [String: Any]? = nil) -> AsyncThrowingStream<SSEEvent, Error> {
        // Serialized here (outside the Task) since [String: Any] isn't
        // Sendable and can't be captured by the detached closure below.
        let bodyData = body.flatMap { try? JSONSerialization.data(withJSONObject: $0) }
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    try ensureAllowed(path: path)
                    var req = request(path: path)
                    req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    if let lastEventId { req.setValue(lastEventId, forHTTPHeaderField: "Last-Event-ID") }
                    if let bodyData {
                        req.httpMethod = "POST"
                        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
                        req.httpBody = bodyData
                    }

                    let (bytes, response) = try await session.bytes(for: req, delegate: self)
                    try check(response)
                    var parser = SSEParser()
                    var lineBuffer = [UInt8]()
                    for try await byte in bytes {
                        if byte == 0x0A {
                            if lineBuffer.last == 0x0D {
                                lineBuffer.removeLast()
                            }
                            let line = String(decoding: lineBuffer, as: UTF8.self)
                            lineBuffer.removeAll(keepingCapacity: true)
                            if let event = parser.feed(line: line) {
                                continuation.yield(event)
                            }
                        } else {
                            lineBuffer.append(byte)
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// Plain JSON POST/GET for non-streaming control routes (pairing,
    /// catalog, status, ...), still over the same pinned-TLS session.
    public func postJSON(path: String, body: [String: Any]) async throws -> Data {
        try ensureAllowed(path: path)
        var req = request(path: path)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, response) = try await session.data(for: req, delegate: self)
        try check(response)
        return data
    }

    /// NET-23: like `postJSON` but returns non-2xx replies too, so the
    /// manual pairing flow can read the Mac's `error.code`.
    public func postJSONWithStatus(path: String, body: [String: Any]) async throws -> (Data, Int) {
        try ensureAllowed(path: path)
        var req = request(path: path)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, response) = try await session.data(for: req, delegate: self)
        guard let http = response as? HTTPURLResponse else { throw PeerClientError.invalidResponse }
        return (data, http.statusCode)
    }

    /// NET-23: the manual pairing's one unpinned request. GETs
    /// `/pocket/v1/hello` accepting whatever certificate the server shows and
    /// returns that certificate's fingerprint with the reply. The caller must
    /// not trust the fingerprint until the user has compared the SAS - all
    /// later requests go through a normal `PeerClient` pinned to it, so a
    /// server that swaps certificates mid-pairing fails the pin.
    public static func probeFingerprint(host: String, port: UInt16) async throws -> (fingerprint: String, hello: Data) {
        guard let url = URL(string: "https://\(host):\(port)/pocket/v1/hello") else {
            throw PeerClientError.invalidResponse
        }
        let probe = FingerprintProbe()
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 10
        let session = URLSession(configuration: config, delegate: probe, delegateQueue: nil)
        defer { session.finishTasksAndInvalidate() }
        let (data, response) = try await session.data(from: url)
        guard (response as? HTTPURLResponse)?.statusCode == 200, let fingerprint = probe.fingerprint else {
            throw PeerClientError.invalidResponse
        }
        return (fingerprint, data)
    }

    /// Base64url (no padding) SHA-256 of the leaf certificate's DER - the
    /// same form the Mac's pairing QR carries.
    static func leafFingerprint(_ trust: SecTrust) -> String? {
        guard let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate], let leaf = chain.first else {
            return nil
        }
        return Data(SHA256.hash(data: SecCertificateCopyData(leaf) as Data)).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .trimmingCharacters(in: CharacterSet(charactersIn: "="))
    }

    public func deleteJSON(path: String) async throws -> Data {
        try ensureAllowed(path: path)
        var req = request(path: path)
        req.httpMethod = "DELETE"
        let (data, response) = try await session.data(for: req, delegate: self)
        try check(response)
        return data
    }

    public func getJSON(path: String) async throws -> Data {
        try ensureAllowed(path: path)
        let (data, response) = try await session.data(for: request(path: path), delegate: self)
        try check(response)
        return data
    }

    /// NET-22: raw bytes in, raw bytes out, for the core's sync driver.
    /// Blocking on purpose (`SyncTransport` is synchronous); never call it on
    /// the main thread. The completion runs on this session's own queue, so
    /// waiting here can't deadlock. A 403 surfaces as `.notEntitled`.
    public func blockingRequest(method: String, path: String, body: Data) throws -> Data {
        try ensureAllowed(path: path)
        var req = request(path: path)
        req.httpMethod = method
        if !body.isEmpty {
            req.httpBody = body
            req.setValue(method == "PUT" ? "application/octet-stream" : "application/json",
                         forHTTPHeaderField: "Content-Type")
        }
        let done = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var result: Result<Data, Error> = .failure(PeerClientError.invalidResponse)
        session.dataTask(with: req) { data, response, error in
            defer { done.signal() }
            if let error { result = .failure(error); return }
            guard let response else { return }
            if (response as? HTTPURLResponse)?.statusCode == 403 {
                result = .failure(Self.forbidden(data))
                return
            }
            result = Result { try self.check(response); return data ?? Data() }
        }.resume()
        done.wait()
        return try result.get()
    }

    /// 403 body `{"error":{"code":...}}`: `sync_disabled` vs everything else.
    static func forbidden(_ data: Data?) -> PeerClientError {
        struct Body: Decodable { struct E: Decodable { let code: String? }; let error: E? }
        let code = data.flatMap { try? JSONDecoder().decode(Body.self, from: $0) }?.error?.code
        return code == "sync_disabled" ? .syncDisabled : .notEntitled
    }

    /// NET-22: long-polls the Mac's `/pocket/v1/sync/wait`. Returns the Mac's
    /// change cursor at once when `since` is nil or stale, else when it moves
    /// (a chat changed there, or its "Sync now"), else after ~25 s. Not a
    /// Network Proof row of its own: the sync it triggers logs one.
    public func waitForSyncChange(since: Int64?) async throws -> Int64 {
        struct Reply: Decodable { let cursor: Int64 }
        let path = "/pocket/v1/sync/wait" + (since.map { "?since=\($0)" } ?? "")
        try ensureAllowed(path: path)
        let (data, response) = try await session.data(for: request(path: path), delegate: self)
        if (response as? HTTPURLResponse)?.statusCode == 403 { throw Self.forbidden(data) }
        try check(response)
        return try JSONDecoder().decode(Reply.self, from: data).cursor
    }

    // MARK: - Session (NET-17 / NET-18)

    /// 15 s lease renewal while the app is foreground; throws
    /// `.disconnectedByHost` once the Mac has let this phone go.
    public func heartbeat() async throws {
        _ = try await postJSON(path: "/pocket/v1/heartbeat", body: [:])
    }

    /// Goodbye: `byUser` for Disconnect (Mac shows Disconnected), otherwise
    /// the app is going to the background (Mac shows Idle).
    public func closeSession(byUser: Bool) async throws {
        _ = try await postJSON(path: "/pocket/v1/session/close", body: ["reason": byUser ? "user" : "background"])
    }

    /// Connect: also lifts a Disconnect the Mac made.
    public func openSession() async throws {
        _ = try await postJSON(path: "/pocket/v1/session/open", body: [:])
    }

    /// Round-trip time to `/pocket/v1/hello`, for the route label (Bonjour
    /// vs. cached vs. manual) shown in the UI.
    public func measureRTT() async -> TimeInterval? {
        guard (try? ensureAllowed(path: "/pocket/v1/hello")) != nil else { return nil }
        let start = Date()
        guard
            let (_, response) = try? await session.data(for: request(path: "/pocket/v1/hello"), delegate: self),
            (response as? HTTPURLResponse)?.statusCode == 200
        else { return nil }
        return Date().timeIntervalSince(start)
    }
}

extension PeerClient: URLSessionDelegate, URLSessionTaskDelegate {
    public func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        handleChallenge(challenge, completionHandler: completionHandler)
    }

    public func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        handleChallenge(challenge, completionHandler: completionHandler)
    }

    private func handleChallenge(
        _ challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard
            challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
            let serverTrust = challenge.protectionSpace.serverTrust,
            let chain = SecTrustCopyCertificateChain(serverTrust) as? [SecCertificate],
            let leaf = chain.first
        else {
            fputs("[TLS_CHALLENGE_FAILED] guard failed\n", stderr)
            fflush(stderr)
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }

        let certData = SecCertificateCopyData(leaf) as Data
        let rawHash = Data(SHA256.hash(data: certData))
        let stdB64 = rawHash.base64EncodedString()
        let b64url = stdB64
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .trimmingCharacters(in: CharacterSet(charactersIn: "="))
        let normEndpoint = endpoint.fingerprint
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .trimmingCharacters(in: CharacterSet(charactersIn: "="))
        fputs("[TLS_CHALLENGE] leafHash=\(stdB64) b64url=\(b64url) expected=\(endpoint.fingerprint)\n", stderr)
        fflush(stderr)
        if stdB64 == endpoint.fingerprint || b64url == endpoint.fingerprint || b64url == normEndpoint {
            completionHandler(.useCredential, URLCredential(trust: serverTrust))
        } else {
            fputs("[TLS_CHALLENGE_FAILED] mismatch!\n", stderr)
            fflush(stderr)
            completionHandler(.cancelAuthenticationChallenge, nil)
        }
    }
}

/// NET-23: trust-on-first-use for exactly one probe session; records the
/// leaf it accepted. Used only by `PeerClient.probeFingerprint`.
private final class FingerprintProbe: NSObject, URLSessionDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var seen: String?

    var fingerprint: String? { lock.withLock { seen } }

    func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust,
              let fingerprint = PeerClient.leafFingerprint(trust) else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        // One connection per probe; if a second handshake shows a different
        // cert, refuse it rather than guess which one the reply came from.
        let accepted: Bool = lock.withLock {
            if seen == nil { seen = fingerprint }
            return seen == fingerprint
        }
        completionHandler(accepted ? .useCredential : .cancelAuthenticationChallenge,
                          accepted ? URLCredential(trust: trust) : nil)
    }
}
