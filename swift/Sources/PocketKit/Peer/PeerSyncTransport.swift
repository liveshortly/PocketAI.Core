import Foundation

/// NET-22: the core's `SyncTransport` over a paired Mac's `PeerClient`
/// (pinned TLS + bearer token). Errors cross the FFI as `PocketError` only.
public final class PeerSyncTransport: SyncTransport, @unchecked Sendable {
    private let client: PeerClient
    private let lock = NSLock()
    private var bytesOut: UInt64 = 0
    private var bytesIn: UInt64 = 0

    public init(client: PeerClient) {
        self.client = client
    }

    public func request(method: String, path: String, body: Data) throws -> Data {
        do {
            let data = try client.blockingRequest(method: method, path: path, body: body)
            lock.lock(); bytesOut += UInt64(body.count); bytesIn += UInt64(data.count); lock.unlock()
            return data
        } catch PeerClientError.notEntitled {
            throw PocketError.NotEntitled(feature: "sync")
        } catch let error as PocketError {
            throw error
        } catch {
            throw PocketError.Network(message: "\(error)")
        }
    }

    /// One full sync with `peerId`, then one "To your Mac" row in this
    /// device's Network Proof log with every byte the run moved.
    /// ponytail: one row per run, not per request - a periodic sync logging
    /// 4-5 rows each time would bury real internet egress in the log.
    /// Blocking: call off the main thread.
    public static func sync(core: CoreClient, peerId: String, client: PeerClient) throws -> SyncReport {
        let transport = PeerSyncTransport(client: client)
        defer {
            transport.lock.lock()
            let (out, inn) = (transport.bytesOut, transport.bytesIn)
            transport.lock.unlock()
            if out + inn > 0 {
                core.recordEgress(EgressEntry(
                    ts: Int64(Date().timeIntervalSince1970 * 1000),
                    host: client.endpoint.host,
                    purpose: NetPurpose.peerSync.rawValue,
                    bytesOut: out, bytesIn: inn, initiatedBy: "sync"))
            }
        }
        return try core.syncNow(peerId: peerId, transport: transport)
    }
}
