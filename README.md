# openworkers-runtime-nova

> **Status: renders SvelteKit.** The 303 KB SSR bundle of
> openworkers-website runs on this backend and returns a page byte-identical
> to the V8 reference render. Synchronous `fetch` and `task` handlers run
> end-to-end against `openworkers-core` v0.14. No host I/O from JS yet.
> See NOTES-nova-api.md for the engine study.

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

- `Worker::new`: builds a `GcAgent` with per-worker host hooks, installs the
  platform layer (below) plus the native builtins, then evaluates the guest
  script.
- **Web platform**: `URL`, `URLSearchParams`, `Headers`, `Request`,
  `Response`, `TextEncoder`, `TextDecoder`, `atob`, `btoa`,
  `queueMicrotask`, `crypto.getRandomValues`, `crypto.randomUUID`,
  `DOMException`, `console`. `URL` parsing and its setters run in the host
  through the `url` crate; the rest is JS in `src/*.js`.
- **Handler shapes**: `addEventListener('fetch'|'task')` and the module
  convention `globalThis.default = { fetch(request, env, ctx), task(...) }`.
  A listener wins over the module export.
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
| `Worker::new` (bootstrap + guest script eval) | 0.45 ms |
| `exec(Event::Task)` on a warm worker | 0.04 ms |

Most of `Worker::new` is the platform layer: the same measurement was
0.16 ms when the bootstrap was one small script. Nova has no snapshot, so
every worker re-evaluates `src/*.js` from source.

`cargo run --release --example ssr -- <bundle.js> [expected.html]` runs the
real workload: wake a worker, render a SvelteKit page, return the HTML. The
fixture is the 354 KB classic-script lowering of openworkers-website; every
route of that site is prerendered, so the bench asks for `/ssr-bench`, the
one path that reaches the renderer (a 404 page, 2615 bytes, through the full
SSR pipeline). Same laptop, under load, medians of 10 to 20 runs:

| Step | min | median |
| --- | --- | --- |
| parse + compile 354 KB (never evaluated) | 3.98 ms | 4.13 ms |
| `Worker::new` (parse + compile + top-level eval) | | 5.4 ms |
| first render | | 5.2 ms |
| warm render | 3.69 ms | 3.84 ms |
| cold cycle (new worker + one render) | 9.74 ms | 9.96 ms |
| RSS per resident worker | | 6.0 MB |

The rendered bytes match the V8 reference render exactly, down to
SvelteKit's `etag` over the body. Parse and compile of a real-world bundle
costs about what V8 charges; guest compute is where the engine's own
"acceptable, but not fast" shows, at roughly 40x a V8 warm render.

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
- **The engine's RegExp blocks three patterns a real bundle hits**: no
  lookaround or backreferences, lone surrogate escapes
  (`/[\ud800-\udbff]/`) are rejected outright, and `\0` in a character
  class is read as a backreference. Named groups compile but never
  populate `match.groups`. In SvelteKit terms: pages render, but the error
  page (`escape_html`) and `__data.json` (devalue) do not. A pattern is
  compiled on first use, not at construction, so the throw lands far from
  the literal.
- **No streams**: no `ReadableStream`, so a `Response` built on one is
  refused rather than silently stringified. No `AbortController`, no
  `fetch()`, no `crypto.subtle`.
- `Request` has no `body` property (that would be a stream) and no
  `signal`. Response bodies reach the host as text.
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
4. `ReadableStream`, and streaming bodies through it, once a workload needs
   one: the SvelteKit fixture renders without it.
5. Revisit heap limits upstream: the data-oriented heap should make
   per-worker memory accounting easier than FFI engines, but nova_vm 1.0
   exposes no API for it yet.
6. File the RegExp gaps upstream (NOTES-nova-api.md): they are what stands
   between this backend and an unmodified SvelteKit app.
