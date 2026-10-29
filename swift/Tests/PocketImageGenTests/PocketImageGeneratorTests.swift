import CoreGraphics
import Foundation
import Hub
import PocketImageGenVendor
import XCTest
@testable import PocketImageGen

final class PocketImageGeneratorTests: XCTestCase {
    private let turboId = "stabilityai/sdxl-turbo"
    private let sd21Id = "stabilityai/stable-diffusion-2-1-base"

    // MARK: - Unit Tests: Preset Selection
    //
    // Real generation needs multi-gigabyte weights, so these assert the one thing that can be
    // checked without them: that a model id selects the right preset — and so the right
    // pipeline class, file layout and tuned parameters.

    func testModelIdSelectsMatchingPreset() throws {
        let turbo = try PocketImageGenerator.configuration(forModelId: turboId)
        XCTAssertEqual(turbo.id, StableDiffusionConfiguration.presetSDXLTurbo.id)

        let sd21 = try PocketImageGenerator.configuration(forModelId: sd21Id)
        XCTAssertEqual(sd21.id, StableDiffusionConfiguration.presetStableDiffusion21Base.id)

        // Distinct presets: a selection bug that returned the same one for both ids would
        // otherwise pass the two checks above.
        XCTAssertNotEqual(turbo.id, sd21.id)
    }

    func testPresetSelectionCarriesTunedParameters() throws {
        // SD 2.1 base is a far heavier generation than Turbo (50 steps at cfg 7.5 vs 2 at 0).
        // Anything estimating time or fit reads these, so they must track the preset.
        let turbo = try PocketImageGenerator.configuration(forModelId: turboId).defaultParameters()
        XCTAssertEqual(turbo.steps, 2)
        XCTAssertEqual(turbo.cfgWeight, 0)

        let sd21 = try PocketImageGenerator.configuration(forModelId: sd21Id).defaultParameters()
        XCTAssertEqual(sd21.steps, 50)
        XCTAssertEqual(sd21.cfgWeight, 7.5)

        XCTAssertEqual(try PocketImageGenerator.defaultSteps(forModelId: turboId), 2)
        XCTAssertEqual(try PocketImageGenerator.defaultSteps(forModelId: sd21Id), 50)
    }

    func testCatalogRevisionSuffixStillSelectsPreset() throws {
        // Catalog ids carry the pinned revision; the repo part selects the preset.
        let pinned = "\(turboId)@71153311d3dbb46851df1931d3ca6e939de83304"
        XCTAssertEqual(
            try PocketImageGenerator.configuration(forModelId: pinned).id,
            StableDiffusionConfiguration.presetSDXLTurbo.id
        )
    }

    func testUnknownModelIdFailsRatherThanLoadingTheWrongModel() {
        // The failure mode this guards against is silence: falling back to a default preset
        // would load one model's weights with another model's pipeline.
        for unknown in ["", "stabilityai/sdxl-turbo-xl", "mlx-community/flux-schnell", "sdxl-turbo"] {
            XCTAssertThrowsError(try PocketImageGenerator.configuration(forModelId: unknown)) { error in
                XCTAssertEqual(error as? ImageGenError, ImageGenError.unsupportedModel(unknown))
            }
        }
    }

    func testSupportedModelIdsComeFromThePresets() {
        XCTAssertEqual(Set(PocketImageGenerator.supportedModelIds), [turboId, sd21Id])
    }

    // MARK: - Unit Tests: Hub Path Shim

    func testHubShimPathDerivesFromModelId() throws {
        let fm = FileManager.default
        for modelId in [turboId, sd21Id] {
            // Case 3: a bare weights directory, bridged by a temporary symlink.
            let weights = fm.temporaryDirectory
                .appendingPathComponent("weights-\(UUID().uuidString)", isDirectory: true)
            try fm.createDirectory(at: weights, withIntermediateDirectories: true)
            defer { try? fm.removeItem(at: weights) }

            let (hub, cleanup) = try ImageGenModelActor.prepareLocalHubApi(
                for: weights, modelId: modelId)
            defer { cleanup?() }

            let resolved = hub.localRepoLocation(Hub.Repo(id: modelId))
            XCTAssertTrue(
                resolved.standardizedFileURL.path.hasSuffix("models/\(modelId)"),
                "shim for \(modelId) resolved to \(resolved.path)"
            )
            XCTAssertEqual(
                try fm.destinationOfSymbolicLink(atPath: resolved.path),
                weights.standardizedFileURL.path
            )
        }
    }

    func testHubShimAcceptsADirectoryAlreadyAtTheHubLayout() throws {
        // Case 2: the directory already ends with models/<owner>/<name>; the base is above it.
        let fm = FileManager.default
        let base = fm.temporaryDirectory
            .appendingPathComponent("base-\(UUID().uuidString)", isDirectory: true)
        let repoDir = base.appendingPathComponent("models/\(sd21Id)", isDirectory: true)
        try fm.createDirectory(at: repoDir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: base) }

        let (hub, cleanup) = try ImageGenModelActor.prepareLocalHubApi(
            for: repoDir, modelId: sd21Id)
        XCTAssertNil(cleanup, "a directory already at the hub layout needs no symlink")
        XCTAssertEqual(
            hub.localRepoLocation(Hub.Repo(id: sd21Id)).standardizedFileURL.path,
            repoDir.standardizedFileURL.path
        )
    }

    // MARK: - Unit Tests: Step Clamping

    func testStepClamping() {
        // Turbo's ceiling: a few-step distilled model degrades past its schedule.
        let turboMax = PocketImageGenerator.maxSteps(for: .presetSDXLTurbo)
        XCTAssertEqual(turboMax, 4)
        XCTAssertEqual(PocketImageGenerator.clampSteps(-10, max: turboMax), 1)
        XCTAssertEqual(PocketImageGenerator.clampSteps(0, max: turboMax), 1)
        XCTAssertEqual(PocketImageGenerator.clampSteps(1, max: turboMax), 1)
        XCTAssertEqual(PocketImageGenerator.clampSteps(2, max: turboMax), 2)
        XCTAssertEqual(PocketImageGenerator.clampSteps(3, max: turboMax), 3)
        XCTAssertEqual(PocketImageGenerator.clampSteps(4, max: turboMax), 4)
        XCTAssertEqual(PocketImageGenerator.clampSteps(5, max: turboMax), 4)
        XCTAssertEqual(PocketImageGenerator.clampSteps(10, max: turboMax), 4)
        XCTAssertEqual(PocketImageGenerator.clampSteps(100, max: turboMax), 4)
    }

    func testStepClampingFollowsTheModelNotTurbo() {
        // The regression this guards: Turbo's 1...4 ceiling applied to a 50-step model.
        let sd21Max = PocketImageGenerator.maxSteps(for: .presetStableDiffusion21Base)
        XCTAssertEqual(sd21Max, 50)
        XCTAssertEqual(PocketImageGenerator.clampSteps(50, max: sd21Max), 50)
        XCTAssertEqual(PocketImageGenerator.clampSteps(30, max: sd21Max), 30)
        XCTAssertEqual(PocketImageGenerator.clampSteps(100, max: sd21Max), 50)
        XCTAssertEqual(PocketImageGenerator.clampSteps(0, max: sd21Max), 1)
    }

    // MARK: - Unit Tests: Size Validation

    func testSupportedSizesValidation() {
        // Supported sizes: 512, 768, 1024
        XCTAssertNoThrow(try PocketImageGenerator.validateSize(512))
        XCTAssertNoThrow(try PocketImageGenerator.validateSize(768))
        XCTAssertNoThrow(try PocketImageGenerator.validateSize(1024))

        XCTAssertEqual(try? PocketImageGenerator.validateSize(512), 512)
        XCTAssertEqual(try? PocketImageGenerator.validateSize(768), 768)
        XCTAssertEqual(try? PocketImageGenerator.validateSize(1024), 1024)

        // Unsupported sizes should throw ImageGenError.invalidSize
        let invalidSizes = [-512, 0, 64, 128, 256, 384, 600, 1000, 1025, 2048]
        for size in invalidSizes {
            XCTAssertThrowsError(try PocketImageGenerator.validateSize(size)) { error in
                guard let imageGenError = error as? ImageGenError else {
                    return XCTFail("Expected ImageGenError, got \(error)")
                }
                XCTAssertEqual(imageGenError, ImageGenError.invalidSize(size))
            }
        }
    }

    // MARK: - Unit Tests: Error Descriptions & Equatable

    func testImageGenErrorDescriptions() {
        let notLoaded = ImageGenError.notLoaded
        XCTAssertTrue(notLoaded.localizedDescription.contains("not loaded"))

        let invalidSize = ImageGenError.invalidSize(256)
        XCTAssertTrue(invalidSize.localizedDescription.contains("256"))

        let notFound = ImageGenError.modelNotFound("/non/existent/path")
        XCTAssertTrue(notFound.localizedDescription.contains("/non/existent/path"))

        let cancelled = ImageGenError.cancelled
        XCTAssertTrue(cancelled.localizedDescription.contains("cancelled"))

        let failed = ImageGenError.generationFailed("CUDA out of core")
        XCTAssertTrue(failed.localizedDescription.contains("CUDA out of core"))
    }

    // MARK: - Unit Tests: Generator Initial State

    func testGeneratorInitialState() async {
        let generator = PocketImageGenerator()
        let isLoaded = await generator.isLoaded
        XCTAssertFalse(isLoaded)

        #if os(iOS)
        XCTAssertTrue(generator.conserveMemory)
        #else
        XCTAssertFalse(generator.conserveMemory)
        #endif

        let customGenerator = PocketImageGenerator(conserveMemory: true)
        XCTAssertTrue(customGenerator.conserveMemory)
    }

    // MARK: - Unit Tests: Load Missing Model Throws

    func testLoadMissingModelThrows() async {
        let generator = PocketImageGenerator()
        let nonExistentURL = URL(fileURLWithPath: "/path/that/definitely/does/not/exist/\(UUID().uuidString)")

        do {
            try await generator.load(modelDirectory: nonExistentURL, modelId: turboId)
            XCTFail("load should have failed for non-existent directory")
        } catch let error as ImageGenError {
            XCTAssertEqual(error, ImageGenError.modelNotFound(nonExistentURL.path))
        } catch {
            XCTFail("Unexpected error type: \(error)")
        }

        let isLoaded = await generator.isLoaded
        XCTAssertFalse(isLoaded)
    }

    func testLoadRejectsAnUnknownModelIdBeforeTouchingDisk() async throws {
        // An existing directory, so only the id can fail — and it must, rather than
        // defaulting to a preset that does not match these weights.
        let fm = FileManager.default
        let weights = fm.temporaryDirectory
            .appendingPathComponent("weights-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: weights, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: weights) }

        let generator = PocketImageGenerator()
        do {
            try await generator.load(modelDirectory: weights, modelId: "mlx-community/flux-schnell")
            XCTFail("load should have refused an unsupported model id")
        } catch let error as ImageGenError {
            XCTAssertEqual(error, ImageGenError.unsupportedModel("mlx-community/flux-schnell"))
        }

        let isLoaded = await generator.isLoaded
        XCTAssertFalse(isLoaded)
    }

    // MARK: - Unit Tests: Generate Before Load Throws

    func testGenerateBeforeLoadThrowsNotLoaded() async {
        let generator = PocketImageGenerator()

        do {
            _ = try await generator.generate(
                prompt: "A cute orange cat in a cardboard box",
                negativePrompt: nil,
                size: 512,
                steps: 2,
                seed: 42,
                onStep: { _, _, _ in }
            )
            XCTFail("generate should have failed when model is not loaded")
        } catch let error as ImageGenError {
            XCTAssertEqual(error, ImageGenError.notLoaded)
        } catch {
            XCTFail("Unexpected error type: \(error)")
        }
    }

    // MARK: - Device-Gated Real Model Test

    /// Real image generation test requiring MLX, Apple Silicon, and downloaded SDXL-Turbo weights.
    /// Skipped unless `POCKET_MLX_IMAGE_TEST=1` is set in the environment.
    ///
    /// To run manually:
    /// `POCKET_MLX_IMAGE_TEST=1 POCKET_MLX_IMAGE_MODEL_PATH=/path/to/sdxl-turbo swift test --filter PocketImageGeneratorTests/testRealModelSDXLTurboGeneration`
    func testRealModelSDXLTurboGeneration() async throws {
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["POCKET_MLX_IMAGE_TEST"] != "1",
            "POCKET_MLX_IMAGE_TEST!=1: skipping real image generation model test requiring Apple Silicon and weights. Set POCKET_MLX_IMAGE_TEST=1 to run."
        )

        let modelId = ProcessInfo.processInfo.environment["POCKET_MLX_IMAGE_MODEL_ID"] ?? turboId
        let modelPath = ProcessInfo.processInfo.environment["POCKET_MLX_IMAGE_MODEL_PATH"]
            ?? ("\(NSHomeDirectory())/Library/Application Support/app.getpocketai/Models/\(modelId)")

        let modelURL = URL(fileURLWithPath: modelPath)
        try XCTSkipUnless(
            FileManager.default.fileExists(atPath: modelURL.path),
            "\(modelId) weights not found at \(modelPath)"
        )

        let generator = PocketImageGenerator()
        try await generator.load(modelDirectory: modelURL, modelId: modelId)

        let isLoaded = await generator.isLoaded
        XCTAssertTrue(isLoaded)

        final class ProgressBox: @unchecked Sendable {
            var stepProgressCount = 0
            var receivedPreview = false
        }
        let box = ProgressBox()

        let image = try await generator.generate(
            prompt: "A vibrant cinematic portrait of a red fox in the snow, 8k",
            negativePrompt: "blurry, low quality",
            size: 512,
            steps: 2,
            seed: 42
        ) { currentStep, totalSteps, preview in
            box.stepProgressCount += 1
            XCTAssertLessThanOrEqual(currentStep, totalSteps)
            if preview != nil {
                box.receivedPreview = true
            }
        }

        XCTAssertEqual(image.width, 512)
        XCTAssertEqual(image.height, 512)
        XCTAssertGreaterThan(box.stepProgressCount, 0)

        await generator.unload()
        let isLoadedAfterUnload = await generator.isLoaded
        XCTAssertFalse(isLoadedAfterUnload)
    }
}
