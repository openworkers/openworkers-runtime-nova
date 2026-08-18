mod common;

use openworkers_core::Event;
use openworkers_core::TerminationReason;
use openworkers_core::Worker as WorkerTrait;

use serde_json::json;

use common::URL;
use common::exception_message;
use common::get;
use common::invoke;
use common::invoke_err;
use common::run_task;
use common::run_task_err;
use common::send;
use common::task_data;
use common::worker;

#[tokio::test]
async fn test_a_module_task_gets_the_payload_and_its_return_value_becomes_the_data() {
    let script = "globalThis.default = { task: (event) => ({ doubled: event.payload.n * 2 }) };";

    let data = task_data(script, Some(json!({ "n": 21 }))).await;

    assert_eq!(data["doubled"], 42);
}

#[tokio::test]
async fn test_a_task_listener_can_answer_with_respond_with() {
    let script = r#"
        addEventListener('task', (event) => {
            event.respondWith({ success: true, data: { ok: true } });
        });
    "#;

    assert_eq!(task_data(script, None).await["ok"], true);
}

#[tokio::test]
async fn test_respond_with_wins_over_the_returned_value() {
    let script = r#"
        addEventListener('task', (event) => {
            event.respondWith('responded');

            return 'returned';
        });
    "#;

    assert_eq!(task_data(script, None).await, "responded");
}

#[tokio::test]
async fn test_a_result_shaped_return_value_is_passed_through() {
    let script = "globalThis.default = { task: () => ({ success: false, error: 'nope' }) };";

    let result = run_task(script, None).await;

    assert!(!result.success);
    assert_eq!(result.error.expect("should carry an error"), "nope");
}

#[tokio::test]
async fn test_a_thrown_error_becomes_a_failed_result() {
    let script = "addEventListener('task', () => { throw new Error('boom'); });";

    let result = run_task(script, None).await;

    assert!(!result.success);
    assert_eq!(result.error.expect("should carry an error"), "boom");
}

#[tokio::test]
async fn test_a_rejected_task_promise_becomes_a_failed_result() {
    let script = "globalThis.default = { task: async () => { throw new Error('async boom'); } };";

    let result = run_task(script, None).await;

    assert!(!result.success);
    assert_eq!(result.error.expect("should carry an error"), "async boom");
}

#[tokio::test]
async fn test_a_missing_task_handler_fails_the_execution() {
    let script = "addEventListener('fetch', (event) => event.respondWith(new Response('hi')));";

    let message = exception_message(run_task_err(script).await);

    assert!(message.contains("no task handler registered"), "{message}");
}

#[tokio::test]
async fn test_a_task_that_never_settles_is_reported() {
    let script = "globalThis.default = { task: () => new Promise(() => {}) };";

    let message = exception_message(run_task_err(script).await);

    assert!(message.contains("did not settle"), "{message}");
}

#[tokio::test]
async fn test_an_async_task_awaits_its_promise() {
    let script = r#"
        globalThis.default = {
            task: async (event) => {
                const n = await Promise.resolve(event.payload.n);

                return n + 1;
            },
        };
    "#;

    assert_eq!(task_data(script, Some(json!({ "n": 41 }))).await, 42);
}

#[tokio::test]
async fn test_promise_combinators_inside_a_task_survive_the_drain() {
    let script = r#"
        globalThis.default = {
            task: () => Promise.all([1, Promise.resolve(2)]),
        };
    "#;

    assert_eq!(task_data(script, None).await, json!([1, 2]));
}

#[tokio::test]
async fn test_a_returned_undefined_succeeds_without_data() {
    let result = run_task("addEventListener('task', () => {});", None).await;

    assert!(result.success);
    assert!(result.data.is_none());
}

#[tokio::test]
async fn test_every_registered_task_listener_runs_and_the_last_return_wins() {
    let script = r#"
        globalThis.seen = [];

        addEventListener('task', () => { globalThis.seen.push('a'); return 'a'; });
        addEventListener('task', () => { globalThis.seen.push('b'); return globalThis.seen.join(','); });
    "#;

    assert_eq!(task_data(script, None).await, "a,b");
}

#[tokio::test]
async fn test_a_task_listener_takes_precedence_over_the_module_export() {
    let script = r#"
        globalThis.default = { task: () => 'module' };

        addEventListener('task', () => 'listener');
    "#;

    assert_eq!(task_data(script, None).await, "listener");
}

#[tokio::test]
async fn test_the_task_metadata_reaches_the_guest() {
    let script = r#"
        globalThis.default = {
            task: (event) => ({
                id: event.taskId,
                kind: event.source.type,
                origin: event.source.origin,
                attempt: event.attempt,
            }),
        };
    "#;

    let data = task_data(script, None).await;

    assert_eq!(data["id"], "task-1");
    assert_eq!(data["kind"], "invoke");
    assert_eq!(data["origin"], "test");
    assert_eq!(data["attempt"], 1);
}

#[tokio::test]
async fn test_a_scheduled_source_exposes_the_scheduled_time() {
    let script = "globalThis.default = { task: (event) => ({ at: event.scheduledTime }) };";

    let (event, rx) = Event::from_schedule("task-1".to_string(), 1234);

    let mut worker = worker(script).await;
    worker.exec(event).await.expect("task should execute");

    let result = rx.await.expect("should receive a task result");

    assert_eq!(result.data.expect("should carry data")["at"], 1234);
}

#[tokio::test]
async fn test_a_missing_payload_arrives_as_null() {
    let script = "globalThis.default = { task: (event) => ({ null: event.payload === null }) };";

    assert_eq!(task_data(script, None).await["null"], true);
}

#[tokio::test]
async fn test_lone_surrogates_in_the_result_are_replaced() {
    let script = r#"
        globalThis.default = {
            task: () => {
                const data = { text: 'a\uD800b' };

                data['k\uDC00'] = 'v';

                return data;
            },
        };
    "#;

    let data = task_data(script, None).await;

    assert_eq!(data["text"], "a\u{FFFD}b");
    assert_eq!(data["k\u{FFFD}"], "v");
}

#[tokio::test]
async fn test_wait_until_runs_before_the_result_is_delivered() {
    let script = r#"
        globalThis.background = false;

        addEventListener('task', (event) => {
            if (globalThis.background) {
                return 'done';
            }

            event.waitUntil(Promise.resolve().then(() => { globalThis.background = true; }));

            return 'pending';
        });
    "#;

    let mut worker = worker(script).await;

    assert_eq!(
        invoke(&mut worker, None)
            .await
            .data
            .expect("should carry data"),
        "pending"
    );
    assert_eq!(
        invoke(&mut worker, None)
            .await
            .data
            .expect("should carry data"),
        "done"
    );
}

#[tokio::test]
async fn test_a_rejected_wait_until_does_not_sink_the_result() {
    let script = r#"
        addEventListener('task', (event) => {
            event.waitUntil(Promise.reject(new Error('background')));

            return 'kept';
        });
    "#;

    let result = run_task(script, None).await;

    assert!(result.success);
    assert_eq!(result.data.expect("should carry data"), "kept");
}

#[tokio::test]
async fn test_respond_with_one_microtask_after_the_handler_returns_still_counts() {
    let script = r#"
        addEventListener('task', (event) => {
            Promise.resolve().then(() => event.respondWith('late'));

            return 'early';
        });
    "#;

    assert_eq!(task_data(script, None).await, "late");
}

#[tokio::test]
async fn test_respond_with_two_microtasks_after_the_handler_returns_is_too_late() {
    let script = r#"
        addEventListener('task', (event) => {
            Promise.resolve()
                .then(() => {})
                .then(() => event.respondWith('too late'));

            return 'early';
        });
    "#;

    assert_eq!(task_data(script, None).await, "early");
}

#[tokio::test]
async fn test_globals_survive_between_tasks_on_one_worker() {
    let script = r#"
        globalThis.runs = 0;

        addEventListener('task', () => ++globalThis.runs);
    "#;

    let mut worker = worker(script).await;

    assert_eq!(invoke(&mut worker, None).await.data, Some(json!(1)));
    assert_eq!(invoke(&mut worker, None).await.data, Some(json!(2)));
}

#[tokio::test]
async fn test_a_task_and_a_fetch_share_one_worker() {
    let script = r#"
        addEventListener('fetch', (event) => event.respondWith(new Response('served')));
        addEventListener('task', () => 'tasked');
    "#;

    let mut worker = worker(script).await;

    assert_eq!(send(&mut worker, get(URL)).await.status, 200);
    assert_eq!(invoke(&mut worker, None).await.data, Some(json!("tasked")));
    assert_eq!(send(&mut worker, get(URL)).await.status, 200);
}

#[tokio::test]
async fn test_a_failed_task_still_reaches_the_caller_when_the_dispatch_fails() {
    let mut worker = worker("globalThis.default = {};").await;

    let (event, rx) = Event::invoke("task-1".to_string(), None, None);

    worker.exec(event).await.expect_err("task should fail");

    let result = rx.await.expect("should receive a task result");

    assert!(!result.success);
    assert!(
        result
            .error
            .expect("should carry an error")
            .contains("task")
    );
}

#[tokio::test]
async fn test_an_aborted_worker_rejects_further_tasks() {
    let mut worker = worker("addEventListener('task', () => 'ok');").await;

    worker.abort();

    assert!(matches!(
        invoke_err(&mut worker).await,
        TerminationReason::Aborted
    ));
}
