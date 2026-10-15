import CoreGraphics
import Foundation
import XCTest
@testable import PocketImageGen

final class PocketImageGeneratorTests: XCTestCase {
    // MARK: - Unit Tests: Step Clamping

    func testStepClamping() {
        XCTAssertEqual(PocketImageGenerator.clampSteps(-10), 1)
        XCTAssertEqual(PocketImageGenerator.clampSteps(0), 1)
        XCTAssertEqual(PocketImageGenerator.clampSteps(1), 1)
        XCTAssertEqual(PocketImageGenerator.clampSteps(2), 2)
        XCTAssertEqual(PocketImageGenerator.clampSteps(3), 3)
        XCTAssertEqual(PocketImageGenerator.clampSteps(4), 4)
        XCTAssertEqual(PocketImageGenerator.clampSteps(5), 4)
        XCTAssertEqual(PocketImageGenerator.clampSteps(10), 4)
        XCTAssertEqual(PocketImageGenerator.clampSteps(100), 4)
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
            try await generator.load(modelDirectory: nonExistentURL)
            XCTFail("load should have failed for non-existent directory")
        } catch let error as ImageGenError {
            XCTAssertEqual(error, ImageGenError.modelNotFound(nonExistentURL.path))
        } catch {
            XCTFail("Unexpected error type: \(error)")
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

        let modelPath = ProcessInfo.processInfo.environment["POCKET_MLX_IMAGE_MODEL_PATH"]
            ?? ("\(NSHomeDirectory())/Library/Application Support/app.getpocketai/Models/stabilityai/sdxl-turbo")

        let modelURL = URL(fileURLWithPath: modelPath)
        try XCTSkipUnless(
            FileManager.default.fileExists(atPath: modelURL.path),
            "SDXL-Turbo weights not found at \(modelPath)"
        )

        let generator = PocketImageGenerator()
        try await generator.load(modelDirectory: modelURL)

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
