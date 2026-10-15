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
}
