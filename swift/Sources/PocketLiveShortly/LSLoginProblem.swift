import Foundation

/// LSI-15: why a `live login` (or the reachability check before it) failed,
/// as one plain sentence. The raw Go error stays in the Log; this is what the
/// card shows.
public struct LSLoginProblem: Equatable, Sendable {
    public enum Kind: Equatable, Sendable { case unreachable, serverTrouble, clientTooOld, other }

    public let kind: Kind
    public let message: String

    public init(kind: Kind, server: String) {
        self.kind = kind
        let host = LSServer.host(server)
        switch kind {
        case .unreachable where LSServer.isProduction(server):
            message = "Can't reach \(host). Check your connection."
        case .unreachable where LSServer.isLocal(server):
            message = "Can't reach \(host). Your config points to a local server that isn't running. Start it, or switch to liveshortly.com."
        case .unreachable:
            message = "Can't reach \(host). Check your connection, or switch to liveshortly.com."
        case .serverTrouble:
            message = "LiveShortly is having trouble. Try again shortly."
        case .clientTooOld:
            message = "This version of live is too old for the server. Update live (`live update`)."
        case .other:
            message = "Sign-in didn't finish. The Log has the details."
        }
    }

    /// Classifies `live login` output (a Go error line). Nil when it isn't a
    /// failure.
    public init?(loginOutput out: String, server: String) {
        guard out.contains("login failed") else { return nil }
        let s = out.lowercased()
        let kind: Kind
        if ["connection refused", "no such host", "i/o timeout", "network is unreachable",
            "can't reach", "connection reset"].contains(where: s.contains) {
            kind = .unreachable
        } else if s.contains("x509") || s.contains("tls:") || s.contains("certificate")
            || s.range(of: #"http 5\d\d"#, options: .regularExpression) != nil {
            kind = .serverTrouble
        } else if s.range(of: #"device start: (bad response \()?http 4\d\d"#, options: .regularExpression) != nil {
            kind = .clientTooOld
        } else {
            kind = .other
        }
        self.init(kind: kind, server: server)
    }
}

/// The server `live` talks to, resolved the way the CLI does (host.json, then
/// LIVESHORTLY_API_URL, then the hosted default).
public enum LSServer {
    public static let production = "https://liveshortly.com"

    public static func resolve(home: String, env: [String: String]) -> String {
        let path = home + "/.liveshortly/host.json"
        if let data = FileManager.default.contents(atPath: path),
           let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let api = (obj["api_url"] as? String)?.trimmingCharacters(in: .whitespaces), !api.isEmpty {
            return trim(api)
        }
        if let v = env["LIVESHORTLY_API_URL"]?.trimmingCharacters(in: .whitespaces), !v.isEmpty {
            return trim(v)
        }
        return production
    }

    /// "liveshortly.com" or "localhost:8080 · custom".
    public static func label(_ server: String) -> String {
        isProduction(server) ? host(server) : host(server) + " · custom"
    }

    public static func host(_ server: String) -> String {
        guard let c = URLComponents(string: server), let h = c.host else { return server }
        return c.port.map { "\(h):\($0)" } ?? h
    }

    public static func isProduction(_ server: String) -> Bool { trim(server) == production }

    public static func isLocal(_ server: String) -> Bool {
        let h = URLComponents(string: server)?.host ?? ""
        return h == "localhost" || h == "::1" || h.hasPrefix("127.")
    }

    private static func trim(_ s: String) -> String {
        s.hasSuffix("/") ? String(s.dropLast()) : s
    }
}
