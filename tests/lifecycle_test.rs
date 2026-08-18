mod common;

use std::collections::HashMap;

use openworkers_core::Event;
use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::RequestBody;
use openworkers_core::RuntimeLimits;
use openworkers_core::Script;
use openworkers_core::TerminationReason;
use openworkers_core::Worker as WorkerTrait;
use openworkers_core::WorkerCode;

use openworkers_runtime_nova::Worker;

use common::URL;
use common::body_text;
use common::exception_message;
use common::get;
use common::send;
use common::send_err;
use common::worker;
use common::worker_err;

const OK: &str = "addEventListener('fetch', (event) => event.respondWith(new Response('ok')));";

#[tokio::test]
async fn test_a_worker_serves_many_requests() {
    let script = r#"
        let counter = 0;

        addEventListener('fetch', (event) => {
            counter += 1;
            event.respondWith(new Response(String(counter)));
        });
    "#;

    let mut worker = worker(script).await;

    for expected in 1..=20 {
        let body = body_text(send(&mut worker, get(URL)).await).await;

        assert_eq!(body, expected.to_string());
    }
}

#[tokio::test]
async fn test_globals_set_by_one_request_are_visible_to_the_next() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const before = String(globalThis.leaked);
            globalThis.leaked = 'from the previous request';
            event.respondWith(new Response(before));
        });
    "#;

    let mut worker = worker(script).await;

    assert_eq!(
        body_text(send(&mut worker, get(URL)).await).await,
        "undefined"
    );
    assert_eq!(
        body_text(send(&mut worker, get(URL)).await).await,
        "from the previous request"
    );
}

#[tokio::test]
async fn test_the_worker_serves_again_after_a_handler_exception() {
    let script = r#"
        addEventListener('fetch', (event) => {
            if (event.request.url.endsWith('/boom')) {
                throw new Error('boom');
            }

            event.respondWith(new Response('recovered'));
        });
    "#;

    let mut worker = worker(script).await;

    assert!(
        exception_message(send_err(&mut worker, get("http://localhost/boom")).await)
            .contains("boom")
    );
    assert_eq!(
        body_text(send(&mut worker, get(URL)).await).await,
        "recovered"
    );
}

#[tokio::test]
async fn test_the_worker_serves_again_after_a_rejected_streaming_body() {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);

    let streaming = HttpRequest {
        method: HttpMethod::Post,
        url: URL.to_string(),
        headers: HashMap::new(),
        body: RequestBody::Stream(rx),
    };

    let mut worker = worker(OK).await;

    assert!(matches!(
        send_err(&mut worker, streaming).await,
        TerminationReason::Other(_)
    ));
    assert_eq!(body_text(send(&mut worker, get(URL)).await).await, "ok");
}

#[tokio::test]
async fn test_abort_rejects_later_exec() {
    let mut worker = worker(OK).await;

    assert_eq!(body_text(send(&mut worker, get(URL)).await).await, "ok");

    worker.abort();

    assert_eq!(
        send_err(&mut worker, get(URL)).await,
        TerminationReason::Aborted
    );
    assert_eq!(
        send_err(&mut worker, get(URL)).await,
        TerminationReason::Aborted
    );
}

#[tokio::test]
async fn test_a_dropped_response_receiver_does_not_fail_exec() {
    let mut worker = worker(OK).await;

    let (task, rx) = Event::fetch(get(URL));
    drop(rx);

    worker.exec(task).await.expect("task should execute");
}

#[tokio::test]
async fn test_an_already_taken_fetch_init_is_rejected() {
    let mut worker = worker(OK).await;

    let (mut task, _rx) = Event::fetch(get(URL));

    if let Event::Fetch(init) = &mut task {
        init.take();
    }

    assert!(matches!(
        worker.exec(task).await.expect_err("exec should fail"),
        TerminationReason::Other(_)
    ));
}

#[tokio::test]
async fn test_an_already_taken_task_init_is_rejected() {
    let mut worker = worker(OK).await;

    let (mut task, _rx) = Event::invoke("task-1".to_string(), None, None);

    if let Event::Task(init) = &mut task {
        init.take();
    }

    assert!(matches!(
        worker.exec(task).await.expect_err("exec should fail"),
        TerminationReason::Other(_)
    ));
}

#[tokio::test]
async fn test_the_worker_serves_again_after_a_rejected_task_event() {
    let mut worker = worker(OK).await;

    let (task, _rx) = Event::invoke("task-1".to_string(), None, None);
    worker.exec(task).await.expect_err("exec should fail");

    assert_eq!(body_text(send(&mut worker, get(URL)).await).await, "ok");
}

#[tokio::test]
async fn test_non_javascript_code_is_rejected() {
    let script = Script::new(WorkerCode::snapshot(vec![1, 2, 3]));
    let reason = Worker::new(script, None)
        .await
        .err()
        .expect("worker init should fail");

    assert!(matches!(reason, TerminationReason::InitializationError(_)));
}

#[tokio::test]
async fn test_a_broken_script_fails_initialization() {
    let cases = [
        ("this is not javascript", "SyntaxError"),
        ("function (", "SyntaxError"),
        ("throw new Error('eval boom');", "eval boom"),
        ("undefinedFunction();", "ReferenceError"),
        ("await Promise.resolve(1);", "SyntaxError"),
    ];

    for (script, expected) in cases {
        let message = exception_message(worker_err(script).await);

        assert!(
            message.contains(expected),
            "script {script:?} gave {message}"
        );
    }
}

#[tokio::test]
async fn test_an_empty_script_initializes() {
    worker("").await;
}

#[tokio::test]
async fn test_runtime_limits_are_accepted_but_not_enforced() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response(String('x'.repeat(2000000).length)));
        });
    "#;

    let limits = RuntimeLimits {
        heap_max_mb: 1,
        max_cpu_time_ms: 1,
        max_wall_clock_time_ms: 1,
        ..Default::default()
    };

    let mut worker = Worker::new(Script::new(script), Some(limits))
        .await
        .expect("worker should initialize");

    assert_eq!(
        body_text(send(&mut worker, get(URL)).await).await,
        "2000000"
    );
}

#[tokio::test]
async fn test_many_workers_can_coexist() {
    let mut workers = Vec::new();

    for i in 0..8 {
        let script =
            format!("addEventListener('fetch', (e) => e.respondWith(new Response('w{i}')));");
        workers.push(worker(&script).await);
    }

    for (i, worker) in workers.iter_mut().enumerate() {
        assert_eq!(
            body_text(send(worker, get(URL)).await).await,
            format!("w{i}")
        );
    }
}
