import Foundation

/// The one protocol PocketAI's apps program against (A§4.6). `LiveCore` wraps
/// the generated UniFFI `PocketCore`; `PreviewCore` is deterministic fixtures
/// so UI lanes never wait for Rust.
///
/// Covers the slice of A§4.5's surface actually needed by what's built so
/// far (catalog, conversations, chat, pairing, devices, entitlements,
/// server lifecycle, engine registration). Methods are added here as more
/// of the surface gets a real caller.
public protocol CoreClient: AnyObject, Sendable {
    /// Proves the Swift-to-Rust-to-Swift streaming pattern that real chat
    /// streaming (`send`) will use: a token sink implemented in Swift, driven
    /// from Rust, turned into an `AsyncThrowingStream`.
    func mockGenerate(seed: Int64, tokensPerSec: Float) -> AsyncThrowingStream<String, Error>

    // MARK: - Catalog

    func catalog(filter: CatalogFilter) -> [CatalogEntry]

    /// Kicks off (or resumes) a model's download; progress shows up on
    /// subsequent `catalog()` calls via that entry's `bytesDone`/`state`.
    func startDownload(modelId: String) throws
    func deleteModel(modelId: String) throws

    // MARK: - Conversations

    func conversations(query: ConversationQuery) -> [ConversationSummary]
    func createConversation(_ input: NewConversation) throws -> Conversation
    func messages(conversationId: String) -> [Message]
    func deleteConversation(id: String) throws
    func archiveConversation(id: String, archived: Bool) throws
    func renameConversation(id: String, title: String) throws

    /// Streams one chat turn's deltas; the final element carries the
    /// saved assistant `Message` once generation completes. `send()`
    /// itself blocks synchronously at the FFI boundary (D21), so this
    /// runs it off the calling task via `Task.detached`.
    func sendMessage(conversationId: String, input: UserInput) -> AsyncThrowingStream<ChatStreamEvent, Error>

    func registerEngine(_ engine: Engine)

    // MARK: - Pairing & devices

    func devices() -> [PairedDevice]
    func revokeDevice(id: String) throws
    func pairingOffer() throws -> PairingOffer
    func parsePairingQr(_ qr: String) throws -> PairingOffer
    func savePeer(_ approved: ApprovedPairing) throws -> Peer

    // MARK: - Entitlements

    func entitlements() -> Entitlements
    func activateLicense(key: String) throws -> Entitlements

    // MARK: - Server

    func serverStart(cfg: ServerConfig) throws -> ServerStatus
    func serverStop() throws
    func serverStatus() -> ServerStatus
}

public enum ChatStreamEvent: Sendable {
    case delta(GenerateDelta)
    case done(Message)
}
