import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

/// APL-16: wraps Apple Intelligence's on-device `SystemLanguageModel` /
/// `LanguageModelSession` as an `Engine`. `#if canImport(FoundationModels)`
/// so this type (and anything referencing it) still compiles on older
/// SDKs; `@available(iOS 26, macOS 26, *)` gates actual use at runtime,
/// since the framework can be present in the SDK while unavailable on the
/// running OS/device. No tools in V1 (per the card); context is capped at
/// a 3.5K-token budget by whatever truncates `GenerateRequest.messages`
/// before this engine sees them - not done here.
public final class AppleFoundationModelsEngine: Engine {
    /// A sentinel model id for the one catalog entry this engine ever
    /// serves - "Apple Intelligence (on-device)" per the card. No such
    /// catalog entry exists yet (CORE-06's job); wiring one in is a
    /// follow-on, so `supports()` is correct but currently unreachable.
    public static let modelId = "apple-intelligence@on-device"

    /// Whether Apple Intelligence's on-device model is usable right now -
    /// used by onboarding (UI-07) to decide whether to show the "Start
    /// instantly with Apple's on-device model" card, with no catalog
    /// entry or `Engine` instance needed.
    public static var isAvailable: Bool {
        #if canImport(FoundationModels)
        if #available(iOS 26, macOS 26, *) {
            return SystemLanguageModel.default.availability == .available
        }
        #endif
        return false
    }

    public init() {}

    public func kind() -> EngineKind { .mlx }

    public func supports(model: ModelManifest) -> Bool {
        model.id == Self.modelId && Self.isAvailable
    }

    public func load(model: ModelManifest, opts: LoadOptions) throws -> LoadedModel {
        #if canImport(FoundationModels)
        guard #available(iOS 26, macOS 26, *) else {
            throw PocketError.EngineFailed(engine: .mlx, message: "Apple Intelligence requires iOS/macOS 26+")
        }
        guard SystemLanguageModel.default.availability == .available else {
            throw PocketError.NotEntitled(feature: "apple_intelligence")
        }
        return LoadedModel(handle: Self.modelId)
        #else
        throw PocketError.EngineFailed(engine: .mlx, message: "FoundationModels not available in this SDK")
        #endif
    }

    public func unload(handle: String) throws {}

    public func generate(handle: String, req: GenerateRequest, sink: TokenSink) throws -> GenerateStats {
        #if canImport(FoundationModels)
        guard #available(iOS 26, macOS 26, *) else {
            throw PocketError.EngineFailed(engine: .mlx, message: "Apple Intelligence requires iOS/macOS 26+")
        }
        let semaphore = DispatchSemaphore(value: 0)
        let box = ResultBox()
        Task {
            do {
                let stats = try await Self.run(req: req, sink: sink)
                box.result = .success(stats)
            } catch {
                sink.onError(err: .EngineFailed(engine: .mlx, message: "\(error)"))
                box.result = .failure(error)
            }
            semaphore.signal()
        }
        semaphore.wait()
        switch box.result {
        case .success(let stats): return stats
        case .failure(let error): throw PocketError.EngineFailed(engine: .mlx, message: "\(error)")
        case .none: throw PocketError.EngineFailed(engine: .mlx, message: "no result")
        }
        #else
        throw PocketError.EngineFailed(engine: .mlx, message: "FoundationModels not available in this SDK")
        #endif
    }

    public func embed(handle: String, inputs: [String]) throws -> [[Float]] {
        throw PocketError.Internal(message: "AppleFoundationModelsEngine.embed: not supported")
    }

    public func cancel(requestId: String) {}

    public func residentBytes(handle: String) -> UInt64 { 0 }

    private final class ResultBox: @unchecked Sendable {
        var result: Result<GenerateStats, Error>?
    }

    #if canImport(FoundationModels)
    @available(iOS 26, macOS 26, *)
    private static func run(req: GenerateRequest, sink: TokenSink) async throws -> GenerateStats {
        let instructions = req.messages.first { $0.role == "system" }.map(messageText)
        let session = LanguageModelSession(instructions: instructions)
        let userTurns = req.messages.filter { $0.role != "system" }.map(messageText).joined(separator: "\n\n")

        var previous = ""
        let stream = session.streamResponse(to: userTurns)
        for try await snapshot in stream {
            let cumulative = snapshot.content
            if cumulative.count > previous.count {
                let delta = String(cumulative.dropFirst(previous.count))
                sink.onDelta(delta: GenerateDelta(text: delta, reasoning: "", toolCalls: []))
                previous = cumulative
            }
        }

        let stats = GenerateStats(promptTokens: 0, completionTokens: 0, ttftMs: 0, decodeTps: 0, finish: .stop)
        sink.onDone(stats: stats)
        return stats
    }

    private static func messageText(_ message: ChatMessage) -> String {
        message.parts.map { part in
            if case .text(let text) = part { return text }
            return ""
        }.joined()
    }
    #endif
}
