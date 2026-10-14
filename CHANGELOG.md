# Changelog

All notable changes to `PocketAI.Core`'s FFI surface and server routes.
Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [0.2.0] - 2026-10-14

### Added
- `PocketCore.send()`: a real chat turn (persist user message → resolve
  model → load → generate → persist assistant message), proven against a
  real `pocket_engine::MockEngine` bridged through the FFI-local `Engine`
  trait.
- `pocket_server::start_with_routes`: merges a caller-supplied `Router` so
  `pocket-ffi` can compose its own HTTP routes without `pocket-server`
  depending on `pocket-ffi`'s types.
- NET-10 pairing route layer (`POST /pocket/v1/pair`, `GET
  /pocket/v1/pair/{id}` long-poll) with real constant-time secret checks,
  single-use enforcement, and `approve_pairing`/`deny_pairing` wired to
  `DevicesRepo`.
- NET-12 control API (`GET /pocket/v1/catalog`, `GET /pocket/v1/status`,
  `POST /pocket/v1/models/{id}/download`, `DELETE /pocket/v1/models/{id}`)
  calling back into `PocketCore` via a `Weak<PocketCore>` self-reference.
- NET-05 `POST /v1/chat/completions`: OpenAI-compatible non-stream and SSE
  stream, backed by `PocketCore::generate_chat` (shared with `send()`).
- NET-08 `/v1/audio/transcriptions`, `/v1/audio/speech`,
  `/v1/images/generations`, behind the (now actually wired)
  `register_stt`/`register_tts`/`register_image`.
- NET-16: `crates/pocket-server/src/bin/pocket-serve.rs --mock-engine`, a
  standalone headless OpenAI-compatible server for SDK tests.
- `pocket_engine::{benchmark_after_load, BenchmarkStore}` (ENG-09): measures
  real throughput after a model's first load, skip/cancel-aware.

### Fixed
- `pocket-ffi`'s dedicated FFI runtime was single-threaded
  (`new_current_thread`), which silently stopped a spawned server from
  accepting connections the instant `server_start()` returned. Switched to
  a 2-worker multi-thread runtime.

### Changed
- `register_stt`/`register_tts`/`register_image` went from no-op stubs to
  actually storing the registered engine.

See `../strategy/DECISIONS.md` D30–D40 for the full reasoning behind each
of these.

## [0.1.0] - 2026-10-14
- L0-07 contract freeze: `PocketCore` skeleton with every A§4.5 method
  stubbed, all 9 foreign traits, Swift package/app shells building and
  calling through the FFI boundary in both directions.
