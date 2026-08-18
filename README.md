# openworkers-runtime-nova

> **Status: placeholder.** This repo exists so we don't forget that
> [Nova](https://github.com/trynova/nova) exists. Nothing is implemented yet.

OpenWorkers runtime backend for the [Nova JavaScript engine](https://trynova.dev)
— a pure-Rust, data-oriented JS/TS interpreter.

## Why Nova

- **Pure Rust, no C++ FFI surface** — unlike V8/JSC, the whole engine is
  auditable Rust. Boa already proved a pure-Rust engine slots into the
  `openworkers-core::Worker` trait; Nova is the more ambitious take
  (data-oriented heap, ECS-style object storage).
- **Security narrative fit** — a memory-safe engine aligns with the
  "trusted runtime for untrusted code" positioning, and Nova is itself
  funded by NGI Zero Core (NLnet), same programme as our application.
- Explicitly named as a target we like in our NLnet round-2 reply
  (`NLNET_ROUND2_REPLY_MIXED.md`, question 3).

## Nova state of play (August 2026)

- Crate: [`nova_vm`](https://crates.io/crates/nova_vm) **1.0.0** (March 2026), MPL-2.0.
- Embedding entry point: `GcAgent::new(Default::default(), &DefaultHostHooks)`
  (`nova_vm::ecmascript::{DefaultHostHooks, GcAgent}`, `nova_vm::engine::GcScope`).
- Known limitations (their README): performance "acceptable, but not fast",
  no sparse arrays, RegExp without lookaheads/lookbehinds/backreferences,
  no Promise subclassing, **no WebAssembly**.
- MPL-2.0 is file-level copyleft: fine as an unmodified dependency of an
  MIT project; contributions to the engine go upstream.

## Plan (when we pick this up)

Follow the boa/quickjs pattern: smallest useful surface first, everything
I/O-shaped through `OperationsHandler`.

1. `Worker::new` — build a `GcAgent`, evaluate the script.
2. `console` + `fetch` + timers wired to `OperationsHandle`.
3. `Event::Fetch` end-to-end (Request/Response marshaling).
4. Import the generic conformance tests (`generate_worker_tests!`).

The interesting research question vs. boa: does Nova's data-oriented heap
make per-worker memory accounting and heap limits *easier* than with
engines that hide their heap behind FFI?
