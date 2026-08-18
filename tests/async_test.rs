mod common;

use openworkers_core::TerminationReason;

use common::URL;
use common::body_text;
use common::exception_message;
use common::get;
use common::send;
use common::send_err;
use common::serve_body;
use common::serve_err;
use common::worker;
use common::worker_err;

#[tokio::test]
async fn test_handler_awaiting_many_microtask_turns() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            let n = 0;

            for (let i = 0; i < 100; i++) {
                n = await Promise.resolve(n + 1);
            }

            event.respondWith(new Response(String(n)));
        });
    "#;

    assert_eq!(serve_body(script).await, "100");
}

#[tokio::test]
async fn test_a_two_thousand_link_promise_chain_completes() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            let chain = Promise.resolve(0);

            for (let i = 0; i < 2000; i++) {
                chain = chain.then((value) => value + 1);
            }

            event.respondWith(new Response(String(await chain)));
        });
    "#;

    assert_eq!(serve_body(script).await, "2000");
}

#[tokio::test]
async fn test_a_rejected_chain_recovered_by_catch_still_responds() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                Promise.reject(new Error('early'))
                    .then(() => new Response('unreachable'))
                    .catch((error) => new Response('caught ' + error.message))
            );
        });
    "#;

    assert_eq!(serve_body(script).await, "caught early");
}

#[tokio::test]
async fn test_work_started_by_a_handler_finishes_before_exec_returns() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const before = String(globalThis.background);

            Promise.resolve()
                .then(() => Promise.resolve())
                .then(() => { globalThis.background = 'done'; });

            event.respondWith(new Response(before));
        });
    "#;

    let mut worker = worker(script).await;

    assert_eq!(
        body_text(send(&mut worker, get(URL)).await).await,
        "undefined"
    );
    assert_eq!(body_text(send(&mut worker, get(URL)).await).await, "done");
}

#[tokio::test]
async fn test_promise_all_resolves() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const values = await Promise.all([1, Promise.resolve(2), Promise.resolve(3).then(v => v * 2)]);
            event.respondWith(new Response(values.join('-')));
        });
    "#;

    assert_eq!(serve_body(script).await, "1-2-6");
}

#[tokio::test]
async fn test_promise_race_any_and_all_settled_resolve() {
    let cases = [
        ("Promise.race([Promise.resolve('r')])", "r"),
        (
            "Promise.any([Promise.reject(1), Promise.resolve('a')])",
            "a",
        ),
        (
            "Promise.allSettled([Promise.resolve('s')]).then(r => r[0].status)",
            "fulfilled",
        ),
    ];

    for (expression, expected) in cases {
        let script = format!(
            "addEventListener('fetch', async (event) => {{
                const value = await {expression};
                event.respondWith(new Response(String(value)));
            }});"
        );

        assert_eq!(
            serve_body(&script).await,
            expected,
            "expression {expression}"
        );
    }
}

#[tokio::test]
async fn test_for_await_over_an_async_generator() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            async function* numbers() { yield 1; yield 2; yield 3; }

            let sum = 0;

            for await (const value of numbers()) {
                sum += value;
            }

            event.respondWith(new Response(String(sum)));
        });
    "#;

    assert_eq!(serve_body(script).await, "6");
}

#[tokio::test]
async fn test_respond_with_a_promise_of_a_response() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(Promise.resolve().then(() => new Response('promised')));
        });
    "#;

    assert_eq!(serve_body(script).await, "promised");
}

#[tokio::test]
async fn test_respond_with_a_thenable() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith({ then(resolve) { resolve(new Response('thenable')); } });
        });
    "#;

    assert_eq!(serve_body(script).await, "thenable");
}

#[tokio::test]
async fn test_respond_with_a_rejected_promise_is_an_exception() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(Promise.reject(new Error('rejected')));
        });
    "#;

    assert!(exception_message(serve_err(script).await).contains("rejected"));
}

#[tokio::test]
async fn test_an_unhandled_rejection_does_not_kill_the_worker() {
    let script = r#"
        Promise.reject(new Error('ignored at eval'));

        addEventListener('fetch', (event) => {
            Promise.reject(new Error('ignored in handler'));
            event.respondWith(new Response('ok'));
        });
    "#;

    assert_eq!(serve_body(script).await, "ok");
}

#[tokio::test]
async fn test_a_never_settling_response_promise_fails() {
    let script = "addEventListener('fetch', (event) => event.respondWith(new Promise(() => {})));";

    assert!(exception_message(serve_err(script).await).contains("did not settle"));
}

#[tokio::test]
async fn test_an_endless_microtask_loop_hits_the_job_cap() {
    let script = r#"
        addEventListener('fetch', (event) => {
            function spin() { Promise.resolve().then(spin); }

            spin();
            event.respondWith(new Response('never delivered'));
        });
    "#;

    assert_eq!(
        serve_err(script).await,
        TerminationReason::MaxIterationsReached
    );
}

#[tokio::test]
async fn test_a_self_resolving_thenable_hits_the_job_cap() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const loop = { then(resolve) { resolve(loop); } };
            event.respondWith(Promise.resolve().then(() => loop));
        });
    "#;

    assert_eq!(
        serve_err(script).await,
        TerminationReason::MaxIterationsReached
    );
}

#[tokio::test]
async fn test_an_endless_microtask_loop_at_eval_time_fails_initialization() {
    let script = "function spin() { Promise.resolve().then(spin); } spin();";

    assert_eq!(
        worker_err(script).await,
        TerminationReason::MaxIterationsReached
    );
}

#[tokio::test]
async fn test_a_never_notified_atomics_wait_async_hits_the_job_cap() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const cell = new Int32Array(new SharedArrayBuffer(8));
            Atomics.waitAsync(cell, 0, 0);
            event.respondWith(new Response('never delivered'));
        });
    "#;

    assert_eq!(
        serve_err(script).await,
        TerminationReason::MaxIterationsReached
    );
}

#[tokio::test]
async fn test_the_worker_serves_again_after_the_job_cap_fires() {
    let script = r#"
        addEventListener('fetch', (event) => {
            if (event.request.url.endsWith('/spin')) {
                function spin() { Promise.resolve().then(spin); }
                spin();
            }

            event.respondWith(new Response('ok'));
        });
    "#;

    let mut worker = worker(script).await;

    assert_eq!(
        send_err(&mut worker, get("http://localhost/spin")).await,
        TerminationReason::MaxIterationsReached
    );
    assert_eq!(body_text(send(&mut worker, get(URL)).await).await, "ok");
}
