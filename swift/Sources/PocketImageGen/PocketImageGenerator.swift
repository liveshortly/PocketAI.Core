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
    case cancelled
    case generationFailed(String)

    public var errorDescription: String? {
        switch self {
        case .notLoaded:
            return "Model is not loaded. Call load(modelDirectory:) first."
        case .invalidSize(let size):
            return "Invalid image size: \(size). Supported sizes are 512, 768, and 1024."
        case .modelNotFound(let path):
            return "Model directory or weights not found at path: \(path)"
        case .cancelled:
            return "Image generation was cancelled."
        case .generationFailed(let message):
            return "Image generation failed: \(message)"
        }
    }
}

// MARK: - Standalone Image Generator Service

/// On-device text-to-image generator using MLX Stable Diffusion (SDXL-Turbo).
///
/// This is a standalone service (not an `Engine` foreign-trait conformance, since image gen
/// has no Rust FFI contract and is consumed directly by Swift UI layers).
///
/// Concurrency discipline:
/// - Thread-safe and `@MainActor`-friendly.
/// - Model evaluation and weights state are isolated inside an `actor` (`ImageGenModelActor`).
/// - No `@unchecked Sendable` on shared mutable state.
public final class PocketImageGenerator: Sendable {
    /// Supported square dimensions for SDXL-Turbo generation.
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

    /// Clamps denoising steps to the supported 1...4 range for SDXL-Turbo.
    public static func clampSteps(_ steps: Int) -> Int {
        min(max(steps, 1), 4)
    }

    // MARK: - Public Lifecycle

    /// Whether the model weights are currently loaded and ready for generation.
    public var isLoaded: Bool {
        get async {
            await modelActor.isLoaded
        }
    }

    /// Loads an SDXL-Turbo model from local weights.
    ///
    /// - Parameter modelDirectory: Directory containing the model weights and configuration files.
    /// - Throws: `ImageGenError.modelNotFound` if the directory does not exist, or `ImageGenError.generationFailed` on failure.
    public func load(modelDirectory: URL) async throws {
        try await modelActor.load(modelDirectory: modelDirectory)
    }

    /// Unloads the model and releases GPU/system memory cache.
    public func unload() async {
        await modelActor.unload()
    }

    // MARK: - Image Generation

    /// Generates an image from a text prompt using SDXL-Turbo.
    ///
    /// - Parameters:
    ///   - prompt: The textual description of the desired image.
    ///   - negativePrompt: Optional negative prompt.
    ///   - size: Output image size (must be 512, 768, or 1024; defaults to 512).
    ///   - steps: Number of denoising steps (clamped to 1...4; defaults to 2).
    ///   - seed: Optional deterministic RNG seed.
    ///   - onStep: Progress callback invoked on every step with `(currentStep, totalSteps, previewCGImage)`.
    /// - Returns: The final generated `CGImage`.
    /// - Throws: `ImageGenError.notLoaded` if not loaded, `ImageGenError.invalidSize` for unsupported sizes,
    ///           `ImageGenError.cancelled` on task cancellation, or `ImageGenError.generationFailed`.
    public func generate(
        prompt: String,
        negativePrompt: String? = nil,
        size: Int = 512,
        steps: Int = 2,
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
        steps: Int,
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
        MLX.Memory.clearCache()
    }

    func load(modelDirectory: URL) throws {
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

        let (hub, cleanup) = try Self.prepareLocalHubApi(for: modelDirectory)
        defer {
            cleanup?()
        }

        let configuration = StableDiffusionConfiguration.presetSDXLTurbo
        let loadConfig = LoadConfiguration(float16: true, quantize: false)

        do {
            let sdXL = try StableDiffusionXL(
                hub: hub,
                configuration: configuration,
                dType: loadConfig.dType
            )

            if !conserveMemory {
                sdXL.ensureLoaded()
            }

            self.generator = sdXL
            self.modelDirectory = modelDirectory
        } catch {
            throw ImageGenError.generationFailed("Failed to load model weights: \(error.localizedDescription)")
        }
    }

    func generate(
        prompt: String,
        negativePrompt: String?,
        size: Int,
        steps: Int,
        seed: Int?,
        onStep: (@Sendable (Int, Int, CGImage?) -> Void)?
    ) throws -> CGImage {
        guard let generator = self.generator else {
            throw ImageGenError.notLoaded
        }

        try PocketImageGenerator.validateSize(size)
        let clampedSteps = PocketImageGenerator.clampSteps(steps)
        let latentDim = size / 8

        let uintSeed: UInt64? = seed.map { s in
            s >= 0 ? UInt64(s) : UInt64(bitPattern: Int64(s))
        }

        let parameters = EvaluateParameters(
            cfgWeight: 0.0, // SDXL-Turbo is optimized for cfgWeight 0
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

    /// Configures a `HubApi` instance to resolve local SDXL-Turbo weights.
    ///
    /// StableDiffusion's loader resolves paths via `hub.localRepoLocation(Repo(id: "stabilityai/sdxl-turbo"))`,
    /// which appends `models/stabilityai/sdxl-turbo` to `hub.downloadBase`.
    /// This helper bridges arbitrary local directories using temporary symlinks when needed.
    static func prepareLocalHubApi(for modelDirectory: URL) throws -> (hub: HubApi, cleanup: (@Sendable () -> Void)?) {
        let fm = FileManager.default
        let standardRelativePath = "models/stabilityai/sdxl-turbo"

        // Case 1: modelDirectory itself is the base directory containing models/stabilityai/sdxl-turbo/...
        let candidateNested = modelDirectory.appendingPathComponent(standardRelativePath)
        if fm.fileExists(atPath: candidateNested.path) {
            return (HubApi(downloadBase: modelDirectory), nil)
        }

        // Case 2: modelDirectory already ends with models/stabilityai/sdxl-turbo
        let pathString = modelDirectory.standardizedFileURL.path
        if pathString.hasSuffix(standardRelativePath) {
            let base = modelDirectory
                .deletingLastPathComponent()
                .deletingLastPathComponent()
                .deletingLastPathComponent()
            return (HubApi(downloadBase: base), nil)
        }

        // Case 3: modelDirectory directly contains the weights (e.g. unet/config.json).
        // Create a temporary directory structure: <tempBase>/models/stabilityai/sdxl-turbo -> modelDirectory
        let tempBase = fm.temporaryDirectory.appendingPathComponent("pocket-sdxl-\(UUID().uuidString)", isDirectory: true)
        let stabilityDir = tempBase.appendingPathComponent("models/stabilityai", isDirectory: true)
        try fm.createDirectory(at: stabilityDir, withIntermediateDirectories: true)

        let symlinkURL = stabilityDir.appendingPathComponent("sdxl-turbo")
        try fm.createSymbolicLink(at: symlinkURL, withDestinationURL: modelDirectory.standardizedFileURL)

        let cleanup: @Sendable () -> Void = {
            try? FileManager.default.removeItem(at: tempBase)
        }

        return (HubApi(downloadBase: tempBase), cleanup)
    }
}
