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
        // GGUF entries: the real catalog ships these and the core routes them
        // to the built-in llama.cpp engine, so preview must show both engines
        // or the model library looks MLX-only.
        catalogEntry(
            id: "unsloth/Qwen3-4B-Instruct-2507-GGUF@a06e946bb6b655725eafa393f4a9745d460374c9",
            name: "Qwen3-4B-Instruct-2507-GGUF", family: "Qwen3", modalities: [.text],
            tags: ["recommended"], runtimeBytes: 2_620_000_000,
            verdict: .runsWell, reason: "Fits comfortably in 8 GB.", estTokPerSec: 16,
            format: .gguf
        ),
        catalogEntry(
            id: "unsloth/Llama-3.2-3B-Instruct-GGUF@e7d0997e49c9cb00d88b4c1a6a16aa894b0a1234",
            name: "Llama-3.2-3B-Instruct-GGUF", family: "Llama", modalities: [.text],
            tags: [], runtimeBytes: 2_020_000_000,
            verdict: .runsWell, reason: "Fits comfortably in 8 GB.", estTokPerSec: 21,
            format: .gguf
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
        runtimeBytes: UInt64, verdict: FitVerdict, reason: String, estTokPerSec: Float?,
        format: ModelFormat = .mlxSafetensors
    ) -> CatalogEntry {
        CatalogEntry(
            manifest: ModelManifest(
                id: id, displayName: name, family: family, paramsB: 0, quant: "4bit",
                format: format, modalities: modalities, contextLength: 32768, files: [],
                license: LicenseInfo(spdx: "apache-2.0", commercialOk: true, url: "https://huggingface.co/\(id)"),
                hfRepo: id, hfRevision: "", minRamBytes: 0, estRuntimeBytes: runtimeBytes,
                tags: tags, curated: true, reasoning: tags.contains("reasoning")
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
    private var downloadTasks: [String: Task<Void, Never>] = [:]

    public func startDownload(modelId: String) throws {
        guard let total = Self.fixtureCatalog.first(where: { $0.manifest.id == modelId })?.manifest.estRuntimeBytes else {
            throw PreviewCoreError.unknownModel(modelId)
        }
        downloadLock.withLock { downloadOverrides[modelId] = (.downloading, 0) }
        runDownloadTask(modelId: modelId, total: total, startingAt: 0)
    }

    public func pauseDownload(modelId: String) throws {
        downloadTasks[modelId]?.cancel()
        downloadTasks[modelId] = nil
        downloadLock.withLock {
            if let current = downloadOverrides[modelId] {
                downloadOverrides[modelId] = (.paused, current.bytesDone)
            }
        }
    }

    public func resumeDownload(modelId: String) throws {
        guard let total = Self.fixtureCatalog.first(where: { $0.manifest.id == modelId })?.manifest.estRuntimeBytes else {
            throw PreviewCoreError.unknownModel(modelId)
        }
        let resumeFrom = downloadLock.withLock { downloadOverrides[modelId]?.bytesDone ?? 0 }
        downloadLock.withLock { downloadOverrides[modelId] = (.downloading, resumeFrom) }
        runDownloadTask(modelId: modelId, total: total, startingAt: resumeFrom)
    }

    public func cancelDownload(modelId: String) throws {
        downloadTasks[modelId]?.cancel()
        downloadTasks[modelId] = nil
        downloadLock.withLock { downloadOverrides[modelId] = (.available, 0) }
    }

    private func runDownloadTask(modelId: String, total: UInt64, startingAt: UInt64) {
        downloadTasks[modelId] = Task {
            let steps = 50
            let startStep = Int(Double(startingAt) / Double(total) * Double(steps))
            for step in (startStep + 1)...steps {
                try? await Task.sleep(nanoseconds: 400_000_000)
                if Task.isCancelled { return }
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

    /// No models are ever resident in previews, so unload is a no-op.
    public func unloadModel(id: String) throws {}

    public func onMemoryWarning() {}

    /// Previews measure nothing: a fixed, plausible result so the benchmark UI
    /// has something to lay out without waiting on real inference.
    public func benchmarkModel(id: String) throws -> Benchmark {
        Benchmark(tps: 23.4, ttftMs: 420, peakBytes: 2_100_000_000)
    }

    public func storedBenchmark(id: String) -> Benchmark? { nil }

    // MARK: - Conversations

    /// UI-03: "conversations (5, incl. code, table, long reasoning, vision)" -
    /// titles only (message fixtures for those bodies are a later UI-09 fixture).
    private var fixtureConversations: [Conversation] = [
        "Explain this Rust borrow checker error", "Summarize this quarterly report table",
        "Why is my SwiftUI view re-rendering", "What's in this photo of my garden",
        "Draft a reply to this email",
    ].enumerated().map { index, title in
        Conversation(
            id: "fixture-\(index)", title: title, projectId: nil, modelId: nil, systemPrompt: nil,
            createdAt: Int64(1_700_000_000_000 + index * 60_000),
            updatedAt: Int64(1_700_000_000_000 + index * 60_000), archived: false
        )
    }
    private var fixtureMessages: [String: [Message]] = [
        "fixture-0": [
            Message(
                id: "msg-0-user",
                conversationId: "fixture-0",
                role: "user",
                parts: [.text(text: "Why can't I borrow `data` as mutable more than once in this loop?\n\n```rust\nlet mut data = vec![1, 2, 3];\nfor item in &data {\n    data.push(*item);\n}\n```")],
                reasoning: nil,
                stats: nil,
                modelId: nil,
                createdAt: 1_700_000_000_000,
                status: .done
            ),
            Message(
                id: "msg-0-asst",
                conversationId: "fixture-0",
                role: "assistant",
                parts: [.text(text: "In Rust, the borrow checker enforces the aliasing XOR mutability rule: you can have either any number of immutable references (`&T`) or exactly one mutable reference (`&mut T`), but never both simultaneously.\n\n**Why the code fails:**\nWhen you write `for item in &data`, you create an immutable borrow of `data` that lasts for the entire loop scope. Inside the loop, calling `data.push(*item)` requires a mutable borrow (`&mut data`) to potentially reallocate the underlying buffer.\n\n**Idiomatic Fix:**\nIf you want to duplicate or append elements, clone or collect first:\n\n```rust\nlet mut data = vec![1, 2, 3];\nlet items: Vec<i32> = data.clone();\nfor item in items {\n    data.push(item);\n}\n```\n\nThis decouples the iterator borrow from the vector mutation.")],
                reasoning: "The user is attempting to mutate a `Vec` while actively iterating over an immutable borrow. I need to explain the borrow checker violation clearly, show the exact conflict, and provide the idiomatic Rust solution using cloning or indexing.",
                stats: GenerateStats(promptTokens: 48, completionTokens: 142, ttftMs: 110, decodeTps: 34, finish: .stop),
                modelId: "Qwen3-4B-Instruct-2507-4bit",
                createdAt: 1_700_000_005_000,
                status: .done
            )
        ]
    ]

    public func conversation(id: String) -> Conversation? {
        fixtureConversations.first { $0.id == id }
    }

    public func cancelConversation(id: String) {}

    public func regenerate(conversationId: String) -> AsyncThrowingStream<ChatStreamEvent, Error> {
        AsyncThrowingStream { $0.finish() }
    }

    public func conversations(query: ConversationQuery) -> [ConversationSummary] {
        fixtureConversations
            .filter { conversation in
                if let archived = query.archived, conversation.archived != archived { return false }
                if let search = query.search, !search.isEmpty {
                    return conversation.title?.localizedCaseInsensitiveContains(search) ?? false
                }
                return true
            }
            .map {
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

    public func archiveConversation(id: String, archived: Bool) throws {
        guard let index = fixtureConversations.firstIndex(where: { $0.id == id }) else { return }
        fixtureConversations[index].archived = archived
    }

    public func renameConversation(id: String, title: String) throws {
        guard let index = fixtureConversations.firstIndex(where: { $0.id == id }) else { return }
        fixtureConversations[index].title = title
    }

    public func setConversationModel(id: String, modelId: String) throws {
        guard let index = fixtureConversations.firstIndex(where: { $0.id == id }) else { return }
        fixtureConversations[index].modelId = modelId
    }

    public func sendMessage(conversationId: String, input: UserInput, reasoning: ReasoningMode) -> AsyncThrowingStream<ChatStreamEvent, Error> {
        let userText = input.parts.compactMap { part -> String? in
            if case .text(let text) = part { return text }
            return nil
        }.joined()
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        fixtureMessages[conversationId, default: []].append(
            Message(
                id: UUID().uuidString, conversationId: conversationId, role: "user",
                parts: [.text(text: userText)], reasoning: nil, stats: nil, modelId: nil,
                createdAt: now, status: .done
            )
        )

        return AsyncThrowingStream { continuation in
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
                    stats: GenerateStats(promptTokens: 12, completionTokens: UInt32(Self.fixtureWords.count), ttftMs: 120, decodeTps: 25, finish: .stop),
                    modelId: nil,
                    createdAt: Int64(Date().timeIntervalSince1970 * 1000),
                    status: .done
                )
                self.fixtureMessages[conversationId, default: []].append(message)
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

    public func pairingIdentity() -> PairingIdentity {
        PairingIdentity(hostId: "preview-host", hostName: "Preview Mac", certFingerprint: "preview")
    }

    public func parsePairingQr(_ qr: String) throws -> PairingOffer {
        try pairingOffer()
    }

    /// Empty by default so the Mac tab shows its unpaired onboarding, which is
    /// what most preview captures want. `-PocketPreviewPairedMac` seeds one so
    /// the paired detail — route, reachability, reconnect — can be seen without
    /// a real Mac on the network.
    private var fixturePeers: [Peer] = {
        guard ProcessInfo.processInfo.arguments.contains("-PocketPreviewPairedMac") else { return [] }
        return [
            Peer(
                id: "preview-peer",
                name: "Studio MacBook Pro",
                certSha256: "ab:cd:ef:01:23:45:67:89",
                addrs: ["192.168.1.20"],
                port: 8765,
                tokenRef: "peer-preview",
                pairedAt: Int64(Date().timeIntervalSince1970 * 1000) - 86_400_000,
                lastSeenAt: Int64(Date().timeIntervalSince1970 * 1000) - 60_000
            )
        ]
    }()

    public func savePeer(_ approved: ApprovedPairing) throws -> Peer {
        let peer = Peer(
            id: approved.deviceId,
            // UI-24: mirror core's save_peer — name over id, never a path.
            name: approved.hostName.isEmpty ? "Mac" : approved.hostName,
            certSha256: approved.certFingerprint,
            addrs: [approved.addr],
            port: approved.port,
            tokenRef: approved.tokenKeychainRef,
            pairedAt: Int64(Date().timeIntervalSince1970 * 1000),
            lastSeenAt: nil
        )
        fixturePeers.append(peer)
        return peer
    }

    public func peers() -> [Peer] {
        fixturePeers
    }

    public func revokePeer(id: String) throws {
        fixturePeers.removeAll { $0.id == id }
    }

    public func pairingOfferQrString() throws -> String {
        "pocketai://pair?v=1&id=preview-host&n=Preview%20Mac&h=192.168.1.5&p=7480&fp=preview&s=preview-secret"
    }

    private var fixturePendingPairings: [PairingRequest] = [
        PairingRequest(id: "req-1", deviceName: "Rohit's iPhone", platform: "ios", sas: nil),
    ]

    public func pairingManualCode() throws -> ManualPairingCode {
        ManualPairingCode(code: "K7QM2XPA", addrs: ["192.168.1.5"], port: 7480,
                          expiresAtMs: Int64(Date().timeIntervalSince1970 * 1000) + 300_000)
    }

    public func pendingPairings() -> [PairingRequest] {
        fixturePendingPairings
    }

    public func approvePairing(id: String) throws {
        fixturePendingPairings.removeAll { $0.id == id }
    }

    public func denyPairing(id: String) throws {
        fixturePendingPairings.removeAll { $0.id == id }
    }

    // MARK: - Entitlements

    private var fixtureStoreEntitlement: StoreEntitlement?

    public func entitlements() -> Entitlements {
        Entitlements(pro: true, source: "preview", expiresAtMs: nil)
    }

    public func activateLicense(key: String) throws -> Entitlements {
        entitlements()
    }

    public func requestFreeLicense(email: String) throws -> String {
        "preview-free-key"
    }

    public func setStoreEntitlement(_ e: StoreEntitlement) {
        fixtureStoreEntitlement = e
    }

    // MARK: - Server

    private var fixtureServerStatus = ServerStatus(running: false, lanEnabled: false, port: 0, connectedDevices: 0)

    public func serverStart(cfg: ServerConfig) throws -> ServerStatus {
        fixtureServerStatus = ServerStatus(running: true, lanEnabled: cfg.lanEnabled, port: 7480, connectedDevices: 0)
        return fixtureServerStatus
    }

    public func serverStop() throws {
        fixtureServerStatus = ServerStatus(running: false, lanEnabled: false, port: 0, connectedDevices: 0)
    }

    public func serverStatus() -> ServerStatus {
        fixtureServerStatus
    }

    private var fixtureHasKey = false

    public func generateLocalApiKey() -> String {
        fixtureHasKey = true
        return "pk-preview00000000000000000000000"
    }

    public func hasLocalApiKey() -> Bool {
        fixtureHasKey
    }

    public func hostCertPem() -> String {
        """
        -----BEGIN CERTIFICATE-----
        MIICCTCCAbGgAwIBAgIUQkS5H89+p4R6s3F1vQ8J0qA0cWEwDAYIKoZIzj0EAwIF
        ADASMRAwDgYDVQQDDAdwb2NrZXRhMh4XDTI2MDEwMTAwMDAwMFoXDTI2MDEwMjAw
        MDAwMFowEjEQMA4GA1UEAwwHcG9ja2V0YTBZMBMGByqGSM49AgEGCCqGSM49AwEH
        A0IABJbOqf3sF5+lq9+X7QfJmZ5u7sYwX5aX+w6n+JkM5q5u4uJ3hQ3h6Xl6n5h4
        zQh2WpQ3u5lQv7b5g==
        -----END CERTIFICATE-----
        """
    }

    public func stopEverything() {
        fixtureServerStatus = ServerStatus(running: false, lanEnabled: false, port: 0, connectedDevices: 0)
    }

    // MARK: - Privacy

    private var offline = false
    /// UI-03/NET-15: 3 fixture egress entries, per UI-03's own spec.
    private let fixtureEgress: [EgressEntry] = [
        EgressEntry(ts: 1_700_000_100_000, host: "huggingface.co", purpose: "Model download", bytesOut: 1200, bytesIn: 1_126_428_293, initiatedBy: "user"),
        EgressEntry(ts: 1_700_000_200_000, host: "pocket.liveshortly.com", purpose: "Catalog refresh", bytesOut: 400, bytesIn: 182_000, initiatedBy: "system"),
        EgressEntry(ts: 1_700_000_300_000, host: "huggingface.co", purpose: "Hugging Face search", bytesOut: 600, bytesIn: 48_000, initiatedBy: "user"),
    ]

    public func isOffline() -> Bool { offline }

    public func setOfflineMode(on: Bool) { offline = on }

    public func egressLog(limit: UInt32) -> [EgressEntry] {
        limit == 0 ? fixtureEgress : Array(fixtureEgress.prefix(Int(limit)))
    }

    public func searchHub(query: String, format: ModelFormat) throws -> [CatalogEntry] {
        guard !offline else { throw PreviewCoreError.offline }
        // Honour `format`, like the live core does: a caller searching one
        // engine must not get the other's models back.
        return Self.fixtureCatalog.filter {
            $0.manifest.format == format
                && $0.manifest.displayName.localizedCaseInsensitiveContains(query)
        }
    }

    public func auditLog(query: AuditQuery) -> [AuditEntry] {
        [
            AuditEntry(id: 1, ts: 1_700_000_100_000, actor: "user", action: "license.activate", target: "pro", detailJson: "{}"),
            AuditEntry(id: 2, ts: 1_700_000_200_000, actor: "system", action: "offline_mode.set", target: "true", detailJson: "{}"),
            AuditEntry(id: 3, ts: 1_700_000_300_000, actor: "user", action: "device.revoke", target: "preview-host", detailJson: "{}"),
        ]
    }

    // MARK: - Projects & documents

    private var fixtureProjects: [Project] = [
        Project(
            id: "proj-1",
            name: "iOS Architecture",
            instructions: "Focus on clean Swift concurrency and SwiftUI patterns.",
            createdAt: 1_700_000_000_000
        ),
        Project(
            id: "proj-2",
            name: "Lexical Search Spec",
            instructions: "BM25 retrieval engine notes and design decisions.",
            createdAt: 1_700_000_060_000
        ),
    ]

    private var fixtureDocuments: [String: [Document]] = [
        "proj-1": [
            Document(
                id: "doc-1",
                projectId: "proj-1",
                name: "ArchitectureGuide.md",
                mime: "text/markdown",
                bytes: 4096,
                path: "ArchitectureGuide.md",
                textChars: 2400,
                embedModel: nil,
                state: "indexed",
                createdAt: 1_700_000_010_000
            ),
            Document(
                id: "doc-2",
                projectId: "proj-1",
                name: "DesignPatterns.pdf",
                mime: "application/pdf",
                bytes: 18432,
                path: "DesignPatterns.pdf",
                textChars: 8100,
                embedModel: nil,
                state: "indexed",
                createdAt: 1_700_000_020_000
            ),
        ],
        "proj-2": [
            Document(
                id: "doc-3",
                projectId: "proj-2",
                name: "BM25-Overview.txt",
                mime: "text/plain",
                bytes: 2048,
                path: "BM25-Overview.txt",
                textChars: 1500,
                embedModel: nil,
                state: "indexed",
                createdAt: 1_700_000_070_000
            ),
        ],
    ]

    public func createProject(name: String, instructions: String?) throws -> Project {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let project = Project(
            id: UUID().uuidString,
            name: name,
            instructions: instructions,
            createdAt: now
        )
        fixtureProjects.append(project)
        return project
    }

    public func listProjects() -> [Project] {
        fixtureProjects
    }

    public func renameProject(id: String, name: String) throws {
        guard let index = fixtureProjects.firstIndex(where: { $0.id == id }) else { return }
        fixtureProjects[index].name = name
    }

    public func setProjectInstructions(id: String, instructions: String?) throws {
        guard let index = fixtureProjects.firstIndex(where: { $0.id == id }) else { return }
        let trimmed = instructions?.trimmingCharacters(in: .whitespacesAndNewlines)
        fixtureProjects[index].instructions = (trimmed?.isEmpty ?? true) ? nil : trimmed
    }

    public func deleteProject(id: String) throws {
        fixtureProjects.removeAll { $0.id == id }
        fixtureDocuments.removeValue(forKey: id)
    }

    public func addDocument(projectId: String, name: String, mime: String, pages: [DocumentPage]) throws -> Document {
        let totalChars = pages.reduce(0) { $0 + $1.text.count }
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let document = Document(
            id: UUID().uuidString,
            projectId: projectId,
            name: name,
            mime: mime,
            bytes: UInt64(totalChars * 2),
            path: name,
            textChars: UInt32(totalChars),
            embedModel: nil,
            state: "indexed",
            createdAt: now
        )
        if fixtureDocuments[projectId] != nil {
            fixtureDocuments[projectId]?.append(document)
        } else {
            fixtureDocuments[projectId] = [document]
        }
        return document
    }

    public func listDocuments(projectId: String) -> [Document] {
        fixtureDocuments[projectId] ?? []
    }

    public func deleteDocument(id: String) throws {
        for key in fixtureDocuments.keys {
            fixtureDocuments[key]?.removeAll { $0.id == id }
        }
    }

    public func searchProject(projectId: String, query: String, k: UInt32) -> [ChunkHit] {
        let docs = fixtureDocuments[projectId] ?? []
        let limit = min(Int(k), max(1, docs.count))
        return (0..<limit).map { index in
            let doc = docs.indices.contains(index) ? docs[index] : (docs.first ?? Document(
                id: "fixture-doc", projectId: projectId, name: "ProjectDoc.txt",
                mime: "text/plain", bytes: 1024, path: "ProjectDoc.txt", textChars: 500,
                embedModel: nil, state: "indexed", createdAt: 0
            ))
            return ChunkHit(
                documentId: doc.id,
                ord: UInt32(index),
                text: "Excerpt from \(doc.name) addressing '\(query)': Core architectural component design and implementation notes.",
                page: UInt32(index + 1),
                score: Float(1.0) / Float(index + 1)
            )
        }
    }
}

public enum PreviewCoreError: Error {
    case unknownModel(String)
    case offline
}
