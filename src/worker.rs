use std::any::Any;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use bytes::Bytes;

use serde::Deserialize;

use nova_vm::ecmascript::Agent;
use nova_vm::ecmascript::AgentOptions;
use nova_vm::ecmascript::ArgumentsList;
use nova_vm::ecmascript::Behaviour;
use nova_vm::ecmascript::BuiltinFunctionArgs;
use nova_vm::ecmascript::GcAgent;
use nova_vm::ecmascript::HostHooks;
use nova_vm::ecmascript::InternalMethods;
use nova_vm::ecmascript::Job;
use nova_vm::ecmascript::JsResult;
use nova_vm::ecmascript::Object;
use nova_vm::ecmascript::OrdinaryObject;
use nova_vm::ecmascript::PropertyDescriptor;
use nova_vm::ecmascript::PropertyKey;
use nova_vm::ecmascript::RealmRoot;
use nova_vm::ecmascript::RegularFn;
use nova_vm::ecmascript::String as JsString;
use nova_vm::ecmascript::Value;
use nova_vm::ecmascript::create_builtin_function;
use nova_vm::engine::Bindable;
use nova_vm::engine::GcScope;
use nova_vm::engine::Scopable;

use openworkers_core::Event;
use openworkers_core::HttpRequest;
use openworkers_core::HttpResponse;
use openworkers_core::LogLevel;
use openworkers_core::RequestBody;
use openworkers_core::ResponseBody;
use openworkers_core::RuntimeLimits;
use openworkers_core::Script;
use openworkers_core::TaskInit;
use openworkers_core::TaskResult;
use openworkers_core::TaskSource;
use openworkers_core::TerminationReason;

/// The platform layer, evaluated in order: `bootstrap.js` defines the globals
/// the later scripts build on.
const RUNTIME_JS: &[&str] = &[
    include_str!("bootstrap.js"),
    include_str!("ops.js"),
    include_str!("encoding.js"),
    include_str!("headers.js"),
    include_str!("http.js"),
    include_str!("crypto.js"),
];

/// Ops this runtime answers on the native namespace. A surface module is taken
/// only when every op it reads is here, so the set grows one op at a time.
const PROVIDED_OPS: &[&str] = &[
    "performanceNow",
    "timeOrigin",
    "urlParse",
    "urlUpdate",
    "userAgent",
];

/// What `navigator.userAgent` reports. `Product/Version (comment)` is the HTTP
/// grammar, so a reader splitting on the slash still finds the version.
fn user_agent() -> String {
    format!("OpenWorkers/{} (nova)", env!("CARGO_PKG_VERSION"))
}

/// Keep the surface in its own order: a module patches what the one before it
/// defined.
fn surface() -> impl Iterator<Item = &'static str> {
    openworkers_wintertc::SURFACE
        .iter()
        .filter(|module| {
            module
                .required_ops
                .iter()
                .all(|op| PROVIDED_OPS.contains(op))
        })
        .map(|module| module.source)
}

/// Cap on jobs per drain, our only guard against runaway microtask loops
/// until Nova grows a resource-limit API.
const MAX_JOBS_PER_DRAIN: usize = 10_000;

/// A timer the guest set. The callback stays in JavaScript, where the guest put
/// it; the host keeps only when to ask for it back.
struct Timer {
    handle: f64,
    at: Instant,
    /// Set for an interval, which is rescheduled by this much once it fires.
    period: Option<Duration>,
    /// Insertion order, so two timers due at once fire as they were set.
    seq: u64,
}

/// State the bootstrap glue hands back through the `__ow_native_*` builtins.
struct HostSlots {
    /// The only dispatch `__ow_native_respond` currently answers for.
    dispatch: Cell<i64>,
    outcome: RefCell<Option<String>>,
    /// Where console output goes when the embedder gave one; stderr otherwise.
    ops: Option<openworkers_core::OperationsHandle>,
    /// What `performance.now()` counts from.
    start: Instant,
    timers: RefCell<Vec<Timer>>,
    timer_seq: Cell<u64>,
}

impl Default for HostSlots {
    fn default() -> Self {
        Self {
            dispatch: Cell::default(),
            outcome: RefCell::default(),
            ops: None,
            start: Instant::now(),
            timers: RefCell::default(),
            timer_seq: Cell::default(),
        }
    }
}

/// Nova hands every job to the embedder, so each worker keeps its own queue.
#[derive(Default)]
struct WorkerHostHooks {
    jobs: RefCell<VecDeque<Job>>,
    slots: HostSlots,
}

impl std::fmt::Debug for WorkerHostHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerHostHooks")
            .field("queued_jobs", &self.jobs.borrow().len())
            .field("dispatch", &self.slots.dispatch.get())
            .finish()
    }
}

impl HostHooks for WorkerHostHooks {
    fn enqueue_generic_job(&self, job: Job) {
        self.jobs.borrow_mut().push_back(job);
    }

    fn enqueue_promise_job(&self, job: Job) {
        self.jobs.borrow_mut().push_back(job);
    }

    fn enqueue_timeout_job(&self, job: Job, _milliseconds: u64) {
        // No timer wheel yet: timeout jobs run with the microtasks, without waiting.
        self.jobs.borrow_mut().push_back(job);
    }

    fn get_host_data(&self) -> &dyn Any {
        &self.slots
    }
}

/// What `__ow_native_respond` delivered: the event's outcome, or a dispatch
/// that never got that far.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DispatchOutcome<T> {
    Value { value: T },
    Error { error: String },
}

#[derive(Debug, Deserialize)]
struct DispatchResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

pub struct Worker {
    agent: ManuallyDrop<GcAgent>,
    realm: RealmRoot,
    hooks: NonNull<WorkerHostHooks>,
    dispatches: i64,
    aborted: bool,
    /// How long a drain may wait on timers before the request is over, unless
    /// the embedder disabled the limit.
    wall_clock: Option<Duration>,
}

impl Worker {
    fn hooks(&self) -> &WorkerHostHooks {
        // SAFETY: the pointee was leaked in new() and is freed only in Drop.
        unsafe { self.hooks.as_ref() }
    }

    fn eval(&mut self, source: &str) -> Result<(), String> {
        self.agent.run_in_realm(&self.realm, |agent, mut gc| {
            let source = JsString::from_str(agent, source, gc.nogc());

            match agent.run_script(source.unbind(), gc.reborrow()) {
                Ok(_) => Ok(()),
                Err(e) => {
                    let message = e
                        .unbind()
                        .to_string(agent, gc)
                        .to_string_lossy(agent)
                        .into_owned();

                    Err(message)
                }
            }
        })
    }

    /// Runs jobs, then waits for whatever timer comes next, until neither is
    /// left. A timer callback queues jobs of its own, so the two alternate.
    async fn drain(&mut self) -> Result<(), TerminationReason> {
        let deadline = self.wall_clock.map(|budget| Instant::now() + budget);

        loop {
            self.drain_jobs()?;

            let Some((handle, at)) = self.next_timer() else {
                return Ok(());
            };

            if deadline.is_some_and(|deadline| at > deadline) {
                self.hooks().slots.timers.borrow_mut().clear();

                return Err(TerminationReason::WallClockTimeout);
            }

            if let Some(wait) = at.checked_duration_since(Instant::now()) {
                tokio::time::sleep(wait).await;
            }

            self.fire_timer(handle);
        }
    }

    /// The timer due first, and when. An interval is rescheduled here, so a
    /// callback that clears it during its own run still stops it.
    fn next_timer(&self) -> Option<(f64, Instant)> {
        let mut timers = self.hooks().slots.timers.borrow_mut();

        let (index, _) = timers
            .iter()
            .enumerate()
            .min_by_key(|(_, timer)| (timer.at, timer.seq))?;

        let timer = &mut timers[index];
        let due = (timer.handle, timer.at);

        match timer.period {
            Some(period) => timer.at += period.max(Duration::from_millis(1)),
            None => {
                timers.remove(index);
            }
        }

        Some(due)
    }

    /// A timer callback that throws is like an unhandled rejection: the worker
    /// outlives it.
    fn fire_timer(&mut self, handle: f64) {
        if let Err(message) = self.eval(&format!("__ow_run_timer({handle});")) {
            eprintln!("uncaught error in timer: {message}");
        }
    }

    /// Promise resolution only happens here: nova hands every reaction job to
    /// the host hooks instead of running it itself.
    fn drain_jobs(&mut self) -> Result<(), TerminationReason> {
        for _ in 0..MAX_JOBS_PER_DRAIN {
            let job = self.hooks().jobs.borrow_mut().pop_front();

            let Some(job) = job else {
                return Ok(());
            };

            // Running an unfinished Atomics.waitAsync job blocks on a thread join.
            if !job.is_finished() {
                self.hooks().jobs.borrow_mut().push_back(job);
                continue;
            }

            // Not GcAgent::run_job: it unwraps the job's realm, which nova
            // leaves empty for promise combinators and async iteration.
            let error = self.agent.run_in_realm(&self.realm, |agent, mut gc| {
                match job.run(agent, gc.reborrow()).unbind() {
                    Ok(()) => None,
                    Err(e) => Some(e.to_string(agent, gc).to_string_lossy(agent).into_owned()),
                }
            });

            if let Some(message) = error {
                // Like an unhandled rejection: report, don't kill the worker.
                eprintln!("uncaught error in job: {message}");
            }
        }

        // Finishing on the last job of the budget is still finishing.
        if self.hooks().jobs.borrow().is_empty() {
            return Ok(());
        }

        // Leftover jobs would spend the next request's budget on this one.
        self.hooks().jobs.borrow_mut().clear();

        Err(TerminationReason::MaxIterationsReached)
    }

    async fn dispatch(
        &mut self,
        dispatcher: &str,
        event: &serde_json::Value,
    ) -> Result<(), TerminationReason> {
        // Double encoding turns the JSON text into a JS string literal.
        let literal = serde_json::Value::String(event.to_string()).to_string();

        // A promise an earlier dispatch left pending can settle during this
        // one; the id is what keeps its response from landing here.
        self.dispatches += 1;
        self.hooks().slots.dispatch.set(self.dispatches);
        self.hooks().slots.outcome.borrow_mut().take();

        self.eval(&format!("{dispatcher}({}, {literal});", self.dispatches))
            .map_err(TerminationReason::Exception)?;

        self.drain().await
    }

    fn take_outcome<T: serde::de::DeserializeOwned>(
        &self,
        kind: &str,
    ) -> Result<T, TerminationReason> {
        let payload = self
            .hooks()
            .slots
            .outcome
            .borrow_mut()
            .take()
            .ok_or_else(|| {
                TerminationReason::Exception(format!(
                    "{kind} handler did not settle: async host operations are not supported yet"
                ))
            })?;

        let outcome: DispatchOutcome<T> = serde_json::from_str(&payload)
            .map_err(|e| TerminationReason::Other(format!("invalid dispatch payload: {e}")))?;

        match outcome {
            DispatchOutcome::Value { value } => Ok(value),
            DispatchOutcome::Error { error } => Err(TerminationReason::Exception(error)),
        }
    }

    async fn handle_fetch(&mut self, req: HttpRequest) -> Result<HttpResponse, TerminationReason> {
        let body = match req.body {
            RequestBody::None => None,
            RequestBody::Bytes(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            RequestBody::Stream(_) => {
                return Err(TerminationReason::Other(
                    "Streaming request bodies are not supported".to_string(),
                ));
            }
        };

        let request = serde_json::json!({
            "method": req.method.as_str(),
            "url": req.url,
            "headers": req.headers,
            "body": body,
        });

        self.dispatch("__ow_dispatch", &request).await?;

        let response: DispatchResponse = self.take_outcome("fetch")?;

        Ok(HttpResponse {
            status: response.status,
            headers: response.headers,
            body: ResponseBody::Bytes(Bytes::from(response.body)),
        })
    }

    async fn handle_task(&mut self, init: &TaskInit) -> Result<TaskResult, TerminationReason> {
        let scheduled_time = match &init.source {
            Some(TaskSource::Schedule { time, .. }) => Some(*time),
            _ => None,
        };

        let event = serde_json::json!({
            "taskId": init.task_id,
            "payload": init.payload,
            "source": init.source,
            "attempt": init.attempt,
            "scheduledTime": scheduled_time,
        });

        self.dispatch("__ow_dispatch_task", &event).await?;

        self.take_outcome("task")
    }
}

impl Worker {
    /// The constructor an embedder uses: the handle is what carries console
    /// output back to it. Nova serves no binding through it yet.
    pub async fn new_with_ops(
        script: Script,
        limits: Option<RuntimeLimits>,
        ops: openworkers_core::OperationsHandle,
    ) -> Result<Self, TerminationReason> {
        Self::build(script, limits, Some(ops)).await
    }

    pub async fn exec(&mut self, task: Event) -> Result<(), TerminationReason> {
        <Self as openworkers_core::Worker>::exec(self, task).await
    }

    pub fn abort(&mut self) {
        <Self as openworkers_core::Worker>::abort(self)
    }

    async fn build(
        script: Script,
        limits: Option<RuntimeLimits>,
        ops: Option<openworkers_core::OperationsHandle>,
    ) -> Result<Self, TerminationReason> {
        // Nova 1.0 has no heap or instruction limit API (see NOTES-nova-api.md);
        // the wall clock is the one budget this runtime can hold to, and it
        // bounds how long a drain waits on timers.
        let wall_clock = match limits.unwrap_or_default().max_wall_clock_time_ms {
            0 => None,
            milliseconds => Some(Duration::from_millis(milliseconds)),
        };

        let code = script.code.as_js().ok_or_else(|| {
            TerminationReason::InitializationError(
                "Nova runtime only supports JavaScript code".to_string(),
            )
        })?;

        // GcAgent::new demands &'static hooks; leaked here, freed in Drop.
        let hooks = WorkerHostHooks {
            slots: HostSlots {
                ops,
                ..Default::default()
            },
            ..Default::default()
        };
        let hooks = NonNull::from(Box::leak(Box::new(hooks)));

        // SAFETY: the pointee stays alive until Drop.
        let hooks_ref = unsafe { hooks.as_ref() };

        let options = AgentOptions {
            // Guest JS must not block the runner thread: makes Atomics.wait() throw.
            no_block: true,
            ..Default::default()
        };
        let mut agent = GcAgent::new(options, hooks_ref);

        let create_global_object: Option<for<'a> fn(&mut Agent, GcScope<'a, '_>) -> Object<'a>> =
            None;
        let create_global_this_value: Option<
            for<'a> fn(&mut Agent, GcScope<'a, '_>) -> Object<'a>,
        > = None;
        let realm = agent.create_realm(
            create_global_object,
            create_global_this_value,
            Some(initialize_global_object),
        );

        let mut worker = Self {
            agent: ManuallyDrop::new(agent),
            realm,
            hooks,
            dispatches: 0,
            aborted: false,
            wall_clock,
        };

        for script in RUNTIME_JS.iter().copied().chain(surface()) {
            worker
                .eval(script)
                .map_err(TerminationReason::InitializationError)?;
        }

        worker.eval(code).map_err(TerminationReason::Exception)?;
        worker.drain().await?;

        Ok(worker)
    }
}

impl openworkers_core::Worker for Worker {
    async fn new(script: Script, limits: Option<RuntimeLimits>) -> Result<Self, TerminationReason> {
        Self::build(script, limits, None).await
    }

    async fn exec(&mut self, mut task: Event) -> Result<(), TerminationReason> {
        if self.aborted {
            return Err(TerminationReason::Aborted);
        }

        match &mut task {
            Event::Fetch(init) => {
                let init = init.take().ok_or_else(|| {
                    TerminationReason::Other("FetchInit already taken".to_string())
                })?;
                let response = self.handle_fetch(init.req).await?;
                let _ = init.res_tx.send(response);

                Ok(())
            }
            Event::Task(init) => {
                let init = init.take().ok_or_else(|| {
                    TerminationReason::Other("TaskInit already taken".to_string())
                })?;

                match self.handle_task(&init).await {
                    Ok(result) => {
                        let _ = init.res_tx.send(result);

                        Ok(())
                    }
                    Err(reason) => {
                        let _ = init.res_tx.send(TaskResult::err(reason.description()));

                        Err(reason)
                    }
                }
            }
        }
    }

    fn abort(&mut self) {
        // Nova has no interrupt API: this only rejects future exec() calls,
        // it cannot stop JS that is already running.
        self.aborted = true;
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Queued jobs hold roots into the agent heap; drop them first.
        self.hooks().jobs.borrow_mut().clear();

        // SAFETY: dropped exactly once, and self is unusable afterwards.
        unsafe { ManuallyDrop::drop(&mut self.agent) };

        // SAFETY: leaked in new(); the agent held the only other reference.
        unsafe { drop(Box::from_raw(self.hooks.as_ptr())) };
    }
}

fn initialize_global_object(agent: &mut Agent, global: Object, mut gc: GcScope) {
    define_builtin(
        agent,
        global,
        "__ow_native_respond",
        2,
        native_respond,
        gc.reborrow(),
    );
    define_builtin(
        agent,
        global,
        "__ow_native_log",
        2,
        native_log,
        gc.reborrow(),
    );
    define_builtin(
        agent,
        global,
        "__ow_native_url",
        3,
        crate::url::native_url,
        gc.reborrow(),
    );
    define_builtin(
        agent,
        global,
        "__ow_native_random_hex",
        1,
        crate::crypto::native_random_hex,
        gc.reborrow(),
    );
    define_builtin(
        agent,
        global,
        "__ow_native_digest",
        2,
        crate::crypto::native_digest,
        gc.reborrow(),
    );
    define_builtin(
        agent,
        global,
        "__ow_native_hmac",
        4,
        crate::crypto::native_hmac,
        gc.reborrow(),
    );
    define_builtin(
        agent,
        global,
        "__ow_native_aes_gcm",
        4,
        crate::crypto::native_aes_gcm,
        gc.reborrow(),
    );
    define_builtin(
        agent,
        global,
        "__ow_native_timer_start",
        3,
        native_timer_start,
        gc.reborrow(),
    );
    define_builtin(
        agent,
        global,
        "__ow_native_timer_clear",
        1,
        native_timer_clear,
        gc.reborrow(),
    );

    define_native_namespace(agent, global, gc);
}

/// The one global the shared surface reads its ops out of. A module reads an op
/// at call time, so what is missing here is a module that was never installed,
/// not a call that fails.
fn define_native_namespace(agent: &mut Agent, global: Object, mut gc: GcScope) {
    let ops = OrdinaryObject::create_empty_object(agent, gc.nogc()).unbind();

    define_builtin(
        agent,
        ops.into(),
        "performanceNow",
        0,
        native_performance_now,
        gc.reborrow(),
    );

    // The wall clock when this worker started, which is what `now` counts from.
    let origin = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs_f64() * 1000.0)
        .unwrap_or(0.0);

    let origin = Value::from_f64(agent, origin, gc.nogc()).unbind();

    define_value(agent, ops.into(), "timeOrigin", origin, gc.reborrow());

    let agent_string = JsString::from_string(agent, user_agent(), gc.nogc()).unbind();

    define_value(
        agent,
        ops.into(),
        "userAgent",
        agent_string.into(),
        gc.reborrow(),
    );

    define_value(
        agent,
        global,
        openworkers_wintertc::NATIVE_NAMESPACE,
        ops.into(),
        gc,
    );
}

fn define_builtin(
    agent: &mut Agent,
    target: Object,
    name: &'static str,
    length: u32,
    behaviour: RegularFn,
    mut gc: GcScope,
) {
    let function = create_builtin_function(
        agent,
        Behaviour::Regular(behaviour),
        BuiltinFunctionArgs::new(length, name),
        gc.nogc(),
    )
    .unbind();

    define_value(agent, target, name, function.into(), gc.reborrow());
}

fn define_value(agent: &mut Agent, target: Object, name: &'static str, value: Value, gc: GcScope) {
    let key = PropertyKey::from_static_str(agent, name, gc.nogc());
    let descriptor = PropertyDescriptor {
        value: Some(value.unbind()),
        ..Default::default()
    };

    target
        .internal_define_own_property(agent, key.unbind(), descriptor, gc)
        .expect("defining a property on a fresh object cannot fail");
}

/// Records when the guest wants a timer's callback back. Handles are the guest's
/// to mint, so a clear that names an unknown one is a no-op, as the standard
/// prescribes.
fn native_timer_start<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let handle = args.get(0).to_number(agent, gc.reborrow()).unbind()?;
    let handle = handle.into_f64(agent);
    let delay = args.get(1).to_number(agent, gc.reborrow()).unbind()?;
    let delay = delay.into_f64(agent);
    let repeating = matches!(args.get(2), Value::Boolean(true));

    // A delay that is not a number, or is negative, means now.
    let delay = if delay.is_finite() && delay > 0.0 {
        Duration::from_secs_f64(delay / 1000.0)
    } else {
        Duration::ZERO
    };

    let slots = host_slots(agent);
    let seq = slots.timer_seq.get();

    slots.timer_seq.set(seq + 1);
    slots.timers.borrow_mut().push(Timer {
        handle,
        at: Instant::now() + delay,
        period: repeating.then_some(delay),
        seq,
    });

    Ok(Value::Undefined)
}

fn native_timer_clear<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let handle = args.get(0).to_number(agent, gc.reborrow()).unbind()?;
    let handle = handle.into_f64(agent);

    host_slots(agent)
        .timers
        .borrow_mut()
        .retain(|timer| timer.handle != handle);

    Ok(Value::Undefined)
}

/// Milliseconds since the worker started, as `performance.now()` reports them.
fn native_performance_now<'gc>(
    agent: &mut Agent,
    _this: Value,
    _args: ArgumentsList,
    gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let elapsed = host_slots(agent).start.elapsed().as_secs_f64() * 1000.0;

    Ok(Value::from_f64(agent, elapsed, gc.into_nogc()))
}

fn host_slots(agent: &Agent) -> &HostSlots {
    agent
        .get_host_data()
        .downcast_ref::<HostSlots>()
        .expect("host data is always HostSlots in this runtime")
}

fn native_respond<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    // Guest code can pass objects here, so root arg 1 across arg 0's conversion.
    let payload = args.get(1).scope(agent, gc.nogc());

    let dispatch = args
        .get(0)
        .to_number(agent, gc.reborrow())
        .unbind()?
        .into_i64(agent);

    // A response from a dispatch that already ended belongs to nobody.
    if dispatch != host_slots(agent).dispatch.get() {
        return Ok(Value::Undefined);
    }

    let payload = payload
        .get(agent)
        .to_string(agent, gc)
        .unbind()?
        .to_string_lossy(agent)
        .into_owned();

    *host_slots(agent).outcome.borrow_mut() = Some(payload);

    Ok(Value::Undefined)
}

fn native_log<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    // Guest code can pass objects here, so root arg 1 across arg 0's conversion.
    let message = args.get(1).scope(agent, gc.nogc());

    let level = args.get(0).to_string(agent, gc.reborrow()).unbind()?;
    let level = level.to_string_lossy(agent).into_owned();

    let message = message.get(agent).to_string(agent, gc).unbind()?;
    let message = message.to_string_lossy(agent).into_owned();

    match &host_slots(agent).ops {
        Some(ops) => ops.handle_log(level.parse().unwrap_or(LogLevel::Log), message),
        None => eprintln!("[worker:{level}] {message}"),
    }

    Ok(Value::Undefined)
}
