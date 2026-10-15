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
}
