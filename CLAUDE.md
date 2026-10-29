# PocketAI.Core

Rust workspace + Swift packages shared by PocketAI apps. Proprietary — see LICENSE.
Plan and task cards: ../strategy/ (01-architecture.md is normative).

## Build & test
- Rust: `cargo build --workspace && cargo test --workspace`
- XCFramework + Swift bindings: `scripts/build-xcframework.sh` (requires rustup targets aarch64-apple-ios, aarch64-apple-ios-sim, aarch64-apple-darwin; outputs build/pocket_ffiFFI.xcframework and swift/Sources/PocketKit/pocket_ffi.swift)
- Swift: `swift test` (after running build-xcframework.sh at least once)

## Headless server for SDK tests
`cargo run -p pocket-server --bin pocket-serve -- --mock-engine` starts a
loopback OpenAI-compatible server (`/v1/chat/completions`, stream and
non-stream) backed by `pocket_engine::MockEngine`, and prints `base_url=...`
and `api_key=...` to stdout. Point any OpenAI SDK at `base_url` with
`Authorization: Bearer <api_key>`. `--gguf <path>` is accepted but not wired
into pocket-serve yet (the llama.cpp engine exists as
`pocket_engine::llama::LlamaCppEngine`; this binary only runs `--mock-engine`);
it exits 1 with an explanation. Stop with Ctrl-C.

## Rules
- Only pocket-ffi is exported to Swift. Changes to pocket-ffi need L0 review.
- Errors: PocketError only across FFI.
- `build/` is generated, not committed; CI/each dev runs `scripts/build-xcframework.sh`.
