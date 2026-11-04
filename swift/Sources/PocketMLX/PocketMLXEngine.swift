// TODO(APL-06 follow-up): register via core.registerEngine at app launch
import Foundation
import MLX
import MLXLMCommon
import MLXLLM
import MLXEmbedders
import PocketKit
import Tokenizers
import Hub
import MLXVLM
import CoreImage
import CoreGraphics

// MARK: - Free Utility Functions

/// Chunks an array into batches of up to `size` elements (default 32).
public func chunkInputs<T>(_ inputs: [T], size: Int = 32) -> [[T]] {
    guard size > 0 else { return [inputs] }
    guard !inputs.isEmpty else { return [] }
    var chunks: [[T]] = []
    var index = 0
    while index < inputs.count {
        let chunkEnd = min(index + size, inputs.count)
        chunks.append(Array(inputs[index..<chunkEnd]))
        index = chunkEnd
    }
    return chunks
}

/// Normalizes a vector to unit length (L2 norm).
/// If the norm is 0 (or below `eps`), the original vector is returned unchanged.
public func l2Normalize(_ vector: [Float], eps: Float = 1e-12) -> [Float] {
    var sumOfSquares: Float = 0
    for val in vector {
        sumOfSquares += val * val
    }
    let norm = sqrt(sumOfSquares)
    guard norm > eps else {
        return vector
    }
    return vector.map { $0 / norm }
}

/// Computes the cosine similarity between two vectors.
public func cosineSimilarity(_ a: [Float], _ b: [Float]) -> Float {
    guard a.count == b.count, !a.isEmpty else { return 0 }
    var dot: Float = 0
    var normA: Float = 0
    var normB: Float = 0
    for i in 0..<a.count {
        dot += a[i] * b[i]
        normA += a[i] * a[i]
        normB += b[i] * b[i]
    }
    let denom = sqrt(normA) * sqrt(normB)
    guard denom > 0 else { return 0 }
    return dot / denom
}

/// Computes the target dimensions for downscaling an image so its longest edge is at most `maxLongEdge` (default 1024),
/// preserving aspect ratio. Images already within bounds are returned with their original dimensions.
public func downscaledDimensions(width: CGFloat, height: CGFloat, maxLongEdge: CGFloat = 1024) -> (width: CGFloat, height: CGFloat) {
    guard width > 0, height > 0 else { return (width, height) }
    let longEdge = max(width, height)
    guard longEdge > maxLongEdge else {
        return (width, height)
    }
    let scale = maxLongEdge / longEdge
    return (
        width: (width * scale).rounded(),
        height: (height * scale).rounded()
    )
}

/// Convenience returning a `CGSize` with longest edge constrained to `maxLongEdge`.
public func downscaledSize(for size: CGSize, maxLongEdge: CGFloat = 1024) -> CGSize {
    let dims = downscaledDimensions(width: size.width, height: size.height, maxLongEdge: maxLongEdge)
    return CGSize(width: dims.width, height: dims.height)
}

/// Checks whether any message contains at least one `.imageRef` part.
public func messagesContainImages(_ messages: [ChatMessage]) -> Bool {
    messages.contains { message in
        message.parts.contains { part in
            if case .imageRef = part {
                return true
            }
            return false
        }
    }
}

/// Loads and downscales an image from disk so its longest edge is at most `maxLongEdge` (default 1024).
///
/// Throws `PocketError.EngineFailed` if the file does not exist, cannot be decoded, or has invalid dimensions.
public func loadAndDownscaleImage(from path: String, maxLongEdge: CGFloat = 1024) throws -> CIImage {
    let url = URL(fileURLWithPath: path)
    guard FileManager.default.fileExists(atPath: url.path) else {
        throw PocketError.EngineFailed(engine: .mlx, message: "Image not found at path: \(path)")
    }
    guard let ciImage = CIImage(contentsOf: url) else {
        throw PocketError.EngineFailed(engine: .mlx, message: "Failed to decode image at path: \(path)")
    }
    let extent = ciImage.extent
    guard extent.width > 0, extent.height > 0 else {
        throw PocketError.EngineFailed(engine: .mlx, message: "Image at path \(path) has invalid dimensions: \(extent.size)")
    }

    let target = downscaledDimensions(width: extent.width, height: extent.height, maxLongEdge: maxLongEdge)
    if target.width < extent.width || target.height < extent.height {
        let scale = target.width / extent.width
        let transform = CGAffineTransform(scaleX: scale, y: scale)
        return ciImage.transformed(by: transform)
    }
    return ciImage
}

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

    /// A `mac:<peer>/<model>` id means "run this on that paired Mac" (A§ router
    /// rule 5), so MLX must decline it even though the manifest carries the
    /// remote model's `mlx-safetensors` format — otherwise this engine wins the
    /// `RoutingEngine` priority order ahead of `LazyRemoteEngine` and tries to
    /// load weights that only exist on the Mac.
    public func supports(model: ModelManifest) -> Bool {
        model.format == .mlxSafetensors && !model.id.hasPrefix("mac:")
    }

    /// Determines whether the model is a vision-language model (VLM).
    public static func isVisionModel(manifest: ModelManifest) -> Bool {
        if manifest.modalities.contains(.vision) || manifest.modalities.contains(.image) {
            return true
        }
        if VLMRegistry.shared.contains(id: manifest.hfRepo) || VLMRegistry.shared.contains(id: manifest.id) {
            return true
        }
        return false
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

        let isVision = Self.isVisionModel(manifest: model)
        let semaphore = DispatchSemaphore(value: 0)
        let box = ResultBox<LoadedModel>()

        Task {
            do {
                let startMemory = MLX.Memory.snapshot().activeMemory

                // Note: In mlx-swift-lm 3.32.x, local weights are loaded via
                // `LLMModelFactory.shared.loadContainer(from:using:)` or `VLMModelFactory.shared.loadContainer(from:using:)`,
                // which accepts a local URL and a `TokenizerLoader` with no remote downloader.
                let tokenizerLoader = LocalTokenizerLoader()
                let container: ModelContainer
                if isVision {
                    container = try await VLMModelFactory.shared.loadContainer(
                        from: localURL,
                        using: tokenizerLoader
                    )
                } else {
                    container = try await LLMModelFactory.shared.loadContainer(
                        from: localURL,
                        using: tokenizerLoader
                    )
                }

                let endMemory = MLX.Memory.snapshot().activeMemory
                let resident = UInt64(max(0, endMemory - startMemory))

                let handle = UUID().uuidString
                await self.registry.register(handle: handle, container: container, residentBytes: resident, isVision: isVision)
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
            } catch let pocketError as PocketError {
                sink.onError(err: pocketError)
                box.result = .failure(pocketError)
            } catch {
                let engineError = PocketError.EngineFailed(engine: .mlx, message: "\(error)")
                sink.onError(err: engineError)
                box.result = .failure(engineError)
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
            if let pocketError = error as? PocketError {
                throw pocketError
            }
            throw PocketError.EngineFailed(engine: .mlx, message: "\(error)")
        case .none:
            throw PocketError.EngineFailed(engine: .mlx, message: "MLX generate produced no result")
        }
    }

    public func embed(handle: String, inputs: [String]) throws -> [[Float]] {
        guard !inputs.isEmpty else {
            return []
        }

        let semaphore = DispatchSemaphore(value: 0)
        let box = ResultBox<[[Float]]>()

        Task {
            do {
                let container = try await self.registry.getOrCreateEmbedder {
                    try await self.loadEmbedderContainer()
                }

                let batches = chunkInputs(inputs, size: 32)
                var allEmbeddings: [[Float]] = []

                for batch in batches {
                    let batchEmbeddings: [[Float]] = await container.perform { context in
                        let tokenizer = context.tokenizer
                        let model = context.model
                        let pooling = context.pooling

                        let maxSeqLen = model.maxPositionEmbeddings ?? 512
                        let tokenized = batch.map { text -> [Int] in
                            let tokens = tokenizer.encode(text: text, addSpecialTokens: true)
                            if tokens.count > maxSeqLen {
                                return Array(tokens.prefix(maxSeqLen))
                            }
                            return tokens
                        }

                        let maxLength = max(1, tokenized.reduce(into: 1) { acc, elem in
                            acc = max(acc, elem.count)
                        })

                        let padId = tokenizer.convertTokenToId("[PAD]") ?? tokenizer.eosTokenId ?? 0

                        let padded = stacked(
                            tokenized.map { elem in
                                let padCount = maxLength - elem.count
                                return MLXArray(elem + Array(repeating: padId, count: padCount))
                            }
                        )

                        let mask = (padded .!= padId)
                        let tokenTypes = MLXArray.zeros(like: padded)

                        let modelOutput = model(
                            padded,
                            positionIds: nil,
                            tokenTypeIds: tokenTypes,
                            attentionMask: mask
                        )

                        let result = pooling(
                            modelOutput,
                            mask: mask,
                            normalize: true,
                            applyLayerNorm: true
                        )
                        result.eval()

                        return result.map { row in
                            l2Normalize(row.asArray(Float.self))
                        }
                    }

                    allEmbeddings.append(contentsOf: batchEmbeddings)
                }

                box.result = .success(allEmbeddings)
            } catch {
                box.result = .failure(error)
            }
            semaphore.signal()
        }

        semaphore.wait()

        switch box.result {
        case .success(let embeddings):
            return embeddings
        case .failure(let error):
            throw PocketError.EngineFailed(engine: .mlx, message: "MLX embed failed: \(error)")
        case .none:
            throw PocketError.EngineFailed(engine: .mlx, message: "MLX embed produced no result")
        }
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

        let hasImages = messagesContainImages(req.messages)
        if hasImages {
            let isVision = await registry.isVisionModel(handle: handle)
            guard isVision else {
                throw PocketError.EngineFailed(
                    engine: .mlx,
                    message: "Model does not support vision inputs (not a VLM)"
                )
            }
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
            let chatMessages: [MLXLMCommon.Chat.Message] = try req.messages.map { msg in
                var textParts: [String] = []
                var imageInputs: [MLXLMCommon.UserInput.Image] = []

                for part in msg.parts {
                    switch part {
                    case .text(let t):
                        textParts.append(t)
                    case .imageRef(let path):
                        let ciImage = try loadAndDownscaleImage(from: path)
                        imageInputs.append(MLXLMCommon.UserInput.Image.ciImage(ciImage))
                    }
                }

                let text = textParts.joined()
                switch msg.role.lowercased() {
                case "system":
                    return .system(text, images: imageInputs)
                case "assistant":
                    return .assistant(text, images: imageInputs)
                case "tool":
                    return .tool(text)
                case "user":
                    fallthrough
                default:
                    return .user(text, images: imageInputs)
                }
            }
            // AGT-12: Core sends tool specs only when the chat's Tools switch
            // is on; the chat template renders them.
            let tools = req.tools.isEmpty ? nil : req.tools.map(mlxToolSpec)
            let userInput = MLXLMCommon.UserInput(chat: chatMessages, tools: tools)

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

                case .toolCall(let call):
                    // MLX lifts `<tool_call>` blocks out of the text; hand them
                    // back to Core's tool loop in the Hermes text form it parses.
                    if !req.tools.isEmpty, let json = try? JSONEncoder().encode(call.function),
                       let body = String(data: json, encoding: .utf8) {
                        accumulatedText += "<tool_call>\(body)</tool_call>"
                    }
                case .rejectedToolCall:
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

    // MARK: - Embedder Resolution & Loading

    /// Resolves the local on-disk directory for the dedicated bge-small embedding model, if present.
    public static func resolveEmbedderModelDirectory() -> URL? {
        let fm = FileManager.default
        if let envPath = ProcessInfo.processInfo.environment["POCKET_MLX_EMBED_MODEL_PATH"], !envPath.isEmpty {
            let envURL = URL(fileURLWithPath: envPath)
            if fm.fileExists(atPath: envURL.path) {
                return envURL
            }
        }

        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(Bundle.main.bundleIdentifier ?? "app.getpocketai", isDirectory: true)
            .appendingPathComponent("Models", isDirectory: true)

        let candidates = [
            base.appendingPathComponent("mlx-community/bge-small-en-v1.5-4bit/6d66cdf333ff5d4cdad084f2fc2a619575d064ab"),
            base.appendingPathComponent("mlx-community/bge-small-en-v1.5-4bit"),
            base.appendingPathComponent("BAAI/bge-small-en-v1.5"),
        ]

        for candidate in candidates {
            if fm.fileExists(atPath: candidate.path) {
                return candidate
            }
        }
        return nil
    }

    private func loadEmbedderContainer() async throws -> EmbedderModelContainer {
        let tokenizerLoader = LocalTokenizerLoader()
        if let localURL = Self.resolveEmbedderModelDirectory() {
            return try await EmbedderModelFactory.shared.loadContainer(
                from: localURL,
                using: tokenizerLoader
            )
        } else {
            return try await EmbedderModelFactory.shared.loadContainer(
                from: HubDownloader(),
                using: tokenizerLoader,
                configuration: EmbedderRegistry.bge_small
            )
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
/// resident memory footprints, in-flight generation tasks, and cached embedding models under Swift 6 strict concurrency.
actor MLXModelRegistry {
    private var containers: [String: ModelContainer] = [:]
    private var isVisionMap: [String: Bool] = [:]
    private var residentBytesMap: [String: UInt64] = [:]
    private var activeTasks: [String: Task<Void, Never>] = [:]
    private var embedderContainer: EmbedderModelContainer?

    func register(handle: String, container: ModelContainer, residentBytes: UInt64, isVision: Bool) {
        containers[handle] = container
        isVisionMap[handle] = isVision
        residentBytesMap[handle] = residentBytes
    }

    func remove(handle: String) {
        containers.removeValue(forKey: handle)
        isVisionMap.removeValue(forKey: handle)
        residentBytesMap.removeValue(forKey: handle)
    }

    func getContainer(handle: String) -> ModelContainer? {
        containers[handle]
    }

    func isVisionModel(handle: String) -> Bool {
        isVisionMap[handle] ?? false
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

    func getOrCreateEmbedder(
        loader: @Sendable () async throws -> EmbedderModelContainer
    ) async throws -> EmbedderModelContainer {
        if let container = embedderContainer {
            return container
        }
        let container = try await loader()
        if let existing = embedderContainer {
            return existing
        }
        self.embedderContainer = container
        return container
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

// MARK: - Hub Downloader Bridge

/// Bridges `Hub.HubApi` from `swift-transformers` to the `MLXLMCommon.Downloader` protocol.
struct HubDownloader: MLXLMCommon.Downloader {
    func download(
        id: String,
        revision: String?,
        matching patterns: [String],
        useLatest: Bool,
        progressHandler: @Sendable @escaping (Progress) -> Void
    ) async throws -> URL {
        let repo = Hub.Repo(id: id)
        // HubApi.shared downloads into ~/Documents/huggingface, which outside
        // the sandbox (MAC-19) is the user's real Documents folder and trips a
        // TCC prompt. Keep fallback downloads beside the app's other models.
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(Bundle.main.bundleIdentifier ?? "app.getpocketai", isDirectory: true)
            .appendingPathComponent("huggingface", isDirectory: true)
        return try await HubApi(downloadBase: base).snapshot(
            from: repo,
            revision: revision ?? "main",
            matching: patterns,
            progressHandler: progressHandler
        )
    }
}

// MARK: - Tool specs (AGT-12)

/// Core's `ToolSpec` as the OpenAI-style function spec chat templates expect.
private func mlxToolSpec(_ spec: PocketKit.ToolSpec) -> MLXLMCommon.ToolSpec {
    let schema = (try? JSONSerialization.jsonObject(with: Data(spec.parametersJsonSchema.utf8))) ?? [String: Any]()
    return [
        "type": "function",
        "function": [
            "name": spec.name,
            "description": spec.description,
            "parameters": sendableJSON(schema),
        ] as [String: any Sendable],
    ]
}

private func sendableJSON(_ value: Any) -> any Sendable {
    switch value {
    case let dict as [String: Any]: return dict.mapValues(sendableJSON)
    case let array as [Any]: return array.map(sendableJSON)
    case let string as String: return string
    case let number as NSNumber:
        if CFGetTypeID(number) == CFBooleanGetTypeID() { return number.boolValue }
        return number.doubleValue == number.doubleValue.rounded() ? number.intValue : number.doubleValue
    default: return "\(value)"
    }
}
