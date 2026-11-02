import Foundation

/// UI-26: family-derived reasoning capability, until the catalog manifest
/// carries a real `reasoning` flag. Lowercase substring match on the model id
/// (ids embed the HF repo name, e.g. "mlx-community/Qwen3-8B-4bit@…").
public enum ReasoningSupport {
    /// Families that emit thinking tokens (hybrid or always-on).
    private static let reasoningFamilies = [
        "qwen3",        // hybrid thinking (/no_think toggle)
        "qwq",          // QwQ reasoning models
        "deepseek-r1",  // R1 and its distills
        "gpt-oss",      // openai gpt-oss reasoning models
        "smollm3",      // hybrid /think //no_think
    ]

    public static func supportsReasoning(modelID: String) -> Bool {
        let id = modelID.lowercased()
        // ponytail: naive heuristic — Qwen3 "*-Instruct-2507" variants are the
        // non-thinking split; replace with a manifest capability flag when the
        // catalog grows one.
        if id.contains("instruct-2507") { return false }
        return reasoningFamilies.contains { id.contains($0) }
    }
}
