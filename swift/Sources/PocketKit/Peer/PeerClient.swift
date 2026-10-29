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

public enum PeerClientError: Error {
    case pinningFailed
    case invalidResponse
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
                    var req = request(path: path)
                    req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    if let lastEventId { req.setValue(lastEventId, forHTTPHeaderField: "Last-Event-ID") }
                    if let bodyData {
                        req.httpMethod = "POST"
                        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
                        req.httpBody = bodyData
                    }

                    let (bytes, response) = try await session.bytes(for: req, delegate: self)
                    guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                        throw PeerClientError.invalidResponse
                    }
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
        var req = request(path: path)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, response) = try await session.data(for: req, delegate: self)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw PeerClientError.invalidResponse
        }
        return data
    }

    public func deleteJSON(path: String) async throws -> Data {
        var req = request(path: path)
        req.httpMethod = "DELETE"
        let (data, response) = try await session.data(for: req, delegate: self)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw PeerClientError.invalidResponse
        }
        return data
    }

    public func getJSON(path: String) async throws -> Data {
        let (data, response) = try await session.data(for: request(path: path), delegate: self)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw PeerClientError.invalidResponse
        }
        return data
    }

    /// Round-trip time to `/pocket/v1/hello`, for the route label (Bonjour
    /// vs. cached vs. manual) shown in the UI.
    public func measureRTT() async -> TimeInterval? {
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
