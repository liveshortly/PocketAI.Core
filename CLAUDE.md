# PocketAI.Core

Rust workspace + Swift packages shared by PocketAI apps. Proprietary — see LICENSE.
Plan and task cards: ../strategy/ (01-architecture.md is normative).

## Build & test
- Rust: `cargo build --workspace && cargo test --workspace`
- XCFramework + Swift bindings: `scripts/build-xcframework.sh` (requires rustup targets aarch64-apple-ios, aarch64-apple-ios-sim, aarch64-apple-darwin; outputs build/pocket_ffiFFI.xcframework and swift/Sources/PocketKit/pocket_ffi.swift)
- Swift: `swift test` (after running build-xcframework.sh at least once)

## Rules
- Only pocket-ffi is exported to Swift. Changes to pocket-ffi need L0 review.
- Errors: PocketError only across FFI.
- `build/` is generated, not committed; CI/each dev runs `scripts/build-xcframework.sh`.
