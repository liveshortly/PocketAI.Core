import Foundation

/// APL-08 (Mac side): advertises this host's paired-pairing-ready HTTPS
/// server over Bonjour. Uses `NetService` rather than `NWListener`: the
/// Rust server already owns the real TCP listener on `port`, and an
/// `NWListener` publishing `.service` would have to bind that same port
/// itself (a conflict) or advertise a different port than clients should
/// actually connect to. `NetService(domain:type:name:port:)` publishes the
/// mDNS record without binding any socket of its own, so it can advertise
/// a port some other component (the Rust server) is already listening on.
public final class Advertiser: NSObject, @unchecked Sendable {
    public static let serviceType = "_pocketai._tcp"

    private var service: NetService?
    private let queue = DispatchQueue(label: "app.getpocketai.advertiser")

    public override init() {
        super.init()
    }

    /// MAC-21: the instance name is the readable `name` ("Rohit's MacBook
    /// Pro"), trimmed to Bonjour's 63-byte limit; the stable `hostId` and the
    /// TLS cert `fingerprint` (A§7) go in TXT, so a phone re-finds its Mac by
    /// id/pin even if the name changes or Bonjour renames it on a collision.
    public func start(hostId: String, name: String, fingerprint: String, port: UInt16) {
        stop()
        let service = NetService(
            domain: "local.", type: Self.serviceType, name: Self.instanceName(name), port: Int32(port))
        service.delegate = self
        service.setTXTRecord(
            NetService.data(
                fromTXTRecord: [
                    "v": Data("1".utf8),
                    "name": Data(name.utf8),
                    "fp": Data(fingerprint.utf8),
                    "id": Data(hostId.utf8),
                ]
            )
        )
        service.schedule(in: .main, forMode: .default)
        service.publish()
        self.service = service
    }

    /// Bonjour instance names are capped at 63 UTF-8 bytes; cut on a
    /// character boundary so a long name still publishes.
    static func instanceName(_ name: String) -> String {
        var out = ""
        for ch in (name.isEmpty ? "PocketAI" : name) {
            if out.utf8.count + String(ch).utf8.count > 63 { break }
            out.append(ch)
        }
        return out
    }

    public func stop() {
        service?.stop()
        service = nil
    }
}

extension Advertiser: NetServiceDelegate {
    public func netServiceDidPublish(_ sender: NetService) {}

    public func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
        // Most commonly NetService.errorCode .collision (name already in
        // use on this network) or .invalid. Surfaced to the caller as a
        // plain stop; no in-process retry here since a name collision
        // across two paired Macs is a real naming conflict, not a
        // transient failure worth retrying automatically.
        NSLog("PocketAI: Bonjour publish of %@ failed: %@", sender.name, errorDict)
        stop()
    }
}
