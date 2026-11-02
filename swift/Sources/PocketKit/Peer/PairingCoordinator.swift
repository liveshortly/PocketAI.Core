import Foundation

public enum PairingPhase: Sendable, Equatable {
    case idle
    case connecting
    case waitingForApproval(pairingId: String)
    case approved
    case denied
    case failed(String)
}

/// APL-10 (phone side): scans a Mac's pairing QR, races its candidate
/// addresses with pinned TLS, posts the secret, long-polls for approval,
/// and persists the result. Takes `PocketCore` directly rather than going
/// through `CoreClient` - that protocol still only exposes `mockGenerate`
/// (D22/D52), with no expanded surface yet to delegate to.
@MainActor
public final class PairingCoordinator: ObservableObject {
    @Published public private(set) var phase: PairingPhase = .idle

    private let core: PocketCore
    private let keychain: PlatformHooks

    public init(core: PocketCore, keychain: PlatformHooks) {
        self.core = core
        self.keychain = keychain
    }

    public func pair(qr: String, devicePlatform: String = "ios") async {
        phase = .connecting
        do {
            let offer = try core.parsePairingQr(qr: qr)
            guard let (client, host) = await firstReachable(offer) else {
                phase = .failed("no reachable host among \(offer.addrs)")
                return
            }

            let body: [String: Any] = [
                "secret": offer.secret,
                "device_name": deviceName(),
                "device_pubkey": "",
                "platform": devicePlatform,
            ]
            let createData = try await client.postJSON(path: "/pocket/v1/pair", body: body)
            guard
                let created = try JSONSerialization.jsonObject(with: createData) as? [String: Any],
                let pairingId = created["pairing_id"] as? String
            else {
                phase = .failed("malformed pairing response")
                return
            }
            phase = .waitingForApproval(pairingId: pairingId)

            // The server already long-polls up to 60s per attempt; a few
            // rounds covers a user who takes a while to tap Allow.
            for _ in 0..<5 {
                let pollData = try await client.getJSON(path: "/pocket/v1/pair/\(pairingId)")
                guard let polled = try JSONSerialization.jsonObject(with: pollData) as? [String: Any],
                    let status = polled["status"] as? String
                else { continue }

                switch status {
                case "approved":
                    guard let token = polled["token"] as? String else {
                        phase = .failed("approved with no token")
                        return
                    }
                    try save(offer: offer, host: host, token: token)
                    phase = .approved
                    return
                case "denied":
                    phase = .denied
                    return
                default:
                    continue // still pending; poll again
                }
            }
            phase = .failed("timed out waiting for approval")
        } catch {
            phase = .failed("\(error)")
        }
    }

    private func firstReachable(_ offer: PairingOffer) async -> (PeerClient, String)? {
        await withTaskGroup(of: (PeerClient, String)?.self) { group in
            for addr in offer.addrs {
                group.addTask {
                    let endpoint = PeerEndpoint(host: addr, port: offer.port, fingerprint: offer.certFingerprint)
                    let client = PeerClient(endpoint: endpoint, token: "")
                    return await client.measureRTT() != nil ? (client, addr) : nil
                }
            }
            for await result in group {
                if let result {
                    group.cancelAll()
                    return result
                }
            }
            return nil
        }
    }

    private func save(offer: PairingOffer, host: String, token: String) throws {
        let keychainKey = "peer.token.\(offer.hostId)"
        guard keychain.keychainSet(key: keychainKey, value: Data(token.utf8)) else {
            throw PairingCoordinatorError.keychainWriteFailed
        }
        let approved = ApprovedPairing(
            hostId: offer.hostId,
            hostName: offer.hostName,
            certFingerprint: offer.certFingerprint,
            addr: host,
            port: offer.port,
            deviceId: UUID().uuidString,
            tokenKeychainRef: keychainKey
        )
        _ = try core.savePeer(approved: approved)
    }

    private func deviceName() -> String {
        #if os(iOS)
        return UIDevice.current.name
        #else
        return Host.current().localizedName ?? "Mac"
        #endif
    }
}

public enum PairingCoordinatorError: Error {
    case keychainWriteFailed
}

#if os(iOS)
import UIKit
#endif
