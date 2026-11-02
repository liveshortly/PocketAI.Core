import Foundation

/// What the agent is doing right now, named from the newest event on the
/// stream instead of the flat "working" both apps used to print whatever was
/// happening. LSI-12.
///
/// Event types are the ones the server contract emits: `prompt`, `response`,
/// `tool_call`, `pre_tool`, `file_write`, `output`, `input_requested`.
public enum LSActivity {
    /// Short verb for the status strip: "thinking", "running bash",
    /// "editing ChatView.swift", "writing", "waiting for you".
    ///
    /// `latest` is the most recent event of the run — pass nil (or an event
    /// this doesn't recognise) and it falls back to "working", which is what
    /// the UI said before and is never wrong, only vague.
    public static func label(latest: LSEvent?) -> String {
        guard let e = latest else { return "working" }
        let p = e.payload
        let str: (String) -> String? = { p[$0]?.stringValue }

        switch e.eventType {
        case "file_write":
            let path = str("file_path") ?? str("path") ?? str("file")
            return path.map { "editing \(basename($0))" } ?? "editing"

        case "tool_call", "pre_tool":
            // "tool" is what lsToolSummary reads; the hooks have also written
            // "tool_name"/"name" over time, so accept all three.
            guard let tool = str("tool") ?? str("tool_name") ?? str("name") else { return "running" }
            switch tool {
            case "Edit", "MultiEdit", "Write", "NotebookEdit":
                let path = p["input"]?["file_path"]?.stringValue ?? str("file_path")
                return path.map { "editing \(basename($0))" } ?? "editing"
            case "Read":
                let path = p["input"]?["file_path"]?.stringValue ?? str("file_path")
                return path.map { "reading \(basename($0))" } ?? "reading"
            case "Grep", "Glob":
                return "searching"
            case "WebSearch", "WebFetch":
                return "searching the web"
            default:
                return "running \(tool.lowercased())"
            }

        case "output", "response":
            return "writing"

        case "prompt":
            // The prompt landed and nothing has happened since.
            return "thinking"

        case "input_requested":
            return "waiting for you"

        default:
            return "working"
        }
    }

    /// Last path component without dragging in Foundation's NSString bridge.
    private static func basename(_ path: String) -> String {
        path.split(separator: "/").last.map(String.init) ?? path
    }
}
