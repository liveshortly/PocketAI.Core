import XCTest
@testable import PocketKit

final class ReasoningSupportTests: XCTestCase {
    func testReasoningFamiliesMatch() {
        XCTAssertTrue(ReasoningSupport.supportsReasoning(modelID: "mlx-community/Qwen3-8B-4bit@545d"))
        XCTAssertTrue(ReasoningSupport.supportsReasoning(modelID: "unsloth/DeepSeek-R1-Distill-Qwen-7B-GGUF"))
        XCTAssertTrue(ReasoningSupport.supportsReasoning(modelID: "mlx-community/QwQ-32B-4bit"))
        XCTAssertTrue(ReasoningSupport.supportsReasoning(modelID: "mlx-community/gpt-oss-20b-MXFP4-Q8@773a"))
        // Remote (mac:) composite ids still match by substring.
        XCTAssertTrue(ReasoningSupport.supportsReasoning(modelID: "mac:peer1/mlx-community/Qwen3-1.7B-4bit@3b1b"))
    }

    func testNonReasoningModelsDoNotMatch() {
        XCTAssertFalse(ReasoningSupport.supportsReasoning(modelID: "mlx-community/Llama-3.2-3B-Instruct-4bit"))
        XCTAssertFalse(ReasoningSupport.supportsReasoning(modelID: "mlx-community/gemma-3-4b-it-qat-4bit"))
        // Qwen3 *-Instruct-2507 is the non-thinking split.
        XCTAssertFalse(ReasoningSupport.supportsReasoning(modelID: "mlx-community/Qwen3-4B-Instruct-2507-4bit@50d4"))
    }
}
