import Foundation

/// APL-05: implements the `DownloadCoordinator` foreign trait so Mac
/// control-API requests and local UI requests share one transfer queue.
/// Up to 3 concurrent file tasks per the card, FIFO beyond that. Runs on
/// `Net.makeBackgroundSessionConfiguration` (APL-04) so transfers keep
/// going while backgrounded; `application(_:handleEventsForBackgroundURLSession:)`
/// re-attaching this session's delegate after a relaunch is L5's job
/// (that's what actually makes "kill app -> relaunch resumes" work - not
/// exercised here since it needs a real device/relaunch).
public final class ModelDownloadCoordinator: NSObject, DownloadCoordinator, @unchecked Sendable {
    public static let sessionIdentifier = "app.getpocketai.downloads"
    private static let maxConcurrent = 3

    private let core: PocketCore
    private lazy var session = URLSession(
        configuration: Net.makeBackgroundSessionConfiguration(identifier: Self.sessionIdentifier),
        delegate: self,
        delegateQueue: nil
    )

    private let lock = NSLock()
    private var queued: [String: [PlannedFile]] = [:] // modelId -> remaining files
    private var active: [Int: (modelId: String, file: PlannedFile)] = [:] // taskIdentifier -> context
    private var resumeData: [String: Data] = [:] // "modelId:url" -> resume data

    public init(core: PocketCore) {
        self.core = core
        super.init()
    }

    public func start(plan: DownloadPlan) throws {
        lock.lock()
        queued[plan.modelId] = plan.files
        lock.unlock()
        fillSlots()
    }

    public func pause(id: String) throws {
        session.getAllTasks { [weak self] tasks in
            guard let self else { return }
            self.lock.lock()
            let taskIds = Set(self.active.filter { $0.value.modelId == id }.keys)
            self.lock.unlock()
            for task in tasks where taskIds.contains(task.taskIdentifier) {
                (task as? URLSessionDownloadTask)?.cancel(byProducingResumeData: { [weak self] data in
                    guard let self else { return }
                    self.lock.lock()
                    if let data, let context = self.active[task.taskIdentifier] {
                        self.resumeData["\(context.modelId):\(context.file.url)"] = data
                    }
                    self.active.removeValue(forKey: task.taskIdentifier)
                    self.lock.unlock()
                })
            }
        }
    }

    public func resume(id: String) throws {
        fillSlots(modelId: id)
    }

    public func cancel(id: String) throws {
        lock.lock()
        queued.removeValue(forKey: id)
        let taskIds = Set(active.filter { $0.value.modelId == id }.keys)
        lock.unlock()
        session.getAllTasks { tasks in
            for task in tasks where taskIds.contains(task.taskIdentifier) {
                task.cancel()
            }
        }
    }

    public func pauseAllInternetTransfers() throws {
        lock.lock()
        let modelIds = Set(queued.keys).union(active.values.map(\.modelId))
        lock.unlock()
        for id in modelIds { try? pause(id: id) }
    }

    private func fillSlots(modelId: String? = nil) {
        lock.lock()
        defer { lock.unlock() }
        let candidates = modelId.map { [$0] } ?? Array(queued.keys)
        for id in candidates {
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
}

extension ModelDownloadCoordinator: URLSessionDownloadDelegate {
    public func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        lock.lock()
        let context = active.removeValue(forKey: downloadTask.taskIdentifier)
        lock.unlock()
        guard let context else { return }

        let destPartURL = URL(fileURLWithPath: context.file.destPart)
        do {
            try? FileManager.default.removeItem(at: destPartURL)
            try FileManager.default.createDirectory(at: destPartURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try FileManager.default.moveItem(at: location, to: destPartURL)
        } catch {
            fillSlots(modelId: context.modelId)
            return
        }

        let remaining: Bool
        lock.lock()
        remaining = !(queued[context.modelId]?.isEmpty ?? true) || active.contains { $0.value.modelId == context.modelId }
        lock.unlock()
        if !remaining {
            _ = try? core.verifyAndCommit(id: context.modelId)
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
        let context = active[downloadTask.taskIdentifier]
        lock.unlock()
        guard let context else { return }
        core.recordDownloadProgress(id: context.modelId, file: context.file.destFinal, bytesDone: UInt64(totalBytesWritten))
    }
}
