use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;

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
    include_str!("encoding.js"),
    include_str!("url.js"),
    include_str!("headers.js"),
    include_str!("http.js"),
    include_str!("crypto.js"),
];

/// Cap on jobs per drain, our only guard against runaway microtask loops
/// until Nova grows a resource-limit API.
const MAX_JOBS_PER_DRAIN: usize = 10_000;

/// State the bootstrap glue hands back through the `__ow_native_*` builtins.
#[derive(Debug, Default)]
struct HostSlots {
    outcome: RefCell<Option<String>>,
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
            .field("slots", &self.slots)
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
    aborted: bool,
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

        // Leftover jobs would spend the next request's budget on this one.
        self.hooks().jobs.borrow_mut().clear();

        Err(TerminationReason::MaxIterationsReached)
    }

    fn dispatch(
        &mut self,
        dispatcher: &str,
        event: &serde_json::Value,
    ) -> Result<(), TerminationReason> {
        // Double encoding turns the JSON text into a JS string literal.
        let literal = serde_json::Value::String(event.to_string()).to_string();

        // Drop any stale payload from a previous dispatch.
        self.hooks().slots.outcome.borrow_mut().take();

        self.eval(&format!("{dispatcher}({literal});"))
            .map_err(TerminationReason::Exception)?;

        self.drain_jobs()
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

    fn handle_fetch(&mut self, req: HttpRequest) -> Result<HttpResponse, TerminationReason> {
        let body = match req.body {
            RequestBody::None => None,
            RequestBody::Bytes(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            RequestBody::Stream(_) => {
                return Err(TerminationReason::Other(
                    "streaming request bodies are not supported yet".to_string(),
                ));
            }
        };

        let request = serde_json::json!({
            "method": req.method.as_str(),
            "url": req.url,
            "headers": req.headers,
            "body": body,
        });

        self.dispatch("__ow_dispatch", &request)?;

        let response: DispatchResponse = self.take_outcome("fetch")?;

        Ok(HttpResponse {
            status: response.status,
            headers: response.headers,
            body: ResponseBody::Bytes(Bytes::from(response.body)),
        })
    }

    fn handle_task(&mut self, init: &TaskInit) -> Result<TaskResult, TerminationReason> {
        let scheduled_time = match &init.source {
            Some(TaskSource::Schedule { time }) => Some(*time),
            _ => None,
        };

        let event = serde_json::json!({
            "taskId": init.task_id,
            "payload": init.payload,
            "source": init.source,
            "attempt": init.attempt,
            "scheduledTime": scheduled_time,
        });

        self.dispatch("__ow_dispatch_task", &event)?;

        self.take_outcome("task")
    }
}

impl openworkers_core::Worker for Worker {
    async fn new(script: Script, limits: Option<RuntimeLimits>) -> Result<Self, TerminationReason> {
        // Nova 1.0 has no heap or time limit API (see NOTES-nova-api.md).
        let _ = limits;

        let code = script.code.as_js().ok_or_else(|| {
            TerminationReason::InitializationError(
                "Nova runtime only supports JavaScript code".to_string(),
            )
        })?;

        // GcAgent::new demands &'static hooks; leaked here, freed in Drop.
        let hooks = NonNull::from(Box::leak(Box::new(WorkerHostHooks::default())));

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
            aborted: false,
        };

        for script in RUNTIME_JS {
            worker
                .eval(script)
                .map_err(TerminationReason::InitializationError)?;
        }

        worker.eval(code).map_err(TerminationReason::Exception)?;
        worker.drain_jobs()?;

        Ok(worker)
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
                let response = self.handle_fetch(init.req)?;
                let _ = init.res_tx.send(response);

                Ok(())
            }
            Event::Task(init) => {
                let init = init.take().ok_or_else(|| {
                    TerminationReason::Other("TaskInit already taken".to_string())
                })?;

                match self.handle_task(&init) {
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
        1,
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
        gc,
    );
}

fn define_builtin(
    agent: &mut Agent,
    global: Object,
    name: &'static str,
    length: u32,
    behaviour: RegularFn,
    gc: GcScope,
) {
    let function = create_builtin_function(
        agent,
        Behaviour::Regular(behaviour),
        BuiltinFunctionArgs::new(length, name),
        gc.nogc(),
    );
    let key = PropertyKey::from_static_str(agent, name, gc.nogc());
    let descriptor = PropertyDescriptor {
        value: Some(function.unbind().into()),
        ..Default::default()
    };

    global
        .internal_define_own_property(agent, key.unbind(), descriptor, gc)
        .expect("defining a builtin on a fresh global object cannot fail");
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
    gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let payload = args
        .get(0)
        .to_string(agent, gc)?
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

    eprintln!("[worker:{level}] {}", message.to_string_lossy(agent));

    Ok(Value::Undefined)
}
