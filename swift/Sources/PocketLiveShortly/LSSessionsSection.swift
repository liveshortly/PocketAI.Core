import SwiftUI
import PocketUI

/// The "LiveShortly" section embedded in each app's Chats list. Renders
/// nothing when the account is not connected — the integration is invisible
/// until the user opts in from Settings. Shows the most recent sessions
/// (live first) with a "See all" escape hatch into a searchable list.
public struct LSSessionsSection: View {
    private var account: LSAccountModel
    /// When set, rows call this instead of pushing a NavigationLink — the Mac
    /// sidebar opens sessions in its detail pane, not a navigation stack.
    private let onOpen: ((String) -> Void)?
    /// When set, "New session…" calls this so the host view can present the
    /// sheet from a stable ancestor (a sheet presented from inside a List
    /// section gets torn down when the section first reloads).
    private let onNewSession: (() -> Void)?
    @Environment(\.scenePhase) private var scenePhase
    @State private var newSession = false
    @State private var showAll = false
    @State private var pendingDelete: LSSession?
    @State private var pendingEnd: LSSession?
    @State private var deleteError: String?

    /// ponytail: flat cap, no paging — "See all" covers the long tail
    private static let recentLimit = 5

    public init(
        account: LSAccountModel,
        onOpen: ((String) -> Void)? = nil,
        onNewSession: (() -> Void)? = nil
    ) {
        self.account = account
        self.onOpen = onOpen
        self.onNewSession = onNewSession
    }

    // The list is the account's shared feed (LSI-17): it polls, so a session
    // started anywhere shows up without a refresh.
    private var feed: LSSessionFeed { account.feed }
    private var sessions: [LSSession] { feed.sessions }
    private var loaded: Bool { feed.loaded }
    private var loadError: String? { feed.loadError }
    /// Before a first answer the section showed nothing at all, so a slow
    /// server looked like an account with no sessions (LSI-11).
    private var isLoading: Bool { feed.isLoading }

    public var body: some View {
        if account.isConnected, let client = account.client {
            Section {
                ForEach(ordered.prefix(Self.recentLimit)) { s in
                    Group {
                        if let onOpen {
                            Button { onOpen(s.id) } label: { LSSessionRow(session: s) }
                                .buttonStyle(.plain)
                        } else {
                            NavigationLink {
                                LSSessionView(account: account, sessionID: s.id)
                            } label: {
                                LSSessionRow(session: s)
                            }
                        }
                    }
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        Button(role: .destructive) {
                            pendingDelete = s
                        } label: {
                            Label("Delete", systemImage: "trash")
                        }
                        if s.isLive {
                            Button {
                                pendingEnd = s
                            } label: {
                                Label("End", systemImage: "stop.circle")
                            }
                            .tint(PocketColor.statusWarn)
                        }
                    }
                }
                if isLoading, !loaded {
                    HStack(spacing: PocketSpacing.s8) {
                        PocketMarkView(state: .loading, size: 24)
                        Text("Loading sessions…")
                            .font(.pocket(.footnote))
                            .foregroundStyle(PocketColor.textTertiary)
                    }
                    .padding(.vertical, PocketSpacing.s4)
                    .accessibilityIdentifier("ls_sessions_loading")
                }
                if loaded, sessions.isEmpty, loadError == nil {
                    // Teaching empty state: say how a session comes to exist,
                    // not just that none do.
                    VStack(alignment: .leading, spacing: PocketSpacing.s4) {
                        Text("No sessions yet")
                            .font(.pocket(.body))
                        Text("Run `live claude` in a terminal to stream an agent here, or tap **New session** to start one on a registered machine.")
                            .font(.pocket(.caption))
                            .foregroundStyle(PocketColor.textTertiary)
                    }
                    .padding(.vertical, PocketSpacing.s4)
                }
                if let deleteError {
                    Text(deleteError)
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.statusBad)
                }
                if let loadError {
                    VStack(alignment: .leading, spacing: PocketSpacing.s8) {
                        Text(loadError)
                            .font(.pocket(.caption))
                            .foregroundStyle(PocketColor.statusBad)
                        // An error with no way out left the list permanently
                        // empty until the tab was switched away and back.
                        Button {
                            Task { await feed.reload() }
                        } label: {
                            Label("Retry", systemImage: "arrow.clockwise")
                                .font(.pocket(.footnote))
                        }
                        .disabled(isLoading)
                        .accessibilityIdentifier("ls_sessions_retry")
                    }
                    .padding(.vertical, PocketSpacing.s4)
                }
                if sessions.count > Self.recentLimit {
                    Button {
                        showAll = true
                    } label: {
                        Label("See all sessions (\(sessions.count))", systemImage: "list.bullet")
                            .font(.pocket(.body))
                            .foregroundStyle(LSColor.accent)
                    }
                    .accessibilityIdentifier("ls_see_all_row")
                }
                Button {
                    if let onNewSession { onNewSession() } else { newSession = true }
                } label: {
                    Label("New session…", systemImage: "plus.circle.fill")
                        .font(.pocket(.body))
                        .foregroundStyle(LSColor.accent)
                }
                .accessibilityIdentifier("ls_new_session_row")
            } header: {
                HStack(spacing: PocketSpacing.s8) {
                    Text("LiveShortly")
                    Spacer()
                    if sessions.contains(where: \.isLive) {
                        LSLiveBadge()
                    }
                    // Explicit, because pull-to-refresh is undiscoverable and
                    // this same section is used on the Mac, where there is no
                    // pull gesture at all.
                    Button {
                        Task { await feed.reload() }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                            .font(.system(size: 11, weight: .semibold))
                    }
                    .buttonStyle(.plain)
                    .disabled(isLoading)
                    .foregroundStyle(isLoading ? PocketColor.textTertiary : LSColor.accent)
                    .accessibilityLabel("Refresh sessions")
                    .accessibilityIdentifier("ls_sessions_refresh")
                }
            }
            .task { await feed.run() }
            .onChange(of: scenePhase, initial: true) { _, phase in feed.phase = phase }
            .refreshable { await feed.reload() }
            .sheet(isPresented: $newSession) {
                LSNewSessionSheet(account: account)
            }
            .sheet(isPresented: $showAll) {
                LSAllSessionsView(account: account, sessions: ordered)
            }
            .confirmationDialog(
                "End \"\(pendingEnd?.title ?? "session")\"? The agent will stop.",
                isPresented: .init(
                    get: { pendingEnd != nil },
                    set: { if !$0 { pendingEnd = nil } }),
                titleVisibility: .visible
            ) {
                Button("End session", role: .destructive) {
                    guard let s = pendingEnd else { return }
                    Task {
                        try? await client.stop(s.id)
                        await feed.reload()
                    }
                }
            }
            .confirmationDialog(
                "Delete \"\(pendingDelete?.title ?? "session")\" permanently? This cannot be undone.",
                isPresented: .init(
                    get: { pendingDelete != nil },
                    set: { if !$0 { pendingDelete = nil } }),
                titleVisibility: .visible
            ) {
                Button("Delete session", role: .destructive) {
                    guard let s = pendingDelete else { return }
                    Task {
                        do {
                            try await client.deleteSession(s.id)
                            feed.remove(s.id)
                        } catch { deleteError = error.localizedDescription }
                    }
                }
            }
        }
    }

    private var ordered: [LSSession] { feed.ordered }
}

/// Searchable full list of sessions, presented as a sheet from "See all".
/// One code path for iPhone and Mac: rows push the viewer inside the
/// sheet's own navigation stack.
public struct LSAllSessionsView: View {
    private var account: LSAccountModel
    @State private var sessions: [LSSession]
    @State private var query = ""
    @State private var pendingDelete: LSSession?
    @State private var pendingEnd: LSSession?
    @Environment(\.dismiss) private var dismiss

    public init(account: LSAccountModel, sessions: [LSSession]) {
        self.account = account
        _sessions = State(initialValue: sessions)
    }

    public var body: some View {
        NavigationStack {
            List(filtered) { s in
                NavigationLink {
                    LSSessionView(account: account, sessionID: s.id)
                } label: {
                    LSSessionRow(session: s)
                }
                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                    Button(role: .destructive) {
                        pendingDelete = s
                    } label: {
                        Label("Delete", systemImage: "trash")
                    }
                    if s.isLive {
                        Button {
                            pendingEnd = s
                        } label: {
                            Label("End", systemImage: "stop.circle")
                        }
                        .tint(PocketColor.statusWarn)
                    }
                }
            }
            .searchable(text: $query, prompt: "Search sessions")
            .confirmationDialog(
                "End \"\(pendingEnd?.title ?? "session")\"? The agent will stop.",
                isPresented: .init(
                    get: { pendingEnd != nil },
                    set: { if !$0 { pendingEnd = nil } }),
                titleVisibility: .visible
            ) {
                Button("End session", role: .destructive) {
                    guard let s = pendingEnd, let client = account.client else { return }
                    Task {
                        try? await client.stop(s.id)
                        if var updated = sessions.first(where: { $0.id == s.id }) {
                            updated.status = "ended"
                            sessions = sessions.map { $0.id == s.id ? updated : $0 }
                        }
                    }
                }
            }
            .confirmationDialog(
                "Delete \"\(pendingDelete?.title ?? "session")\" permanently? This cannot be undone.",
                isPresented: .init(
                    get: { pendingDelete != nil },
                    set: { if !$0 { pendingDelete = nil } }),
                titleVisibility: .visible
            ) {
                Button("Delete session", role: .destructive) {
                    guard let s = pendingDelete, let client = account.client else { return }
                    Task {
                        try? await client.deleteSession(s.id)
                        sessions.removeAll { $0.id == s.id }
                    }
                }
            }
            .navigationTitle("All sessions")
            .lsInlineTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        #if os(macOS)
        .frame(minWidth: 480, minHeight: 520)
        #endif
    }

    private var filtered: [LSSession] {
        guard !query.isEmpty else { return sessions }
        return sessions.filter {
            $0.title.localizedCaseInsensitiveContains(query)
                || ($0.framework ?? "").localizedCaseInsensitiveContains(query)
                || ($0.model ?? "").localizedCaseInsensitiveContains(query)
                || $0.ownerHandle.localizedCaseInsensitiveContains(query)
        }
    }
}

/// Shared live indicator: pulsing dot + LIVE capsule. Used in rows, section
/// headers and the session view so "live" reads the same everywhere.
public struct LSLiveBadge: View {
    @State private var dim = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init() {}

    public var body: some View {
        HStack(spacing: 4) {
            Circle()
                .fill(PocketColor.statusOk)
                .frame(width: 6, height: 6)
                .opacity(dim && !reduceMotion ? 0.25 : 1)
                .animation(
                    reduceMotion ? nil : .easeInOut(duration: 0.8).repeatForever(autoreverses: true),
                    value: dim)
            Text("LIVE")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(PocketColor.statusOk)
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 2)
        .background(Capsule().fill(PocketColor.statusOk.opacity(0.15)))
        .onAppear { dim = true }
        .accessibilityLabel("Live")
    }
}

public struct LSSessionRow: View {
    let session: LSSession

    public init(session: LSSession) {
        self.session = session
    }

    public var body: some View {
        HStack(spacing: PocketSpacing.s12) {
            LSSessionMark(session: session)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: PocketSpacing.s8) {
                    Text(session.title.isEmpty ? "Untitled session" : session.title)
                        .font(.pocket(.body))
                        .lineLimit(1)
                    if session.isLive {
                        LSLiveBadge()
                    }
                }
                Text(subtitle)
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
                    .lineLimit(1)
            }
            if unread > 0 {
                Spacer(minLength: PocketSpacing.s4)
                Text(unread > 99 ? "99+" : "\(unread)")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(LSColor.onAccent)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(LSColor.accent))
                    .accessibilityLabel("\(unread) new events")
            }
        }
    }

    /// New events since this session was last opened on this device. Only
    /// sessions opened at least once badge — a fresh install stays quiet.
    private var unread: Int {
        let seen = LSSeen.count(session.id)
        guard seen > 0 else { return 0 }
        return max(0, session.eventCount - seen)
    }

    private var subtitle: String {
        // Prefer the human "what happened last" summary; fall back to metadata.
        if session.isStarting { return "Starting…" }
        if let le = session.lastEvent, !le.snippet.isEmpty {
            let who = le.type == "response" ? "◆ " : "› "
            return who + le.snippet
        }
        var bits: [String] = []
        if let f = session.framework, !f.isEmpty { bits.append(f) }
        if let m = session.model, !m.isEmpty { bits.append(m) }
        bits.append("\(session.eventCount) events")
        if !session.isLive, let ended = session.endedAt {
            bits.append("ended \(ended.formatted(.relative(presentation: .named)))")
        }
        return bits.joined(separator: " · ")
    }
}
