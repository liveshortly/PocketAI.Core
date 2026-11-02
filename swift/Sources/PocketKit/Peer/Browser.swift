import Foundation
import Network

public struct DiscoveredPeer: Sendable, Equatable {
    public let hostId: String
    public let name: String
    public let fingerprint: String
    public let endpoint: NWEndpoint
}

public enum BrowserState: Sendable, Equatable {
    case idle
    case browsing
    /// First browse attempt before the Local Network permission prompt has
    /// been answered - retried with backoff rather than surfaced as failure.
    case waitingForPermission
    case denied
    case failed(String)
}

/// APL-08 (iPhone side): discovers Mac hosts advertising
/// `Advertiser.serviceType` on the local network.
@MainActor
public final class Browser: ObservableObject {
    @Published public private(set) var peers: [DiscoveredPeer] = []
    @Published public private(set) var state: BrowserState = .idle

    private var browser: NWBrowser?
    private var retryWorkItem: DispatchWorkItem?
    private var retryDelay: TimeInterval = 0.5

    public init() {}

    public func start() {
        stop()
        state = .browsing
        retryDelay = 0.5

        let params = NWParameters()
        params.includePeerToPeer = false
        let browser = NWBrowser(for: .bonjour(type: Advertiser.serviceType, domain: nil), using: params)
        browser.stateUpdateHandler = { [weak self] newState in
            Task { @MainActor in self?.handleStateUpdate(newState) }
        }
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            Task { @MainActor in self?.handleResults(results) }
        }
        browser.start(queue: .main)
        self.browser = browser
    }

    public func stop() {
        browser?.cancel()
        browser = nil
        retryWorkItem?.cancel()
        retryWorkItem = nil
        state = .idle
    }

    private func handleStateUpdate(_ newState: NWBrowser.State) {
        switch newState {
        case .ready:
            state = .browsing
            retryDelay = 0.5
        case .waiting(let error):
            if isPolicyDenied(error) {
                state = .denied
            } else {
                state = .waitingForPermission
                scheduleRetry()
            }
        case .failed(let error):
            state = .failed(String(describing: error))
            scheduleRetry()
        default:
            break
        }
    }

    private func isPolicyDenied(_ error: NWError) -> Bool {
        if case .dns(let code) = error {
            return code == kDNSServiceErr_PolicyDenied
        }
        return false
    }

    private func scheduleRetry() {
        retryWorkItem?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.start() }
        retryWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + retryDelay, execute: work)
        retryDelay = min(retryDelay * 2, 8)
    }

    private func handleResults(_ results: Set<NWBrowser.Result>) {
        peers = results.compactMap { result in
            guard case .service(let name, _, _, _) = result.endpoint else { return nil }
            // NWBrowser reports a service as soon as it is seen, before its TXT
            // record has resolved — metadata is `.none` on that first pass. A
            // peer published then carried an empty fingerprint, which is not a
            // peer anyone can use: the fingerprint IS the TLS pin, so a
            // consumer either fails to connect or connects unpinned. Wait for
            // the TXT instead; `browseResultsChangedHandler` fires again when
            // the metadata lands, so the peer appears a moment later.
            guard case .bonjour(let txt) = result.metadata else { return nil }
            guard let fingerprint = txt.dictionary["fp"], !fingerprint.isEmpty else { return nil }
            let displayName = txt.dictionary["name"] ?? name
            return DiscoveredPeer(
                hostId: name, name: displayName, fingerprint: fingerprint,
                endpoint: result.endpoint)
        }
    }
}
