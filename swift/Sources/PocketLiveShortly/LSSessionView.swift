import SwiftUI
import PocketUI

/// Full session screen: transcript (replay + live SSE), composer, permission
/// probes, model switch and owner actions. The transcript idiom mirrors the
/// LiveShortly web v4 thread: typed input = cards, agent = full-width prose,
/// consecutive tool activity folded into one growing work run.
public struct LSSessionView: View {
    private var account: LSAccountModel
    private let sessionID: String

    @State private var detail: LSSessionDetail?
    @State private var events: [LSEvent] = []
    @State private var watchers = 0
    @State private var live = false
    @State private var loadError: String?
    @State private var draft = ""
    @State private var sending = false
    @State private var streamTask: Task<Void, Never>?
    @State private var confirmStop = false
    @State private var confirmDelete = false
    @State private var showRename = false
    @State private var renameDraft = ""
    @State private var showShares = false
    @State private var showProbeDetail = false
    /// Cached transcript fold (see "Incremental transcript fold" below).
    @State private var foldedItems: [LSTranscriptItem] = []
    @State private var runOpen = false
    @State private var busy = false
    @State private var lastFolded: LSEvent?
    #if os(iOS)
    @State private var dictation = LSDictation()
    #endif
    @Environment(\.dismiss) private var dismiss
    /// Optimistic local comment echoes: local id → message text, swapped out
    /// when the server's viewer_comment event arrives on the stream.
    @State private var localEchoes: [String: String] = [:]
    /// Other viewers currently typing: name → expiry (from SSE typing frames).
    @State private var typers: [String: Date] = [:]
    @State private var lastTypingSent = Date.distantPast

    public init(account: LSAccountModel, sessionID: String) {
        self.account = account
        self.sessionID = sessionID
    }

    public var body: some View {
        VStack(spacing: 0) {
            if detail != nil {
                statusBar
            }
            if let loadError {
                Text(loadError)
                    .font(.pocket(.footnote))
                    .foregroundStyle(PocketColor.statusBad)
                    .padding(PocketSpacing.s12)
            }
            if detail == nil, loadError == nil {
                Spacer()
                ProgressView()
                Text("Loading session…")
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
                    .padding(.top, PocketSpacing.s8)
                Spacer()
            } else {
                transcript
            }
            if let who = typingLine {
                Text(who)
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, PocketSpacing.s16)
                    .padding(.bottom, 2)
            }
            if let probe = pendingProbe {
                LSPermissionBanner(
                    probe: probe,
                    toolCall: probeToolCall(before: probe),
                    showDetail: { showProbeDetail = true }
                ) { decision in
                    Task { try? await account.client?.decide(sessionID, decision: decision) }
                }
                .padding(.horizontal, PocketSpacing.s16)
                .padding(.bottom, PocketSpacing.s8)
            }
            if live, detail?.canComment == true {
                composer
            } else if detail != nil, !live {
                endedFooter
            }
        }
        .background(PocketColor.bgCanvas)
        .onReceive(Timer.publish(every: 2, on: .main, in: .common).autoconnect()) { now in
            if !typers.isEmpty { typers = typers.filter { $0.value > now } }
        }
        .navigationTitle(detail?.title ?? "Session")
        .lsInlineTitle()
        .toolbar { toolbarContent }
        .task { await open() }
        .onDisappear {
            streamTask?.cancel()
            #if os(iOS)
            dictation.stop()
            #endif
            LSSeen.mark(sessionID, max(detail?.eventCount ?? 0, events.count))
        }
        .confirmationDialog("End this session?", isPresented: $confirmStop) {
            Button("End session", role: .destructive) {
                Task { try? await account.client?.stop(sessionID) }
            }
        }
        .confirmationDialog("Delete this session permanently?", isPresented: $confirmDelete) {
            Button("Delete session", role: .destructive) {
                Task {
                    do {
                        try await account.client?.deleteSession(sessionID)
                        dismiss()
                    } catch { loadError = error.localizedDescription }
                }
            }
        }
        .sheet(isPresented: $showRename) {
            LSRenameSheet(title: renameDraft) { newTitle in
                Task {
                    do {
                        try await account.client?.rename(sessionID, title: newTitle)
                        detail?.title = newTitle
                    } catch { loadError = error.localizedDescription }
                }
            }
        }
        .sheet(isPresented: $showShares) {
            if let client = account.client {
                LSSharesSheet(client: client, sessionID: sessionID)
            }
        }
        .sheet(isPresented: $showProbeDetail) {
            if let probe = pendingProbe {
                LSProbeDetailSheet(probe: probe, toolCall: probeToolCall(before: probe)) { decision in
                    Task { try? await account.client?.decide(sessionID, decision: decision) }
                }
            }
        }
    }

    // MARK: - Status

    /// Thin strip under the title: pulsing LIVE badge (+ watcher count) while
    /// live, "Session ended" once over — same badge idiom as the chats list.
    private var statusBar: some View {
        HStack(spacing: PocketSpacing.s8) {
            if live {
                LSLiveBadge()
                if agentBusy {
                    PocketMarkView(state: .working(tokPerSec: nil), size: 16)
                    Text("working")
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                }
                if watchers > 1 {
                    Text("\(watchers) watching")
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                }
            } else {
                Text("ENDED")
                    .font(.system(size: 9, weight: .bold))
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(Capsule().fill(PocketColor.bgSunken))
                    .foregroundStyle(PocketColor.textTertiary)
                if let ended = detail?.endedAt {
                    Text(ended.formatted(.relative(presentation: .named)))
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                }
            }
            Spacer()
        }
        .padding(.horizontal, PocketSpacing.s16)
        .padding(.vertical, PocketSpacing.s4)
    }

    private var endedFooter: some View {
        Text("Session ended — read only")
            .font(.pocket(.caption))
            .foregroundStyle(PocketColor.textTertiary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, PocketSpacing.s8)
    }

    // MARK: - Transcript

    private var typingLine: String? {
        let now = Date()
        let names = typers.filter { $0.value > now }.keys.sorted()
        guard !names.isEmpty else { return nil }
        return names.count == 1
            ? "\(names[0]) is typing…"
            : "\(names.joined(separator: ", ")) are typing…"
    }

    private var transcript: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: PocketSpacing.s12) {
                    if foldedItems.isEmpty {
                        // A fresh session has no events yet — fill the window
                        // instead of rendering a lone status strip.
                        VStack(spacing: PocketSpacing.s12) {
                            PocketMarkView(
                                state: live ? .working(tokPerSec: nil) : .idle, size: 44)
                            Text(live
                                ? "Session is live — say something below or watch the agent start up."
                                : "No activity was recorded in this session.")
                                .font(.pocket(.footnote))
                                .foregroundStyle(PocketColor.textSecondary)
                                .multilineTextAlignment(.center)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.top, 120)
                        .padding(.horizontal, PocketSpacing.s24)
                    }
                    ForEach(foldedItems) { item in
                        LSTranscriptItemView(
                            item: item,
                            agentWorking: agentBusy && item.id == foldedItems.last?.id,
                            myHandle: myHandle)
                    }
                    if live {
                        HStack(spacing: PocketSpacing.s8) {
                            PocketMarkView(
                                state: agentBusy ? .working(tokPerSec: nil) : .idle,
                                size: 18)
                            Text(agentBusy ? "working" : statusLine)
                                .font(.pocket(.caption))
                                .foregroundStyle(PocketColor.textTertiary)
                        }
                        .id("tail")
                    } else {
                        Color.clear.frame(height: 1).id("tail")
                    }
                }
                .padding(PocketSpacing.s16)
            }
            .onChange(of: events.count) {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    proxy.scrollTo("tail", anchor: .bottom)
                }
            }
            #if os(iOS)
            .scrollDismissesKeyboard(.immediately)
            #endif
            .onTapGesture { lsHideKeyboard() }
            .refreshable { await open() }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var statusLine: String {
        var s = "live"
        if watchers > 1 { s += " · \(watchers) watching" }
        return s
    }

    /// Agent is mid-work — cached, maintained per folded event instead of an
    /// O(n) reverse scan on every render.
    private var agentBusy: Bool { live && busy }

    // MARK: - Incremental transcript fold
    //
    // The transcript used to be a computed `items` that re-folded all events on
    // every body evaluation — O(n) per render, O(n²) over a streaming session.
    // Now the fold is cached state: stream appends fold just the new event
    // (the open work run grows in place), and only replay/sort/removal paths
    // rebuild from scratch.

    private static let breakers: Set<String> = [
        "prompt", "response", "viewer_comment", "input_requested", "viewer_decision",
    ]

    private func rebuildTranscript() {
        foldedItems = []
        runOpen = false
        busy = false
        lastFolded = nil
        for e in events { foldEvent(e) }
    }

    private func foldEvent(_ e: LSEvent) {
        // Busy signal: response / turn end / probe = idle, activity = working.
        switch e.eventType {
        case "response", "session_ended", "stream_end", "input_requested":
            busy = false
        case "prompt", "viewer_comment", "tool_call", "pre_tool", "file_write", "output":
            busy = true
        default: break
        }

        // Day separator whenever the calendar day changes (and before the
        // first event, so every transcript opens with its date).
        if lastFolded == nil || !Calendar.current.isDate(lastFolded!.ts, inSameDayAs: e.ts) {
            runOpen = false
            foldedItems.append(.init(kind: .day(e.ts), id: "d\(e.seq)"))
        }

        if e.eventType == "stream_end" || e.eventType == "session_ended" {
            runOpen = false // turn markers: break the run, render nothing
        } else if Self.breakers.contains(e.eventType) {
            runOpen = false
            var item = LSTranscriptItem(kind: .single(e), id: "e\(e.seq)")
            // Author grouping: consecutive messages from the same hand within
            // 5 minutes drop the repeated header.
            if let prev = lastFolded, Self.breakers.contains(prev.eventType),
               prev.eventType == e.eventType,
               handle(of: prev, fallback: "") == handle(of: e, fallback: ""),
               e.ts.timeIntervalSince(prev.ts) < 300 {
                item.grouped = true
            }
            foldedItems.append(item)
        } else {
            if runOpen, case .work(let run) = foldedItems[foldedItems.count - 1].kind {
                foldedItems[foldedItems.count - 1].kind = .work(run + [e])
            } else {
                foldedItems.append(.init(kind: .work([e]), id: "w\(e.seq)"))
                runOpen = true
            }
        }
        lastFolded = e
    }

    /// The tool_call the agent is asking permission for: the last tool_call
    /// before the probe within the same turn. The PreToolUse hook fires (and
    /// emits the full tool input) right before Claude's permission prompt, so
    /// for a permission probe this is the very action awaiting approval.
    private func probeToolCall(before probe: LSEvent) -> LSEvent? {
        guard probe.payload["kind"]?.stringValue == "permission" else { return nil }
        for e in events.reversed() where e.seq < probe.seq {
            switch e.eventType {
            case "tool_call": return e
            case "prompt", "response", "viewer_comment": return nil
            default: continue
            }
        }
        return nil
    }

    /// The most recent unanswered permission/input probe, if the session is
    /// still live: an input_requested with no later viewer_decision.
    private var pendingProbe: LSEvent? {
        guard live else { return nil }
        for e in events.reversed() {
            switch e.eventType {
            case "viewer_decision", "prompt", "response": return nil
            case "input_requested": return e
            default: continue
            }
        }
        return nil
    }

    // MARK: - Composer

    private var composer: some View {
        HStack(spacing: PocketSpacing.s8) {
            TextField("Message this session…", text: $draft, axis: .vertical)
                .font(.pocket(.body))
                .lineLimit(1...5)
                .padding(.horizontal, PocketSpacing.s12)
                .padding(.vertical, 9)
                .onChange(of: draft) {
                    // Presence ping, throttled — others see "@you is typing".
                    guard !draft.isEmpty, live,
                          Date().timeIntervalSince(lastTypingSent) > 3,
                          let client = account.client else { return }
                    lastTypingSent = Date()
                    Task { await client.typing(sessionID) }
                }
            composerTrailing
                .padding(.trailing, 5)
        }
        .background(
            RoundedRectangle(cornerRadius: PocketRadius.field, style: .continuous)
                .fill(PocketColor.bgRaised)
                .stroke(PocketColor.lineStrong, lineWidth: 1)
        )
        .padding(.horizontal, PocketSpacing.s16)
        .padding(.bottom, PocketSpacing.s8)
    }

    /// Trailing composer control, local-chat parity: mic when the field is
    /// empty, stop while dictating, send once there is text.
    @ViewBuilder private var composerTrailing: some View {
        #if os(iOS)
        if dictation.recording || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            Button {
                lsImpact()
                dictation.toggle(base: draft) { draft = $0 }
            } label: {
                Image(systemName: dictation.recording ? "stop.fill" : "mic")
                    .font(.system(size: 14, weight: .bold))
                    .foregroundStyle(dictation.recording ? LSColor.onAccent : PocketColor.textSecondary)
                    .frame(width: 30, height: 30)
                    .background(Circle().fill(dictation.recording ? Color.red : Color.clear))
            }
            .disabled(sending)
            .accessibilityLabel(dictation.recording ? "Stop dictation" : "Dictate")
        } else {
            sendButton
        }
        #else
        sendButton
        #endif
    }

    private var sendButton: some View {
        Button {
            send()
        } label: {
            Image(systemName: sending ? "ellipsis" : "arrow.up")
                .font(.system(size: 14, weight: .bold))
                .foregroundStyle(LSColor.onAccent)
                .frame(width: 30, height: 30)
                .background(Circle().fill(LSColor.accent))
        }
        .disabled(sending || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        .accessibilityLabel("Send message")
    }

    private func send() {
        #if os(iOS)
        dictation.stop()
        #endif
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let client = account.client else { return }
        lsImpact()
        // Optimistic echo: show the bubble immediately; the SSE round-trip
        // replaces it (or a failure removes it and restores the draft).
        let localID = "local-\(UUID().uuidString)"
        let echo = LSEvent(
            id: localID,
            seq: (events.last?.seq ?? 0) + 1,
            actor: "viewer",
            eventType: "viewer_comment",
            payload: .object(["message": .string(text), "username": .string(myHandle)]),
            ts: Date())
        events.append(echo)
        foldEvent(echo)
        localEchoes[localID] = text
        draft = ""
        sending = true
        Task {
            defer { sending = false }
            do {
                try await client.comment(sessionID, message: text)
            } catch {
                events.removeAll { $0.id == localID }
                localEchoes[localID] = nil
                rebuildTranscript()
                draft = text
                loadError = error.localizedDescription
            }
        }
    }

    private var myHandle: String {
        if case .connected(let u) = account.phase {
            if !u.name.isEmpty { return u.name }
            if let prefix = u.email.split(separator: "@").first { return String(prefix) }
        }
        return "you"
    }

    // MARK: - Toolbar

    @ToolbarContentBuilder private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            if let detail, detail.isOwner == true {
                Menu {
                    if live, detail.framework == "claude-code" {
                        Menu("Model") {
                            ForEach(LS.claudeModels, id: \.self) { m in
                                Button(m) {
                                    Task { try? await account.client?.setModel(sessionID, model: m) }
                                }
                            }
                        }
                    }
                    Button {
                        renameDraft = detail.title
                        showRename = true
                    } label: {
                        Label("Rename…", systemImage: "pencil")
                    }
                    visibilityMenu(detail)
                    Button {
                        if let link = account.client?.sessionLink(sessionID) {
                            lsCopyToPasteboard(link.absoluteString)
                        }
                    } label: {
                        Label("Copy link", systemImage: "link")
                    }
                    if detail.publishedAt == nil {
                        Button {
                            feed(publish: true)
                        } label: {
                            Label("Publish to feed", systemImage: "globe")
                        }
                    } else {
                        Button {
                            feed(publish: false)
                        } label: {
                            Label("Remove from feed", systemImage: "globe")
                        }
                    }
                    Button {
                        showShares = true
                    } label: {
                        Label("Share with people…", systemImage: "person.badge.plus")
                    }
                    Divider()
                    if live {
                        Button("End session", role: .destructive) { confirmStop = true }
                    }
                    Button("Delete session", role: .destructive) { confirmDelete = true }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
            }
        }
    }

    private func visibilityMenu(_ detail: LSSessionDetail) -> some View {
        Menu {
            ForEach(["private", "link", "public", "open"], id: \.self) { v in
                Button {
                    setVisibility(visibility: v)
                } label: {
                    if (detail.visibility ?? "private") == v {
                        Label(v.capitalized, systemImage: "checkmark")
                    } else {
                        Text(v.capitalized)
                    }
                }
            }
            Divider()
            ForEach(["viewer", "commenter"], id: \.self) { r in
                Button {
                    setVisibility(linkRole: r)
                } label: {
                    if (detail.linkRole ?? "viewer") == r {
                        Label("Link: \(r)", systemImage: "checkmark")
                    } else {
                        Text("Link: \(r)")
                    }
                }
            }
        } label: {
            Label("Visibility", systemImage: "eye")
        }
    }

    private func setVisibility(visibility: String? = nil, linkRole: String? = nil) {
        Task {
            do {
                try await account.client?.setVisibility(
                    sessionID, visibility: visibility, linkRole: linkRole)
                if let visibility { detail?.visibility = visibility }
                if let linkRole { detail?.linkRole = linkRole }
            } catch { loadError = error.localizedDescription }
        }
    }

    private func feed(publish: Bool) {
        Task {
            do {
                if publish {
                    try await account.client?.publish(sessionID)
                    detail?.publishedAt = Date()
                } else {
                    try await account.client?.unpublish(sessionID)
                    detail?.publishedAt = nil
                }
            } catch { loadError = error.localizedDescription }
        }
    }

    // MARK: - Data

    private func open() async {
        guard let client = account.client else { return }
        do {
            let d = try await client.session(sessionID)
            detail = d
            events = d.events
            rebuildTranscript()
            live = d.isLive
            loadError = nil
            LSSeen.mark(sessionID, max(d.eventCount, events.count))
            if d.isLive { startStream(client) }
        } catch {
            loadError = error.localizedDescription
        }
    }

    private func startStream(_ client: LSClient) {
        streamTask?.cancel()
        streamTask = Task {
            do {
                for try await frame in await client.stream(sessionID) {
                    switch frame {
                    case .event(let e):
                        var removedEcho = false
                        if e.eventType == "viewer_comment",
                           let (localID, _) = localEchoes.first(where: { $0.value == lsBody(of: e) }) {
                            events.removeAll { $0.id == localID }
                            localEchoes[localID] = nil
                            removedEcho = true
                        }
                        if !events.contains(where: { $0.id == e.id }) {
                            // In-order arrivals (the common case) append and
                            // fold just this event; removal or out-of-order
                            // arrival rebuilds the cached fold.
                            if let last = events.last, e.seq < last.seq {
                                events.append(e)
                                events.sort { $0.seq < $1.seq }
                                rebuildTranscript()
                            } else {
                                events.append(e)
                                if removedEcho { rebuildTranscript() } else { foldEvent(e) }
                            }
                        } else if removedEcho {
                            rebuildTranscript()
                        }
                    case .watchers(let n): watchers = n
                    case .typing(let who, let until):
                        if who != myHandle { typers[who] = until }
                    case .sessionEnded: live = false
                    case .connected(let status): live = status == "live"
                    case .other: break
                    }
                }
            } catch is CancellationError {
            } catch {
                guard !Task.isCancelled else { return }
                loadError = "stream dropped — pull to refresh"
            }
        }
    }
}

// MARK: - Transcript pieces

struct LSTranscriptItem: Identifiable {
    enum Kind {
        case single(LSEvent)
        case work([LSEvent])
        case day(Date)
    }
    var kind: Kind
    var id: String
    /// Set when this message continues the previous author's run — the
    /// repeated header is dropped.
    var grouped = false
}

struct LSTranscriptItemView: View {
    let item: LSTranscriptItem
    let agentWorking: Bool
    var myHandle: String = ""

    var body: some View {
        switch item.kind {
        case .single(let e): single(e)
        case .work(let run): LSWorkRunView(run: run, running: agentWorking)
        case .day(let d): daySeparator(d)
        }
    }

    private func daySeparator(_ d: Date) -> some View {
        HStack(spacing: PocketSpacing.s8) {
            Rectangle().fill(PocketColor.lineStrong).frame(height: 1)
            Text(d.formatted(.dateTime.weekday(.wide).month().day()))
                .font(.pocket(.caption))
                .foregroundStyle(PocketColor.textTertiary)
                .fixedSize()
            Rectangle().fill(PocketColor.lineStrong).frame(height: 1)
        }
        .padding(.vertical, PocketSpacing.s4)
    }

    @ViewBuilder private func single(_ e: LSEvent) -> some View {
        switch e.eventType {
        case "prompt", "viewer_comment":
            // Your own comments read as "you" (accent), not as a foreign
            // viewer — same as the web thread.
            let mine = e.payload["username"]?.stringValue == myHandle
            LSBubble(event: e,
                     viewer: e.eventType == "viewer_comment" && !mine,
                     grouped: item.grouped)
        case "response":
            VStack(alignment: .leading, spacing: PocketSpacing.s4) {
                if !item.grouped {
                    HStack(spacing: PocketSpacing.s8) {
                        Text("◆ \(handle(of: e, fallback: "AGENT").uppercased())")
                            .font(.system(size: 10, weight: .bold))
                        Text(e.ts.formatted(date: .omitted, time: .shortened))
                            .font(.system(size: 10))
                    }
                    .foregroundStyle(PocketColor.textTertiary)
                }
                Text(markdown(lsBody(of: e)))
                    .font(.pocket(.reading))
                    .textSelection(.enabled)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case "input_requested", "viewer_decision":
            Text(systemLine(e))
                .font(.pocket(.caption))
                .foregroundStyle(PocketColor.textTertiary)
                .frame(maxWidth: .infinity, alignment: .center)
        default:
            EmptyView()
        }
    }

    private func systemLine(_ e: LSEvent) -> String {
        switch e.eventType {
        case "input_requested":
            let kind = e.payload["kind"]?.stringValue == "permission" ? "permission" : "input"
            return "awaiting \(kind)"
        case "viewer_decision":
            let who = e.payload["username"]?.stringValue ?? "viewer"
            let what = e.payload["decision"]?.stringValue == "deny" ? "denied" : "allowed"
            return "\(who) \(what) the request"
        default:
            return e.eventType
        }
    }
}

/// Typed input: right-aligned card (owner = LS green, viewer = info tint).
struct LSBubble: View {
    let event: LSEvent
    let viewer: Bool
    var grouped = false

    var body: some View {
        VStack(alignment: .trailing, spacing: PocketSpacing.s4) {
            if !grouped {
                HStack(spacing: PocketSpacing.s8) {
                    Text(handle(of: event, fallback: viewer ? "viewer" : "you"))
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(viewer ? PocketColor.statusInfo : LSColor.accent)
                    Text(event.ts.formatted(date: .omitted, time: .shortened))
                        .font(.system(size: 10))
                        .foregroundStyle(PocketColor.textTertiary)
                }
            }
            Text(lsBody(of: event))
                .font(.pocket(.body))
                .padding(.horizontal, PocketSpacing.s12)
                .padding(.vertical, PocketSpacing.s8)
                .background(
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(viewer
                            ? PocketColor.statusInfo.opacity(0.12)
                            : LSColor.accentSoft)
                )
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
    }
}

/// One folded run of tool activity — a growing monospace block, with a
/// "running" footer while the agent is still working at the tail.
struct LSWorkRunView: View {
    let run: [LSEvent]
    let running: Bool
    @State private var expanded = false

    private static let collapsedTail = 6

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Label the gray block for what it is: the agent's tool activity
            // (commands, edits, output) folded between messages.
            HStack(spacing: PocketSpacing.s8) {
                Image(systemName: "gearshape.2")
                    .font(.system(size: 9))
                Text("AGENT ACTIVITY · \(run.count) step\(run.count == 1 ? "" : "s")")
                    .font(.system(size: 9, weight: .bold))
                Spacer()
            }
            .foregroundStyle(PocketColor.textTertiary)
            .padding(.horizontal, PocketSpacing.s12)
            .padding(.top, PocketSpacing.s8)
            .padding(.bottom, PocketSpacing.s4)
            if run.count > Self.collapsedTail, !expanded {
                Button {
                    expanded = true
                } label: {
                    Text("··· \(run.count - Self.collapsedTail) earlier steps")
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.textTertiary)
                        .padding(.horizontal, PocketSpacing.s12)
                        .padding(.vertical, PocketSpacing.s4)
                }
            }
            ForEach(visible) { e in
                HStack(alignment: .top, spacing: PocketSpacing.s8) {
                    Text(marker(e).0)
                        .foregroundStyle(marker(e).1)
                    Text(stepLine(e))
                        .foregroundStyle(PocketColor.textSecondary)
                        .lineLimit(e.eventType == "output" ? 12 : 2)
                }
                .font(.pocket(.mono))
                .padding(.horizontal, PocketSpacing.s12)
                .padding(.vertical, 3)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if running {
                HStack(spacing: PocketSpacing.s8) {
                    PocketMarkView(state: .working(tokPerSec: nil), size: 16)
                    Text(runningLabel)
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.statusOk)
                }
                .padding(PocketSpacing.s12)
            }
        }
        .padding(.vertical, PocketSpacing.s4)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(PocketColor.bgSunken)
                .stroke(PocketColor.lineHairline, lineWidth: 1)
        )
    }

    private var visible: [LSEvent] {
        expanded ? run : Array(run.suffix(Self.collapsedTail))
    }

    private var runningLabel: String {
        guard let last = run.last else { return "working" }
        return "running — \(stepLine(last).prefix(60))"
    }

    private func marker(_ e: LSEvent) -> (String, Color) {
        switch e.eventType {
        case "tool_call", "pre_tool": return ("$", PocketColor.statusWarn)
        case "file_write": return ("✎", PocketColor.statusBad)
        case "output": return ("·", PocketColor.textTertiary)
        default: return ("•", PocketColor.textTertiary)
        }
    }

    private func stepLine(_ e: LSEvent) -> String {
        let p = e.payload
        let str: (String) -> String? = { p[$0]?.stringValue }
        switch e.eventType {
        case "file_write":
            return "edit \(str("file_path") ?? str("path") ?? str("file") ?? "file")"
        case "tool_call", "pre_tool":
            let tool = (str("tool_name") ?? str("tool") ?? str("name") ?? "run").lowercased()
            let target = str("file_path") ?? str("path") ?? str("command") ?? ""
            return target.isEmpty ? tool : "\(tool) \(target)"
        case "output":
            return str("text") ?? str("message") ?? "output"
        default:
            return e.eventType
        }
    }
}

/// Amber card for a pending permission/input probe. Compact on purpose: the
/// message plus, for a permission probe, a one-line summary of the tool call
/// awaiting approval — tap it (or "Review") to open the full detail sheet
/// with the command / diff before deciding.
struct LSPermissionBanner: View {
    let probe: LSEvent
    var toolCall: LSEvent?
    var showDetail: () -> Void = {}
    let decide: (String) -> Void
    @State private var answered = false

    var body: some View {
        VStack(alignment: .leading, spacing: PocketSpacing.s8) {
            Text("⚠ Permission requested")
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(PocketColor.statusWarn)
            if let msg = probe.payload["message"]?.stringValue {
                Text(msg)
                    .font(.pocket(.mono))
                    .lineLimit(4)
                    .padding(PocketSpacing.s8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(PocketColor.bgSunken))
            }
            if let tc = toolCall {
                Button(action: { lsImpact(); showDetail() }) {
                    HStack(spacing: PocketSpacing.s8) {
                        Image(systemName: "terminal")
                            .font(.system(size: 10))
                        Text(lsToolSummary(tc))
                            .font(.pocket(.mono))
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer()
                        Text("Review")
                            .font(.system(size: 11, weight: .semibold))
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                    }
                    .foregroundStyle(PocketColor.textSecondary)
                    .padding(PocketSpacing.s8)
                    .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(PocketColor.bgSunken))
                }
                .buttonStyle(.plain)
            }
            HStack(spacing: PocketSpacing.s8) {
                Button("Allow") { answer("allow") }
                    .buttonStyle(.borderedProminent)
                    .tint(PocketColor.statusOk)
                Button("Deny") { answer("deny") }
                    .buttonStyle(.bordered)
                    .tint(PocketColor.statusBad)
            }
            .font(.pocket(.footnote))
            .disabled(answered)
        }
        .padding(PocketSpacing.s12)
        .background(
            RoundedRectangle(cornerRadius: 13, style: .continuous)
                .fill(PocketColor.statusWarn.opacity(0.10))
                .stroke(PocketColor.statusWarn.opacity(0.35), lineWidth: 1)
        )
    }

    private func answer(_ d: String) {
        lsImpact()
        answered = true
        decide(d)
    }
}

/// One-line "Tool · gist" label for a tool_call event (banner row, sheet title).
func lsToolSummary(_ e: LSEvent) -> String {
    let tool = e.payload["tool"]?.stringValue ?? "tool"
    let input = e.payload["input"]
    let gist: String? = switch tool {
    case "Bash":
        input?["command"]?.stringValue?
            .split(separator: "\n", maxSplits: 1).first.map(String.init)
    case "Edit", "MultiEdit", "Write", "Read", "NotebookEdit":
        (input?["file_path"]?.stringValue as NSString?)?.lastPathComponent
    default:
        nil
    }
    return gist.map { "\(tool) · \($0)" } ?? tool
}

/// Full-screen review of a pending permission probe: the prompt message plus
/// the exact tool call awaiting approval — command for Bash, diff for Edit,
/// file content for Write, raw input for anything else — then Allow / Deny.
struct LSProbeDetailSheet: View {
    let probe: LSEvent
    var toolCall: LSEvent?
    let decide: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: PocketSpacing.s16) {
                    if let msg = probe.payload["message"]?.stringValue {
                        Text(msg)
                            .font(.pocket(.body))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    if let tc = toolCall {
                        toolSection(tc)
                    } else {
                        Text("No tool details available for this request.")
                            .font(.pocket(.footnote))
                            .foregroundStyle(PocketColor.textTertiary)
                    }
                }
                .padding(PocketSpacing.s16)
            }
            .background(PocketColor.bgCanvas)
            .navigationTitle(probe.payload["kind"]?.stringValue == "permission"
                ? "Permission request" : "Input request")
            .lsInlineTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { dismiss() }
                }
            }
            .safeAreaInset(edge: .bottom) {
                HStack(spacing: PocketSpacing.s12) {
                    Button { answer("deny") } label: {
                        Text("Deny").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .tint(PocketColor.statusBad)
                    Button { answer("allow") } label: {
                        Text("Allow").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(PocketColor.statusOk)
                }
                .font(.pocket(.headline))
                .padding(PocketSpacing.s16)
                .background(.bar)
            }
        }
    }

    private func answer(_ d: String) {
        lsImpact()
        decide(d)
        dismiss()
    }

    @ViewBuilder private func toolSection(_ tc: LSEvent) -> some View {
        let tool = tc.payload["tool"]?.stringValue ?? "tool"
        let input = tc.payload["input"]
        VStack(alignment: .leading, spacing: PocketSpacing.s8) {
            Text(tool.uppercased())
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(PocketColor.textTertiary)
            switch tool {
            case "Bash":
                if let desc = input?["description"]?.stringValue {
                    Text(desc)
                        .font(.pocket(.footnote))
                        .foregroundStyle(PocketColor.textSecondary)
                }
                codeBlock(input?["command"]?.stringValue ?? "")
            case "Edit", "Write", "NotebookEdit":
                if let path = input?["file_path"]?.stringValue {
                    Text(path)
                        .font(.pocket(.mono))
                        .foregroundStyle(PocketColor.textSecondary)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
                if tool == "Edit" {
                    diffBlock(old: input?["old_string"]?.stringValue ?? "",
                              new: input?["new_string"]?.stringValue ?? "")
                } else {
                    codeBlock(input?["content"]?.stringValue
                        ?? input?["new_source"]?.stringValue ?? "")
                }
            default:
                codeBlock(lsPrettyJSON(input))
            }
        }
    }

    /// Scrollable monospace block, capped so a huge Write doesn't lock the sheet.
    private func codeBlock(_ text: String) -> some View {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        let cap = 120
        let shown = lines.prefix(cap).joined(separator: "\n")
        return VStack(alignment: .leading, spacing: PocketSpacing.s4) {
            ScrollView(.horizontal, showsIndicators: false) {
                Text(shown)
                    .font(.pocket(.mono))
                    .textSelection(.enabled)
                    .padding(PocketSpacing.s12)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(PocketColor.bgSunken))
            if lines.count > cap {
                Text("… \(lines.count - cap) more lines")
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
            }
        }
    }

    /// Minimal unified diff: removed lines in red, added in green.
    private func diffBlock(old: String, new: String) -> some View {
        let removed = old.split(separator: "\n", omittingEmptySubsequences: false)
        let added = new.split(separator: "\n", omittingEmptySubsequences: false)
        let cap = 60
        return ScrollView(.horizontal, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 1) {
                ForEach(Array(removed.prefix(cap).enumerated()), id: \.offset) { _, line in
                    Text("- \(line)")
                        .foregroundStyle(PocketColor.statusBad)
                }
                if removed.count > cap {
                    Text("… \(removed.count - cap) more removed")
                        .foregroundStyle(PocketColor.textTertiary)
                }
                ForEach(Array(added.prefix(cap).enumerated()), id: \.offset) { _, line in
                    Text("+ \(line)")
                        .foregroundStyle(PocketColor.statusOk)
                }
                if added.count > cap {
                    Text("… \(added.count - cap) more added")
                        .foregroundStyle(PocketColor.textTertiary)
                }
            }
            .font(.pocket(.mono))
            .textSelection(.enabled)
            .padding(PocketSpacing.s12)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(PocketColor.bgSunken))
    }
}

/// Pretty-print an LSJSON payload for the fallback tool view.
func lsPrettyJSON(_ v: LSJSON?) -> String {
    guard let v else { return "" }
    let enc = JSONEncoder()
    enc.outputFormatting = [.prettyPrinted, .sortedKeys]
    guard let data = try? enc.encode(v) else { return "" }
    return String(decoding: data, as: UTF8.self)
}

/// Proper rename form (the stock alert text field is cramped and unstyled).
struct LSRenameSheet: View {
    @State var title: String
    let save: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @FocusState private var focused: Bool

    private var trimmed: String {
        title.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: PocketSpacing.s12) {
                Text("SESSION TITLE")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(PocketColor.textTertiary)
                TextField("Untitled session", text: $title)
                    .font(.pocket(.headline))
                    .focused($focused)
                    .submitLabel(.done)
                    .onSubmit { commit() }
                    .padding(.horizontal, PocketSpacing.s12)
                    .padding(.vertical, PocketSpacing.s12)
                    .background(
                        RoundedRectangle(cornerRadius: PocketRadius.field, style: .continuous)
                            .fill(PocketColor.bgSunken)
                            .stroke(PocketColor.lineStrong, lineWidth: 1)
                    )
                Text("\(trimmed.count)/200")
                    .font(.pocket(.caption))
                    .foregroundStyle(PocketColor.textTertiary)
                Spacer()
            }
            .padding(PocketSpacing.s16)
            .background(PocketColor.bgCanvas)
            .navigationTitle("Rename session")
            .lsInlineTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { commit() }
                        .fontWeight(.semibold)
                        .disabled(trimmed.isEmpty || trimmed.count > 200)
                }
            }
            .onAppear { focused = true }
        }
        #if os(iOS)
        .presentationDetents([.height(230)])
        #else
        .frame(minWidth: 420, minHeight: 200)
        #endif
    }

    private func commit() {
        guard !trimmed.isEmpty, trimmed.count <= 200 else { return }
        save(trimmed)
        dismiss()
    }
}

/// Owner sheet: who has access, add by email (viewer/commenter), revoke.
struct LSSharesSheet: View {
    let client: LSClient
    let sessionID: String
    @State private var shares: [LSShare] = []
    @State private var email = ""
    @State private var role = "viewer"
    @State private var error: String?
    @State private var busy = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("Add person") {
                    TextField("email@example.com", text: $email)
                        .lsURLField()
                    Picker("Role", selection: $role) {
                        Text("Viewer").tag("viewer")
                        Text("Commenter").tag("commenter")
                    }
                    Button("Add") { add() }
                        .disabled(busy || email.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                Section("People with access") {
                    if shares.isEmpty {
                        Text("No one yet")
                            .font(.pocket(.caption))
                            .foregroundStyle(PocketColor.textTertiary)
                    }
                    ForEach(shares) { s in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(s.granteeEmail ?? "user")
                                    .font(.pocket(.body))
                                Text(s.role)
                                    .font(.pocket(.caption))
                                    .foregroundStyle(PocketColor.textTertiary)
                            }
                            Spacer()
                            Button {
                                remove(s)
                            } label: {
                                Image(systemName: "trash")
                                    .foregroundStyle(PocketColor.statusBad)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                if let error {
                    Text(error)
                        .font(.pocket(.caption))
                        .foregroundStyle(PocketColor.statusBad)
                }
            }
            .lsGroupedList()
            .navigationTitle("Share session")
            .lsInlineTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .task { await load() }
        }
        #if os(macOS)
        .frame(minWidth: 420, minHeight: 420)
        #endif
    }

    private func load() async {
        do {
            shares = try await client.shares(sessionID)
            error = nil
        } catch { self.error = error.localizedDescription }
    }

    private func add() {
        let e = email.trimmingCharacters(in: .whitespaces).lowercased()
        guard !e.isEmpty else { return }
        busy = true
        Task {
            defer { busy = false }
            do {
                let s = try await client.createShare(sessionID, email: e, role: role)
                shares.append(s)
                email = ""
                error = nil
            } catch { self.error = error.localizedDescription }
        }
    }

    private func remove(_ s: LSShare) {
        Task {
            do {
                try await client.deleteShare(sessionID, shareID: s.id)
                shares.removeAll { $0.id == s.id }
            } catch { self.error = error.localizedDescription }
        }
    }
}

// MARK: - Shared helpers

func lsBody(of e: LSEvent) -> String {
    for k in ["message", "text", "content", "prompt"] {
        if let s = e.payload[k]?.stringValue, !s.trimmingCharacters(in: .whitespaces).isEmpty {
            return s
        }
    }
    return "(\(e.eventType))"
}

func handle(of e: LSEvent, fallback: String) -> String {
    if let u = e.payload["username"]?.stringValue, !u.isEmpty { return "@\(u)" }
    if let a = e.actor, a != "agent" { return a }
    return fallback
}

func markdown(_ s: String) -> AttributedString {
    (try? AttributedString(
        markdown: s,
        options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
        ?? AttributedString(s)
}

#if os(iOS)
import Speech
import AVFoundation

/// Minimal live dictation for the LS composer: system speech recognition
/// streamed into the draft. The on-device PocketKit transcription engine stays
/// with the local-chat path; this is the small system-framework version.
/// ponytail: no level metering, no hands-free send — add by reusing the app's
/// VoiceDictation if LS dictation ever needs parity beyond "talk, see text".
@MainActor @Observable
final class LSDictation {
    var recording = false
    private var engine: AVAudioEngine?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?

    nonisolated init() {}

    func toggle(base: String, onText: @escaping @MainActor (String) -> Void) {
        if recording { stop() } else { start(base: base, onText: onText) }
    }

    private func start(base: String, onText: @escaping @MainActor (String) -> Void) {
        SFSpeechRecognizer.requestAuthorization { auth in
            guard auth == .authorized else { return }
            AVAudioApplication.requestRecordPermission { granted in
                guard granted else { return }
                Task { @MainActor in self.begin(base: base, onText: onText) }
            }
        }
    }

    private func begin(base: String, onText: @escaping @MainActor (String) -> Void) {
        guard let recognizer = SFSpeechRecognizer(), recognizer.isAvailable else { return }
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.record, mode: .measurement, options: .duckOthers)
        try? session.setActive(true, options: .notifyOthersOnDeactivation)

        let engine = AVAudioEngine()
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        let input = engine.inputNode
        input.installTap(onBus: 0, bufferSize: 1024, format: input.outputFormat(forBus: 0)) { buffer, _ in
            request.append(buffer)
        }
        engine.prepare()
        guard (try? engine.start()) != nil else { return }
        self.engine = engine
        self.request = request
        recording = true

        let prefix = base.isEmpty ? "" : base + " "
        task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            Task { @MainActor in
                if let result {
                    onText(prefix + result.bestTranscription.formattedString)
                }
                if error != nil || result?.isFinal == true {
                    self?.stop()
                }
            }
        }
    }

    func stop() {
        engine?.stop()
        engine?.inputNode.removeTap(onBus: 0)
        request?.endAudio()
        task?.cancel()
        engine = nil
        request = nil
        task = nil
        recording = false
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}
#endif
