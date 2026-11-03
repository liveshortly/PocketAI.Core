import SwiftUI

/// A real on-device catalog of UI-04's components, for visual
/// verification (light/dark/AX3) without relying on Xcode Previews alone
/// - launched via `-PocketComponentGallery` (see each app's entry point).
public struct ComponentGallery: View {
    public init() {}

    public var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                galleryContent
            }
            .onAppear {
                if ProcessInfo.processInfo.arguments.contains("-PocketComponentGalleryBottom") {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                        proxy.scrollTo("gallery-bottom", anchor: .bottom)
                    }
                }
            }
        }
        .background(PocketColor.bgCanvas)
    }

    @ViewBuilder
    private var galleryContent: some View {
            VStack(alignment: .leading, spacing: PocketSpacing.s24) {
                section("PocketMark (C01)") {
                    HStack(spacing: PocketSpacing.s24) {
                        PocketMarkView(state: .idle, size: 48)
                        PocketMarkView(state: .thinking(tokPerSec: 42), size: 48)
                        PocketMarkView(state: .loading, size: 48)
                        PocketMarkView(state: .live, size: 48)
                        PocketMarkView(state: .still, size: 48)
                        PocketMarkView(state: .offline, size: 48)
                    }
                }
                section("FitBadge (C09)") {
                    FitBadge(kind: .runsWell, reason: "Fits comfortably.")
                    FitBadge(kind: .willNotFit, reason: "Needs 9.2 GB; only 6.0 GB available.")
                }
                section("StatTile (C20)") {
                    HStack {
                        StatTile(label: "Requests today", value: "142", hint: "+12%")
                        StatTile(label: "Devices", value: "2")
                    }
                }
                section("Toast (C18)") {
                    ToastView(ToastData(message: "Model deleted", undo: {}))
                }
                section("SettingsRow (C19)") {
                    SettingsRow(icon: "network", title: "Network Proof", accessory: .navigation)
                    SettingsRow(icon: "airplane", tint: PocketColor.statusWarn, title: "Offline mode", accessory: .toggle(.constant(true)))
                }
                section("DeviceRow (C13)") {
                    DeviceRow(name: "Rohit's iPhone", isConnected: true, lastSeenText: "", route: "Wi-Fi · 2 ms", onRevoke: {})
                }
                section("UserBubble (C04)") {
                    UserBubble(text: "Can you summarize this for me?")
                }
                section("CodeBlock (C05)") {
                    CodeBlock(code: "print(\"hello\")", language: "swift")
                }
                section("ModelRow (C08)") {
                    ModelRow(
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
                }
                section("DownloadProgress (C10)") {
                    DownloadProgress(
                        state: .downloading(bytesDone: 1_200_000_000, bytesTotal: 2_300_000_000, bytesPerSecond: 18_000_000),
                        onPauseResume: {},
                        onCancel: {}
                    )
                }
                section("MessageView (C03)") {
                    MessageView(
                        text: "Here's a **short** summary with some code:\n\n```swift\nprint(\"hi\")\n```",
                        reasoning: "Considered a few phrasings.",
                        statsText: "32 tok/s · 1.2s"
                    )
                }
                section("EmptyState (C17)") {
                    EmptyState(title: "Ask me anything.", suggestions: ["Summarize a document", "Help me write an email"], onTapSuggestion: { _ in })
                        .frame(height: 260)
                }
                section("Composer (C06)") {
                    Composer(text: .constant(""), modelChipLabel: "Qwen3 4B", isFocused: true, isGenerating: false, onAttach: { _ in }, onMic: {}, onSend: {}, onStop: {})
                }
                section("PulseStrip (C02)") {
                    PulseStrip(state: .generating(modelName: "Qwen3 4B", location: "On this iPhone", tokPerSec: 27), onTap: {})
                    PulseStrip(state: .loading(modelName: "2.3 GB", detail: "Loading"), onTap: {})
                    PulseStrip(state: .offlineMode, onTap: {})
                }
                section("PulseDetailSheet (C02)") {
                    PulseDetailSheet(
                        ramUsedBytes: 2_300_000_000,
                        ramBudgetBytes: 4_600_000_000,
                        ramTotalBytes: 8_000_000_000,
                        thermalLabel: "Nominal",
                        loadedModels: [LoadedModelSummary(id: "1", name: "Qwen3 4B", residentBytes: 2_300_000_000, onEject: {})],
                        lastReply: LastReplyStats(ttftMs: 320, tokens: 84, tokPerSec: 27, isMeasured: true),
                        egressCountToday: 0
                    )
                    .frame(height: 360)
                }
                Color.clear.frame(height: 1).id("gallery-bottom")
            }
            .padding()
    }

    @ViewBuilder
    private func section(_ title: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s8) {
            Text(title).font(.pocket(.caption)).foregroundStyle(PocketColor.textTertiary)
            content()
        }
    }
}

#Preview("Light") {
    ComponentGallery()
}

#Preview("Dark") {
    ComponentGallery()
        .preferredColorScheme(.dark)
}
