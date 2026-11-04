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
    /// NET-23: for a manual-code pairing, the 6 digits the user compares
    /// with the Mac's Allow sheet ("123456"). Nil for QR pairing.
    @Published public private(set) var sas: String?

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
            fail("qr", String(localized: "That isn't a PocketAI pairing code. On your Mac, open Devices ▸ Pair iPhone and scan the code shown there.", bundle: .module))
            return
        }
        NSLog("PocketAI pairing: qr ok host=%@ addrs=%@ port=%d", offer.hostName, offer.addrs.joined(separator: ","), offer.port)
        let client: PeerClient, host: String
        switch await firstReachable(offer) {
        case .success(let reached): client = reached.0; host = reached.1
        case .failure(let reason): fail("connect", reason.message(addrs: offer.addrs, port: offer.port)); return
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
                fail("pair", String(localized: "Your Mac didn't accept this code. Codes last 5 minutes and work once — show a new code on the Mac and scan again.", bundle: .module))
                return
            }
            guard
                let created = try JSONSerialization.jsonObject(with: createData) as? [String: Any],
                let pairingId = created["pairing_id"] as? String
            else {
                fail("pair", String(localized: "Your Mac sent an unexpected reply. Update PocketAI on both devices and try again.", bundle: .module))
                return
            }
            phase = .waitingForApproval(pairingId: pairingId)
            try await awaitDecision(client: client, pairingId: pairingId, fail: fail) { token, _ in
                try self.save(hostId: offer.hostId, hostName: offer.hostName,
                              fingerprint: offer.certFingerprint, host: host, port: offer.port, token: token)
            }
        } catch {
            fail("exchange", String(localized: "Lost the connection to your Mac while pairing (\(error.localizedDescription)). Try again.", bundle: .module))
        }
    }

    /// NET-23: pairs from a typed address and 8-character code instead of a
    /// QR. The phone connects without a pin, computes the SAS from the
    /// certificate it actually saw, and shows it while the Mac's Allow sheet
    /// shows its own; only once the Mac allows does it pin that certificate.
    /// From then on the peer is stored exactly like a QR-paired one.
    public func pair(host: String, port: UInt16, code typed: String, devicePlatform: String = "ios") async {
        phase = .connecting
        sas = nil
        func fail(_ step: String, _ message: String) {
            NSLog("PocketAI manual pairing: %@ failed: %@", step, message)
            sas = nil
            phase = .failed(message)
        }
        guard let code = normalizePairingCode(code: typed) else {
            fail("code", String(localized: "A pairing code is 8 letters and numbers, like K7QM-2XPA. Check the code under Enter code manually on your Mac.", bundle: .module))
            return
        }
        // 1. Unpinned: learn which certificate this server presents.
        let fingerprint: String, hostId: String
        do {
            let probed = try await PeerClient.probeFingerprint(host: host, port: port)
            let hello = try JSONSerialization.jsonObject(with: probed.hello) as? [String: Any]
            guard let id = hello?["host_id"] as? String, !id.isEmpty else {
                fail("hello", String(localized: "That address answered, but it isn't PocketAI on a Mac. Check the address on your Mac.", bundle: .module))
                return
            }
            fingerprint = probed.fingerprint
            hostId = id
        } catch {
            fail("connect", ReachFailure(error).message(addrs: [host], port: port))
            return
        }
        // 2. The SAS binds the code to that certificate.
        guard let digits = try? pairingSas(certFingerprint: fingerprint, code: code) else {
            fail("sas", String(localized: "Couldn't check this Mac's identity. Try again.", bundle: .module))
            return
        }
        // 3. Everything else is pinned to the certificate the SAS covers.
        let client = PeerClient(endpoint: PeerEndpoint(host: host, port: port, fingerprint: fingerprint), token: "")
        do {
            let (data, status) = try await client.postJSONWithStatus(path: "/pocket/v1/pair", body: [
                "code": code,
                "device_name": deviceName(),
                "device_pubkey": "",
                "platform": devicePlatform,
            ])
            let reply = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            guard status == 200, let pairingId = reply?["pairing_id"] as? String else {
                let error = reply?["error"] as? [String: Any]
                fail("pair", Self.codeErrorMessage(error?["code"] as? String, attemptsLeft: error?["attempts_left"] as? Int))
                return
            }
            sas = digits
            phase = .waitingForApproval(pairingId: pairingId)
            try await awaitDecision(client: client, pairingId: pairingId, fail: fail) { token, hostName in
                try self.save(hostId: hostId, hostName: hostName ?? "Mac", fingerprint: fingerprint,
                              host: host, port: port, token: token)
            }
        } catch {
            fail("exchange", String(localized: "Lost the connection to your Mac while pairing (\(error.localizedDescription)). Try again.", bundle: .module))
        }
    }

    static func codeErrorMessage(_ code: String?, attemptsLeft: Int?) -> String {
        switch code {
        case "wrong_code":
            let left = attemptsLeft ?? 0
            return left == 1
                ? String(localized: "That code doesn't match the one on your Mac. 1 try left before it stops working.", bundle: .module)
                : String(localized: "That code doesn't match the one on your Mac. \(left) tries left before it stops working.", bundle: .module)
        case "code_locked":
            return String(localized: "Too many wrong codes, so that one no longer works. On your Mac, click New code and enter the new one.", bundle: .module)
        case "code_expired":
            return String(localized: "That code expired. Codes last 5 minutes — on your Mac, click New code and enter the new one.", bundle: .module)
        case "no_code":
            return String(localized: "Your Mac isn't showing a pairing code right now, or it was already used. On your Mac, open Devices › Pair a device… › Enter code manually.", bundle: .module)
        default:
            return String(localized: "Your Mac didn't accept this code. Show a new code on your Mac and try again.", bundle: .module)
        }
    }

    /// Long-polls the Mac's decision. On approval runs `store(token, hostName)`
    /// and moves to `.approved`; Deny stores nothing.
    private func awaitDecision(
        client: PeerClient, pairingId: String, fail: (String, String) -> Void,
        store: (String, String?) throws -> Void
    ) async throws {
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
                    fail("approve", String(localized: "Your Mac approved, but sent no access key. Try pairing again.", bundle: .module))
                    return
                }
                do {
                    try store(token, polled["host_name"] as? String)
                } catch {
                    fail("store", String(localized: "Your Mac approved, but this iPhone couldn't save the pairing (\(String(describing: error))). Try again.", bundle: .module))
                    return
                }
                NSLog("PocketAI pairing: stored peer")
                phase = .approved
                return
            case "denied":
                phase = .denied
                return
            default:
                continue // still pending; poll again
            }
        }
        fail("approve", String(localized: "No answer from your Mac. Click Allow on the Mac when it asks, then try again.", bundle: .module))
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

    private func save(hostId: String, hostName: String, fingerprint: String, host: String, port: UInt16, token: String) throws {
        let keychainKey = "peer.token.\(hostId)"
        guard keychain.keychainSet(key: keychainKey, value: Data(token.utf8)) else {
            throw PairingCoordinatorError.keychainWriteFailed
        }
        let approved = ApprovedPairing(
            hostId: hostId,
            hostName: hostName,
            certFingerprint: fingerprint,
            addr: host,
            port: port,
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

    func message(addrs: [String], port: UInt16) -> String {
        switch self {
        case .pinMismatch:
            return String(localized: "Reached your Mac, but its security certificate doesn't match the code. Close Pair iPhone on the Mac, open it again, and scan the new code.", bundle: .module)
        case .localNetworkDenied:
            return String(localized: "PocketAI can't see your local network. Turn on Settings ▸ Privacy & Security ▸ Local Network ▸ PocketAI, then try again.", bundle: .module)
        case .unreachable:
            return String(localized: "Couldn't reach your Mac at \(addrs.joined(separator: ", ")) (port \(Int(port))). Check both are on the same Wi-Fi and PocketAI is open on the Mac.", bundle: .module)
        }
    }
}

#if os(iOS)
import UIKit
#endif
