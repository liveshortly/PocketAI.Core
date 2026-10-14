import Foundation

public struct SSEEvent: Sendable, Equatable {
    public var id: String?
    public var event: String?
    public var data: String
}

/// APL-09: a minimal Server-Sent Events parser over a line stream (e.g.
/// `URLSession.AsyncBytes.lines`) - `id:`, `event:`, `data:` (joined with
/// `\n` across repeated `data:` lines), `:` comment lines (ignored), and
/// blank-line-terminated dispatch, per the WHATWG EventSource spec's wire
/// format. `id` persists across events until a new `id:` line overrides it,
/// matching the spec's "last event ID buffer" behavior, so a caller can
/// always read `lastEventId` to reconnect with `Last-Event-ID`.
public struct SSEParser {
    public private(set) var lastEventId: String?
    private var pendingEvent: String?
    private var pendingData: [String] = []

    public init() {}

    /// Feed one line at a time (without its trailing newline terminator).
    /// Returns a completed event when a blank line dispatches one.
    public mutating func feed(line: String) -> SSEEvent? {
        if line.isEmpty {
            guard !pendingData.isEmpty || pendingEvent != nil else { return nil }
            let event = SSEEvent(id: lastEventId, event: pendingEvent, data: pendingData.joined(separator: "\n"))
            pendingEvent = nil
            pendingData = []
            return event
        }
        if line.hasPrefix(":") { return nil }

        let name: String
        var value: String
        if let colonIndex = line.firstIndex(of: ":") {
            name = String(line[line.startIndex..<colonIndex])
            value = String(line[line.index(after: colonIndex)...])
            if value.hasPrefix(" ") { value.removeFirst() }
        } else {
            name = line
            value = ""
        }

        switch name {
        case "id": lastEventId = value
        case "event": pendingEvent = value
        case "data": pendingData.append(value)
        default: break
        }
        return nil
    }
}
