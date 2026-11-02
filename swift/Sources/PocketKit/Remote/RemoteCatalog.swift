import Foundation

/// UI-13: JSON mirrors of `pocket-ffi`'s catalog types, decoded from the
/// Control API's `/pocket/v1/catalog` response on a paired Mac. Kept
/// separate from the FFI-generated `CatalogEntry`/`ModelManifest` (used
/// for the *local* catalog over FFI) since those aren't `Codable` and
/// retrofitting that onto generated bindings is fragile across
/// regenerations - this is a second, independent transport (HTTPS to a
/// peer, not FFI to the local `PocketCore`), so a second, small DTO set
/// is the honest shape, not duplication.
public struct RemoteModelManifest: Codable, Sendable {
    public let id: String
    public let displayName: String
    public let family: String
    public let quant: String
    public let estRuntimeBytes: UInt64
    public let tags: [String]
    /// CORE-14: catalog reasoning flag; nil from a Mac on a pre-CORE-14 build.
    public let reasoning: Bool?
}

public struct RemoteFitReport: Codable, Sendable {
    public let verdict: String
    public let reason: String
    public let estTokensPerSec: Float?
}

public enum RemoteModelState: Sendable, Equatable {
    case available, queued, downloading, paused, verifying, installed
    case failed(reason: String)
}

extension RemoteModelState: Codable {
    private enum CodingKeys: String, CodingKey { case failed = "Failed" }
    private enum FailedKeys: String, CodingKey { case reason }

    public init(from decoder: Decoder) throws {
        if let single = try? decoder.singleValueContainer(), let name = try? single.decode(String.self) {
            switch name {
            case "Available": self = .available
            case "Queued": self = .queued
            case "Downloading": self = .downloading
            case "Paused": self = .paused
            case "Verifying": self = .verifying
            case "Installed": self = .installed
            default: self = .available
            }
            return
        }
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let nested = try container.nestedContainer(keyedBy: FailedKeys.self, forKey: .failed)
        self = .failed(reason: try nested.decode(String.self, forKey: .reason))
    }

    public func encode(to encoder: Encoder) throws {
        var single = encoder.singleValueContainer()
        switch self {
        case .available: try single.encode("Available")
        case .queued: try single.encode("Queued")
        case .downloading: try single.encode("Downloading")
        case .paused: try single.encode("Paused")
        case .verifying: try single.encode("Verifying")
        case .installed: try single.encode("Installed")
        case .failed(let reason):
            var container = encoder.container(keyedBy: CodingKeys.self)
            var nested = container.nestedContainer(keyedBy: FailedKeys.self, forKey: .failed)
            try nested.encode(reason, forKey: .reason)
        }
    }
}

public struct RemoteCatalogEntry: Codable, Sendable, Identifiable {
    public let manifest: RemoteModelManifest
    public let fit: RemoteFitReport
    public let state: RemoteModelState
    public let bytesDone: UInt64

    public var id: String { manifest.id }
}

/// Thin wrapper over `PeerClient`'s plain JSON calls for the Control API
/// routes a paired Mac exposes (NET-12) - catalog, start/cancel a
/// download, delete a model.
public final class RemoteCatalogClient: Sendable {
    private let peerClient: PeerClient

    public init(peerClient: PeerClient) {
        self.peerClient = peerClient
    }

    public func catalog() async throws -> [RemoteCatalogEntry] {
        let data = try await peerClient.getJSON(path: "/pocket/v1/catalog")
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        struct Envelope: Decodable { let entries: [RemoteCatalogEntry] }
        return try decoder.decode(Envelope.self, from: data).entries
    }

    public func startDownload(modelId: String) async throws {
        _ = try await peerClient.postJSON(path: "/pocket/v1/models/\(urlEncodeSlash(modelId))/download", body: [:])
    }

    public func deleteModel(modelId: String) async throws {
        _ = try await peerClient.deleteJSON(path: "/pocket/v1/models/\(urlEncodeSlash(modelId))")
    }

    private func urlEncodeSlash(_ id: String) -> String {
        id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? id
    }
}
