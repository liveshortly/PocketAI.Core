import Foundation

public enum PairingPhase: Sendable, Equatable {
    case idle
    case connecting
    case waitingForApproval(pairingId: String)
    case approved
    case denied
    /// MAC-21: a user-facing sentence naming the step that failed.
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
        // MAC-21: each step logs and fails with its own message, never a
        // generic "not reachable".
        func fail(_ step: String, _ message: String) {
            NSLog("PocketAI pairing: %@ failed: %@", step, message)
            phase = .failed(message)
        }
        guard let offer = try? core.parsePairingQr(qr: qr), !offer.secret.isEmpty,
              !offer.certFingerprint.isEmpty, !offer.addrs.isEmpty, offer.port != 0 else {
            fail("qr", "That isn't a PocketAI pairing code. On your Mac, open Devices ▸ Pair iPhone and scan the code shown there.")
            return
        }
        NSLog("PocketAI pairing: qr ok host=%@ addrs=%@ port=%d", offer.hostName, offer.addrs.joined(separator: ","), offer.port)
        let client: PeerClient, host: String
        switch await firstReachable(offer) {
        case .success(let reached): client = reached.0; host = reached.1
        case .failure(let reason): fail("connect", reason.message(offer)); return
        }
        NSLog("PocketAI pairing: reached %@ with pinned TLS", host)
        do {
            let body: [String: Any] = [
                "secret": offer.secret,
                "device_name": deviceName(),
                "device_pubkey": "",
                "platform": devicePlatform,
            ]
            let createData: Data
            do {
                createData = try await client.postJSON(path: "/pocket/v1/pair", body: body)
            } catch PeerClientError.invalidResponse {
                fail("pair", "Your Mac didn't accept this code. Codes last 5 minutes and work once — show a new code on the Mac and scan again.")
                return
            }
            guard
                let created = try JSONSerialization.jsonObject(with: createData) as? [String: Any],
                let pairingId = created["pairing_id"] as? String
            else {
                fail("pair", "Your Mac sent an unexpected reply. Update PocketAI on both devices and try again.")
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
                        fail("approve", "Your Mac approved, but sent no access key. Try pairing again.")
                        return
                    }
                    do {
                        try save(offer: offer, host: host, token: token)
                    } catch {
                        fail("store", "Your Mac approved, but this iPhone couldn't save the pairing (\(error)). Try again.")
                        return
                    }
                    NSLog("PocketAI pairing: stored peer %@", offer.hostName)
                    phase = .approved
                    return
                case "denied":
                    phase = .denied
                    return
                default:
                    continue // still pending; poll again
                }
            }
            fail("approve", "No answer from your Mac. Click Allow on the Mac when it asks, then try again.")
        } catch {
            fail("exchange", "Lost the connection to your Mac while pairing (\(error.localizedDescription)). Try again.")
        }
    }

    /// Races every advertised address; on total failure, reports the most
    /// telling reason (a pin mismatch beats Local Network denied beats plain
    /// unreachable).
    private func firstReachable(_ offer: PairingOffer) async -> Result<(PeerClient, String), ReachFailure> {
        await withTaskGroup(of: Result<(PeerClient, String), ReachFailure>.self) { group in
            for addr in offer.addrs {
                group.addTask {
                    let endpoint = PeerEndpoint(host: addr, port: offer.port, fingerprint: offer.certFingerprint)
                    let client = PeerClient(endpoint: endpoint, token: "")
                    do {
                        _ = try await client.getJSON(path: "/pocket/v1/hello")
                        return .success((client, addr))
                    } catch {
                        return .failure(ReachFailure(error))
                    }
                }
            }
            var worst = ReachFailure.unreachable
            for await result in group {
                switch result {
                case .success:
                    group.cancelAll()
                    return result
                case .failure(let reason):
                    if reason.rank > worst.rank { worst = reason }
                }
            }
            return .failure(worst)
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

/// MAC-21: why no advertised address answered.
enum ReachFailure: Error {
    case unreachable, localNetworkDenied, pinMismatch

    init(_ error: Error) {
        let ns = error as NSError
        if ns.code == NSURLErrorCancelled
            || ns.code == NSURLErrorServerCertificateUntrusted {
            // PeerClient cancels the TLS challenge on a pin mismatch.
            self = .pinMismatch
        } else if // ponytail: string match on the NWPath in userInfo; no public API exposes it
                  "\(ns.userInfo)".contains("Local network prohibited")
                    || "\(ns.userInfo)".contains("localNetworkDenied") {
            self = .localNetworkDenied
        } else {
            self = .unreachable
        }
    }

    var rank: Int {
        switch self {
        case .unreachable: return 0
        case .localNetworkDenied: return 1
        case .pinMismatch: return 2
        }
    }

    func message(_ offer: PairingOffer) -> String {
        switch self {
        case .pinMismatch:
            return "Reached your Mac, but its security certificate doesn't match the code. Close Pair iPhone on the Mac, open it again, and scan the new code."
        case .localNetworkDenied:
            return "PocketAI can't see your local network. Turn on Settings ▸ Privacy & Security ▸ Local Network ▸ PocketAI, then try again."
        case .unreachable:
            return "Couldn't reach your Mac at \(offer.addrs.joined(separator: ", ")) (port \(offer.port)). Check both are on the same Wi-Fi and PocketAI is open on the Mac."
        }
    }
}

#if os(iOS)
import UIKit
#endif
