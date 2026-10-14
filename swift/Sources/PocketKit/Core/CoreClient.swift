import Foundation

/// The one protocol PocketAI's apps program against (A§4.6). `LiveCore` wraps
/// the generated UniFFI `PocketCore`; `PreviewCore` is deterministic fixtures
/// so UI lanes never wait for Rust.
///
/// NOTE: this is intentionally a narrow W0 slice. The full surface in
/// `01-architecture.md` §4.5 (catalog, conversations, pairing, …) needs the
/// `PocketCore` UniFFI object, which isn't implemented yet (see
/// DECISIONS.md D19/D21) — only `ping`, `sampleDeviceProfile`, and the mock
/// generate/callback round trip exist today. Methods are added here as their
/// FFI counterpart lands.
public protocol CoreClient: AnyObject, Sendable {
    /// Proves the Swift-to-Rust-to-Swift streaming pattern that real chat
    /// streaming (`send`) will use: a token sink implemented in Swift, driven
    /// from Rust, turned into an `AsyncThrowingStream`.
    func mockGenerate(seed: Int64, tokensPerSec: Float) -> AsyncThrowingStream<String, Error>
}
