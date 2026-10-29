import Foundation

/// APL-05: implements the `DownloadCoordinator` foreign trait so Mac
/// control-API requests and local UI requests share one transfer queue.
/// Up to 3 concurrent file tasks per the card, FIFO beyond that. Runs on
/// `Net.makeBackgroundSessionConfiguration` (APL-04) so transfers keep
/// going while backgrounded.
///
/// Correctness contract (§8, F06/F07/F10):
/// - Pause/offline requeue the in-flight file (front) and keep its resume data,
///   so Resume actually continues it; a paused model is skipped by `fillSlots`.
/// - Per-file bytes are summed into one aggregate reported to Core — a single
///   file's progress never overwrites the model total.
/// - Every terminal path (`didFinishDownloadingTo`, `didCompleteWithError`)
///   releases its slot and refills fairly; `active` membership is the single
///   authority, so a paused/cancelled task's late error callback is ignored.
/// - Scheduling is gated on Offline mode; user vs offline vs network pauses are
///   distinct states. Relaunch background-delivery re-attach is L5/device work.
public final class ModelDownloadCoordinator: NSObject, DownloadCoordinator, @unchecked Sendable {
    public static let sessionIdentifier = "app.getpocketai.downloads"
    private static let maxConcurrent = 3

    private enum PauseReason { case user, offline }

    private let core: PocketCore
    private lazy var session = URLSession(
        configuration: Net.makeBackgroundSessionConfiguration(identifier: Self.sessionIdentifier),
        delegate: self,
        delegateQueue: nil
    )

    private let lock = NSLock()
    /// Sha256-ing multi-GB files must not run on the URLSession delegate queue
    /// (it would starve every progress callback); serial so two models
    /// finishing together don't hash concurrently.
    private let verifyQueue = DispatchQueue(label: "app.getpocketai.downloads.verify", qos: .utility)
    /// From application(_:handleEventsForBackgroundURLSession:); called on main
    /// in urlSessionDidFinishEvents so iOS can suspend the relaunched app.
    public var backgroundEventsCompletionHandler: (() -> Void)?
    private var queued: [String: [PlannedFile]] = [:]        // modelId -> remaining files
    private var active: [Int: (modelId: String, file: PlannedFile)] = [:] // taskId -> context
    private var resumeData: [String: Data] = [:]            // "modelId:url" -> resume data
    private var paused: Set<String> = []                    // models paused by user/offline
    private var completedBytes: [String: UInt64] = [:]      // sum of finished file sizes
    private var activeBytes: [Int: UInt64] = [:]            // taskId -> current bytes

    public init(core: PocketCore) {
        self.core = core
        super.init()
    }

    /// Relaunch recovery. Everything here dies with the process — `queued`,
    /// `active`, Core's in-memory download plans — so a model mid-download when
    /// iOS killed the app was stranded forever: bytes on disk, DB state stuck at
    /// downloading/verifying, and the background session's finished tasks dropped
    /// by the nil-context guards below. Re-plans every such model, counts files
    /// already fully on disk as done, verifies when nothing remains, reattaches
    /// the session's still-running tasks by URL, and refills the slots.
    /// Call once at startup, before any user-driven download action.
    public func rehydrate() {
        let entries = core.catalog(filter: CatalogFilter(modality: nil, curatedOnly: false, installedOnly: false))
        for entry in entries {
            let wasPaused: Bool
            switch entry.state {
            case .downloading, .verifying, .queued: wasPaused = false
            case .paused: wasPaused = true
            default: continue
            }
            let id = entry.manifest.id
            // Re-registers the plan Core-side (verifyAndCommit needs it) and
            // flips state to "downloading"; corrected below for paused models.
            guard let plan = try? core.planDownload(id: id) else { continue }

            lock.lock()
            if wasPaused { paused.insert(id) }
            lock.unlock()
            if scanAndQueue(id: id, plan: plan) {
                // ponytail: resume data is only ever held in memory, so a paused
                // file restarts from byte 0 after relaunch; persist resume data if
                // that ever matters for multi-GB files.
                if wasPaused { core.setDownloadState(id: id, state: "paused") }
            } else {
                // Everything is on disk — including the "earlier run committed
                // everything, only the DB row was stale" case: verifyAndCommit
                // skips committed files and is the only path allowed to mark
                // "installed" (setDownloadState refuses that state by design).
                verifyAndCommitOffQueue(id: id)
            }
        }

        // Reattach tasks the background session kept alive across the relaunch,
        // so fillSlots doesn't start duplicates. Tasks that finished while the
        // app was dead deliver didFinishDownloadingTo with no context and are
        // claimed from `queued` there.
        session.getAllTasks { [weak self] tasks in
            guard let self else { return }
            for task in tasks {
                guard let dl = task as? URLSessionDownloadTask, dl.state == .running,
                      let claimed = self.claimQueuedFile(url: dl.originalRequest?.url ?? dl.currentRequest?.url)
                else { continue }
                self.lock.lock()
                self.active[dl.taskIdentifier] = claimed
                self.lock.unlock()
            }
            self.fillSlots()
        }
    }

    // MARK: DownloadCoordinator

    public func start(plan: DownloadPlan) throws {
        lock.lock()
        queued[plan.modelId] = plan.files
        completedBytes[plan.modelId] = 0
        paused.remove(plan.modelId)
        lock.unlock()
        if core.isOffline() {
            core.setDownloadState(id: plan.modelId, state: "paused:offline")
            lock.lock(); paused.insert(plan.modelId); lock.unlock()
            return
        }
        fillSlots()
    }

    public func pause(id: String) throws { pauseInternal(id: id, reason: .user) }

    public func resume(id: String) throws {
        guard !core.isOffline() else {
            core.setDownloadState(id: id, state: "paused:offline")
            return
        }
        lock.lock()
        paused.remove(id)
        let hasWork = !(queued[id]?.isEmpty ?? true) || active.contains { $0.value.modelId == id }
        lock.unlock()
        core.setDownloadState(id: id, state: "downloading")
        if !hasWork {
            // A failed:* model after relaunch (rehydrate skips those) has nothing
            // in memory — re-plan from disk so Resume isn't a no-op that sticks
            // the row at "downloading" forever.
            if !scanAndQueue(id: id, plan: try core.planDownload(id: id)) {
                verifyAndCommitOffQueue(id: id)
                return
            }
        }
        fillSlots(modelId: id)
    }

    public func cancel(id: String) throws {
        lock.lock()
        queued.removeValue(forKey: id)
        completedBytes.removeValue(forKey: id)
        paused.remove(id)
        resumeData = resumeData.filter { !$0.key.hasPrefix("\(id):") }
        let taskIds = Set(active.filter { $0.value.modelId == id }.keys)
        for tid in taskIds { active.removeValue(forKey: tid); activeBytes.removeValue(forKey: tid) }
        lock.unlock()
        session.getAllTasks { tasks in
            for task in tasks where taskIds.contains(task.taskIdentifier) { task.cancel() }
        }
        // Every other transition here reports itself — pause, resume, offline.
        // Cancel did not, so the row stayed "downloading" in the store and the
        // button looked dead: the transfer really did stop, but nothing on
        // screen moved.
        core.setDownloadState(id: id, state: "available")
    }

    public func pauseAllInternetTransfers() throws {
        lock.lock()
        let modelIds = Set(queued.keys).union(active.values.map(\.modelId))
        lock.unlock()
        for id in modelIds { pauseInternal(id: id, reason: .offline) }
    }

    // MARK: internals

    private func pauseInternal(id: String, reason: PauseReason) {
        lock.lock()
        paused.insert(id)
        let taskIds = Set(active.filter { $0.value.modelId == id }.keys)
        lock.unlock()
        core.setDownloadState(id: id, state: reason == .offline ? "paused:offline" : "paused")
        session.getAllTasks { [weak self] tasks in
            guard let self else { return }
            for task in tasks where taskIds.contains(task.taskIdentifier) {
                (task as? URLSessionDownloadTask)?.cancel(byProducingResumeData: { [weak self] data in
                    guard let self else { return }
                    self.lock.lock()
                    // Requeue the in-flight file at the FRONT and keep its resume
                    // data so Resume continues it (F06). Remove from active so a
                    // late error callback for this task is ignored.
                    if let context = self.active.removeValue(forKey: task.taskIdentifier) {
                        self.activeBytes.removeValue(forKey: task.taskIdentifier)
                        if let data { self.resumeData["\(context.modelId):\(context.file.url)"] = data }
                        self.queued[context.modelId, default: []].insert(context.file, at: 0)
                    }
                    self.lock.unlock()
                })
            }
        }
    }

    private func fillSlots(modelId: String? = nil) {
        guard !core.isOffline() else { return } // Offline mode blocks new internet tasks (F10)
        lock.lock()
        defer { lock.unlock() }
        let candidates = modelId.map { [$0] } ?? Array(queued.keys)
        for id in candidates where !paused.contains(id) {
            guard var files = queued[id] else { continue }
            while active.count < Self.maxConcurrent, !files.isEmpty {
                let file = files.removeFirst()
                let task: URLSessionDownloadTask
                if let data = resumeData.removeValue(forKey: "\(id):\(file.url)") {
                    task = session.downloadTask(withResumeData: data)
                } else if let url = URL(string: file.url) {
                    task = session.downloadTask(with: url)
                } else {
                    continue
                }
                active[task.taskIdentifier] = (id, file)
                task.resume()
            }
            queued[id] = files
        }
    }

    /// Claims a rehydrated-but-unstarted file by URL — the only identity that
    /// survives a relaunch, since task identifiers and contexts do not.
    private func claimQueuedFile(url: URL?) -> (modelId: String, file: PlannedFile)? {
        guard let url = url?.absoluteString else { return nil }
        lock.lock()
        defer { lock.unlock() }
        for (id, files) in queued {
            if let idx = files.firstIndex(where: { $0.url == url }) {
                var files = files
                let file = files.remove(at: idx)
                queued[id] = files
                return (id, file)
            }
        }
        return nil
    }

    /// Aggregate bytes for a model = finished files + current bytes of its active
    /// tasks (F07). Caller holds `lock`.
    private func aggregateBytesLocked(_ modelId: String) -> UInt64 {
        let completed = completedBytes[modelId] ?? 0
        let activeSum = active.reduce(UInt64(0)) { sum, entry in
            entry.value.modelId == modelId ? sum + (activeBytes[entry.key] ?? 0) : sum
        }
        return completed + activeSum
    }

    /// Scans a plan against disk: files fully present (committed, or a .part of
    /// the right size — a wrong hash is caught by verifyAndCommit) count as
    /// done, the rest are queued. Returns false when nothing needs downloading;
    /// the caller decides whether that means "verify now".
    private func scanAndQueue(id: String, plan: DownloadPlan) -> Bool {
        let fm = FileManager.default
        var remaining: [PlannedFile] = []
        var doneBytes: UInt64 = 0
        for f in plan.files {
            if fm.fileExists(atPath: f.destFinal) {
                doneBytes += f.size
                continue
            }
            let partSize = (try? fm.attributesOfItem(atPath: f.destPart)[.size] as? NSNumber)?.uint64Value
            if partSize == f.size { doneBytes += f.size } else { remaining.append(f) }
        }
        lock.lock()
        queued[id] = remaining
        completedBytes[id] = doneBytes
        lock.unlock()
        core.recordDownloadProgress(id: id, file: "", bytesDone: doneBytes)
        return !remaining.isEmpty
    }

    /// Verification off the delegate queue, with real error handling: a swallowed
    /// verifyAndCommit error used to leave the row at "verifying" forever (I/O
    /// error) or deleted the bad .part without queueing anything, making Resume a
    /// no-op (checksum mismatch). On failure, re-plan + requeue what's missing
    /// and set a failed:* state; `paused` blocks auto-restart until the user
    /// Resumes (same no-tight-retry policy as the network-error path).
    private func verifyAndCommitOffQueue(id: String) {
        core.setDownloadState(id: id, state: "verifying")
        verifyQueue.async { [weak self] in
            guard let self else { return }
            do {
                try self.core.verifyAndCommit(id: id)
                self.lock.lock()
                self.completedBytes.removeValue(forKey: id)
                self.queued.removeValue(forKey: id)
                self.lock.unlock()
            } catch {
                let isChecksum = String(describing: error).lowercased().contains("checksum")
                self.lock.lock()
                self.paused.insert(id)
                self.lock.unlock()
                if let plan = try? self.core.planDownload(id: id) {
                    _ = self.scanAndQueue(id: id, plan: plan)
                }
                // After planDownload reset the row to "downloading", mark the
                // real outcome so Retry/Resume surfaces in the UI.
                self.core.setDownloadState(id: id, state: isChecksum ? "failed:checksum" : "failed:disk")
            }
        }
    }
}

extension ModelDownloadCoordinator: URLSessionDownloadDelegate {
    public func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        lock.lock()
        var context = active.removeValue(forKey: downloadTask.taskIdentifier)
        activeBytes.removeValue(forKey: downloadTask.taskIdentifier)
        lock.unlock()
        if context == nil {
            // Task finished while the app was dead: no in-memory context exists
            // on relaunch. Claim it from the rehydrated queue by URL instead of
            // silently dropping the downloaded file.
            context = claimQueuedFile(url: downloadTask.originalRequest?.url ?? downloadTask.currentRequest?.url)
        }
        guard let context else { return }

        // A 4xx/5xx body must not become the .part file — it would surface later
        // as a baffling failed:checksum. Mark the real cause and requeue; `paused`
        // keeps fillSlots from instantly re-fetching a 404 in a tight loop.
        let status = (downloadTask.response as? HTTPURLResponse)?.statusCode
        guard status == 200 || status == 206 else {
            lock.lock()
            queued[context.modelId, default: []].insert(context.file, at: 0)
            paused.insert(context.modelId)
            lock.unlock()
            core.setDownloadState(id: context.modelId, state: "failed:network")
            fillSlots() // let other queued models use the freed slot
            return
        }

        let destPartURL = URL(fileURLWithPath: context.file.destPart)
        do {
            try? FileManager.default.removeItem(at: destPartURL)
            try FileManager.default.createDirectory(at: destPartURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try FileManager.default.moveItem(at: location, to: destPartURL)
        } catch {
            core.setDownloadState(id: context.modelId, state: "failed:disk")
            lock.lock()
            // Put the file back so a retry can re-fetch it; `paused` blocks
            // auto-restart until the user Resumes.
            queued[context.modelId, default: []].insert(context.file, at: 0)
            paused.insert(context.modelId)
            lock.unlock()
            return
        }

        lock.lock()
        completedBytes[context.modelId, default: 0] += context.file.size
        let aggregate = aggregateBytesLocked(context.modelId)
        let remaining = !(queued[context.modelId]?.isEmpty ?? true) || active.contains { $0.value.modelId == context.modelId }
        lock.unlock()
        core.recordDownloadProgress(id: context.modelId, file: context.file.destFinal, bytesDone: aggregate)

        if !remaining {
            verifyAndCommitOffQueue(id: context.modelId)
        }
        fillSlots(modelId: context.modelId)
    }

    public func urlSession(
        _ session: URLSession,
        downloadTask: URLSessionDownloadTask,
        didWriteData bytesWritten: Int64,
        totalBytesWritten: Int64,
        totalBytesExpectedToWrite: Int64
    ) {
        lock.lock()
        guard let context = active[downloadTask.taskIdentifier] else { lock.unlock(); return }
        activeBytes[downloadTask.taskIdentifier] = UInt64(max(0, totalBytesWritten))
        let aggregate = aggregateBytesLocked(context.modelId)
        lock.unlock()
        core.recordDownloadProgress(id: context.modelId, file: context.file.destFinal, bytesDone: aggregate)
    }

    /// Terminal error path (F07): release the slot, keep resume data, requeue the
    /// file for a later Resume/Retry, and mark the model failed. A paused or
    /// cancelled task was already removed from `active`, so it is ignored here.
    public func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        guard error != nil else { return } // success handled in didFinishDownloadingTo
        lock.lock()
        guard let context = active.removeValue(forKey: task.taskIdentifier) else { lock.unlock(); return }
        activeBytes.removeValue(forKey: task.taskIdentifier)
        if let data = (error as NSError?)?.userInfo[NSURLSessionDownloadTaskResumeData] as? Data {
            resumeData["\(context.modelId):\(context.file.url)"] = data
        }
        queued[context.modelId, default: []].insert(context.file, at: 0)
        // Without this, the fillSlots below would restart the requeued file
        // immediately — exactly the tight retry loop the comment forbids.
        paused.insert(context.modelId)
        lock.unlock()
        // Mark failed and release the slot; do NOT auto-refill this model (avoid a
        // tight retry loop). Retry/offline-recovery is user- or monitor-driven.
        core.setDownloadState(id: context.modelId, state: "failed:network")
        fillSlots() // let other queued models use the freed slot
    }

    /// Background-session wrap-up: iOS relaunched the app to deliver queued
    /// events; calling the handler stored by the app delegate lets it suspend.
    public func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        DispatchQueue.main.async { [weak self] in
            self?.backgroundEventsCompletionHandler?()
            self?.backgroundEventsCompletionHandler = nil
        }
    }
}
