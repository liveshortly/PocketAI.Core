import XCTest
import SwiftUI
@testable import PocketUI

final class ComponentSnapshotTests: XCTestCase {
    @MainActor
    func testFitBadgeSnapshot() {
        let view = FitBadge(kind: .runsWell, reason: "Fits comfortably.")
        assertPocketSnapshot(view, name: "FitBadge")
    }

    @MainActor
    func testThinkingBadgeSnapshot() {
        let view = ThinkingBadge()
        assertPocketSnapshot(view, name: "ThinkingBadge")
    }

    @MainActor
    func testStatTileSnapshot() {
        let view = StatTile(label: "Requests today", value: "142", hint: "+12%")
        assertPocketSnapshot(view, name: "StatTile")
    }

    @MainActor
    func testToastSnapshot() {
        let view = ToastView(ToastData(message: "Model deleted", undo: {}))
        assertPocketSnapshot(view, name: "Toast")
    }

    @MainActor
    func testSettingsRowSnapshot() {
        let view = SettingsRow(icon: "network", title: "Network Proof", accessory: .navigation)
        assertPocketSnapshot(view, name: "SettingsRow")
    }

    @MainActor
    func testDeviceRowSnapshot() {
        let view = DeviceRow(name: "Rohit's iPhone", isConnected: true, lastSeenText: "", route: "Wi-Fi · 2 ms", onRevoke: {})
        assertPocketSnapshot(view, name: "DeviceRow")
    }

    @MainActor
    func testUserBubbleSnapshot() {
        let view = UserBubble(text: "Can you summarize this for me?")
        assertPocketSnapshot(view, name: "UserBubble")
    }

    @MainActor
    func testCodeBlockSnapshot() {
        let view = CodeBlock(code: "print(\"hello\")", language: "swift")
        assertPocketSnapshot(view, name: "CodeBlock")
    }

    @MainActor
    func testModelRowSnapshot() {
        let view = ModelRow(
            data: ModelRowData(
                name: "Qwen3 4B",
                familyParamsQuant: "Qwen3 · 4B · 4-bit",
                fitKind: .runsWell,
                fitReason: "Fits comfortably.",
                sizeOnDisk: "2.3 GB",
                speedText: "32 tok/s",
                speedIsMeasured: true,
                licenseLabel: "Apache 2.0"
            ),
            primaryActionLabel: "Open",
            onPrimaryAction: {}
        )
        assertPocketSnapshot(view, name: "ModelRow")
    }

    @MainActor
    func testDownloadProgressSnapshot() {
        let view = DownloadProgress(
            state: .downloading(bytesDone: 1_200_000_000, bytesTotal: 2_300_000_000, bytesPerSecond: 18_000_000),
            onPauseResume: {},
            onCancel: {}
        )
        assertPocketSnapshot(view, name: "DownloadProgress")
    }

    @MainActor
    func testMessageViewSnapshot() {
        let view = MessageView(
            text: "Here's a **short** summary with some code:\n\n```swift\nprint(\"hi\")\n```",
            reasoning: "Considered a few phrasings.",
            statsText: "32 tok/s · 1.2s"
        )
        assertPocketSnapshot(view, name: "MessageView")
    }

    @MainActor
    func testEmptyStateSnapshot() {
        let view = EmptyState(
            title: "Ask me anything.",
            suggestions: ["Summarize a document", "Help me write an email"],
            onTapSuggestion: { _ in }
        )
        .frame(height: 260)
        assertPocketSnapshot(view, name: "EmptyState")
    }

    @MainActor
    func testComposerSnapshot() {
        let view = Composer(
            text: .constant(""),
            modelChipLabel: "Qwen3 4B",
            isFocused: true,
            isGenerating: false,
            onAttach: { _ in },
            onMic: {},
            onSend: {},
            onStop: {}
        )
        assertPocketSnapshot(view, name: "Composer")
    }
}
