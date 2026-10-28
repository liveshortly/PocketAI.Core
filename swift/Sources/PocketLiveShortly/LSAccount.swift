import Foundation
import Observation
import Security

/// Connection state for the LiveShortly integration. Opt-in: nothing network
/// happens until the user connects from Settings; Disconnect wipes the
/// Keychain entry. One instance per app (create it in AppModel).
@Observable
@MainActor
public final class LSAccountModel {
    public enum Phase: Equatable, Sendable {
        case disconnected
        case connecting(userCode: String, verifyURL: URL)
        case connected(LSUser)
    }

    public private(set) var phase: Phase = .disconnected

    /// How this account's own messages are labelled in a session transcript —
    /// the name when there is one, else the local part of the email.
    public var handle: String {
        guard case .connected(let user) = phase else { return "you" }
        if !user.name.isEmpty { return user.name }
        if let prefix = user.email.split(separator: "@").first { return String(prefix) }
        return "you"
    }
    public private(set) var serverURL: URL
    public private(set) var client: LSClient?
    public var lastError: String?

    private var pollTask: Task<Void, Never>?

    public static let defaultServer = URL(string: "https://liveshortly.com")!

    public init() {
        serverURL = Self.defaultServer
        restore()
    }

    public var isConnected: Bool {
        if case .connected = phase { return true }
        return false
    }

    // MARK: - Connect (device flow)

    /// Kick off the device flow against `server`. Publishes the user code +
    /// verification URL (open it in the browser), then polls until approved.
    public func connect(server: URL) {
        cancelConnect()
        serverURL = server
        lastError = nil
        let c = LSClient(baseURL: server)
        pollTask = Task { [weak self] in
            do {
                let start = try await c.deviceStart()
                let verify = URL(string: start.verificationUriComplete ?? start.verificationUri)
                    ?? server.appendingPathComponent("device")
                await MainActor.run {
                    self?.phase = .connecting(userCode: start.userCode, verifyURL: verify)
                }
                let interval = UInt64(max(start.interval ?? 5, 2))
                let deadline = Date().addingTimeInterval(TimeInterval(start.expiresIn ?? 600))
                while !Task.isCancelled, Date() < deadline {
                    try await Task.sleep(nanoseconds: interval * 1_000_000_000)
                    do {
                        if let done = try await c.devicePoll(deviceCode: start.deviceCode) {
                            guard let access = done.accessToken else { continue }
                            let user = done.user ?? LSUser(email: "", name: "")
                            self?.finishConnect(
                                server: server, access: access,
                                refresh: done.refreshToken, user: user)
                            return
                        }
                    } catch let e as LSAPIError where e.status == 400 || e.status == 404 || e.status == 410 {
                        // The code itself is dead (expired/consumed) — stop.
                        throw e
                    } catch {
                        // Transient: iOS suspends the app (and kills in-flight
                        // requests, "connection lost") while the user approves
                        // in the browser. Keep polling until the code expires.
                        continue
                    }
                }
                guard !Task.isCancelled else { return }
                throw LSAPIError(status: 0, message: "Code expired — tap Connect to get a new one.")
            } catch {
                guard !Task.isCancelled else { return }
                await MainActor.run {
                    self?.lastError = error.localizedDescription
                    self?.phase = .disconnected
                }
            }
        }
    }

    public func cancelConnect() {
        pollTask?.cancel()
        pollTask = nil
        if case .connecting = phase { phase = .disconnected }
    }

    public func disconnect() {
        cancelConnect()
        LSKeychain.delete()
        client = nil
        phase = .disconnected
    }

    private func finishConnect(server: URL, access: String, refresh: String?, user: LSUser) {
        let saved = LSKeychain.Saved(
            server: server.absoluteString, access: access,
            refresh: refresh, email: user.email, name: user.name)
        LSKeychain.save(saved)
        adopt(saved)
    }

    private func restore() {
        guard let saved = LSKeychain.load() else { return }
        adopt(saved)
    }

    private func adopt(_ saved: LSKeychain.Saved) {
        guard let url = URL(string: saved.server) else { return }
        serverURL = url
        let c = LSClient(baseURL: url, accessToken: saved.access, refreshToken: saved.refresh)
        Task {
            await c.setOnTokensUpdated { access, refresh in
                var next = saved
                next.access = access
                next.refresh = refresh
                LSKeychain.save(next)
            }
        }
        client = c
        phase = .connected(LSUser(email: saved.email, name: saved.name))
    }
}

/// One keychain item holding the whole connection (server + token pair + who).
enum LSKeychain {
    struct Saved: Codable {
        var server: String
        var access: String
        var refresh: String?
        var email: String
        var name: String
    }

    private static let service = "app.getpocketai.liveshortly"
    private static let account = "connection"

    static func save(_ s: Saved) {
        guard let data = try? JSONEncoder().encode(s) else { return }
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(base as CFDictionary)
        var add = base
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(add as CFDictionary, nil)
    }

    static func load() -> Saved? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        let status = withUnsafeMutablePointer(to: &out) {
            SecItemCopyMatching(query as CFDictionary, $0)
        }
        query.removeValue(forKey: kSecReturnData as String)
        guard status == errSecSuccess, let data = out as? Data else { return nil }
        return try? JSONDecoder().decode(Saved.self, from: data)
    }

    static func delete() {
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(base as CFDictionary)
    }
}
