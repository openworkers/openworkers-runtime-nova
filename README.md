# openworkers-runtime-nova

OpenWorkers runtime backend for the [Nova JavaScript engine](https://trynova.dev)
- a pure-Rust, data-oriented JS/TS interpreter. `fetch` and `task` handlers
run end-to-end against `openworkers-core` v0.14; no host I/O from JS yet.
Engine study in NOTES-nova-api.md.

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
- MPL-2.0 is file-level copyleft: fine as a near-unmodified dependency of an
  MIT project; contributions to the engine go upstream.
- **No `temporal`:** the feature pulls `temporal_rs` 0.1.2, which compiles
  only against `icu_calendar` 2.1, while `v8` 152 forces 2.2.1 through
  `temporal_capi`. A lockfile shared with the V8 backend can satisfy one or
  the other, never both, so this crate takes the nova_vm defaults minus
  `temporal`. Nothing here exposed `Temporal` to the guest.
- **`[patch.crates-io]` on `nova_vm`:** 1.0.0 does not build without
  `temporal`, because `Intrinsics::temporal*` read heap constants that only
  exist under the feature. The sibling `../nova-vm` checkout is the published
  1.0.0 plus the seven missing `#[cfg(feature = "temporal")]`; drop the patch
  once upstream ships them.

## What works today

- `Worker::new`: builds a `GcAgent` with per-worker host hooks, installs the
  platform layer (below) plus the native builtins, then evaluates the guest
  script.
- **Web platform**: most of it is
  [`openworkers-wintertc`](https://github.com/openworkers/openworkers-wintertc),
  the surface the V8 backend runs: `Headers`, `Request`, `Response`,
  `FormData`, `Blob`, `File`, the streams, `Event` and `EventTarget`,
  `AbortController`, `structuredClone`, `atob`, `btoa`, `DOMException`. A
  module is taken when this runtime answers every op it reads, and
  `PROVIDED_OPS` is empty, so the six that read one are left out:
  `TextEncoder`/`TextDecoder`, `URL`, `URLPattern`, `navigator`, `performance`
  and the compression streams. The rest is here: `URL` and `URLSearchParams`
  (parsing and the setters run in the host through the `url` crate),
  `TextEncoder`/`TextDecoder`, `crypto.getRandomValues`, `crypto.randomUUID`,
  `queueMicrotask`, `console`, and the glue between the wire and a `Request`.
  `Headers` iterates sorted, as the Fetch standard prescribes, but the
  response goes on the wire in the order the handler set it, which is what the
  V8, JSC and Boa backends send and what an HTTP header list is.
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
| `Worker::new` (bootstrap + guest script eval) | 1.45 ms |
| `exec(Event::Task)` on a warm worker | 0.06 ms |

Nearly all of `Worker::new` is the platform layer, most of it the surface: the
same measurement is 0.45 ms without it, 0.16 ms with a one-script bootstrap.
Nova has no snapshot, so every worker evaluates the layer from source; V8 pays
for it once, at build time.

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

Measured without the surface, which costs about a millisecond on a cold worker
and nothing on a warm one.

The rendered bytes match the V8 reference render exactly, down to
SvelteKit's `etag` over the body. Parse and compile of a real-world bundle
costs about what V8 charges; guest compute is where the engine's own
"acceptable, but not fast" shows, at roughly 40x a V8 warm render.

## Conformance

`openworkers-conformance` scores this backend at **368 of 448**, second behind
v8's 448 and ahead of boa's 319. `Headers`, `Request`, `Response` and `URL` are
complete; timers (0/16), `crypto.subtle` (9/30) and the globals that stand on
them are what is missing.

`cargo run --release --example conformance` replays the 17 requests of
`openworkers-conformance/fixtures/sveltekit-app` against the responses V8
recorded, on the same lowered bytes. Ten match byte for byte, status,
header order and body; the other seven die in `cookies.set()`, on a regex
the engine will not compile. Answer that one `match` call with `null` and
16 of 17 match, the last being the scenario where the oracle keeps a `+`
that WHATWG decodes to a space. The bindings the fixture wants are a JS
shim in the runner, since this backend does not expose `Script.env` yet.
NOTES-nova-api.md has the patterns and the diagnostics.

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
- Bodies cross the host boundary as UTF-8 text (request bodies
  lossy-decoded, response bodies and headers lose lone surrogates to
  U+FFFD). A response built on a stream is drained before it goes on the
  wire, so nothing streams out incrementally.
- **`charCodeAt` aborts the process** on a heap-allocated string whose first
  character is a surrogate pair: `'\u{1f600}\u{20ac}a'.charCodeAt(1)` reads a
  mapping entry that nova_vm never filled in. `codePointAt` does the same one
  index further on. The string iterator is the way around it, and what this
  crate's own encoder uses.
- **The engine's RegExp is not safe for untrusted guests.** Beyond the
  patterns it refuses to compile (lookaround, backreferences, surrogate
  escapes, `\0` and `\b` and unescaped `[` inside a character class - all
  thrown at first use, far from the literal), it gets non-ASCII input
  wrong: `match.index` is a UTF-8 byte offset, so a regex `replace` or
  `split` whose match is non-ASCII **panics the process**, and one whose
  match merely sits after non-ASCII text returns a silently wrong string.
  `$1` and `$&` are never substituted, the `m` flag is ignored, and a
  regex literal is a shared singleton whose `lastIndex` carries over from
  the previous request. In SvelteKit terms: pages render, but
  `cookies.set()`, the fatal-error fallback page and the CSP meta tag do
  not. See NOTES-nova-api.md for the reconnaissance and the upstream asks.
- **No `crypto.subtle`.** Streams, `AbortController` and the DOM event core
  arrived with the surface; the compression streams did not, since they ask
  the host for a codec.
- The request the dispatch glue hands a handler is a string body wrapped in a
  stream: a body that is not UTF-8 text arrives lossy-decoded.
- `respondWith` only counts if it runs within one microtask turn of the
  handler returning; later calls lose the race with the dispatch glue.
  Same rule for a task, where losing the race means the handler's return
  value is delivered instead.
- The glue is not isolated from the guest: `__ow_dispatch`,
  `__ow_headers_to_wire` and the `__ow_native_*` builtins are ordinary
  globals, and replacing an intrinsic the glue uses (`JSON.stringify`,
  ...) breaks dispatch. A response can only reach the caller it belongs
  to, though: `__ow_dispatch` gets a dispatch id and the host drops any
  `__ow_native_respond` that does not echo the one in flight.
- `Atomics.waitAsync` that nobody notifies ends the request with
  `MaxIterationsReached` and leaks its parked waiter thread; nova offers
  no way to cancel it.

## Next steps

1. Wire `OperationsHandler` for guest-visible `fetch()`: hand out a
   resolve/reject function pair per pending operation and interleave host
   futures with the job-queue drain (nova has no public promise
   constructor/inspection API - see NOTES-nova-api.md).
2. Timers via `enqueue_timeout_job`'s milliseconds argument.
3. Answer the ops the surface asks for, starting with `textEncode` and
   `textDecode`: nine of them would take five more modules, `URL` and
   `URLPattern` included, and four more would take the compression streams.
4. Stream a response body out instead of draining it at the wire.
5. Revisit heap limits upstream: the data-oriented heap should make
   per-worker memory accounting easier than FFI engines, but nova_vm 1.0
   exposes no API for it yet.
6. File the RegExp gaps and the `charCodeAt` abort upstream
   (NOTES-nova-api.md): the first is what stands between this backend and an
   unmodified SvelteKit app, the second kills the process from guest code.
