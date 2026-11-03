import Foundation

/// APL-04: matches the `purpose` field `pocket-ffi`'s `EgressEntry` stores
/// as a free-form string - this enum is the one place that string is
/// spelled, so every call site uses the same vocabulary.
public enum NetPurpose: String, Sendable {
    case modelDownload = "model_download"
    case catalogRefresh = "catalog_refresh"
    case hubSearch = "hub_search"
    case licenseCheck = "license_check"
    case other = "other"
    /// NET-22: chat sync with the paired Mac (LAN, pinned TLS, not internet).
    case peerSync = "peer_sync"

    /// What the Network Proof screen shows for a stored `purpose` string.
    public static func label(for purpose: String) -> String {
        purpose == NetPurpose.peerSync.rawValue ? "To your Mac" : purpose
    }
}

public enum NetError: Error, Equatable {
    case offline
}

/// The only allowed internet client in Swift app code
/// (`scripts/check-no-raw-urlsession.sh` enforces this in CI). Checks
/// offline mode before every request and records app-controlled egress on
/// every completed one, so the Network Proof screen (L5) and the offline
/// gate stay accurate. LAN traffic to paired Macs does **not** go through
/// `Net` - that's `PeerClient` (APL-09), which stays usable in offline mode
/// and is logged under Devices, not here.
public final class Net: Sendable {
    private let isOffline: @Sendable () -> Bool
    private let onEgress: @Sendable (_ host: String, _ purpose: NetPurpose, _ bytesOut: Int64, _ bytesIn: Int64) -> Void
    private let session: URLSession

    public init(
        session: URLSession = .shared,
        isOffline: @escaping @Sendable () -> Bool,
        onEgress: @escaping @Sendable (_ host: String, _ purpose: NetPurpose, _ bytesOut: Int64, _ bytesIn: Int64) -> Void
    ) {
        self.session = session
        self.isOffline = isOffline
        self.onEgress = onEgress
    }

    /// Full-body request/response; records the real request/response byte
    /// counts as egress immediately, since both are known once this returns.
    public func data(for request: URLRequest, purpose: NetPurpose) async throws -> (Data, URLResponse) {
        guard !isOffline() else { throw NetError.offline }
        let (data, response) = try await session.data(for: request)
        recordEgress(for: request, purpose: purpose, bytesOut: Int64(request.httpBody?.count ?? 0), bytesIn: Int64(data.count))
        return (data, response)
    }

    /// Streaming response (the downloader's shape). Unlike `data(for:)`,
    /// the eventual byte count isn't known yet when this returns, so no
    /// egress is recorded here - the caller reports it via
    /// `recordStreamedEgress` once it has actually consumed the stream.
    public func bytes(for request: URLRequest, purpose: NetPurpose) async throws -> (URLSession.AsyncBytes, URLResponse) {
        guard !isOffline() else { throw NetError.offline }
        return try await session.bytes(for: request)
    }

    public func recordStreamedEgress(host: String, purpose: NetPurpose, bytesIn: Int64) {
        onEgress(host, purpose, 0, bytesIn)
    }

    private func recordEgress(for request: URLRequest, purpose: NetPurpose, bytesOut: Int64, bytesIn: Int64) {
        onEgress(request.url?.host ?? "unknown", purpose, bytesOut, bytesIn)
    }

    /// Background session configuration for the downloader (APL-05): not
    /// discretionary (the user explicitly asked for this download), waits
    /// for connectivity rather than failing immediately, and relaunches the
    /// app on completion so a finished download isn't silently dropped.
    public static func makeBackgroundSessionConfiguration(identifier: String) -> URLSessionConfiguration {
        let config = URLSessionConfiguration.background(withIdentifier: identifier)
        config.isDiscretionary = false
        config.sessionSendsLaunchEvents = true
        config.waitsForConnectivity = true
        return config
    }
}
