import Foundation

/// Wraps the generated UniFFI bindings. Converts the Rust-driven `GenerateSink`
/// callback into an `AsyncThrowingStream`; `onTermination` has nothing to
/// cancel yet (the mock generator always runs to completion), but real
/// streaming methods will call `core.stop(...)` there.
public final class LiveCore: CoreClient {
    public init() {}

    public func mockGenerate(seed: Int64, tokensPerSec: Float) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let sink = ContinuationSink(continuation: continuation)
            Task.detached {
                PocketKit.mockGenerate(seed: seed, tokensPerSec: tokensPerSec, sink: sink)
            }
        }
    }
}

private final class ContinuationSink: GenerateSink, @unchecked Sendable {
    private let continuation: AsyncThrowingStream<String, Error>.Continuation

    init(continuation: AsyncThrowingStream<String, Error>.Continuation) {
        self.continuation = continuation
    }

    func onDelta(text: String) {
        continuation.yield(text)
    }

    func onDone(completionTokens: UInt32) {
        continuation.finish()
    }

    func onError(message: String) {
        continuation.finish(throwing: LiveCoreError.generationFailed(message))
    }
}

public enum LiveCoreError: Error {
    case generationFailed(String)
}
