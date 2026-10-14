import Foundation

/// Deterministic fixture implementation of `CoreClient` (UI-03). Lets UI
/// lanes build every screen without waiting for Rust; launch argument
/// `-PocketPreviewCore YES` switches the whole app to this implementation.
public final class PreviewCore: CoreClient, @unchecked Sendable {
    public init() {}

    private static let fixtureWords = [
        "Hello", "from", "Preview", "Core", "—", "deterministic", "fake", "streaming", "at", "forty", "tokens", "per", "second.",
    ]

    public func mockGenerate(seed: Int64, tokensPerSec: Float) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            Task {
                let delayMs = UInt64(1000 / max(tokensPerSec, 1))
                for word in Self.fixtureWords {
                    try? await Task.sleep(nanoseconds: delayMs * 1_000_000)
                    continuation.yield(word + " ")
                }
                continuation.finish()
            }
        }
    }

    // MARK: - Catalog

    /// Mirrors a representative slice of CORE-06's curated catalog (real
    /// ids/sizes/tags from `pocket-models/data/catalog.json`) with a fit
    /// report computed against a fixed 8 GB iPhone preview profile, so
    /// onboarding/model-picker UI has real-looking data without Rust.
    private static let fixtureCatalog: [CatalogEntry] = [
        catalogEntry(
            id: "mlx-community/Qwen3-1.7B-4bit@3b1b1768f8f8cf8351c712464f906e86c2b8269e",
            name: "Qwen3-1.7B-4bit", family: "Qwen3", modalities: [.text],
            tags: ["recommended", "tier:6gb_iphone"], runtimeBytes: 1_126_428_293,
            verdict: .runsWell, reason: "Fits comfortably in 8 GB.", estTokPerSec: 38
        ),
        catalogEntry(
            id: "mlx-community/Llama-3.2-1B-Instruct-4bit@08231374eeacb049a0eade7922910865b8fce912",
            name: "Llama-3.2-1B-Instruct-4bit", family: "Llama", modalities: [.text],
            tags: ["tier:6gb_iphone"], runtimeBytes: 819_367_917,
            verdict: .runsWell, reason: "Fits comfortably in 8 GB.", estTokPerSec: 44
        ),
        catalogEntry(
            id: "mlx-community/Qwen3-4B-Instruct-2507-4bit@50d427756c6b1b2fe0c0a10f67fbda1fc8e82c1b",
            name: "Qwen3-4B-Instruct-2507-4bit", family: "Qwen3", modalities: [.text],
            tags: ["recommended", "tier:8gb_iphone"], runtimeBytes: 2_615_611_831,
            verdict: .runsWell, reason: "Fits comfortably; uses most of this iPhone's budget.", estTokPerSec: 22
        ),
        catalogEntry(
            id: "mlx-community/Phi-4-mini-instruct-4bit@ac1c269cb4222a4e136a3d09edad301056c1f36a",
            name: "Phi-4-mini-instruct-4bit", family: "Phi", modalities: [.text],
            tags: ["tier:8gb_iphone"], runtimeBytes: 2_499_668_624,
            verdict: .runsSlow, reason: "Fits, but little memory headroom left for other apps.", estTokPerSec: 14
        ),
        catalogEntry(
            id: "mlx-community/Qwen3-8B-4bit@545dc4251c05440727734bcd94334791f6ab0192",
            name: "Qwen3-8B-4bit", family: "Qwen3", modalities: [.text],
            tags: ["recommended", "tier:mac_16gb"], runtimeBytes: 5_312_146_502,
            verdict: .willNotFit, reason: "Needs 5.3 GB; only 4.6 GB available for models.", estTokPerSec: nil
        ),
        catalogEntry(
            id: "mlx-community/Qwen3-VL-4B-Instruct-4bit@2fd8dacbdb8f1e54b8c005f081ec5bf79c56376b",
            name: "Qwen3-VL-4B-Instruct-4bit", family: "Qwen3", modalities: [.text, .vision],
            tags: [], runtimeBytes: 3_570_968_427,
            verdict: .runsWell, reason: "Fits comfortably in 8 GB.", estTokPerSec: 18
        ),
        catalogEntry(
            id: "argmaxinc/whisperkit-coreml@0f63a7800b00dd0226abd051b906c246e1907482#iphone",
            name: "whisperkit-coreml", family: "Whisper", modalities: [.speechToText],
            tags: ["iphone"], runtimeBytes: 557_101_191,
            verdict: .runsWell, reason: "Fits comfortably in 8 GB.", estTokPerSec: nil
        ),
        catalogEntry(
            id: "FluidInference/kokoro-82m-coreml@d93c5f3eed11847fb81bd6d6a76665e133d7ca77",
            name: "kokoro-82m-coreml", family: "Kokoro", modalities: [.textToSpeech],
            tags: [], runtimeBytes: 830_917_191,
            verdict: .runsWell, reason: "Fits comfortably in 8 GB.", estTokPerSec: nil
        ),
        catalogEntry(
            id: "stabilityai/sdxl-turbo@71153311d3dbb46851df1931d3ca6e939de83304",
            name: "sdxl-turbo", family: "SDXL", modalities: [.image],
            tags: [], runtimeBytes: 7_978_713_144,
            verdict: .willNotFit, reason: "Needs 8.0 GB; only 4.6 GB available for models.", estTokPerSec: nil
        ),
    ]

    private static func catalogEntry(
        id: String, name: String, family: String, modalities: [Modality], tags: [String],
        runtimeBytes: UInt64, verdict: FitVerdict, reason: String, estTokPerSec: Float?
    ) -> CatalogEntry {
        CatalogEntry(
            manifest: ModelManifest(
                id: id, displayName: name, family: family, paramsB: 0, quant: "4bit",
                format: .mlxSafetensors, modalities: modalities, contextLength: 32768, files: [],
                license: LicenseInfo(spdx: "apache-2.0", commercialOk: true, url: "https://huggingface.co/\(id)"),
                hfRepo: id, hfRevision: "", minRamBytes: 0, estRuntimeBytes: runtimeBytes,
                tags: tags, curated: true
            ),
            fit: FitReport(
                verdict: verdict, reason: reason, estTokensPerSec: estTokPerSec,
                measuredTokensPerSec: nil, estRuntimeBytes: runtimeBytes, diskBytes: runtimeBytes,
                confidence: .estimated
            ),
            state: .available, bytesDone: 0
        )
    }

    private let downloadLock = NSLock()
    private var downloadOverrides: [String: (state: ModelState, bytesDone: UInt64)] = [:]

    public func catalog(filter: CatalogFilter) -> [CatalogEntry] {
        let overrides = downloadLock.withLock { downloadOverrides }
        return Self.fixtureCatalog.compactMap { entry in
            if let modality = filter.modality, !entry.manifest.modalities.contains(modality) { return nil }
            if filter.curatedOnly, !entry.manifest.curated { return nil }
            var entry = entry
            if let override = overrides[entry.manifest.id] {
                entry.state = override.state
                entry.bytesDone = override.bytesDone
            }
            if filter.installedOnly, entry.state != .installed { return nil }
            return entry
        }
    }

    /// UI-03: "downloads that progress over 20s" - ticks a fixture
    /// entry's `bytesDone`/`state` on a timer instead of touching the
    /// network, so onboarding/Models screens have something to show in
    /// Preview mode.
    public func startDownload(modelId: String) throws {
        guard let total = Self.fixtureCatalog.first(where: { $0.manifest.id == modelId })?.manifest.estRuntimeBytes else {
            throw PreviewCoreError.unknownModel(modelId)
        }
        downloadLock.withLock { downloadOverrides[modelId] = (.downloading, 0) }
        Task {
            let steps = 50
            for step in 1...steps {
                try? await Task.sleep(nanoseconds: 400_000_000)
                let done = UInt64(Double(total) * Double(step) / Double(steps))
                downloadLock.withLock {
                    downloadOverrides[modelId] = (step == steps ? .installed : .downloading, done)
                }
            }
        }
    }

    public func deleteModel(modelId: String) throws {
        downloadLock.withLock { downloadOverrides[modelId] = (.available, 0) }
    }

    // MARK: - Conversations

    private var fixtureConversations: [Conversation] = []
    private var fixtureMessages: [String: [Message]] = [:]

    public func conversations(query: ConversationQuery) -> [ConversationSummary] {
        fixtureConversations.map {
            ConversationSummary(id: $0.id, title: $0.title, updatedAt: $0.updatedAt, archived: $0.archived)
        }
    }

    public func createConversation(_ input: NewConversation) throws -> Conversation {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let conversation = Conversation(
            id: UUID().uuidString,
            title: input.title,
            projectId: input.projectId,
            modelId: input.modelId,
            systemPrompt: nil,
            createdAt: now,
            updatedAt: now,
            archived: false
        )
        fixtureConversations.append(conversation)
        return conversation
    }

    public func messages(conversationId: String) -> [Message] {
        fixtureMessages[conversationId] ?? []
    }

    public func deleteConversation(id: String) throws {
        fixtureConversations.removeAll { $0.id == id }
        fixtureMessages.removeValue(forKey: id)
    }

    public func sendMessage(conversationId: String, input: UserInput) -> AsyncThrowingStream<ChatStreamEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                var fullText = ""
                for word in Self.fixtureWords {
                    try? await Task.sleep(nanoseconds: 40 * 1_000_000)
                    let text = word + " "
                    fullText += text
                    continuation.yield(.delta(GenerateDelta(text: text, reasoning: "", toolCalls: [])))
                }
                let message = Message(
                    id: UUID().uuidString,
                    conversationId: conversationId,
                    role: "assistant",
                    parts: [.text(text: fullText)],
                    reasoning: nil,
                    stats: nil,
                    modelId: nil,
                    createdAt: Int64(Date().timeIntervalSince1970 * 1000),
                    status: .done
                )
                continuation.yield(.done(message))
                continuation.finish()
            }
        }
    }

    public func registerEngine(_ engine: Engine) {}

    // MARK: - Pairing & devices

    public func devices() -> [PairedDevice] {
        []
    }

    public func revokeDevice(id: String) throws {}

    public func pairingOffer() throws -> PairingOffer {
        PairingOffer(hostId: "preview-host", hostName: "Preview Mac", addrs: ["192.168.1.5"], port: 7480, certFingerprint: "preview", secret: "preview-secret", expiresAtMs: 0)
    }

    public func parsePairingQr(_ qr: String) throws -> PairingOffer {
        try pairingOffer()
    }

    public func savePeer(_ approved: ApprovedPairing) throws -> Peer {
        Peer(
            id: approved.hostId,
            name: "Preview Mac",
            certSha256: approved.certFingerprint,
            addrs: [approved.addr],
            port: approved.port,
            tokenRef: approved.tokenKeychainRef,
            pairedAt: Int64(Date().timeIntervalSince1970 * 1000),
            lastSeenAt: nil
        )
    }

    // MARK: - Entitlements

    public func entitlements() -> Entitlements {
        Entitlements(pro: true, source: "preview", expiresAtMs: nil)
    }

    public func activateLicense(key: String) throws -> Entitlements {
        entitlements()
    }

    // MARK: - Server

    public func serverStart(cfg: ServerConfig) throws -> ServerStatus {
        ServerStatus(running: true, lanEnabled: cfg.lanEnabled, port: 7480, connectedDevices: 0)
    }

    public func serverStop() throws {}

    public func serverStatus() -> ServerStatus {
        ServerStatus(running: false, lanEnabled: false, port: 0, connectedDevices: 0)
    }
}

public enum PreviewCoreError: Error {
    case unknownModel(String)
}
