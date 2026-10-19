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

    private var lineBuffer = [UInt8]()

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

    /// Feed raw bytes or data chunks (e.g. from network reads).
    /// Accumulates bytes, splits on `\n`, strips optional trailing `\r`, and delegates to `feed(line:)`.
    public mutating func feed<S: Sequence>(bytes: S) -> [SSEEvent] where S.Element == UInt8 {
        var events: [SSEEvent] = []
        for byte in bytes {
            if byte == 0x0A {
                if lineBuffer.last == 0x0D {
                    lineBuffer.removeLast()
                }
                let line = String(decoding: lineBuffer, as: UTF8.self)
                lineBuffer.removeAll(keepingCapacity: true)
                if let event = feed(line: line) {
                    events.append(event)
                }
            } else {
                lineBuffer.append(byte)
            }
        }
        return events
    }

    /// Feed a string chunk (e.g. from split reads), accumulating partial lines.
    public mutating func feed(chunk: String) -> [SSEEvent] {
        feed(bytes: chunk.utf8)
    }
}
