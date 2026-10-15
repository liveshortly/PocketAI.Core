import XCTest
import PocketKit
@testable import PocketMLX

final class PocketMLXEngineTests: XCTestCase {
    func testEngineKindAndSupports() {
        let engine = PocketMLXEngine()
        XCTAssertEqual(engine.kind(), .mlx)

        let mlxManifest = ModelManifest(
            id: "mlx-community/Qwen3-1.7B-4bit@3b1b1768f8f8cf8351c712464f906e86c2b8269e",
            displayName: "Qwen3-1.7B-4bit",
            family: "Qwen3",
            paramsB: 1.7,
            quant: "4bit",
            format: .mlxSafetensors,
            modalities: [.text],
            contextLength: 40960,
            files: [],
            license: LicenseInfo(spdx: "apache-2.0", commercialOk: true, url: ""),
            hfRepo: "mlx-community/Qwen3-1.7B-4bit",
            hfRevision: "3b1b1768f8f8cf8351c712464f906e86c2b8269e",
            minRamBytes: 0,
            estRuntimeBytes: 1126428293,
            tags: ["recommended"],
            curated: true
        )
        XCTAssertTrue(engine.supports(model: mlxManifest))

        var ggufManifest = mlxManifest
        ggufManifest.format = .gguf
        XCTAssertFalse(engine.supports(model: ggufManifest))
    }

    func testResolveModelDirectory() {
        let manifest = ModelManifest(
            id: "mlx-community/Qwen3-1.7B-4bit@rev1",
            displayName: "Qwen3-1.7B-4bit",
            family: "Qwen3",
            paramsB: 1.7,
            quant: "4bit",
            format: .mlxSafetensors,
            modalities: [.text],
            contextLength: 40960,
            files: [],
            license: LicenseInfo(spdx: "apache-2.0", commercialOk: true, url: ""),
            hfRepo: "mlx-community/Qwen3-1.7B-4bit",
            hfRevision: "rev1",
            minRamBytes: 0,
            estRuntimeBytes: 1126428293,
            tags: [],
            curated: true
        )

        let resolved = PocketMLXEngine.resolveModelDirectory(manifest: manifest)
        XCTAssertTrue(resolved.path.hasSuffix("Models/mlx-community/Qwen3-1.7B-4bit/rev1"))
    }

    func testResolveEmbedderModelDirectory() {
        // When no env var or local models exist, returns nil
        let resolved = PocketMLXEngine.resolveEmbedderModelDirectory()
        // If not installed locally, returns nil; if installed, returns a valid URL
        if let resolved {
            XCTAssertTrue(FileManager.default.fileExists(atPath: resolved.path))
        }
    }

    func testBatchChunking() {
        // Assert batching 70 inputs yields chunks of 32/32/6
        let items70 = Array(0..<70)
        let chunks70 = chunkInputs(items70, size: 32)
        XCTAssertEqual(chunks70.count, 3)
        XCTAssertEqual(chunks70[0].count, 32)
        XCTAssertEqual(chunks70[1].count, 32)
        XCTAssertEqual(chunks70[2].count, 6)
        XCTAssertEqual(chunks70.flatMap { $0 }, items70)

        // Edge cases
        let empty: [Int] = []
        XCTAssertEqual(chunkInputs(empty, size: 32).count, 0)

        let items32 = Array(0..<32)
        let chunks32 = chunkInputs(items32, size: 32)
        XCTAssertEqual(chunks32.count, 1)
        XCTAssertEqual(chunks32[0].count, 32)

        let items33 = Array(0..<33)
        let chunks33 = chunkInputs(items33, size: 32)
        XCTAssertEqual(chunks33.count, 2)
        XCTAssertEqual(chunks33[0].count, 32)
        XCTAssertEqual(chunks33[1].count, 1)
    }

    func testL2Normalization() {
        // Normalised vector has unit length
        let vec: [Float] = [3.0, 4.0]
        let normalized = l2Normalize(vec)
        XCTAssertEqual(normalized.count, 2)
        XCTAssertEqual(normalized[0], 0.6, accuracy: 1e-6)
        XCTAssertEqual(normalized[1], 0.8, accuracy: 1e-6)

        let normSquared = normalized.reduce(0) { $0 + $1 * $1 }
        XCTAssertEqual(sqrt(normSquared), 1.0, accuracy: 1e-6)

        // Multi-dim vector
        let vec3d: [Float] = [1.0, -2.0, 3.0, -4.0, 5.0]
        let norm3d = l2Normalize(vec3d)
        let length = sqrt(norm3d.reduce(0) { $0 + $1 * $1 })
        XCTAssertEqual(length, 1.0, accuracy: 1e-5)

        // Guard against zero norm
        let zeroVec: [Float] = [0.0, 0.0, 0.0]
        let normZero = l2Normalize(zeroVec)
        XCTAssertEqual(normZero, [0.0, 0.0, 0.0])
    }

    func testCosineSimilarity() {
        let vecA: [Float] = [1.0, 2.0, 3.0]
        let normalizedA = l2Normalize(vecA)

        // Cosine of a vector with itself ≈ 1
        XCTAssertEqual(cosineSimilarity(normalizedA, normalizedA), 1.0, accuracy: 1e-6)

        // Orthogonal vectors
        let vecB: [Float] = [1.0, 0.0]
        let vecC: [Float] = [0.0, 1.0]
        XCTAssertEqual(cosineSimilarity(vecB, vecC), 0.0, accuracy: 1e-6)

        // Opposite vectors
        let vecD: [Float] = [-1.0, 0.0]
        XCTAssertEqual(cosineSimilarity(vecB, vecD), -1.0, accuracy: 1e-6)
    }

    /// End-to-end embedding model sanity test.
    /// Skipped unless `POCKET_MLX_EMBED_TEST=1` is set in the environment.
    ///
    /// To run manually:
    /// `POCKET_MLX_EMBED_TEST=1 swift test --filter PocketMLXEngineTests/testRealModelEmbedCosineSanity`
    func testRealModelEmbedCosineSanity() throws {
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["POCKET_MLX_EMBED_TEST"] != "1",
            "POCKET_MLX_EMBED_TEST!=1: skipping real embedding model test requiring Apple Silicon and weights. Set POCKET_MLX_EMBED_TEST=1 to run."
        )

        let engine = PocketMLXEngine()
        let inputs = [
            "The quick brown fox jumps over the lazy dog.",
            "A fast brown fox leaps over a sleepy canine.",
            "Quantum computing uses superposition and entanglement for computation."
        ]

        let embeddings = try engine.embed(handle: "", inputs: inputs)
        XCTAssertEqual(embeddings.count, 3)

        // Check unit length for each embedding
        for emb in embeddings {
            let length = sqrt(emb.reduce(0) { $0 + $1 * $1 })
            XCTAssertEqual(length, 1.0, accuracy: 1e-4)
        }

        // Paraphrase pair scores higher cosine similarity than unrelated pair
        let paraphraseSim = cosineSimilarity(embeddings[0], embeddings[1])
        let unrelatedSim = cosineSimilarity(embeddings[0], embeddings[2])

        XCTAssertGreaterThan(
            paraphraseSim,
            unrelatedSim,
            "Paraphrase similarity (\(paraphraseSim)) should exceed unrelated similarity (\(unrelatedSim))"
        )
    }

    // MARK: - APL-14 Vision / VLM Unit Tests

    func testDownscaleDimensionsMath() {
        // Long-edge > 1024 downscaled preserving aspect ratio
        let landscape = downscaledDimensions(width: 4000, height: 3000, maxLongEdge: 1024)
        XCTAssertEqual(landscape.width, 1024)
        XCTAssertEqual(landscape.height, 768)

        let portrait = downscaledDimensions(width: 3000, height: 4000, maxLongEdge: 1024)
        XCTAssertEqual(portrait.width, 768)
        XCTAssertEqual(portrait.height, 1024)

        // Images within budget remain unchanged (no upscaling)
        let smaller = downscaledDimensions(width: 800, height: 600, maxLongEdge: 1024)
        XCTAssertEqual(smaller.width, 800)
        XCTAssertEqual(smaller.height, 600)

        // Exact boundary
        let exactSquare = downscaledDimensions(width: 1024, height: 1024, maxLongEdge: 1024)
        XCTAssertEqual(exactSquare.width, 1024)
        XCTAssertEqual(exactSquare.height, 1024)

        let exactLandscape = downscaledDimensions(width: 1024, height: 768, maxLongEdge: 1024)
        XCTAssertEqual(exactLandscape.width, 1024)
        XCTAssertEqual(exactLandscape.height, 768)

        // 2:1 aspect ratio
        let wide = downscaledDimensions(width: 2048, height: 1024, maxLongEdge: 1024)
        XCTAssertEqual(wide.width, 1024)
        XCTAssertEqual(wide.height, 512)

        let tall = downscaledDimensions(width: 1024, height: 2048, maxLongEdge: 1024)
        XCTAssertEqual(tall.width, 512)
        XCTAssertEqual(tall.height, 1024)

        // Zero and negative dimensions handled safely
        let zero = downscaledDimensions(width: 0, height: 0, maxLongEdge: 1024)
        XCTAssertEqual(zero.width, 0)
        XCTAssertEqual(zero.height, 0)

        let negative = downscaledDimensions(width: -100, height: 200, maxLongEdge: 1024)
        XCTAssertEqual(negative.width, -100)
        XCTAssertEqual(negative.height, 200)

        // downscaledSize convenience produces matching CGSize
        let size = downscaledSize(for: CGSize(width: 4000, height: 3000), maxLongEdge: 1024)
        XCTAssertEqual(size.width, 1024)
        XCTAssertEqual(size.height, 768)
    }

    func testMessagesContainImagesDetection() {
        // Empty messages
        XCTAssertFalse(messagesContainImages([]))

        // Text-only messages
        let textOnlyMessages = [
            ChatMessage(role: "system", parts: [.text(text: "You are a helpful assistant")]),
            ChatMessage(role: "user", parts: [.text(text: "Describe this")]),
        ]
        XCTAssertFalse(messagesContainImages(textOnlyMessages))

        // Single message with imageRef part
        let imageMessage = [
            ChatMessage(role: "user", parts: [.imageRef(path: "/tmp/photo.jpg")]),
        ]
        XCTAssertTrue(messagesContainImages(imageMessage))

        // Mixed parts within a single message
        let mixedPartsMessage = [
            ChatMessage(
                role: "user",
                parts: [
                    .text(text: "What is in this picture?"),
                    .imageRef(path: "/tmp/photo.jpg"),
                ]
            ),
        ]
        XCTAssertTrue(messagesContainImages(mixedPartsMessage))

        // Multiple messages where only the second has an image
        let multiMessagesWithImage = [
            ChatMessage(role: "system", parts: [.text(text: "System prompt")]),
            ChatMessage(role: "user", parts: [.text(text: "Hello")]),
            ChatMessage(role: "assistant", parts: [.text(text: "Hi there")]),
            ChatMessage(role: "user", parts: [.imageRef(path: "/tmp/test.png")]),
        ]
        XCTAssertTrue(messagesContainImages(multiMessagesWithImage))
    }

    func testIsVisionModelDetection() {
        func makeManifest(modalities: [Modality], hfRepo: String = "org/model") -> ModelManifest {
            ModelManifest(
                id: "\(hfRepo)@rev",
                displayName: "Test Model",
                family: "test",
                paramsB: 1.0,
                quant: "4bit",
                format: .mlxSafetensors,
                modalities: modalities,
                contextLength: 2048,
                files: [],
                license: LicenseInfo(spdx: "apache-2.0", commercialOk: true, url: ""),
                hfRepo: hfRepo,
                hfRevision: "rev",
                minRamBytes: 0,
                estRuntimeBytes: 0,
                tags: [],
                curated: false
            )
        }

        // Vision modality
        let visionManifest = makeManifest(modalities: [.text, .vision])
        XCTAssertTrue(PocketMLXEngine.isVisionModel(manifest: visionManifest))

        // Image modality
        let imageManifest = makeManifest(modalities: [.image])
        XCTAssertTrue(PocketMLXEngine.isVisionModel(manifest: imageManifest))

        // Text only
        let textManifest = makeManifest(modalities: [.text])
        XCTAssertFalse(PocketMLXEngine.isVisionModel(manifest: textManifest))

        // Known VLM in registry with empty declared modalities
        let registryVLMManifest = makeManifest(
            modalities: [],
            hfRepo: "mlx-community/paligemma-3b-mix-448-8bit"
        )
        XCTAssertTrue(PocketMLXEngine.isVisionModel(manifest: registryVLMManifest))
    }

    func testLoadAndDownscaleImageErrorHandling() throws {
        // Missing file throws PocketError.EngineFailed
        let nonExistentPath = "/tmp/pocket_mlx_non_existent_\(UUID().uuidString).png"
        XCTAssertThrowsError(try loadAndDownscaleImage(from: nonExistentPath)) { error in
            guard case PocketError.EngineFailed(let engine, let message) = error else {
                XCTFail("Expected PocketError.EngineFailed, got \(error)")
                return
            }
            XCTAssertEqual(engine, .mlx)
            XCTAssertTrue(message.contains("Image not found at path"))
        }

        // Corrupted / non-image file throws PocketError.EngineFailed
        let tempCorruptURL = FileManager.default.temporaryDirectory.appendingPathComponent(
            "pocket_mlx_corrupt_\(UUID().uuidString).txt"
        )
        try "Not a valid image data payload".write(to: tempCorruptURL, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: tempCorruptURL) }

        XCTAssertThrowsError(try loadAndDownscaleImage(from: tempCorruptURL.path)) { error in
            guard case PocketError.EngineFailed(let engine, let message) = error else {
                XCTFail("Expected PocketError.EngineFailed, got \(error)")
                return
            }
            XCTAssertEqual(engine, .mlx)
            XCTAssertTrue(message.contains("Failed to decode image at path"))
        }
    }

    func testLoadAndDownscaleImageSuccess() throws {
        let ciContext = CIContext()
        let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!

        // Create a large 4000x3000 image file
        let largeCI = CIImage(color: .blue).cropped(to: CGRect(x: 0, y: 0, width: 4000, height: 3000))
        let largeURL = FileManager.default.temporaryDirectory.appendingPathComponent(
            "pocket_mlx_test_large_\(UUID().uuidString).png"
        )
        try ciContext.writePNGRepresentation(of: largeCI, to: largeURL, format: .RGBA8, colorSpace: colorSpace)
        defer { try? FileManager.default.removeItem(at: largeURL) }

        let downscaled = try loadAndDownscaleImage(from: largeURL.path, maxLongEdge: 1024)
        XCTAssertEqual(downscaled.extent.width, 1024, accuracy: 1.0)
        XCTAssertEqual(downscaled.extent.height, 768, accuracy: 1.0)

        // Create an 800x600 image file (should not be upscaled)
        let smallCI = CIImage(color: .green).cropped(to: CGRect(x: 0, y: 0, width: 800, height: 600))
        let smallURL = FileManager.default.temporaryDirectory.appendingPathComponent(
            "pocket_mlx_test_small_\(UUID().uuidString).png"
        )
        try ciContext.writePNGRepresentation(of: smallCI, to: smallURL, format: .RGBA8, colorSpace: colorSpace)
        defer { try? FileManager.default.removeItem(at: smallURL) }

        let notUpscaled = try loadAndDownscaleImage(from: smallURL.path, maxLongEdge: 1024)
        XCTAssertEqual(notUpscaled.extent.width, 800, accuracy: 1.0)
        XCTAssertEqual(notUpscaled.extent.height, 600, accuracy: 1.0)
    }

    /// Real device-gated VLM test.
    /// Skipped unless `POCKET_MLX_VLM_TEST=1` is set in the environment.
    /// Requires Apple Silicon and downloaded VLM model weights.
    func testRealModelVLMDescribePhoto() throws {
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["POCKET_MLX_VLM_TEST"] != "1",
            "POCKET_MLX_VLM_TEST!=1: skipping real VLM model test requiring Apple Silicon and weights. Set POCKET_MLX_VLM_TEST=1 to run."
        )

        let manifest = ModelManifest(
            id: "mlx-community/paligemma-3b-mix-448-8bit@main",
            displayName: "PaliGemma-3B",
            family: "paligemma",
            paramsB: 3.0,
            quant: "8bit",
            format: .mlxSafetensors,
            modalities: [.vision, .text],
            contextLength: 2048,
            files: [],
            license: LicenseInfo(spdx: "apache-2.0", commercialOk: true, url: ""),
            hfRepo: "mlx-community/paligemma-3b-mix-448-8bit",
            hfRevision: "main",
            minRamBytes: 0,
            estRuntimeBytes: 3_000_000_000,
            tags: ["vision"],
            curated: true
        )

        let engine = PocketMLXEngine()
        let loaded = try engine.load(model: manifest, opts: LoadOptions(contextLength: nil, pinnedEngine: nil))
        defer { try? engine.unload(handle: loaded.handle) }

        // Create a test image
        let ciContext = CIContext()
        let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
        let testImage = CIImage(color: .red).cropped(to: CGRect(x: 0, y: 0, width: 640, height: 480))
        let tempURL = FileManager.default.temporaryDirectory.appendingPathComponent(
            "pocket_mlx_vlm_test_\(UUID().uuidString).png"
        )
        try ciContext.writePNGRepresentation(of: testImage, to: tempURL, format: .RGBA8, colorSpace: colorSpace)
        defer { try? FileManager.default.removeItem(at: tempURL) }

        let req = GenerateRequest(
            requestId: UUID().uuidString,
            messages: [
                ChatMessage(
                    role: "user",
                    parts: [
                        .text(text: "Describe this image in English."),
                        .imageRef(path: tempURL.path),
                    ]
                )
            ],
            tools: [],
            params: SamplingParams(
                temperature: 0.0,
                topP: 1.0,
                topK: 0,
                minP: 0.0,
                maxTokens: 50,
                stop: [],
                seed: 42,
                repetitionPenalty: 0.0
            ),
            reasoning: .off
        )

        final class CollectorSink: TokenSink, @unchecked Sendable {
            var text = ""
            var completed = false
            func onDelta(delta: GenerateDelta) {
                text += delta.text
            }
            func onDone(stats: GenerateStats) {
                completed = true
            }
            func onError(err: PocketError) {}
        }

        let sink = CollectorSink()
        let stats = try engine.generate(handle: loaded.handle, req: req, sink: sink)
        XCTAssertTrue(sink.completed)
        XCTAssertGreaterThan(stats.completionTokens, 0)
        XCTAssertFalse(sink.text.isEmpty)
    }
}
