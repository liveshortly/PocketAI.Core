import Foundation

/// How a model is named on screen.
///
/// Model ids carry everything needed to fetch them — org, repo, revision,
/// quantisation, format, release date — and none of that belongs in a chat
/// header. `mlx-community/Qwen3-4B-Instruct-2507-4bit@50d4277…` is a fine
/// identifier and a terrible label; this turns it into "Qwen3 4B".
///
/// Quantisation and format are deliberately dropped: both are shown as their
/// own badges where they matter (the model library), so repeating them in
/// every label is noise.
public enum ModelNaming {
    /// Tokens that describe how a model was packaged rather than what it is.
    private static let noise: Set<String> = [
        "gguf", "mlx", "safetensors", "coreml", "mlc",
        "instruct", "chat", "it", "hf", "turbo",
        "4bit", "8bit", "3bit", "6bit", "fp16", "bf16", "fp32", "int4", "int8",
        "q2", "q3", "q4", "q5", "q6", "q8", "k", "m", "s", "l", "0", "1",
        "latest", "main", "base",
    ]

    /// A short, human label for a model id or raw name.
    ///
    /// Handles the shapes that actually occur: `org/repo@revision`,
    /// `hf.co/org/repo:quant`, ollama's `name:tag`, and the `mac:<peer>/<model>`
    /// ids used for a model running on a paired Mac.
    public static func short(_ raw: String) -> String {
        var text = raw

        // A remote id points at a model on another machine; name the model.
        if text.hasPrefix("mac:"), let slash = text.firstIndex(of: "/") {
            text = String(text[text.index(after: slash)...])
        }
        // Drop a pinned revision, then any org/host prefix.
        if let at = text.firstIndex(of: "@") { text = String(text[..<at]) }
        text = text.split(separator: "/").last.map(String.init) ?? text
        // ollama writes name:tag, and the tag is packaging.
        if let colon = text.firstIndex(of: ":") { text = String(text[..<colon]) }

        let parts = text
            .replacingOccurrences(of: "_", with: "-")
            .split(separator: "-")
            .map(String.init)
            .filter { part in
                let lowered = part.lowercased()
                if noise.contains(lowered) { return false }
                // A bare release date like 2507 or 20240731 says nothing.
                if part.allSatisfy(\.isNumber), part.count >= 4 { return false }
                return !part.isEmpty
            }

        let name = parts.joined(separator: " ")
        return name.isEmpty ? text : name
    }

    /// Label for a model running on a paired Mac: the model, then which Mac.
    /// Without the machine the user cannot tell local from borrowed.
    public static func remote(_ raw: String, machine: String?) -> String {
        let model = short(raw)
        guard let machine, !machine.isEmpty else { return model }
        return "\(model) · \(machine)"
    }

    /// True when an id refers to a model hosted on a paired Mac.
    public static func isRemote(_ raw: String) -> Bool { raw.hasPrefix("mac:") }
}
