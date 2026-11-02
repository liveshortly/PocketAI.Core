import XCTest
@testable import PocketKit

/// APL-04: Unit tests for `Net` internet client, offline mode enforcement, and egress reporting.
final class NetTests: XCTestCase {
    private final class EgressRecorder: @unchecked Sendable {
        private let lock = NSLock()
        var host: String?
        var purpose: NetPurpose?
        var bytesOut: Int64?
        var bytesIn: Int64?
        var count = 0

        func record(host: String, purpose: NetPurpose, bytesOut: Int64, bytesIn: Int64) {
            lock.lock()
            defer { lock.unlock() }
            self.host = host
            self.purpose = purpose
            self.bytesOut = bytesOut
            self.bytesIn = bytesIn
            self.count += 1
        }
    }

    func testOfflineModeBlocksDataRequests() async throws {
        let recorder = EgressRecorder()
        let net = Net(
            isOffline: { true },
            onEgress: { host, purpose, bytesOut, bytesIn in
                recorder.record(host: host, purpose: purpose, bytesOut: bytesOut, bytesIn: bytesIn)
            }
        )

        let request = URLRequest(url: URL(string: "https://example.com/test")!)
        do {
            _ = try await net.data(for: request, purpose: .other)
            XCTFail("expected NetError.offline")
        } catch let err as NetError {
            XCTAssertEqual(err, .offline)
        }
        XCTAssertEqual(recorder.count, 0)
    }

    func testOfflineModeBlocksBytesRequests() async throws {
        let net = Net(
            isOffline: { true },
            onEgress: { _, _, _, _ in }
        )

        let request = URLRequest(url: URL(string: "https://example.com/test")!)
        do {
            _ = try await net.bytes(for: request, purpose: .modelDownload)
            XCTFail("expected NetError.offline")
        } catch let err as NetError {
            XCTAssertEqual(err, .offline)
        }
    }

    func testRecordStreamedEgressInvokesCallback() {
        let recorder = EgressRecorder()
        let net = Net(
            isOffline: { false },
            onEgress: { host, purpose, bytesOut, bytesIn in
                recorder.record(host: host, purpose: purpose, bytesOut: bytesOut, bytesIn: bytesIn)
            }
        )

        net.recordStreamedEgress(host: "huggingface.co", purpose: .modelDownload, bytesIn: 1048576)
        XCTAssertEqual(recorder.host, "huggingface.co")
        XCTAssertEqual(recorder.purpose, .modelDownload)
        XCTAssertEqual(recorder.bytesIn, 1048576)
        XCTAssertEqual(recorder.count, 1)
    }

    func testBackgroundSessionConfigurationParameters() {
        let config = Net.makeBackgroundSessionConfiguration(identifier: "test.bg.session")
        XCTAssertEqual(config.identifier, "test.bg.session")
        XCTAssertFalse(config.isDiscretionary)
        XCTAssertTrue(config.sessionSendsLaunchEvents)
        XCTAssertTrue(config.waitsForConnectivity)
    }
}

/// NET-17: a Mac the user disconnected gets no traffic, except the session
/// open/close that tells it so.
final class PeerDisconnectTests: XCTestCase {
    func testDisconnectedMacGetsNoTraffic() async {
        let fp = "test-fp-\(UUID().uuidString)"
        // Unroutable on purpose: a request that did go out would fail differently.
        let client = PeerClient(endpoint: PeerEndpoint(host: "192.0.2.1", port: 9, fingerprint: fp), token: "t")
        PeerAddressBook.shared.setUserDisconnected(true, fingerprint: fp)
        defer { PeerAddressBook.shared.setUserDisconnected(false, fingerprint: fp) }

        do {
            _ = try await client.getJSON(path: "/pocket/v1/catalog")
            XCTFail("expected .disconnected")
        } catch {
            XCTAssertEqual(error as? PeerClientError, .disconnected)
        }
        let rtt = await client.measureRTT()
        XCTAssertNil(rtt)
        XCTAssertTrue(PeerAddressBook.shared.isUserDisconnected(fingerprint: fp))
        PeerAddressBook.shared.setUserDisconnected(false, fingerprint: fp)
        XCTAssertFalse(PeerAddressBook.shared.isUserDisconnected(fingerprint: fp))
    }
}

