// TODO(APL-15 follow-up): surfaced to UI by UI-18
import CoreGraphics
import Foundation
import Hub
import MLX
import PocketImageGenVendor

// MARK: - Error Types

/// Errors thrown by `PocketImageGenerator`.
public enum ImageGenError: Error, LocalizedError, Equatable, Sendable {
    case notLoaded
    case invalidSize(Int)
    case modelNotFound(String)
    case unsupportedModel(String)
    case cancelled
    case generationFailed(String)

    public var errorDescription: String? {
        switch self {
        case .notLoaded:
            return "Model is not loaded. Call load(modelDirectory:modelId:) first."
        case .invalidSize(let size):
            return "Invalid image size: \(size). Supported sizes are 512, 768, and 1024."
        case .modelNotFound(let path):
            return "Model directory or weights not found at path: \(path)"
        case .unsupportedModel(let id):
            return "No image generation model matches id: \(id). Supported ids are "
                + PocketImageGenerator.supportedModelIds.joined(separator: ", ") + "."
        case .cancelled:
            return "Image generation was cancelled."
        case .generationFailed(let message):
            return "Image generation failed: \(message)"
        }
    }
}

// MARK: - Standalone Image Generator Service

/// On-device text-to-image generator using MLX Stable Diffusion.
///
/// Which model runs is decided by the model id handed to ``load(modelDirectory:modelId:)``,
/// which selects one of the vendored ``StableDiffusionConfiguration`` presets. The preset owns
/// its file layout, its concrete pipeline class and its tuned parameters, so nothing here may
/// assume a particular model: SDXL-Turbo wants 2 steps at cfg 0, SD 2.1 base wants 50 at 7.5.
///
/// This is a standalone service (not an `Engine` foreign-trait conformance, since image gen
/// has no Rust FFI contract and is consumed directly by Swift UI layers).
///
/// Concurrency discipline:
/// - Thread-safe and `@MainActor`-friendly.
/// - Model evaluation and weights state are isolated inside an `actor` (`ImageGenModelActor`).
/// - No `@unchecked Sendable` on shared mutable state.
public final class PocketImageGenerator: Sendable {
    /// Supported square dimensions. Not model-specific: 512 is in range for every preset.
    public static let supportedSizes: Set<Int> = [512, 768, 1024]

    /// Actor managing isolated model lifecycle and evaluation.
    private let modelActor: ImageGenModelActor

    /// Whether memory conservation is applied (defaulting to true on iOS).
    public let conserveMemory: Bool

    /// Initializes a new image generator.
    /// - Parameter conserveMemory: Whether to conserve memory. If omitted or nil,
    ///   defaults to `true` on iOS and `false` on macOS.
    public init(conserveMemory: Bool? = nil) {
        #if os(iOS)
        let defaultConserve = true
        #else
        let defaultConserve = false
        #endif
        let shouldConserve = conserveMemory ?? defaultConserve
        self.conserveMemory = shouldConserve
        self.modelActor = ImageGenModelActor(conserveMemory: shouldConserve)
    }

    // MARK: - Model Selection

    /// Model ids this generator can run, taken from the vendored presets rather than a
    /// list maintained here, so a new preset is picked up without edits.
    public static var supportedModelIds: [String] {
        StableDiffusionConfiguration.Preset.allCases.map(\.configuration.id)
    }

    /// Resolves a model id to the preset that knows how to load it.
    ///
    /// The id may carry a catalog revision suffix (`stabilityai/sdxl-turbo@71153311…`);
    /// only the repo part selects the preset.
    ///
    /// - Throws: `ImageGenError.unsupportedModel` when no preset claims the id. Failing here
    ///   is the point: the alternative is quietly loading one model's weights with another
    ///   model's pipeline.
    static func configuration(forModelId modelId: String) throws -> StableDiffusionConfiguration {
        let repo = modelId.split(separator: "@", maxSplits: 1).first.map(String.init) ?? modelId
        guard let preset = StableDiffusionConfiguration.Preset.allCases
            .first(where: { $0.configuration.id == repo })
        else {
            throw ImageGenError.unsupportedModel(modelId)
        }
        return preset.configuration
    }

    /// The denoising steps the model's own preset was tuned for — 2 for SDXL-Turbo, 50 for
    /// SD 2.1 base. Anything estimating generation time or fit must read this; a constant
    /// tuned for Turbo understates a full-schedule model by more than an order of magnitude.
    public static func defaultSteps(forModelId modelId: String) throws -> Int {
        try configuration(forModelId: modelId).defaultParameters().steps
    }

    // MARK: - Pure Validation & Clamping Utilities

    /// Validates that the requested image size is one of {512, 768, 1024}.
    /// - Throws: `ImageGenError.invalidSize` if not supported.
    @discardableResult
    public static func validateSize(_ size: Int) throws -> Int {
        guard supportedSizes.contains(size) else {
            throw ImageGenError.invalidSize(size)
        }
        return size
    }

    /// Clamps denoising steps to `1...maxSteps`.
    ///
    /// The ceiling is per-model — see ``maxSteps(for:)`` — because a few-step distilled model
    /// degrades past its schedule while a full-schedule model needs every step.
    public static func clampSteps(_ steps: Int, max maxSteps: Int) -> Int {
        min(max(steps, 1), max(maxSteps, 1))
    }

    /// Step ceiling for a preset: its own tuned schedule, with headroom for the few-step
    /// models where asking for a little more than the default is still reasonable.
    static func maxSteps(for configuration: StableDiffusionConfiguration) -> Int {
        max(configuration.defaultParameters().steps, 4)
    }

    // MARK: - Public Lifecycle

    /// Whether the model weights are currently loaded and ready for generation.
    public var isLoaded: Bool {
        get async {
            await modelActor.isLoaded
        }
    }

    /// Loads a model from local weights.
    ///
    /// - Parameters:
    ///   - modelDirectory: Directory containing the model weights and configuration files.
    ///   - modelId: Which model those weights are, e.g. `stabilityai/sdxl-turbo`, optionally
    ///     with a catalog revision suffix. This picks the preset — and so the pipeline class,
    ///     the file layout and the tuned parameters — so it must describe the weights on disk.
    /// - Throws: `ImageGenError.unsupportedModel` if no preset matches `modelId`,
    ///   `ImageGenError.modelNotFound` if the directory does not exist, or
    ///   `ImageGenError.generationFailed` on failure.
    public func load(modelDirectory: URL, modelId: String) async throws {
        try await modelActor.load(modelDirectory: modelDirectory, modelId: modelId)
    }

    /// Unloads the model and releases GPU/system memory cache.
    public func unload() async {
        await modelActor.unload()
    }

    // MARK: - Image Generation

    /// Generates an image from a text prompt using the loaded model.
    ///
    /// - Parameters:
    ///   - prompt: The textual description of the desired image.
    ///   - negativePrompt: Optional negative prompt.
    ///   - size: Output image size (must be 512, 768, or 1024; defaults to 512).
    ///   - steps: Number of denoising steps. `nil` uses the loaded model's own tuned
    ///     schedule; a value is clamped to `1...`the preset's ceiling.
    ///   - seed: Optional deterministic RNG seed.
    ///   - onStep: Progress callback invoked on every step with `(currentStep, totalSteps, previewCGImage)`.
    /// - Returns: The final generated `CGImage`.
    /// - Throws: `ImageGenError.notLoaded` if not loaded, `ImageGenError.invalidSize` for unsupported sizes,
    ///           `ImageGenError.cancelled` on task cancellation, or `ImageGenError.generationFailed`.
    public func generate(
        prompt: String,
        negativePrompt: String? = nil,
        size: Int = 512,
        steps: Int? = nil,
        seed: Int? = nil,
        onStep: (@Sendable (Int, Int, CGImage?) -> Void)? = nil
    ) async throws -> CGImage {
        try await modelActor.generate(
            prompt: prompt,
            negativePrompt: negativePrompt,
            size: size,
            steps: steps,
            seed: seed,
            onStep: onStep
        )
    }

    /// Helper for non-optional `onStep` closure matching the required specification signature.
    public func generate(
        prompt: String,
        negativePrompt: String?,
        size: Int,
        steps: Int?,
        seed: Int?,
        onStep: @escaping @Sendable (Int, Int, CGImage?) -> Void
    ) async throws -> CGImage {
        try await modelActor.generate(
            prompt: prompt,
            negativePrompt: negativePrompt,
            size: size,
            steps: steps,
            seed: seed,
            onStep: onStep
        )
    }
}

// MARK: - Actor-Isolated Model State

/// Actor managing the loaded Stable Diffusion pipeline, ensuring thread safety and strict concurrency.
actor ImageGenModelActor {
    private var generator: TextToImageGenerator?
    private var modelDirectory: URL?
    /// Preset of the loaded model — the source of its tuned parameters at generate time.
    private var configuration: StableDiffusionConfiguration?
    let conserveMemory: Bool

    init(conserveMemory: Bool) {
        self.conserveMemory = conserveMemory
    }

    var isLoaded: Bool {
        generator != nil
    }

    func unload() {
        generator = nil
        modelDirectory = nil
        configuration = nil
        MLX.Memory.clearCache()
    }

    func load(modelDirectory: URL, modelId: String) throws {
        // Resolve the preset before touching the filesystem: an unknown id is a caller bug,
        // not a missing-weights condition.
        let configuration = try PocketImageGenerator.configuration(forModelId: modelId)

        let fm = FileManager.default
        guard fm.fileExists(atPath: modelDirectory.path) else {
            throw ImageGenError.modelNotFound(modelDirectory.path)
        }

        // Apply MLX cache limits per platform:
        // iPhone 64 MB, Mac 512 MB.
        #if os(iOS)
        let cacheLimitBytes = 64 * 1024 * 1024
        #else
        let cacheLimitBytes = 512 * 1024 * 1024
        #endif
        MLX.Memory.cacheLimit = cacheLimitBytes

        let (hub, cleanup) = try Self.prepareLocalHubApi(
            for: modelDirectory, modelId: configuration.id,
            expecting: configuration.expectedRelativePaths)
        defer {
            cleanup?()
        }

        let loadConfig = LoadConfiguration(float16: true, quantize: false)

        do {
            // Through the preset's own factory, so it picks the concrete pipeline
            // (StableDiffusionXL vs StableDiffusionBase) that matches its file layout.
            guard let generator = try configuration.textToImageGenerator(
                hub: hub, configuration: loadConfig
            ) else {
                throw ImageGenError.generationFailed(
                    "Preset \(configuration.id) does not provide a text-to-image generator.")
            }

            if !conserveMemory {
                generator.ensureLoaded()
            }

            self.generator = generator
            self.configuration = configuration
            self.modelDirectory = modelDirectory
        } catch let error as ImageGenError {
            throw error
        } catch {
            throw ImageGenError.generationFailed("Failed to load model weights: \(error.localizedDescription)")
        }
    }

    func generate(
        prompt: String,
        negativePrompt: String?,
        size: Int,
        steps: Int?,
        seed: Int?,
        onStep: (@Sendable (Int, Int, CGImage?) -> Void)?
    ) throws -> CGImage {
        guard let generator = self.generator, let configuration = self.configuration else {
            throw ImageGenError.notLoaded
        }

        try PocketImageGenerator.validateSize(size)
        // Steps and cfg come from the loaded model's preset, never a constant: Turbo runs
        // 2 steps at cfg 0, SD 2.1 base 50 at 7.5, and swapping those produces mush.
        let presetDefaults = configuration.defaultParameters()
        let clampedSteps = PocketImageGenerator.clampSteps(
            steps ?? presetDefaults.steps,
            max: PocketImageGenerator.maxSteps(for: configuration)
        )
        let latentDim = size / 8

        let uintSeed: UInt64? = seed.map { s in
            s >= 0 ? UInt64(s) : UInt64(bitPattern: Int64(s))
        }

        let parameters = EvaluateParameters(
            cfgWeight: presetDefaults.cfgWeight,
            steps: clampedSteps,
            imageCount: 1,
            decodingBatchSize: 1,
            latentSize: [latentDim, latentDim],
            seed: uintSeed,
            prompt: prompt,
            negativePrompt: negativePrompt ?? ""
        )

        try Task.checkCancellation()

        let latents = generator.generateLatents(parameters: parameters)
        let decoder = generator.detachedDecoder()

        var currentStep = 0
        let totalSteps = clampedSteps
        var lastXt: MLXArray?

        for xt in latents {
            if Task.isCancelled {
                throw ImageGenError.cancelled
            }

            eval(xt)
            lastXt = xt
            currentStep += 1

            var previewCGImage: CGImage? = nil
            if onStep != nil {
                let decoded = decoder(xt)
                let raster = (decoded * 255).asType(.uint8).squeezed()
                eval(raster)
                previewCGImage = Image(raster).asCGImage()
            }

            onStep?(currentStep, totalSteps, previewCGImage)

            if conserveMemory {
                MLX.Memory.clearCache()
            }
        }

        if Task.isCancelled {
            throw ImageGenError.cancelled
        }

        guard let finalXt = lastXt else {
            throw ImageGenError.generationFailed("No latents produced during generation.")
        }

        let finalDecoded = decoder(finalXt)
        let finalRaster = (finalDecoded * 255).asType(.uint8).squeezed()
        eval(finalRaster)
        let finalImage = Image(finalRaster).asCGImage()

        if conserveMemory {
            MLX.Memory.clearCache()
        }

        return finalImage
    }

    // MARK: - Local Weights HubApi Bridge

    /// Configures a `HubApi` instance to resolve a model's local weights.
    ///
    /// StableDiffusion's loader resolves paths via `hub.localRepoLocation(Repo(id: configuration.id))`,
    /// which appends `models/<configuration.id>` to `hub.downloadBase`. That relative path is
    /// derived from the preset's id here — hardcoding one model's path is what previously made
    /// the loader single-model.
    ///
    /// This helper bridges arbitrary local directories using temporary symlinks when needed.
    /// Files a repo publishes only as an `fp16` variant, named as the preset
    /// expects them. The catalog pins the fp16 weights on purpose — the full
    /// unet is ~10 GB and will not run on a phone — but the preset asks for
    /// the plain names, so a downloaded model fails in `loadUnet` with nothing
    /// obviously wrong on disk. Aliasing bridges the two without re-pinning
    /// multi-gigabyte files or editing vendored code.
    static func fp16Alias(for expected: String) -> String {
        guard expected.hasSuffix(".safetensors") else { return expected }
        return expected.replacingOccurrences(
            of: ".safetensors", with: ".fp16.safetensors")
    }

    static func prepareLocalHubApi(
        for modelDirectory: URL, modelId: String, expecting expected: [String] = []
    ) throws -> (hub: HubApi, cleanup: (@Sendable () -> Void)?) {
        let fm = FileManager.default
        let standardRelativePath = "models/\(modelId)"

        // A repo root reached through case 1 or 2 still needs aliasing if its
        // weights are fp16-only, so resolve that first.
        let repoRoot = Self.repoRoot(in: modelDirectory, modelId: modelId) ?? modelDirectory
        if Self.needsFP16Aliases(in: repoRoot, expecting: expected) {
            return try Self.aliasedShim(
                repoRoot: repoRoot, standardRelativePath: standardRelativePath,
                expecting: expected)
        }

        // Case 1: modelDirectory itself is the base directory containing models/<id>/...
        let candidateNested = modelDirectory.appendingPathComponent(standardRelativePath)
        if fm.fileExists(atPath: candidateNested.path) {
            return (HubApi(downloadBase: modelDirectory), nil)
        }

        // Case 2: modelDirectory already ends with models/<id>
        let pathString = modelDirectory.standardizedFileURL.path
        if pathString.hasSuffix(standardRelativePath) {
            // Strip "models/" plus each component of the id (owner/name).
            let depth = standardRelativePath.split(separator: "/").count
            let base = (0..<depth).reduce(modelDirectory) { url, _ in
                url.deletingLastPathComponent()
            }
            return (HubApi(downloadBase: base), nil)
        }

        // Case 3: modelDirectory directly contains the weights (e.g. unet/config.json).
        // Create a temporary structure: <tempBase>/models/<id> -> modelDirectory
        let tempBase = fm.temporaryDirectory
            .appendingPathComponent("pocket-imagegen-\(UUID().uuidString)", isDirectory: true)
        let repoLink = tempBase.appendingPathComponent(standardRelativePath)
        try fm.createDirectory(
            at: repoLink.deletingLastPathComponent(), withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: repoLink, withDestinationURL: modelDirectory.standardizedFileURL)

        let cleanup: @Sendable () -> Void = {
            try? FileManager.default.removeItem(at: tempBase)
        }

        return (HubApi(downloadBase: tempBase), cleanup)
    }

    /// Where the repo's own files sit, whichever of the three shapes the
    /// caller handed us.
    private static func repoRoot(in modelDirectory: URL, modelId: String) -> URL? {
        let fm = FileManager.default
        let relative = "models/\(modelId)"
        let nested = modelDirectory.appendingPathComponent(relative)
        if fm.fileExists(atPath: nested.path) { return nested }
        if modelDirectory.standardizedFileURL.path.hasSuffix(relative) { return modelDirectory }
        if fm.fileExists(atPath: modelDirectory.appendingPathComponent("unet").path) {
            return modelDirectory
        }
        return nil
    }

    private static func needsFP16Aliases(in repoRoot: URL, expecting expected: [String]) -> Bool {
        let fm = FileManager.default
        return expected.contains { relative in
            let alias = fp16Alias(for: relative)
            guard alias != relative else { return false }
            return !fm.fileExists(atPath: repoRoot.appendingPathComponent(relative).path)
                && fm.fileExists(atPath: repoRoot.appendingPathComponent(alias).path)
        }
    }

    /// A throwaway tree of symlinks that gives every expected file its expected
    /// name. Nothing in the real model directory is touched — it is verified
    /// content and re-verification would trip over anything we added.
    private static func aliasedShim(
        repoRoot: URL, standardRelativePath: String, expecting expected: [String]
    ) throws -> (hub: HubApi, cleanup: (@Sendable () -> Void)?) {
        let fm = FileManager.default
        let tempBase = fm.temporaryDirectory
            .appendingPathComponent("pocket-imagegen-\(UUID().uuidString)", isDirectory: true)
        let repoLink = tempBase.appendingPathComponent(standardRelativePath)
        try fm.createDirectory(at: repoLink, withIntermediateDirectories: true)

        for relative in expected {
            let destination = repoLink.appendingPathComponent(relative)
            try fm.createDirectory(
                at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
            let direct = repoRoot.appendingPathComponent(relative)
            let source = fm.fileExists(atPath: direct.path)
                ? direct
                : repoRoot.appendingPathComponent(fp16Alias(for: relative))
            guard fm.fileExists(atPath: source.path) else { continue }
            try? fm.createSymbolicLink(at: destination, withDestinationURL: source.standardizedFileURL)
        }

        let cleanup: @Sendable () -> Void = {
            try? FileManager.default.removeItem(at: tempBase)
        }
        return (HubApi(downloadBase: tempBase), cleanup)
    }
}
