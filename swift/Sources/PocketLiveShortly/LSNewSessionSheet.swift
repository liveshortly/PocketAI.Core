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
    @State private var model = ""
    @State private var bypass = true
    @State private var busy = false
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
                if hosts.isEmpty {
                    Section {
                        Text("No machine online. Run `live daemon` on a machine first.")
                            .font(.pocket(.footnote))
                            .foregroundStyle(PocketColor.textSecondary)
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
                        if agent == "claude" {
                            Picker("Model", selection: $model) {
                                Text("Default").tag("")
                                ForEach(LS.claudeModels, id: \.self) { Text($0).tag($0) }
                            }
                        }
                        if agent == "ollama" {
                            Picker("Model", selection: $model) {
                                ForEach(host.models ?? [], id: \.self) { Text($0).tag($0) }
                            }
                        }
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
                            || (agent == "ollama" && model.isEmpty))
                    } footer: {
                        Text(bypass
                            ? "Runs unattended on \(host.name)."
                            : "Runs on \(host.name). Approve tool calls from the session.")
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
            // Jump straight into the freshly spawned session.
            .navigationDestination(item: $startedID) { started in
                LSSessionView(account: account, sessionID: started.id)
            }
        }
    }

    private func load() async {
        hosts = (try? await account.client?.hosts()) ?? []
        guard let h = hosts.first else { return }
        hostID = h.id
        agent = h.agents.first ?? ""
        cwd = h.dirs.first ?? ""
        model = ""
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
                    model: model.isEmpty ? nil : model,
                    bypassPermissions: bypass)
                if created.spawn?.status == "failed" {
                    error = created.spawn?.error ?? "could not reach that machine"
                    return
                }
                onStarted?(created.id)
                startedID = Started(id: created.id)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
