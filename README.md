# PocketAI.Core

PocketAI.Core is the shared engine behind the PocketAI apps for Mac and iPhone. It is a Rust workspace (model catalog and download/fit logic, a local inference engine with llama.cpp, an OpenAI-compatible HTTP server, the local store, pairing and licensing) exposed to Swift through a single UniFFI crate, `pocket-ffi`. On top of that sit the Swift packages both apps import: `PocketKit`, `PocketUI`, `PocketMLX`, `PocketImageGen` and `PocketLiveShortly`.

## How the repos fit together

| Repo | What it is |
|------|------------|
| [PocketAI.Core](https://github.com/liveshortly/PocketAI.Core) | This repo. The Rust core, the XCFramework build, and the shared Swift packages. |
| [PocketAI.Mac](https://github.com/liveshortly/PocketAI.Mac) | SwiftUI macOS app. Consumes this repo as a local Swift package. |
| [PocketAI.Iphone](https://github.com/liveshortly/PocketAI.Iphone) | SwiftUI iOS app. Consumes this repo as a local Swift package. |

The apps reference Core by relative path (`../PocketAI.Core` in their `project.yml`), so all three must be checked out side by side in one folder:

```
getpocketai/
├── PocketAI.Core
├── PocketAI.Mac
└── PocketAI.Iphone
```

```sh
mkdir getpocketai && cd getpocketai
git clone https://github.com/liveshortly/PocketAI.Core.git
git clone https://github.com/liveshortly/PocketAI.Mac.git
git clone https://github.com/liveshortly/PocketAI.Iphone.git
```

`PocketLiveShortly` is the client for [LiveShortly](https://github.com/liveshortly/liveshortly-server): it signs in to a LiveShortly server and shows, starts and follows sessions over HTTP. The Mac app additionally drives the [`live` CLI](https://github.com/liveshortly/live) (`live daemon`) so the Mac can host sessions. See each app's README for how to point it at a self-hosted server.

## Requirements

- macOS on Apple Silicon. The XCFramework contains only arm64 slices (`aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-apple-darwin`).
- Xcode with a Swift 6 toolchain (`Package.swift` uses `swift-tools-version:6.0`). <!-- TODO(owner): state the exact Xcode version you build with. -->
- Rust via [rustup](https://rustup.rs). The toolchain is pinned to `stable` in `rust-toolchain.toml`. Add the targets:
  ```sh
  rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin
  ```
- CMake. `llama-cpp-sys-2` builds llama.cpp through CMake. <!-- TODO(owner): confirm whether any other system tools (for example a specific CMake version) are needed. -->
- Deployment targets: iOS 18.0, macOS 14.0 (pinned in `Package.swift` and `scripts/build-xcframework.sh`).

## Build

### Rust workspace

```sh
cargo build --workspace
cargo test --workspace
```

CI also runs `cargo fmt --check`, `cargo clippy --workspace -- -D warnings` and `cargo deny check`.

### XCFramework and Swift bindings

The Swift packages link a prebuilt XCFramework that is generated, not committed. Build it once after cloning, and again whenever `pocket-ffi` or anything under it changes:

```sh
scripts/build-xcframework.sh
```

This builds `pocket-ffi` in release mode for the three targets, generates the UniFFI bindings, and writes:

- `build/pocket_ffiFFI.xcframework`
- `swift/Sources/PocketKit/pocket_ffi.swift`

The first build compiles llama.cpp for every target and takes a while. Changing the pinned deployment targets triggers a clean rebuild.

### Swift packages

```sh
swift test
```

Run it after `scripts/build-xcframework.sh`. Set `POCKET_SKIP_KEYCHAIN_TESTS=1` to skip tests that need Keychain access (CI does). To check that the packages build for iOS:

```sh
xcodebuild -scheme PocketKit -destination 'generic/platform=iOS' build
xcodebuild -scheme PocketUI -destination 'generic/platform=iOS' build
```

## Headless OpenAI-compatible server

`pocket-serve` starts a loopback OpenAI-compatible server (`/v1/chat/completions`, streaming and non-streaming) backed by a mock engine:

```sh
cargo run -p pocket-server --bin pocket-serve -- --mock-engine
```

It prints `base_url=...` and `api_key=...`. Point any OpenAI SDK at `base_url` with `Authorization: Bearer <api_key>`. Stop it with Ctrl-C. `--gguf <path>` is accepted but not wired into this binary yet; it exits with an explanation. The llama.cpp engine itself lives in `pocket_engine::llama::LlamaCppEngine`.

To run the official OpenAI Python and Node SDKs against a test server (needs `python3` and `npm`):

```sh
scripts/test-sdk.sh
```

## Other scripts

| Script | Purpose |
|--------|---------|
| `scripts/validate-openai-fixtures.sh` | Structural check of `contracts/openai/*` fixtures. |
| `scripts/check-no-raw-http.sh`, `scripts/check-no-raw-urlsession.sh` | Enforce that network calls go through the shared HTTP layer. |
| `scripts/catalog-pin.py` | Model catalog maintenance. <!-- TODO(owner): one-line description of catalog-pin.py. --> |
| `scripts/sign-catalog.sh` | Maintainer only. Signs the model catalog with the catalog private key, which is not in this repo. |
| `scripts/release.sh` | Maintainer only. Builds and zips the XCFramework and publishes a `core-vX.Y.Z` GitHub release (needs an authenticated `gh`). |

## Repo layout

| Path | Contents |
|------|----------|
| `crates/pocket-types` | Shared domain types (chat, model, device, project, errors). |
| `crates/pocket-store` | SQLite store, migrations and repositories. |
| `crates/pocket-models` | Model catalog, hardware fit, downloads, Hugging Face hub access. |
| `crates/pocket-engine` | Inference engine abstraction, llama.cpp engine, mock engine, RAG, tools. |
| `crates/pocket-trust` | HTTP client, TLS, pairing and license verification. |
| `crates/pocket-server` | OpenAI-compatible HTTP server and the `pocket-serve` binary. |
| `crates/pocket-audit` | Audit log service. |
| `crates/pocket-agents` | Agent sessions, approvals and risk classification. |
| `crates/pocket-bridge` | Mac bridge binary (early skeleton). |
| `crates/pocket-ffi` | The only crate exported to Swift (UniFFI). |
| `swift/Sources` | Swift packages: `PocketKit`, `PocketUI`, `PocketMLX`, `PocketImageGen`, `PocketImageGenVendor`, `PocketLiveShortly`. |
| `swift/Tests` | Swift package tests. |
| `contracts` | OpenAPI spec (`pocket-api.yaml`), event schema and OpenAI fixtures. |
| `tests/sdk` | OpenAI SDK compatibility tests (Python, Node). |
| `scripts` | Build, check and release scripts. |
| `brand` | Logo and app icon SVGs. |
| `docs` | App review notes and i18n inventory. |

Third-party code vendored into this repo is listed in `THIRD_PARTY.md`.

## Contributing

- Only `pocket-ffi` is exported to Swift; errors cross the FFI boundary as `PocketError` only.
- `build/` is generated. Do not commit it.
- Run `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace` and `swift test` before opening a pull request.

<!-- TODO(owner): add contribution terms (issues/PRs welcome? CLA?). The current LICENSE grants no rights to use or modify the code. -->

## License

Copyright (c) 2026 Rohit Sehgal. All rights reserved. This is proprietary software; see [`LICENSE`](LICENSE). Making the repository public does not grant any license to use, copy, modify or distribute it.

<!-- TODO(owner): a public repo keeps this proprietary "All rights reserved" license unless you change LICENSE (and the `license` field in Cargo.toml). -->
