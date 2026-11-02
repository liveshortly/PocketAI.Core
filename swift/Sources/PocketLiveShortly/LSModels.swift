import Foundation

// Wire types for the LiveShortly API (liveshortly-server CONTRACT.md).
// Field names mirror the JSON exactly via snake_case decoding.

public enum LS {
    /// Fixed allowlist mirrored by server + daemon (CONTRACT.md).
    public static let claudeModels = ["sonnet", "opus", "haiku"]
}

public struct LSUser: Codable, Sendable, Equatable {
    public var email: String
    public var name: String
}

public struct LSSession: Codable, Identifiable, Sendable, Equatable {
    public var id: String
    public var title: String
    public var ownerHandle: String
    public var model: String?
    public var framework: String?
    public var status: String // "live" | "ended"
    public var eventCount: Int
    public var viewCount: Int
    public var createdAt: Date
    public var endedAt: Date?
    public var visibility: String?
    public var clientHandle: String?
    public var shareCount: Int?
    public var publishedAt: Date?
    public var forkCount: Int?
    public var endedReason: String?
    /// List-row enrichment: the newest prompt/response/viewer_comment summary.
    public var lastEvent: LSLastEvent?

    public var isLive: Bool { status == "live" }
}

/// `last_event` on a session list row (CONTRACT.md).
public struct LSLastEvent: Codable, Sendable, Equatable {
    public var type: String
    public var snippet: String
    public var ts: Date
}

/// A per-person access grant on a session (owner-managed).
public struct LSShare: Codable, Identifiable, Sendable, Equatable {
    public var id: String
    public var sessionId: String?
    public var granteeEmail: String?
    public var role: String
    public var createdAt: Date?
}

public struct LSForkRef: Codable, Sendable, Equatable {
    public var id: String
    public var title: String
    public var ownerHandle: String
    public var seq: Int
}

public struct LSEvent: Codable, Identifiable, Sendable, Equatable {
    public var id: String
    public var seq: Int
    public var actor: String?
    public var eventType: String
    public var payload: LSJSON
    public var ts: Date

    /// Public so host apps can build the optimistic echo of a comment they
    /// just sent, before the server's own event arrives on the stream.
    public init(
        id: String, seq: Int, actor: String?, eventType: String, payload: LSJSON, ts: Date
    ) {
        self.id = id
        self.seq = seq
        self.actor = actor
        self.eventType = eventType
        self.payload = payload
        self.ts = ts
    }
}

public struct LSSessionDetail: Codable, Sendable {
    public var id: String
    public var title: String
    public var ownerHandle: String
    public var model: String?
    public var framework: String?
    public var status: String
    public var eventCount: Int
    public var createdAt: Date
    public var endedAt: Date?
    public var watcherCount: Int?
    public var canComment: Bool?
    public var visibility: String?
    public var linkRole: String?
    public var publishedAt: Date?
    public var isOwner: Bool?
    public var agentConnected: Bool?
    public var forkedFrom: LSForkRef?
    public var events: [LSEvent]

    public var isLive: Bool { status == "live" }
}

public struct LSHost: Codable, Identifiable, Sendable, Equatable {
    public var id: String
    public var name: String
    public var os: String
    public var arch: String
    public var dirs: [String]
    public var agents: [String]
    /// Models the daemon reported. These are the machine's self-hosted
    /// (ollama) models — a hosted agent like Claude has its own fixed set, so
    /// this list is only meaningful for a self-hosted agent.
    public var models: [String]?
    /// When the daemon last checked in. Its registration carries a TTL, so a
    /// stale timestamp means the machine is probably gone.
    public var seenAt: Date?
    /// Optional daemon capabilities (LSI-14 contract proposal). Absent on
    /// every daemon today; `extra_dirs` unlocks multi-repo sessions.
    public var caps: [String]?

    /// True once the daemon (and server) accept `extra_dirs` on a spawn.
    /// Until then the multi-repo control stays hidden, not faked.
    public var acceptsExtraDirs: Bool { caps?.contains("extra_dirs") == true }

    /// True while the daemon looks alive. The server expires a registration
    /// after a few minutes, so anything older than that is not worth spawning on.
    public var isOnline: Bool {
        guard let seenAt else { return true }
        return Date().timeIntervalSince(seenAt) < 180
    }

    /// Models the user may pick for a given agent on this host.
    /// Claude runs a fixed set; a self-hosted agent runs whatever the daemon
    /// found locally; anything else takes the agent's own default.
    public func models(forAgent agent: String) -> [String] {
        switch agent {
        case "claude", "claude-code": return LS.claudeModels
        case "ollama": return models ?? []
        default: return []
        }
    }
}

public struct LSSpawnInfo: Codable, Sendable {
    public var status: String // "requested" | "failed"
    public var error: String?
}

public struct LSCreatedSession: Codable, Sendable {
    public var id: String
    public var spawn: LSSpawnInfo?
}

public struct LSDeviceStart: Codable, Sendable {
    public var deviceCode: String
    public var userCode: String
    public var verificationUri: String
    public var verificationUriComplete: String?
    public var interval: Int?
    public var expiresIn: Int?
}

public struct LSDevicePoll: Codable, Sendable {
    public var status: String?
    public var accessToken: String?
    public var refreshToken: String?
    public var expiresIn: Int?
    public var user: LSUser?
}

public struct LSTokenRefresh: Codable, Sendable {
    public var accessToken: String
    public var expiresIn: Int?
    public var refreshToken: String?
}

/// One frame from the session SSE stream.
public enum LSStreamFrame: Sendable {
    case connected(status: String)
    case event(LSEvent)
    case watchers(Int)
    case typing(who: String, until: Date)
    case sessionEnded
    case other(type: String)
}

// MARK: - Minimal JSON value (event payloads are free-form)

public enum LSJSON: Codable, Sendable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([LSJSON])
    case object([String: LSJSON])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([LSJSON].self) { self = .array(a) }
        else { self = .object(try c.decode([String: LSJSON].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }

    public subscript(key: String) -> LSJSON? {
        if case .object(let o) = self { return o[key] }
        return nil
    }

    public var stringValue: String? {
        if case .string(let s) = self { return s }
        return nil
    }
}

// MARK: - Decoding config

/// RFC3339 parsing (Go emits both with and without fractional seconds).
/// ISO8601DateFormatter is documented thread-safe; the annotation just says so.
enum LSDates {
    nonisolated(unsafe) private static let frac: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    nonisolated(unsafe) private static let plain = ISO8601DateFormatter()

    static func parse(_ s: String) -> Date? {
        frac.date(from: s) ?? plain.date(from: s)
    }
}

extension JSONDecoder {
    /// LiveShortly wire format: snake_case keys, RFC3339 dates (with or
    /// without fractional seconds — Go emits both).
    static var liveShortly: JSONDecoder {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        d.dateDecodingStrategy = .custom { dec in
            let s = try dec.singleValueContainer().decode(String.self)
            if let t = LSDates.parse(s) { return t }
            throw DecodingError.dataCorrupted(.init(
                codingPath: dec.codingPath, debugDescription: "bad date: \(s)"))
        }
        return d
    }
}
