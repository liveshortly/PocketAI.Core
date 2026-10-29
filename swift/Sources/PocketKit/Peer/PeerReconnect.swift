import Foundation
import Network

/// A paired host's current LAN address, resolved from Bonjour (APL-08 reconnect).
public struct ResolvedPeerAddress: Sendable, Equatable {
    public let host: String
    public let port: UInt16
    public init(host: String, port: UInt16) {
        self.host = host
        self.port = port
    }
}

/// How the phone is currently reaching a paired Mac.
///
/// The route is not a setting — it falls out of which address wins, and the
/// user needs it named because the three behave differently when something
/// breaks: Bonjour heals itself on the same Wi-Fi, a mesh hostname keeps
/// working from anywhere, and a pairing-time address goes stale the moment
/// the Mac's IP changes.
public enum PeerRoute: Sendable, Equatable {
    /// A hostname the user typed — a VPN or mesh network such as Tailscale
    /// MagicDNS. Works off-LAN, which is the whole reason it exists.
    case customHost(ResolvedPeerAddress)
    /// Rediscovered over Bonjour this run, so the Mac moved and we followed.
    case localDiscovery(ResolvedPeerAddress)
    /// Whatever pairing recorded. Correct until the Mac's address changes.
    case pairedAddress

    public var address: ResolvedPeerAddress? {
        switch self {
        case .customHost(let a), .localDiscovery(let a): return a
        case .pairedAddress: return nil
        }
    }

    public var title: String {
        switch self {
        case .customHost: return "Custom address"
        case .localDiscovery: return "Local network"
        case .pairedAddress: return "Local network"
        }
    }

    /// One line explaining what this route can and cannot survive.
    public var detail: String {
        switch self {
        case .customHost:
            return "Reaches this Mac over a VPN or mesh network, so it keeps working away from home."
        case .localDiscovery:
            return "Found on this Wi-Fi. It will stop working away from this network."
        case .pairedAddress:
            return "Using the address recorded when you paired. If the Mac's IP changed, add a custom address."
        }
    }

    public var isOffLanCapable: Bool {
        if case .customHost = self { return true }
        return false
    }
}

/// Session-scoped address overrides for paired hosts whose LAN IP changed
/// since pairing. Bonjour rediscovery writes here; `PeerClient` builders read
/// here before falling back to the core's stored `addrs`. Bonjour overrides are
/// deliberately NOT persisted — `PocketCore` has no update-peer-address FFI
/// (`save_peer` only inserts), so they heal the current run and the next launch
/// re-resolves. A *manual* address the user typed is persisted in UserDefaults
/// and wins, because off-LAN (VPN / Tailscale) it's the only working route.
/// ponytail: UserDefaults per hostId, move into Core once it gains update-peer.
public final class PeerAddressBook: @unchecked Sendable {
    public static let shared = PeerAddressBook()

    private let lock = NSLock()
    private var overrides: [String: ResolvedPeerAddress] = [:]

    public init() {}

    /// hostId is the Bonjour instance name == `Peer.name` (see `save_peer`).
    /// A user-entered manual address wins over a Bonjour-rediscovered one: it's
    /// the only thing that works where Bonjour doesn't (VPN / mesh networks such
    /// as Tailscale MagicDNS), and the user asked for it explicitly.
    public func override(forHostId hostId: String) -> ResolvedPeerAddress? {
        if let manual = manualAddress(forHostId: hostId) { return manual }
        lock.lock(); defer { lock.unlock() }
        return overrides[hostId]
    }

    public func setOverride(_ addr: ResolvedPeerAddress, forHostId hostId: String) {
        lock.lock(); overrides[hostId] = addr; lock.unlock()
    }

    /// The route in use for this host right now, in the same precedence order
    /// `override(forHostId:)` resolves addresses.
    public func route(forHostId hostId: String) -> PeerRoute {
        if let manual = manualAddress(forHostId: hostId) { return .customHost(manual) }
        lock.lock(); let discovered = overrides[hostId]; lock.unlock()
        if let discovered { return .localDiscovery(discovered) }
        return .pairedAddress
    }

    /// Drops this run's Bonjour rediscovery so the next call re-resolves.
    /// Recovering from a wedged link without un-pairing: a manual address is
    /// left alone, since the user set it deliberately.
    public func clearDiscovered(forHostId hostId: String) {
        lock.lock(); overrides[hostId] = nil; lock.unlock()
    }

    // MARK: - Manual address (persisted)

    private static func manualKey(_ hostId: String) -> String { "peer.manualAddress.\(hostId)" }

    /// The address the user typed for this peer, if any. Persisted (unlike the
    /// Bonjour overrides above) because off-LAN it's the *only* usable route,
    /// so it has to survive relaunch.
    public func manualAddress(forHostId hostId: String) -> ResolvedPeerAddress? {
        guard let raw = UserDefaults.standard.string(forKey: Self.manualKey(hostId)) else { return nil }
        return Self.parse(raw, defaultPort: 0)
    }

    /// Stores (or, with nil, clears) the manual address. Returns false when the
    /// input isn't a usable `host[:port]` — `PeerClient` builds its URL from
    /// this, so a bad value must never be persisted.
    @discardableResult
    public func setManualAddress(_ input: String?, defaultPort: UInt16, forHostId hostId: String) -> Bool {
        guard let input, !input.trimmingCharacters(in: .whitespaces).isEmpty else {
            UserDefaults.standard.removeObject(forKey: Self.manualKey(hostId))
            return true
        }
        guard let parsed = Self.parse(input, defaultPort: defaultPort) else { return false }
        UserDefaults.standard.set("\(parsed.host):\(parsed.port)", forKey: Self.manualKey(hostId))
        return true
    }

    /// Parses `host`, `host:port` (or a pasted `https://host:port`) into a
    /// connectable address. Validated through `URL` since `PeerClient`
    /// interpolates the host straight into an `https://` URL.
    public static func parse(_ input: String, defaultPort: UInt16) -> ResolvedPeerAddress? {
        var text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if let range = text.range(of: "://") { text = String(text[range.upperBound...]) }
        text = text.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        var host = text
        var port = defaultPort
        // Rightmost colon only, and only when it's a port — leaves IPv6 alone,
        // which PeerClient's URL builder can't take unbracketed anyway.
        if let colon = text.lastIndex(of: ":"), let parsed = UInt16(text[text.index(after: colon)...]), parsed > 0 {
            host = String(text[text.startIndex..<colon])
            port = parsed
        }
        guard !host.isEmpty, port > 0,
              host.rangeOfCharacter(from: CharacterSet(charactersIn: "/ \t:?#@")) == nil,
              URL(string: "https://\(host):\(port)/") != nil
        else { return nil }
        return ResolvedPeerAddress(host: host, port: port)
    }
}

extension Browser {
    /// Additive reconnect primitive (APL-08): one-shot Bonjour rediscovery of a
    /// known `hostId` (the Mac's advertised instance name), resolved to a
    /// connectable IPv4 `host:port`. Bounded by `timeout`; returns nil if the
    /// host isn't found/resolvable in time. The cert fingerprint is still
    /// enforced by `PeerClient`'s pinned TLS on the actual connect, so this
    /// only needs the hostId to be safe.
    public static func rediscover(hostId: String, timeout: TimeInterval = 3) async -> ResolvedPeerAddress? {
        await withCheckedContinuation { continuation in
            let resolver = BonjourAddressResolver(hostId: hostId, timeout: timeout) { resolved in
                continuation.resume(returning: resolved)
            }
            resolver.start()
        }
    }
}

/// Classic `NetServiceBrowser`/`NetService` resolver: finds `hostId` on the
/// LAN and resolves it to an IPv4 string that `PeerClient`'s
/// `https://host:port` URL builder consumes cleanly (NWConnection resolution
/// often yields a zone-scoped IPv6 that URLSession handles poorly). Self-retains
/// until it completes exactly once, so delegates (which hold it weakly) stay alive.
private final class BonjourAddressResolver: NSObject, NetServiceBrowserDelegate, NetServiceDelegate, @unchecked Sendable {
    private let hostId: String
    private let timeout: TimeInterval
    private let completion: (ResolvedPeerAddress?) -> Void

    private let browser = NetServiceBrowser()
    private var service: NetService?
    private var selfRetain: BonjourAddressResolver?
    private var finished = false
    private let lock = NSLock()

    init(hostId: String, timeout: TimeInterval, completion: @escaping (ResolvedPeerAddress?) -> Void) {
        self.hostId = hostId
        self.timeout = timeout
        self.completion = completion
    }

    func start() {
        selfRetain = self
        browser.delegate = self
        browser.schedule(in: .main, forMode: .common)
        browser.searchForServices(ofType: Advertiser.serviceType, inDomain: "local.")
        DispatchQueue.main.asyncAfter(deadline: .now() + timeout) { [weak self] in
            self?.finish(nil)
        }
    }

    private func finish(_ result: ResolvedPeerAddress?) {
        lock.lock()
        if finished { lock.unlock(); return }
        finished = true
        lock.unlock()
        browser.stop()
        service?.stop()
        completion(result)
        selfRetain = nil
    }

    // MARK: NetServiceBrowserDelegate

    func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
        guard service.name == hostId else { return }
        self.service = service
        service.delegate = self
        service.resolve(withTimeout: timeout)
    }

    // MARK: NetServiceDelegate

    func netServiceDidResolveAddress(_ sender: NetService) {
        guard let host = Self.firstIPv4(in: sender.addresses ?? []) else { return }
        finish(ResolvedPeerAddress(host: host, port: UInt16(sender.port)))
    }

    func netService(_ sender: NetService, didNotResolve errorDict: [String: NSNumber]) {
        finish(nil)
    }

    private static func firstIPv4(in addresses: [Data]) -> String? {
        for data in addresses {
            let host: String? = data.withUnsafeBytes { raw -> String? in
                guard let base = raw.baseAddress else { return nil }
                let sa = base.assumingMemoryBound(to: sockaddr.self)
                guard sa.pointee.sa_family == sa_family_t(AF_INET) else { return nil }
                var sin = base.assumingMemoryBound(to: sockaddr_in.self).pointee
                var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
                inet_ntop(AF_INET, &sin.sin_addr, &buffer, socklen_t(INET_ADDRSTRLEN))
                return String(cString: buffer)
            }
            if let host { return host }
        }
        return nil
    }
}
