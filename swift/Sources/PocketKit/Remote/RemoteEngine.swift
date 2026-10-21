import Foundation

enum RemoteEngineError: Error {
    case malformedChunk
    case cancelled
}

/// APL-11: implements the `Engine` foreign trait against a paired Mac's
/// `/v1/chat/completions`, streaming over `PeerClient`'s SSE. `generate`
/// is synchronous at the FFI boundary - Rust calls into Swift and blocks
/// until it returns (D21) - so the async network call is bridged via a
/// semaphore rather than exposed as `async`.
public final class RemoteEngine: Engine, @unchecked Sendable {
    private let client: PeerClient
    private let tasksLock = NSLock()
    private var activeTasks: [String: Task<Void, Never>] = [:]

    public init(client: PeerClient) {
        self.client = client
    }

    public func kind() -> EngineKind { .remote }

    /// Per A§ router rule 5: a remote model id is prefixed `mac:<device>/`.
    public func supports(model: ModelManifest) -> Bool {
        model.id.hasPrefix("mac:")
    }

    public func load(model: ModelManifest, opts: LoadOptions) throws -> LoadedModel {
        // The Mac host loads the model lazily on first /v1/chat/completions
        // call; there's no separate remote "load" request. The handle
        // carries the already-stripped local model id through to generate().
        LoadedModel(handle: Self.localModelId(from: model.id))
    }

    public func unload(handle: String) throws {}

    public func generate(handle: String, req: GenerateRequest, sink: TokenSink) throws -> GenerateStats {
        let semaphore = DispatchSemaphore(value: 0)
        let box = ResultBox()
        let requestId = req.requestId

        let task = Task { [client] in
            do {
                let stats = try await Self.streamGenerate(client: client, handle: handle, req: req, sink: sink)
                box.result = .success(stats)
            } catch is CancellationError {
                box.result = .failure(RemoteEngineError.cancelled)
            } catch {
                sink.onError(err: .Network(message: "\(error)"))
                box.result = .failure(error)
            }
            semaphore.signal()
        }
        tasksLock.lock()
        activeTasks[requestId] = task
        tasksLock.unlock()

        semaphore.wait()
        tasksLock.lock()
        activeTasks.removeValue(forKey: requestId)
        tasksLock.unlock()
        switch box.result {
        case .success(let stats):
            return stats
        case .failure(let error):
            throw PocketError.EngineFailed(engine: .remote, message: "\(error)")
        case .none:
            throw PocketError.EngineFailed(engine: .remote, message: "no result")
        }
    }

    /// Bridges the async `streamGenerate` call's result back across the
    /// semaphore wait below - a plain `var` captured by the `Task` closure
    /// trips Swift 6's data-race checker even though the semaphore makes
    /// the handoff safe in practice.
    private final class ResultBox: @unchecked Sendable {
        var result: Result<GenerateStats, Error>?
    }

    public func embed(handle: String, inputs: [String]) throws -> [[Float]] {
        throw PocketError.Internal(message: "RemoteEngine.embed: Mac embeddings aren't proxied in V1")
    }

    public func cancel(requestId: String) {
        tasksLock.lock()
        let task = activeTasks[requestId]
        tasksLock.unlock()
        task?.cancel()
    }

    public func residentBytes(handle: String) -> UInt64 {
        // Resident memory is on the Mac, not this device - nothing to
        // report locally.
        0
    }

    public static func localModelId(from remoteId: String) -> String {
        guard let afterPrefix = remoteId.dropFirst("mac:".count).split(separator: "/", maxSplits: 1).last else {
            return remoteId
        }
        return String(afterPrefix)
    }

    private static func streamGenerate(client: PeerClient, handle: String, req: GenerateRequest, sink: TokenSink) async throws -> GenerateStats {
        let body: [String: Any] = [
            "model": handle,
            "messages": req.messages.map { message in
                [
                    "role": message.role,
                    "content": message.parts.map(contentText).joined(),
                ]
            },
            "temperature": req.params.temperature,
            "max_tokens": req.params.maxTokens,
            "stream": true,
        ]
        var promptTokens: UInt32 = 0
        var completionTokens: UInt32 = 0
        var finish: FinishReason = .stop

        for try await event in client.events(path: "/v1/chat/completions", body: body) {
            try Task.checkCancellation()
            if event.data == "[DONE]" { break }
            guard
                let data = event.data.data(using: .utf8),
                let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { continue }

            if let usage = json["usage"] as? [String: Any] {
                promptTokens = (usage["prompt_tokens"] as? NSNumber)?.uint32Value ?? promptTokens
                completionTokens = (usage["completion_tokens"] as? NSNumber)?.uint32Value ?? completionTokens
            }
            guard
                let choices = json["choices"] as? [[String: Any]],
                let choice = choices.first,
                let delta = choice["delta"] as? [String: Any]
            else { continue }

            if let content = delta["content"] as? String, !content.isEmpty {
                sink.onDelta(delta: GenerateDelta(text: content, reasoning: "", toolCalls: []))
            }
            if let reason = choice["finish_reason"] as? String {
                finish = Self.finishReason(for: reason)
            }
        }

        let stats = GenerateStats(promptTokens: promptTokens, completionTokens: completionTokens, ttftMs: 0, decodeTps: 0, finish: finish)
        sink.onDone(stats: stats)
        return stats
    }

    private static func contentText(_ part: ContentPart) -> String {
        if case .text(let text) = part { return text }
        return ""
    }

    private static func finishReason(for value: String) -> FinishReason {
        switch value {
        case "stop": return .stop
        case "length": return .length
        case "tool_calls": return .toolCalls
        case "content_filter": return .contentFilter
        default: return .stop
        }
    }
}
