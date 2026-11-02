import XCTest
@testable import PocketKit

final class CatalogReasoningTests: XCTestCase {
    func testReadsManifestFlagIncludingRemoteIds() {
        let catalog = [entry("mlx-community/Qwen3-8B-4bit@545d", reasoning: true),
                       entry("mlx-community/Qwen3-VL-4B-Instruct-4bit@2fd8", reasoning: false)]
        XCTAssertTrue(catalog.supportsReasoning(modelID: "mlx-community/Qwen3-8B-4bit@545d"))
        XCTAssertTrue(catalog.supportsReasoning(modelID: "mac:peer1/mlx-community/Qwen3-8B-4bit@545d"))
        // The flag, not the name: a Qwen3 id flagged false stays false.
        XCTAssertFalse(catalog.supportsReasoning(modelID: "mlx-community/Qwen3-VL-4B-Instruct-4bit@2fd8"))
    }

    func testIdsMissingFromCatalogFallBackToCore() {
        let catalog = [entry("mlx-community/Qwen3-8B-4bit@545d", reasoning: true)]
        // CORE-15: Mac-only models keep the toggle via core's heuristic.
        XCTAssertTrue(catalog.supportsReasoning(modelID: "mac:peer1/mlx-community/QwQ-32B-4bit"))
        XCTAssertTrue(catalog.supportsReasoning(modelID: "mlx-community/Qwen3-14B-4bit@unknown"))
        XCTAssertFalse(catalog.supportsReasoning(modelID: "mac:peer1/mlx-community/Llama-3.3-70B-Instruct-4bit"))
    }

    private func entry(_ id: String, reasoning: Bool) -> CatalogEntry {
        CatalogEntry(
            manifest: ModelManifest(
                id: id, displayName: id, family: "qwen", paramsB: 0, quant: "4bit",
                format: .mlxSafetensors, modalities: [.text], contextLength: 4096, files: [],
                license: LicenseInfo(spdx: "Apache-2.0", commercialOk: true, url: ""),
                hfRepo: id, hfRevision: "main", minRamBytes: 0, estRuntimeBytes: 0,
                tags: [], curated: true, reasoning: reasoning),
            fit: FitReport(verdict: .runsWell, reason: "", estTokensPerSec: nil, measuredTokensPerSec: nil,
                           estRuntimeBytes: 0, diskBytes: 0, confidence: .estimated),
            state: .available, bytesDone: 0)
    }
}
