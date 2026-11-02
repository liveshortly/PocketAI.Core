import Foundation

/// LSI-10: per-session transcript cache — one JSON file per session under
/// Application Support/liveshortly-cache. Reopening a session renders from
/// disk instantly; the SSE stream (resumed with `Last-Event-ID`) reconciles.
/// Bounds: last `maxEvents` events per session, newest `maxSessions` files.
enum LSTranscriptCache {
    struct Entry: Codable {
        /// Snapshot of the session header; `detail.events` is stored empty —
        /// the transcript lives in `events` so it isn't written twice.
        var detail: LSSessionDetail
        var events: [LSEvent]
        /// True when older events were dropped at save. A truncated cache
        /// can't substitute for a full fetch, so open() refetches instead.
        var truncated: Bool
    }

    static let maxEvents = 500
    static let maxSessions = 20

    /// Overridable so tests write to a temp directory.
    nonisolated(unsafe) static var baseDirectory: URL =
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
        ?? FileManager.default.temporaryDirectory

    private static var dir: URL {
        baseDirectory.appendingPathComponent("liveshortly-cache", isDirectory: true)
    }

    private static func file(_ id: String) -> URL? {
        // Session ids are server-issued; keep only filename-safe characters anyway.
        let safe = id.filter { $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }
        guard !safe.isEmpty else { return nil }
        return dir.appendingPathComponent("\(safe).json")
    }

    static func load(_ id: String) -> Entry? {
        guard let url = file(id), let data = try? Data(contentsOf: url) else { return nil }
        let dec = JSONDecoder()
        dec.dateDecodingStrategy = .millisecondsSince1970
        return try? dec.decode(Entry.self, from: data)
    }

    static func save(_ id: String, detail: LSSessionDetail, events: [LSEvent]) {
        guard let url = file(id) else { return }
        var det = detail
        det.events = []
        // Local comment echoes are pre-server placeholders — never persist them.
        let real = events.filter { !$0.id.hasPrefix("local-") }
        var entry = Entry(detail: det, events: real, truncated: false)
        if real.count > maxEvents {
            entry.events = Array(real.suffix(maxEvents))
            entry.truncated = true
        }
        let enc = JSONEncoder()
        enc.dateEncodingStrategy = .millisecondsSince1970
        guard let data = try? enc.encode(entry) else { return }
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? data.write(to: url, options: .atomic)
        evictIfNeeded()
    }

    static func remove(_ id: String) {
        guard let url = file(id) else { return }
        try? FileManager.default.removeItem(at: url)
    }

    /// ponytail: full directory scan on every save — fine at 20 files.
    private static func evictIfNeeded() {
        let fm = FileManager.default
        guard let urls = try? fm.contentsOfDirectory(
            at: dir, includingPropertiesForKeys: [.contentModificationDateKey]),
            urls.count > maxSessions else { return }
        let dated = urls.map { url -> (URL, Date) in
            let d = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?
                .contentModificationDate
            return (url, d ?? .distantPast)
        }
        for (url, _) in dated.sorted(by: { $0.1 > $1.1 }).dropFirst(maxSessions) {
            try? fm.removeItem(at: url)
        }
    }
}
