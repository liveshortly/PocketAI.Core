// TODO(APL-06 follow-up): register via core.registerEngine at app launch
import Foundation
import MLX
import MLXLMCommon
import MLXLLM
import MLXEmbedders
import PocketKit
import Tokenizers

/// APL-06: wraps Apple Silicon MLX on-device inference as an `Engine`.
///
/// Implements the `Engine` foreign trait against `mlx-swift` and `mlx-swift-lm`.
/// Strict concurrency: loaded `ModelContainer` instances live in a dedicated `actor`
/// (`MLXModelRegistry`) so concurrent loads and generations are safe without `@unchecked Sendable`
/// or `nonisolated(unsafe)`.
///
/// iPhone backgrounding (scenePhase observe -> cancel active generation) is an app-level concern
/// wired to `cancel()` by the app layer.
public final class PocketMLXEngine: Engine, @unchecked Sendable {
    private let registry = MLXModelRegistry()

    public init() {}

    public func kind() -> EngineKind {
        .mlx
    }

    public func supports(model: ModelManifest) -> Bool {
        model.format == .mlxSafetensors
    }

    public func load(model: ModelManifest, opts: LoadOptions) throws -> LoadedModel {
        guard supports(model: model) else {
            throw PocketError.InvalidInput(message: "Model format \(model.format) not supported by MLX engine")
        }

        let localURL = Self.resolveModelDirectory(manifest: model)
        guard FileManager.default.fileExists(atPath: localURL.path) else {
            throw PocketError.NotFound(what: "Model weights not found at \(localURL.path)")
        }

        // Apply MLX memory limits per platform:
        // iPhone 64 MB, Mac 512 MB. The overall memory budget is applied by caller.
        // Note: In MLX 0.32.x, `MLX.Memory.cacheLimit` is used rather than the deprecated `GPU.set(cacheLimit:)`.
        #if os(iOS)
        let cacheLimitBytes = 64 * 1024 * 1024
        #else
        let cacheLimitBytes = 512 * 1024 * 1024
        #endif
        MLX.Memory.cacheLimit = cacheLimitBytes

        let semaphore = DispatchSemaphore(value: 0)
        let box = ResultBox<LoadedModel>()

        Task {
            do {
                let startMemory = MLX.Memory.snapshot().activeMemory

                // Note: In mlx-swift-lm 3.32.x, local weights are loaded via
                // `LLMModelFactory.shared.loadContainer(from:using:)`, which accepts
                // a local URL and a `TokenizerLoader` with no remote downloader.
                let tokenizerLoader = LocalTokenizerLoader()
                let container = try await LLMModelFactory.shared.loadContainer(
                    from: localURL,
                    using: tokenizerLoader
                )

                let endMemory = MLX.Memory.snapshot().activeMemory
                let resident = UInt64(max(0, endMemory - startMemory))

                let handle = UUID().uuidString
                await self.registry.register(handle: handle, container: container, residentBytes: resident)
                box.result = .success(LoadedModel(handle: handle))
            } catch {
                box.result = .failure(error)
            }
            semaphore.signal()
        }

        semaphore.wait()

        switch box.result {
        case .success(let loaded):
            return loaded
        case .failure(let error):
            throw PocketError.EngineFailed(engine: .mlx, message: "MLX load failed: \(error)")
        case .none:
            throw PocketError.EngineFailed(engine: .mlx, message: "MLX load produced no result")
        }
    }

    public func unload(handle: String) throws {
        let semaphore = DispatchSemaphore(value: 0)
        Task {
            await self.registry.remove(handle: handle)
            semaphore.signal()
        }
        semaphore.wait()
    }

    public func generate(handle: String, req: GenerateRequest, sink: TokenSink) throws -> GenerateStats {
        let semaphore = DispatchSemaphore(value: 0)
        let box = ResultBox<GenerateStats>()
        let requestId = req.requestId

        let task = Task {
            do {
                let stats = try await self.streamGenerate(handle: handle, req: req, sink: sink)
                box.result = .success(stats)
            } catch is CancellationError {
                let cancelledStats = GenerateStats(
                    promptTokens: 0,
                    completionTokens: 0,
                    ttftMs: 0,
                    decodeTps: 0,
                    finish: .cancelled
                )
                sink.onDone(stats: cancelledStats)
                box.result = .success(cancelledStats)
            } catch {
                sink.onError(err: .EngineFailed(engine: .mlx, message: "\(error)"))
                box.result = .failure(error)
            }
            semaphore.signal()
        }

        let reg = self.registry
        Task {
            await reg.registerTask(requestId: requestId, task: task)
        }

        semaphore.wait()

        Task {
            await reg.removeTask(requestId: requestId)
        }

        switch box.result {
        case .success(let stats):
            return stats
        case .failure(let error):
            throw PocketError.EngineFailed(engine: .mlx, message: "\(error)")
        case .none:
            throw PocketError.EngineFailed(engine: .mlx, message: "MLX generate produced no result")
        }
    }

    public func embed(handle: String, inputs: [String]) throws -> [[Float]] {
        throw PocketError.Internal(message: "PocketMLXEngine.embed: not supported, embeddings handled by APL-07")
    }

    public func cancel(requestId: String) {
        let reg = self.registry
        Task {
            await reg.cancelTask(requestId: requestId)
        }
    }

    public func residentBytes(handle: String) -> UInt64 {
        let semaphore = DispatchSemaphore(value: 0)
        let box = ValueBox<UInt64>()
        let reg = self.registry
        Task {
            let bytes = await reg.getResidentBytes(handle: handle)
            box.value = bytes
            semaphore.signal()
        }
        semaphore.wait()
        return box.value ?? 0
    }

    // MARK: - Internal Generation Implementation

    private func streamGenerate(handle: String, req: GenerateRequest, sink: TokenSink) async throws -> GenerateStats {
        guard let container = await registry.getContainer(handle: handle) else {
            throw PocketError.NotFound(what: "Loaded model with handle \(handle) not found")
        }

        // Map SamplingParams (Sendable)
        let repetitionPenalty: Float? = req.params.repetitionPenalty > 0 ? req.params.repetitionPenalty : nil
        let seed: UInt64? = req.params.seed.map { UInt64(bitPattern: $0) }
        let parameters = GenerateParameters(
            maxTokens: req.params.maxTokens > 0 ? Int(req.params.maxTokens) : nil,
            temperature: req.params.temperature,
            topP: req.params.topP,
            topK: req.params.topK > 0 ? Int(req.params.topK) : 0,
            minP: req.params.minP,
            repetitionPenalty: repetitionPenalty,
            seed: seed
        )

        return try await container.perform { context in
            try Task.checkCancellation()

            // Build UserInput inside the perform closure so non-Sendable UserInput
            // is not captured across closure boundaries under Swift 6 strict concurrency.
            let chatMessages: [MLXLMCommon.Chat.Message] = req.messages.map { msg in
                let text = msg.parts.compactMap { part -> String? in
                    if case .text(let t) = part { return t }
                    return nil
                }.joined()

                switch msg.role.lowercased() {
                case "system":
                    return .system(text)
                case "assistant":
                    return .assistant(text)
                case "tool":
                    return .tool(text)
                case "user":
                    fallthrough
                default:
                    return .user(text)
                }
            }
            let userInput = MLXLMCommon.UserInput(chat: chatMessages)

            let lmInput = try await context.processor.prepare(input: userInput)
            let stream = try MLXLMCommon.generate(input: lmInput, parameters: parameters, context: context)

            var accumulatedText = ""
            var lastFlushTime = ContinuousClock.now
            let batchInterval: Duration = .milliseconds(16)

            var firstTokenReceived = false
            var promptTokens: UInt32 = UInt32(lmInput.text.tokens.size)
            var completionTokens: UInt32 = 0
            var ttftMs: UInt32 = 0
            let startTime = ContinuousClock.now
            var completionStats: GenerateStats?

            for await event in stream {
                if Task.isCancelled {
                    break
                }

                switch event {
                case .chunk(let text):
                    let now = ContinuousClock.now
                    if !firstTokenReceived {
                        firstTokenReceived = true
                        let elapsed = now - startTime
                        ttftMs = UInt32(max(0, elapsed.components.seconds * 1000 + elapsed.components.attoseconds / 1_000_000_000_000_000))
                    }
                    completionTokens += 1
                    accumulatedText += text

                    if now - lastFlushTime >= batchInterval {
                        sink.onDelta(delta: GenerateDelta(text: accumulatedText, reasoning: "", toolCalls: []))
                        accumulatedText = ""
                        lastFlushTime = now
                    }

                case .info(let info):
                    if !accumulatedText.isEmpty {
                        sink.onDelta(delta: GenerateDelta(text: accumulatedText, reasoning: "", toolCalls: []))
                        accumulatedText = ""
                    }

                    promptTokens = UInt32(info.promptTokenCount)
                    completionTokens = UInt32(info.generationTokenCount)
                    ttftMs = UInt32((info.promptTime * 1000).rounded())

                    let finishReason: FinishReason
                    switch info.stopReason {
                    case .stop:
                        finishReason = .stop
                    case .length:
                        finishReason = .length
                    case .cancelled:
                        finishReason = .cancelled
                    }

                    completionStats = GenerateStats(
                        promptTokens: promptTokens,
                        completionTokens: completionTokens,
                        ttftMs: ttftMs,
                        decodeTps: Float(info.tokensPerSecond),
                        finish: finishReason
                    )

                case .toolCall, .rejectedToolCall:
                    break
                }
            }

            // Flush remaining text
            if !accumulatedText.isEmpty {
                sink.onDelta(delta: GenerateDelta(text: accumulatedText, reasoning: "", toolCalls: []))
                accumulatedText = ""
            }

            if Task.isCancelled {
                let stats = GenerateStats(
                    promptTokens: promptTokens,
                    completionTokens: completionTokens,
                    ttftMs: ttftMs,
                    decodeTps: 0,
                    finish: .cancelled
                )
                sink.onDone(stats: stats)
                return stats
            }

            if let stats = completionStats {
                sink.onDone(stats: stats)
                return stats
            } else {
                let totalDuration = ContinuousClock.now - startTime
                let durationSec = Double(totalDuration.components.seconds) + Double(totalDuration.components.attoseconds) / 1e18
                let tps: Float = durationSec > 0 ? Float(Double(completionTokens) / durationSec) : 0
                let stats = GenerateStats(
                    promptTokens: promptTokens,
                    completionTokens: completionTokens,
                    ttftMs: ttftMs,
                    decodeTps: tps,
                    finish: .stop
                )
                sink.onDone(stats: stats)
                return stats
            }
        }
    }

    // MARK: - Directory Resolution

    /// Resolves the local on-disk model directory for a given manifest.
    /// Follows the `<appSupport>/Models/<hfRepo>/<hfRevision>` convention used by the core downloader.
    public static func resolveModelDirectory(manifest: ModelManifest) -> URL {
        if let first = manifest.files.first, first.path.hasPrefix("/") {
            return URL(fileURLWithPath: first.path).deletingLastPathComponent()
        }

        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(Bundle.main.bundleIdentifier ?? "app.getpocketai", isDirectory: true)
            .appendingPathComponent("Models", isDirectory: true)

        if !manifest.hfRepo.isEmpty && !manifest.hfRevision.isEmpty {
            return base.appendingPathComponent(manifest.hfRepo).appendingPathComponent(manifest.hfRevision)
        } else if !manifest.hfRepo.isEmpty {
            return base.appendingPathComponent(manifest.hfRepo)
        } else {
            return base.appendingPathComponent(manifest.id)
        }
    }

    // MARK: - Helper Boxes

    private final class ResultBox<T>: @unchecked Sendable {
        var result: Result<T, Error>?
    }

    private final class ValueBox<T>: @unchecked Sendable {
        var value: T?
    }
}

// MARK: - Actor-Isolated Model Registry

/// Actor providing thread-safe, isolated storage for loaded `ModelContainer`s,
/// resident memory footprints, and in-flight generation tasks under Swift 6 strict concurrency.
actor MLXModelRegistry {
    private var containers: [String: ModelContainer] = [:]
    private var residentBytesMap: [String: UInt64] = [:]
    private var activeTasks: [String: Task<Void, Never>] = [:]

    func register(handle: String, container: ModelContainer, residentBytes: UInt64) {
        containers[handle] = container
        residentBytesMap[handle] = residentBytes
    }

    func remove(handle: String) {
        containers.removeValue(forKey: handle)
        residentBytesMap.removeValue(forKey: handle)
    }

    func getContainer(handle: String) -> ModelContainer? {
        containers[handle]
    }

    func getResidentBytes(handle: String) -> UInt64 {
        residentBytesMap[handle] ?? 0
    }

    func registerTask(requestId: String, task: Task<Void, Never>) {
        activeTasks[requestId] = task
    }

    func removeTask(requestId: String) {
        activeTasks.removeValue(forKey: requestId)
    }

    func cancelTask(requestId: String) {
        activeTasks[requestId]?.cancel()
        activeTasks.removeValue(forKey: requestId)
    }
}

// MARK: - Local Tokenizer Loader & Bridge

/// Bridges `Tokenizers.AutoTokenizer` from `swift-transformers` to the `MLXLMCommon.TokenizerLoader` protocol.
struct LocalTokenizerLoader: MLXLMCommon.TokenizerLoader {
    func load(from directory: URL) async throws -> any MLXLMCommon.Tokenizer {
        let upstream = try await Tokenizers.AutoTokenizer.from(modelFolder: directory)
        return TokenizerBridge(upstream)
    }
}

/// Adapts `Tokenizers.Tokenizer` to `MLXLMCommon.Tokenizer`.
struct TokenizerBridge: MLXLMCommon.Tokenizer {
    private let upstream: any Tokenizers.Tokenizer

    init(_ upstream: any Tokenizers.Tokenizer) {
        self.upstream = upstream
    }

    func encode(text: String, addSpecialTokens: Bool) -> [Int] {
        upstream.encode(text: text, addSpecialTokens: addSpecialTokens)
    }

    func decode(tokenIds: [Int], skipSpecialTokens: Bool) -> String {
        upstream.decode(tokens: tokenIds, skipSpecialTokens: skipSpecialTokens)
    }

    func convertTokenToId(_ token: String) -> Int? {
        upstream.convertTokenToId(token)
    }

    func convertIdToToken(_ id: Int) -> String? {
        upstream.convertIdToToken(id)
    }

    var bosToken: String? { upstream.bosToken }
    var eosToken: String? { upstream.eosToken }
    var unknownToken: String? { upstream.unknownToken }

    func applyChatTemplate(
        messages: [[String: any Sendable]],
        tools: [[String: any Sendable]]?,
        additionalContext: [String: any Sendable]?
    ) throws -> [Int] {
        do {
            return try upstream.applyChatTemplate(
                messages: messages,
                tools: tools,
                additionalContext: additionalContext
            )
        } catch {
            throw MLXLMCommon.TokenizerError.missingChatTemplate
        }
    }
}
