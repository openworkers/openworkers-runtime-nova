# nova_vm 1.0 embedding API - reconnaissance notes

Findings from reading the `nova_vm` 1.0.0 sources
(`~/.cargo/registry/src/*/nova_vm-1.0.0/`). File references below are relative
to that crate root. Written August 2026 while implementing the first
`Worker` iteration in this repo.

## TL;DR

| Capability | Status | Mechanism |
|---|---|---|
| Create engine + realm | yes | `GcAgent::new` + `create_realm`/`create_default_realm` |
| Evaluate a script | yes | `Agent::run_script` (or `parse_script` + `script_evaluation`) |
| Host (native Rust) functions | yes | `create_builtin_function` + `Behaviour::Regular` |
| Host state shared with builtins | yes | `HostHooks::get_host_data` (`&dyn Any`) |
| Build/read strings | yes | `String::from_str` / `to_string_lossy` (WTF-8 internally) |
| Build/read objects | yes | `InternalMethods` trait (`internal_get`, `internal_define_own_property`, ...) |
| Call a JS function from Rust | yes | `Function::call` |
| Drive microtasks from embedder | yes | `HostHooks::enqueue_*_job` + `GcAgent::run_job` |
| Inspect a Promise result from Rust | **no public API** | workaround: hand the value to a host function from JS |
| Timers | embedder-provided | `enqueue_timeout_job(job, ms)` exists; no `setTimeout` global |
| Heap/CPU limits, interruption | **missing** | no API in 1.0 |
| Web APIs (fetch, URL, TextEncoder, console, ...) | none | pure ECMAScript engine; embedder provides everything |

## Agent and realm creation

Entry point is `GcAgent` (`src/ecmascript/execution/agent.rs`):

```rust
let mut agent = GcAgent::new(AgentOptions::default(), &DefaultHostHooks);
let realm: RealmRoot = agent.create_default_realm();
let result = agent.run_in_realm(&realm, |agent: &mut Agent, gc: GcScope| {
    // all engine work happens inside this closure
});
```

- `AgentOptions` (agent.rs:60) has exactly three knobs: `disable_gc`,
  `print_internals`, `no_block`. No memory or time limits.
- `GcAgent::new` takes `host_hooks: &'static dyn HostHooks`. The `'static`
  bound means a per-worker hooks instance must be leaked (`Box::leak`) and
  reclaimed manually after the agent is dropped, or be an actual `static`.
- `create_realm(create_global_object, create_global_this_value, initialize_global_object)`
  accepts three optional closures; `initialize_global_object(&mut Agent, Object, GcScope)`
  is where globals (host functions) get installed. The unused `Option`s need
  explicit turbofish-ish typing:
  `let none: Option<for<'a> fn(&mut Agent, GcScope<'a, '_>) -> Object<'a>> = None;`
- `create_default_realm()` is the no-customization variant ("suitable for
  basic testing only").
- Realms are rooted until `remove_realm`; up to 256 simultaneous realms per
  agent (`RealmRoot` is a `u8` index, agent.rs:743).
- `GcAgent::gc()` triggers collection manually; it also runs on heap pressure.

## Script evaluation

`Agent::run_script` (agent.rs:1421) is the convenience path:

```rust
agent.run_in_realm(&realm, |agent, mut gc| {
    let source = String::from_string(agent, source_code, gc.nogc());
    match agent.run_script(source.unbind(), gc.reborrow()) {
        Ok(value) => { /* value of the last expression */ }
        Err(e) => { /* JsError */ }
    }
})
```

- Internally: `parse_script` (oxc parser) + `script_evaluation`. Parse errors
  are turned into a thrown `SyntaxError`.
- The returned value is the script's completion value (last expression), so
  "eval and read a global" can be done in one call:
  `run_script("__result")` returns that global's value.
- `Agent::run_module` exists for ES modules (sync-resolving modules only).

## Host functions (native Rust callable from JS)

`create_builtin_function` (`src/ecmascript/builtins/builtin_function.rs`):

```rust
fn my_fn<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let text = args.get(0).to_string(agent, gc)?.to_string_lossy(agent).into_owned();
    Ok(Value::Undefined)
}

let function = create_builtin_function(
    agent,
    Behaviour::Regular(my_fn),
    BuiltinFunctionArgs::new(1, "my_fn"),
    gc.nogc(),
);
let key = PropertyKey::from_static_str(agent, "my_fn", gc.nogc());
global.internal_define_own_property(
    agent,
    key.unbind(),
    PropertyDescriptor { value: Some(function.unbind().into()), ..Default::default() },
    gc,
)?;
```

- `Behaviour::Regular(RegularFn)` for plain functions,
  `Behaviour::Constructor(ConstructorFn)` for `new`-ables
  (builtin_function.rs:375-395). Plain `fn` pointers only - no closures, so
  state must go through host data (below).
- `ArgumentsList::get(i)` returns `Value::Undefined` when absent
  (builtin_function.rs:207). NOTE: the arguments are NOT rooted across GC
  inside the builtin; if the builtin can trigger GC (e.g. `to_string` on an
  object calls user `toString`) before reading later args, root them first
  with `ArgumentsList::with_scoped`.

### Sharing state between Rust and builtins

`HostHooks::get_host_data(&self) -> &dyn Any` (agent.rs:656, default impl
panics). Builtins reach it via `agent.get_host_data().downcast_ref::<T>()`.
Interior mutability (`RefCell`) required, since only `&dyn Any` is exposed.

## Strings

- Build: `String::from_str(agent, &str, nogc)`, `from_string(agent, String, nogc)`,
  `from_static_str` (`src/ecmascript/types/language/string.rs`).
- Read: `s.to_string_lossy(agent) -> Cow<str>` (internal encoding is WTF-8;
  lone surrogates get replaced), `s.as_str(agent) -> Option<&str>` (None if
  not valid UTF-8), `s.as_wtf8(agent)`.
- Convert any `Value`: `value.to_string(agent, gc)` - may run JS (`toString`)
  and therefore needs the full `GcScope` and can trigger GC.
- Strings up to 7 bytes are stack-allocated (`SmallString`), no heap involved.

## Objects

- Read/write properties from Rust: the `InternalMethods` trait
  (`src/ecmascript/types/language/object/internal_methods.rs`) is public and
  implemented by all object handles: `internal_get`, `internal_set`,
  `internal_define_own_property`, `internal_own_property_keys`, ...
- Keys: `PropertyKey::from_static_str(agent, "name", nogc)` and friends.
- Call a JS function: `Function::call(agent, this, &mut [args], gc)`
  (`src/ecmascript/types/language/function.rs:64`).
- Plain object construction from Rust goes through
  `OrdinaryObject`/`ObjectHeapData` creation - doable but verbose; for
  structured data it is much cheaper to pass a JSON string across the
  boundary and `JSON.parse`/`JSON.stringify` on the JS side (the `json`
  feature, backed by sonic-rs, is on by default).

## Promises, jobs and the microtask queue

The engine has NO built-in job queue. Every job is handed to the embedder
through `HostHooks` (agent.rs:362):

- `enqueue_promise_job(&self, job: Job)` - promise reactions/thenables;
- `enqueue_generic_job(&self, job: Job)` - spec "generic" jobs;
- `enqueue_timeout_job(&self, job: Job, milliseconds: u64)` - used by e.g.
  `Atomics.waitAsync`; also the intended hook for embedder timers.

`DefaultHostHooks` DROPS all jobs (default_host_hooks.rs) - with it, `.then`
callbacks never run even for already-resolved promises. A real embedder keeps
a `RefCell<VecDeque<Job>>` in its hooks and drains it:

```rust
while let Some(job) = hooks.jobs.borrow_mut().pop_front() {
    gc_agent.run_job(job, |agent, result, gc| { /* result: JsResult<()> */ });
}
```

`GcAgent::run_job` (agent.rs:812) must be called OUTSIDE `run_in_realm`
(both assert an empty execution-context stack); the `Job` carries its realm.

### Trap: `GcAgent::run_job` panics on realm-less jobs

`GcAgent::run_job` does `job.realm.take().unwrap()` (agent.rs:1046), but nova
builds jobs with `realm: None` whenever the reaction handler has no function
realm - `PromiseReactionHandler::PromiseGroup` (`Promise.all`, `race`, `any`,
`allSettled`) and the async-iteration handlers (promise_jobs.rs:344-352). Guest
code calling any of them aborts the process. The public `Job::run(agent, gc)`
handles `realm: None` correctly, so the drain loop here calls it inside
`run_in_realm` instead.

### Trap: unfinished jobs block the thread

`Atomics.waitAsync` spawns a waiter thread and `WaitAsyncJob::run` joins it
(atomics_object.rs:1663); with no timeout that join never returns. `Job::is_finished()`
exists to test this before running - a drain loop that ignores it hangs forever
on guest code that waits on a value nobody notifies.

### Gap: no public way to inspect a settled Promise

`Promise::try_get_result` and the `PromiseState` enum are `pub(crate)`
(`src/ecmascript/builtins/promise.rs:84`, `promise/data.rs:19`). There is no
embedder API equivalent to Boa's `promise.state()`. The workaround (used in
this repo): JS glue attaches `.then(ok => __host_fn(...), err => __host_fn(...))`
and the host function stores the settled value into host data; after draining
the job queue, Rust reads it back.

## Error handling

- Everything returns `JsResult<'gc, T> = Result<T, JsError<'gc>>`.
- `JsError::value()` gives the thrown `Value`; `JsError::to_string(agent, gc)`
  gives the display string (agent.rs:97-105); `value.string_repr(agent, gc)`
  is the alternative used by nova's own tests.
- Throw from a builtin: `agent.throw_exception(ExceptionType::TypeError, msg, gc)`
  and variants (agent.rs:1114-1180).
- Uncaught errors inside a job surface as `Err` in `run_job`'s callback.

## GC-scope ergonomics (the hard part)

Nova brands every heap handle with a lifetime tied to a `GcScope` token; GC
can only run at points where a `GcScope` is passed by value. Rules learned:

- `gc.nogc()` gives a `NoGcScope` for allocation-only APIs; `gc.reborrow()`
  lends the scope to a callee while keeping it usable afterwards; passing `gc`
  by value is "last use".
- Holding a handle produced under `gc.reborrow()` keeps the mutable borrow of
  `gc` alive - call `.unbind()` (the `Bindable` trait, also implemented for
  `Result`/`Option`) to detach before touching `gc` again, e.g.
  `let script = parse_script(...).unwrap(); script_evaluation(agent, script.unbind(), gc.reborrow())`.
  `.unbind()` is an assertion that the value survives the next GC point; the
  safe pattern is `.unbind()` immediately followed by re-`bind`/consumption.
- Root across GC points / closures: `Global<T>::new(agent, value)` (explicit
  `take`/`get`, not Drop-released, `src/engine/rootable/global.rs`) or
  `Scoped<T>` (released with the scope).
- Values cannot escape `run_in_realm` un-rooted; the closure's `R` return type
  cannot carry lifetime-branded handles (extract to Rust types inside).
- `run_in_realm` asserts empty vm/context stacks on both ends - never nest it,
  never call it from inside a builtin.

## Missing pieces relevant to openworkers (v1.0)

Ordered by what blocks this host, and phrased so each can be filed as its own
issue against `trynova/nova`. The first four are the ones that keep this
backend from being production-usable; the rest are ergonomics.

1. **No resource limiting.** `AgentOptions` (agent.rs:60) has three knobs -
   `disable_gc`, `print_internals`, `no_block` - and none of them bound heap
   or CPU. A multi-tenant host cannot honour a per-worker memory cap or an
   instruction budget, so `RuntimeLimits` is accepted and ignored here. *Ask:*
   a heap ceiling on `AgentOptions`, and an interrupt-check callback the
   embedder can install (V8's `SetInterruptCallback` / `TerminateExecution`,
   Boa's job budget).
2. **No way to interrupt running JS.** `while (true) {}` in a guest owns the
   thread forever; `abort()` can only refuse the *next* `exec`. Same root
   cause as 1, but worth filing apart: a host can live with an unbounded heap
   far more easily than with an unkillable request.
3. **No call-depth guard.** Guest recursion runs on the host stack with no
   limit, so `(function f() { return f(); })()` overflows the Rust stack and
   aborts the process. It cannot even be covered by a test here, since the
   test process dies with it. *Ask:* a configurable max call depth throwing
   `RangeError`, as every other engine does.
4. **No promise inspection or construction from Rust.**
   `Promise::try_get_result` and `PromiseState` are `pub(crate)`
   (promise.rs:84, promise/data.rs:19), and there is no public
   `Promise::with_resolvers`. Every host operation that resolves
   asynchronously (`fetch`, timers, KV reads) therefore needs JS glue holding
   a resolve/reject pair, plus a native function to carry the value back. This
   is what stands between this backend and guest-visible `fetch()`. *Ask:*
   expose `PromiseState`/`try_get_result` and a `Promise::new_with_resolvers`
   equivalent for embedders.
5. **`GcAgent::run_job` panics on realm-less jobs.** It does
   `job.realm.take().unwrap()` (agent.rs:1046), but nova builds jobs with
   `realm: None` for `PromiseReactionHandler::PromiseGroup` (`Promise.all`,
   `race`, `any`, `allSettled`) and async iteration (promise_jobs.rs:344-352).
   Guest code calling any combinator kills the process. `Job::run` handles it
   correctly, so this is a bug in the convenience wrapper, not a design gap.
6. **`Atomics.waitAsync` leaks a parked thread with no way to cancel it.**
   `WaitAsyncJob::run` joins the waiter thread (atomics_object.rs:1663); with
   nobody to notify, the job never becomes `is_finished()` and the embedder
   can only drop it, leaking the thread for the process lifetime. *Ask:* a
   cancel/abort entry point on the job, or a documented embedder-side timeout.
7. **`&'static dyn HostHooks`** forces per-worker host state to be leaked and
   manually reclaimed after the agent is dropped. *Ask:* accept an `Rc`/`Arc`,
   or tie the hooks lifetime to the agent.
8. **No timer API surface.** `enqueue_timeout_job(job, ms)` exists, but the
   embedder cannot *create* a job, so `setTimeout` cannot be built on it -
   only on a JS-side promise, which loses the delay. Related to 4.
9. **No web platform** (expected for an engine, listed for completeness):
   `console`, `fetch`, `URL`, `TextEncoder/Decoder`, `Request`/`Response`,
   streams are all absent; the bootstrap here ships a minimal polyfill layer.
10. Engine-documented gaps (lib.rs docs): no sparse arrays, non-compliant
    RegExp (no lookaheads/lookbehinds/backreferences), no Promise subclassing,
    no WebAssembly, "acceptable, not fast" performance.
11. **Lockfile hazard, not an API gap:** `temporal_rs` 0.1.2 uses icu4x
    `unstable` APIs and breaks against icu 2.3, so a plain `cargo update`
    breaks the build. Worth an upstream pin.

## Verdict for the openworkers use case

Handler contracts that need no host I/O - a fetch handler returning a
Response, a task handler returning its result - are fully implementable
today: script eval, host functions, JSON marshaling and embedder-driven
microtask draining all work, and both are implemented here. Async host
operations (guest `await fetch(...)`) are architecturally possible - the job
queue is embedder-controlled, so the drain loop can interleave resolving host
futures with `run_job` - but each pending operation needs a JS-side promise
wired to host functions (`resolve`/`reject` handed out through glue), since
Rust cannot create or settle a `Promise` via public API. That plumbing (plus
`OperationsHandler` integration) is the natural next step.

What keeps this off production is not the missing web platform, which is
embedder work, but items 1-3 above: an untrusted guest can exhaust memory,
spin forever, or abort the whole process by recursing. None has a workaround
at the embedding layer.
