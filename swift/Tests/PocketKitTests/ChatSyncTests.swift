import XCTest
@testable import PocketKit

/// NET-22 phase 2: the Swift side of sync - write signal, outbox status, and
/// a transport's NotEntitled reaching the caller intact.
final class ChatSyncTests: XCTestCase {
    final class DeniedTransport: SyncTransport, @unchecked Sendable {
        func request(method: String, path: String, body: Data) throws -> Data {
            throw PocketError.NotEntitled(feature: "sync")
        }
    }

    func testWritePostsSignalAndShowsPendingThenNotEntitledPropagates() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let core: CoreClient = try LiveCore(
            config: CoreConfig(appSupportDir: dir.path, platform: .ios),
            platform: PocketCoreTests.StubPlatformHooks())

        let posted = expectation(forNotification: .pocketLocalChatWrite, object: nil)
        let convo = try core.createConversation(NewConversation(title: "Hi", projectId: nil, modelId: nil))
        wait(for: [posted], timeout: 1)

        let status = core.syncStatus(peerId: "mac-1")
        XCTAssertNil(status.lastSyncedAt)
        XCTAssertTrue(status.pendingConversationIds.contains(convo.id))

        XCTAssertThrowsError(try core.syncNow(peerId: "mac-1", transport: DeniedTransport())) { error in
            XCTAssertEqual(error as? PocketError, .NotEntitled(feature: "sync"))
        }
        XCTAssertEqual(NetPurpose.label(for: NetPurpose.peerSync.rawValue), "To your Mac")
    }

    func testForbiddenBodyTellsSyncDisabledFromNotEntitled() {
        let off = Data(#"{"error":{"message":"off","code":"sync_disabled"}}"#.utf8)
        let pro = Data(#"{"error":{"message":"pro","code":"not_entitled"}}"#.utf8)
        XCTAssertEqual(PeerClient.forbidden(off).description, PeerClientError.syncDisabled.description)
        XCTAssertEqual(PeerClient.forbidden(pro).description, PeerClientError.notEntitled.description)
        XCTAssertEqual(PeerClient.forbidden(nil).description, PeerClientError.notEntitled.description)
    }
}
