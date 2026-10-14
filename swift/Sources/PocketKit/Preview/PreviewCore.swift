import Foundation

/// Deterministic fixture implementation of `CoreClient` (UI-03). Lets UI
/// lanes build every screen without waiting for Rust; launch argument
/// `-PocketPreviewCore YES` switches the whole app to this implementation.
public final class PreviewCore: CoreClient {
    public init() {}

    private static let fixtureWords = [
        "Hello", "from", "Preview", "Core", "—", "deterministic", "fake", "streaming", "at", "forty", "tokens", "per", "second.",
    ]

    public func mockGenerate(seed: Int64, tokensPerSec: Float) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            Task {
                let delayMs = UInt64(1000 / max(tokensPerSec, 1))
                for word in Self.fixtureWords {
                    try? await Task.sleep(nanoseconds: delayMs * 1_000_000)
                    continuation.yield(word + " ")
                }
                continuation.finish()
            }
        }
    }
}
