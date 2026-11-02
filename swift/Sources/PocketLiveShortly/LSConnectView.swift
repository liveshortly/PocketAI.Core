import SwiftUI
import PocketUI

/// Settings destination: connect / manage the LiveShortly account.
/// Device flow: start → open browser (user approves) → poll → connected.
public struct LSConnectView: View {
    @Bindable private var account: LSAccountModel
    @State private var server: String
    @State private var hosts: [LSHost] = []
    @Environment(\.openURL) private var openURL

    public init(account: LSAccountModel) {
        self.account = account
        _server = State(initialValue: account.serverURL.absoluteString)
    }

    public var body: some View {
        List {
            switch account.phase {
            case .disconnected: disconnectedSections
            case .connecting(let code, let url): connectingSections(code: code, url: url)
            case .connected(let user): connectedSections(user: user)
            }
        }
        .lsGroupedList()
        .scrollContentBackground(.hidden)
        .background(PocketColor.bgCanvas)
        .navigationTitle("LiveShortly")
        .lsInlineTitle()
    }

    @ViewBuilder private var disconnectedSections: some View {
        Section {
            VStack(alignment: .leading, spacing: PocketSpacing.s8) {
                Text("Watch and control your coding agents")
                    .font(.pocket(.headline))
                Text("See every `live` session from this phone, talk to the agent, answer permission prompts, and start new sessions on your machines. Sign in happens in your browser — PocketAI only stores a token.")
                    .font(.pocket(.footnote))
                    .foregroundStyle(PocketColor.textSecondary)
            }
            .padding(.vertical, PocketSpacing.s4)
        }
        Section("Server") {
            TextField("https://liveshortly.com", text: $server)
                .font(.pocket(.mono))
                .lsURLField()
                .autocorrectionDisabled()
        }
        if let err = account.lastError {
            Section {
                Label(err, systemImage: "exclamationmark.triangle")
                    .font(.pocket(.footnote))
                    .foregroundStyle(PocketColor.statusBad)
            }
        }
        Section {
            Button {
                guard let url = normalizedServer else { return }
                account.connect(server: url)
            } label: {
                Text("Connect")
                    .font(.pocket(.headline))
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .tint(LSColor.accent)
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets())
            .disabled(normalizedServer == nil)
        }
    }

    @ViewBuilder private func connectingSections(code: String, url: URL) -> some View {
        Section {
            VStack(spacing: PocketSpacing.s12) {
                Text(code)
                    .font(.pocket(.display))
                    .monospaced()
                    .kerning(3)
                    .foregroundStyle(LSColor.accent)
                Text("Approve in your browser, then come back — this screen finishes by itself.")
                    .font(.pocket(.footnote))
                    .foregroundStyle(PocketColor.textSecondary)
                    .multilineTextAlignment(.center)
                Button {
                    openURL(url)
                } label: {
                    Text("Open browser to approve")
                        .font(.pocket(.headline))
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .tint(LSColor.accent)
                HStack(spacing: PocketSpacing.s8) {
                    PocketMarkView(state: .loading, size: 24)
                    Text("Waiting for approval…")
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, PocketSpacing.s12)
        }
        Section {
            Button("Cancel", role: .cancel) { account.cancelConnect() }
                .font(.pocket(.body))
        }
    }

    @ViewBuilder private func connectedSections(user: LSUser) -> some View {
        Section {
            HStack(spacing: PocketSpacing.s12) {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 28))
                    .foregroundStyle(PocketColor.statusOk)
                VStack(alignment: .leading, spacing: 2) {
                    Text(user.name.isEmpty ? user.email : user.name)
                        .font(.pocket(.headline))
                    Text(account.serverURL.host() ?? account.serverURL.absoluteString)
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                        .monospaced()
                }
            }
            .padding(.vertical, PocketSpacing.s4)
        }
        Section("Machines online") {
            if hosts.isEmpty {
                Text("None — run `live daemon` on a machine to spawn sessions from here.")
                    .font(.pocket(.footnote))
                    .foregroundStyle(PocketColor.textSecondary)
            } else {
                ForEach(hosts) { h in
                    HStack {
                        Circle().fill(PocketColor.statusOk).frame(width: 8, height: 8)
                        Text(h.name).font(.pocket(.body))
                        Spacer()
                        Text("\(h.os)/\(h.arch)")
                            .font(.pocket(.caption))
                            .foregroundStyle(PocketColor.textTertiary)
                    }
                }
            }
        }
        Section {
            Button("Disconnect", role: .destructive) { account.disconnect() }
                .font(.pocket(.body))
        } footer: {
            Text("Removes the stored token from this device. Your sessions stay on LiveShortly.")
        }
        .task { hosts = (try? await account.client?.hosts()) ?? [] }
    }

    private var normalizedServer: URL? {
        var s = server.trimmingCharacters(in: .whitespaces)
        guard !s.isEmpty else { return nil }
        if !s.contains("://") { s = "https://" + s }
        guard let u = URL(string: s), u.host() != nil else { return nil }
        return u
    }
}
