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

    public func catalog(filter: CatalogFilter) -> [CatalogEntry] {
        []
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
