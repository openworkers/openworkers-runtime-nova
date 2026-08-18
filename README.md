# openworkers-runtime-nova

> **Status: v0 working.** Synchronous `fetch` and `task` handlers run
> end-to-end against `openworkers-core` v0.14, covered by an integration
> test suite. No host I/O from JS yet. See NOTES-nova-api.md for the
> engine study.

OpenWorkers runtime backend for the [Nova JavaScript engine](https://trynova.dev)
- a pure-Rust, data-oriented JS/TS interpreter.

## Why Nova

- **Pure Rust, no C++ FFI surface** - unlike V8/JSC, the whole engine is
  auditable Rust. Boa already proved a pure-Rust engine slots into the
  `openworkers-core::Worker` trait; Nova is the more ambitious take
  (data-oriented heap, ECS-style object storage).
- **Security narrative fit** - a memory-safe engine aligns with the
  "trusted runtime for untrusted code" positioning.

## Nova state of play (August 2026)

- Crate: [`nova_vm`](https://crates.io/crates/nova_vm) **1.0.0** (March 2026), MPL-2.0.
- Embedding entry point: `GcAgent::new(Default::default(), &DefaultHostHooks)`
  (`nova_vm::ecmascript::{DefaultHostHooks, GcAgent}`, `nova_vm::engine::GcScope`).
- Known limitations (their README): performance "acceptable, but not fast",
  no sparse arrays, RegExp without lookaheads/lookbehinds/backreferences,
  no Promise subclassing, **no WebAssembly**.
- MPL-2.0 is file-level copyleft: fine as an unmodified dependency of an
  MIT project; contributions to the engine go upstream.
- **Lockfile pin (do not `cargo update` blindly):** `temporal_rs` 0.1.2
  (nova_vm dep) uses icu4x `unstable` APIs and breaks against icu 2.3;
  the committed `Cargo.lock` pins the icu4x family to 2.1.0. Re-pin after
  any update, or bump `nova_vm` past the fix.

## What works today

- `Worker::new`: builds a `GcAgent` with per-worker host hooks, installs a
  JS bootstrap (`Request`, `Response`, `console`, `addEventListener`) plus
  two native builtins, then evaluates the guest script.
- `Worker::exec` for `Event::Fetch`: the request is marshaled to JS as
  JSON, dispatched to the registered handlers, and the response (status,
  headers, text body) is sent back through `FetchInit`'s response channel.
- `Worker::exec` for `Event::Task`: the `TaskInit` fields (`taskId`,
  `payload`, `source`, `attempt`, and `scheduledTime` for a cron source)
  reach the guest as a task event, and the handler's result goes back
  through `TaskInit`'s result channel.
- Async guest handlers (`async (event) => ...`, `respondWith(promise)`)
  work as long as they need no host I/O: the embedder drains Nova's job
  queue until the dispatch promise settles.
- Guest exceptions, missing handlers and syntax errors surface as
  `TerminationReason::Exception` with the JS error message. A task that
  throws is the exception: it becomes a failed `TaskResult`, since a queue
  wants the message recorded rather than the worker torn down.
- `console.*` prints to stderr for now.

```js
addEventListener('fetch', (event) => {
  event.respondWith(new Response('Hello, World!', { status: 200 }));
});

addEventListener('task', (event) => ({ doubled: event.payload.n * 2 }));
```

### Task handlers

Both handler styles work, and a `task` listener wins over the module export:

```js
globalThis.default = {
  async task(event, env, ctx) {
    ctx.waitUntil(background());

    return { doubled: event.payload.n * 2 };
  },
};
```

Nova evaluates classic scripts, so `export default { ... }` has to be
lowered to `globalThis.default = { ... }` first (the `openworkers-transform`
SWC pass does this; the embedder runs it, not this crate).

The result is whatever `respondWith()` was given, else the handler's return
value. A value carrying a boolean `success` is taken as a whole
`TaskResult`; anything else becomes its `data`. `waitUntil` promises are
awaited before the result is delivered, and a rejected one does not sink a
result already produced.

## Performance

`cargo run --release --example timings` builds 200 workers and runs one task
on each. On an M-series laptop under load, for a one-line task handler:

| Step | median |
| --- | --- |
| `Worker::new` (bootstrap + guest script eval) | 0.16 ms |
| `exec(Event::Task)` on a warm worker | 0.05 ms |

Nova's own README calls the engine "acceptable, but not fast"; that shows up
in guest compute, not in startup, where having no snapshot to map and no C++
heap to build makes it the cheapest of the backends measured so far.

## Known limitations (v0)

- **No host I/O from JS**: no `fetch()`, no timers, no
  `OperationsHandler` wiring. A handler that never settles its response
  promise fails with an explicit error instead of hanging.
- **`RuntimeLimits` unenforced**: nova_vm 1.0 has no heap cap,
  instruction budget or interrupt API. `abort()` only rejects future
  `exec()` calls. The only guard is a cap on jobs per drain.
- **Unbounded guest recursion aborts the process**: nova_vm 1.0 runs the
  guest call stack on the host stack with no depth guard, so
  `(function f() { return f(); })()` kills the runner.
- **`Script.env` and bindings are not exposed to the guest yet** (the
  guest-facing convention is still to be settled platform-wide).
- Bodies are UTF-8 text only (request bodies lossy-decoded, response
  bodies and headers lose lone surrogates to U+FFFD); streaming bodies
  are rejected.
- Minimal `Request`/`Response` polyfills, not the WHATWG classes
  (headers are plain objects; no `Headers`, `URL`, streams,
  `removeEventListener`, listener objects with `handleEvent`, ...). The
  response status must be an integer in 100-599, so anything that is not
  response-shaped is rejected with a `RangeError`.
- `respondWith` only counts if it runs within one microtask turn of the
  handler returning; later calls lose the race with the dispatch glue.
  Same rule for a task, where losing the race means the handler's return
  value is delivered instead.
- The glue is not isolated from the guest: `__ow_dispatch` and the
  `__ow_native_*` builtins are ordinary globals, and replacing an
  intrinsic the glue uses (`JSON.stringify`, ...) breaks dispatch.
- `Atomics.waitAsync` that nobody notifies ends the request with
  `MaxIterationsReached` and leaks its parked waiter thread; nova offers
  no way to cancel it.

## Next steps

1. Wire `OperationsHandler` for guest-visible `fetch()`: hand out a
   resolve/reject function pair per pending operation and interleave host
   futures with the job-queue drain (nova has no public promise
   constructor/inspection API - see NOTES-nova-api.md).
2. Timers via `enqueue_timeout_job`'s milliseconds argument.
3. Import the generic conformance tests (`generate_worker_tests!`).
4. Revisit heap limits upstream: the data-oriented heap should make
   per-worker memory accounting easier than FFI engines, but nova_vm 1.0
   exposes no API for it yet.
