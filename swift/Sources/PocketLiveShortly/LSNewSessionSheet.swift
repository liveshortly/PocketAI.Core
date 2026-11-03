import SwiftUI
import PocketUI

/// Spawn a session on one of the user's own machines. Every picker is a
/// closed set from `GET /api/hosts` — the app can never name a directory or
/// agent the daemon did not register (CONTRACT.md security model).
public struct LSNewSessionSheet: View {
    private var account: LSAccountModel
    /// Called once a session is successfully spawned (e.g. to link it to a
    /// project). The sheet still pushes the viewer itself.
    private let onStarted: ((String) -> Void)?
    @Environment(\.dismiss) private var dismiss

    @State private var hosts: [LSHost] = []
    @State private var hostID = ""
    @State private var agent = ""
    @State private var cwd = ""
    @State private var model: String?
    @State private var bypass = true
    @State private var busy = false
    @State private var loadingHosts = true
    @State private var error: String?
    private struct Started: Identifiable, Hashable { let id: String }
    @State private var startedID: Started?

    public init(account: LSAccountModel, onStarted: ((String) -> Void)? = nil) {
        self.account = account
        self.onStarted = onStarted
    }

    private var host: LSHost? { hosts.first { $0.id == hostID } ?? hosts.first }

    public var body: some View {
        NavigationStack {
            Form {
                if hosts.isEmpty, loadingHosts {
                    // Was "No machine online" while the request was still in
                    // flight — and forever when it failed (UI-97).
                    Section {
                        HStack(spacing: PocketSpacing.s8) {
                            PocketMarkView(state: .loading, size: 24)
                                .accessibilityHidden(true) // the adjacent text carries the status
                            Text("Looking for your machines…")
                                .font(.pocket(.footnote))
                                .foregroundStyle(PocketColor.textSecondary)
                        }
                    }
                } else if hosts.isEmpty {
                    Section {
                        Text(error ?? "No machine online. Run `live daemon` on a machine first.")
                            .font(.pocket(.footnote))
                            .foregroundStyle(error == nil ? PocketColor.textSecondary : PocketColor.statusBad)
                        Button {
                            Task { await load() }
                        } label: {
                            Label("Check again", systemImage: "arrow.clockwise")
                                .font(.pocket(.body))
                        }
                        .tint(LSColor.accent)
                    }
                } else if let host {
                    Section {
                        Picker("Machine", selection: $hostID) {
                            ForEach(hosts) { h in
                                Text("\(h.name) (\(h.os)/\(h.arch))").tag(h.id)
                            }
                        }
                        Picker("Agent", selection: $agent) {
                            ForEach(host.agents, id: \.self) { Text($0).tag($0) }
                        }
                        Picker("Directory", selection: $cwd) {
                            ForEach(host.dirs, id: \.self) {
                                Text($0).tag($0).font(.pocket(.mono))
                            }
                        }
                        LSModelPicker(host: host, agent: agent, model: $model)
                    }
                    Section {
                        Toggle(isOn: $bypass) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text("Skip permission prompts").font(.pocket(.body))
                                Text("Unattended run — the agent uses its bypass flag on your machine.")
                                    .font(.pocket(.caption))
                                    .foregroundStyle(PocketColor.textTertiary)
                            }
                        }
                        .tint(PocketColor.statusOk)
                    }
                    if let error {
                        Section {
                            Text(error)
                                .font(.pocket(.footnote))
                                .foregroundStyle(PocketColor.statusBad)
                        }
                    }
                    Section {
                        Button {
                            start()
                        } label: {
                            Text(busy ? "Starting…" : "Start \(agent)")
                                .font(.pocket(.headline))
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(LSColor.accent)
                        .listRowBackground(Color.clear)
                        .listRowInsets(EdgeInsets())
                        .disabled(busy || agent.isEmpty || cwd.isEmpty
                            || (agent == "ollama" && model == nil))
                    } footer: {
                        Text(bypass
                            ? "Runs unattended on \(host.name). The agent will not ask before running commands or editing files."
                            : "Runs on \(host.name). You approve each tool call from the session.")
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .background(PocketColor.bgCanvas)
            .navigationTitle("New session")
            .lsInlineTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .task { await load() }
            .onChange(of: agent) { model = host?.lastModel(forAgent: agent) }
            .onChange(of: hostID) { model = host?.lastModel(forAgent: agent) }
            // Jump straight into the freshly spawned session.
            .navigationDestination(item: $startedID) { started in
                LSSessionView(account: account, sessionID: started.id)
            }
        }
    }

    private func load() async {
        guard let client = account.client else { loadingHosts = false; return }
        loadingHosts = true
        defer { loadingHosts = false }
        do {
            hosts = try await client.hosts()
            error = nil
        } catch {
            self.error = "Couldn't load your machines: \(error.localizedDescription)"
            return
        }
        guard let h = hosts.first else { return }
        hostID = h.id
        agent = h.agents.first ?? ""
        cwd = h.dirs.first ?? ""
        model = h.lastModel(forAgent: agent)
    }

    private func start() {
        guard let client = account.client, let host else { return }
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                let created = try await client.createSession(
                    hostID: host.id, agent: agent, cwd: cwd,
                    // Only ever an id the server listed for this agent.
                    model: model.flatMap { host.models(forAgent: agent).contains($0) ? $0 : nil },
                    bypassPermissions: bypass)
                // Every list picks the new row up now, not on the next poll.
                Task { await account.feed.reload() }
                if created.spawn?.status == "failed" {
                    error = created.spawn?.error ?? "could not reach that machine"
                    return
                }
                lsNotify(success: true)
                onStarted?(created.id)
                startedID = Started(id: created.id)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// Model choice for one agent on one host (LSH-16), shared by the iPhone
/// sheet and the Mac's new-session view. Lists only the server's ids for
/// that agent; nil = the agent's own default. Picks are remembered per
/// host+agent.
public struct LSModelPicker: View {
    let host: LSHost
    let agent: String
    @Binding var model: String?

    public init(host: LSHost, agent: String, model: Binding<String?>) {
        self.host = host
        self.agent = agent
        self._model = model
    }

    private var ids: [String] { host.models(forAgent: agent) }

    private var selection: Binding<String> {
        Binding(
            get: { model ?? "" },
            set: { new in
                model = new.isEmpty ? nil : new
                host.rememberModel(model, forAgent: agent)
            })
    }

    private var defaultLabel: String {
        guard let d = host.defaultModel(forAgent: agent) else { return "Agent default" }
        return "Agent default (\(host.modelLabel(d, forAgent: agent)))"
    }

    public var body: some View {
        if ids.isEmpty {
            Picker("Model", selection: .constant("")) {
                Text("Uses the agent's default").tag("")
            }
            .disabled(true)
        } else {
            VStack(alignment: .leading, spacing: 2) {
                Picker("Model", selection: selection) {
                    Text(defaultLabel).tag("")
                    ForEach(ids, id: \.self) { Text(host.modelLabel($0, forAgent: agent)).tag($0) }
                }
                if let model {
                    Text(model)
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                }
            }
        }
    }
}
