mod common;

use common::URL;
use common::body_text;
use common::exception_message;
use common::get;
use common::send;
use common::serve_body;
use common::serve_err;
use common::worker;

#[tokio::test]
async fn test_every_registered_handler_runs_and_the_last_response_wins() {
    let script = r#"
        const seen = [];

        addEventListener('fetch', () => { seen.push('a'); });
        addEventListener('fetch', (event) => {
            seen.push('b');
            event.respondWith(new Response(seen.join(',')));
        });
        addEventListener('fetch', (event) => {
            event.respondWith(new Response('last:' + seen.join(',')));
        });
    "#;

    assert_eq!(serve_body(script).await, "last:a,b");
}

#[tokio::test]
async fn test_an_exception_in_an_early_handler_aborts_the_dispatch() {
    let script = r#"
        addEventListener('fetch', () => { throw new Error('first'); });
        addEventListener('fetch', (event) => event.respondWith(new Response('second')));
    "#;

    assert!(exception_message(serve_err(script).await).contains("first"));
}

#[tokio::test]
async fn test_a_handler_registered_during_dispatch_runs_for_the_same_request() {
    let script = r#"
        addEventListener('fetch', () => {
            addEventListener('fetch', (event) => event.respondWith(new Response('nested')));
        });
    "#;

    assert_eq!(serve_body(script).await, "nested");
}

#[tokio::test]
async fn test_a_handler_registered_during_dispatch_stays_registered() {
    let script = r#"
        globalThis.addedRuns = 0;

        addEventListener('fetch', (event) => {
            if (!globalThis.added) {
                globalThis.added = true;
                addEventListener('fetch', () => { globalThis.addedRuns += 1; });
            }

            event.respondWith(new Response(String(globalThis.addedRuns)));
        });
    "#;

    let mut worker = worker(script).await;

    for expected in ["0", "1", "2"] {
        assert_eq!(body_text(send(&mut worker, get(URL)).await).await, expected);
    }
}

#[tokio::test]
async fn test_a_handler_that_never_responds_fails() {
    let script = "addEventListener('fetch', () => {});";

    assert!(exception_message(serve_err(script).await).contains("respondWith"));
}

#[tokio::test]
async fn test_no_registered_handler_fails() {
    for script in ["", "1 + 1;", "addEventListener('scheduled', () => {});"] {
        let message = exception_message(serve_err(script).await);

        assert!(
            message.contains("no fetch handler registered"),
            "script {script:?} gave {message}"
        );
    }
}

#[tokio::test]
async fn test_a_non_callable_handler_fails() {
    let script = "addEventListener('fetch', 'not a function');";

    assert!(exception_message(serve_err(script).await).contains("TypeError"));
}

#[tokio::test]
async fn test_an_event_listener_object_is_not_supported() {
    let script = "addEventListener('fetch', { handleEvent(event) { event.respondWith(new Response('o')); } });";

    assert!(exception_message(serve_err(script).await).contains("TypeError"));
}

#[tokio::test]
async fn test_add_event_listener_without_arguments_is_ignored() {
    let script = r#"
        addEventListener();
        addEventListener('fetch', (event) => event.respondWith(new Response('ok')));
    "#;

    assert_eq!(serve_body(script).await, "ok");
}

#[tokio::test]
async fn test_the_same_handler_registered_twice_runs_twice() {
    let script = r#"
        let runs = 0;

        const handler = (event) => {
            runs += 1;
            event.respondWith(new Response(String(runs)));
        };

        addEventListener('fetch', handler);
        addEventListener('fetch', handler);
    "#;

    assert_eq!(serve_body(script).await, "2");
}

#[tokio::test]
async fn test_a_handler_returning_a_never_settling_promise_fails() {
    let script = "addEventListener('fetch', () => new Promise(() => {}));";

    assert!(exception_message(serve_err(script).await).contains("did not settle"));
}

#[tokio::test]
async fn test_returning_a_response_without_respond_with_fails() {
    let script = "addEventListener('fetch', () => new Response('returned'));";

    assert!(exception_message(serve_err(script).await).contains("respondWith"));
}

#[tokio::test]
async fn test_respond_with_called_twice_keeps_the_last_response() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response('first'));
            event.respondWith(new Response('second'));
        });
    "#;

    assert_eq!(serve_body(script).await, "second");
}

#[tokio::test]
async fn test_respond_with_without_an_argument_fails() {
    let script = "addEventListener('fetch', (event) => { event.respondWith(); });";

    assert!(exception_message(serve_err(script).await).contains("respondWith"));
}

#[tokio::test]
async fn test_throwing_after_respond_with_discards_the_response() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response('ok'));
            throw new Error('after');
        });
    "#;

    assert!(exception_message(serve_err(script).await).contains("after"));
}

#[tokio::test]
async fn test_respond_with_one_microtask_after_the_handler_returns_still_counts() {
    let script = r#"
        addEventListener('fetch', (event) => {
            Promise.resolve().then(() => event.respondWith(new Response('late')));
        });
    "#;

    assert_eq!(serve_body(script).await, "late");
}

#[tokio::test]
async fn test_respond_with_two_microtasks_after_the_handler_returns_is_too_late() {
    let script = r#"
        addEventListener('fetch', (event) => {
            Promise.resolve()
                .then(() => {})
                .then(() => event.respondWith(new Response('too late')));
        });
    "#;

    assert!(exception_message(serve_err(script).await).contains("respondWith"));
}

#[tokio::test]
async fn test_responding_on_a_previous_request_event_has_no_effect() {
    let script = r#"
        let previous = null;

        addEventListener('fetch', (event) => {
            if (previous) {
                previous.respondWith(new Response('stale'));
            }

            previous = event;
            event.respondWith(new Response('fresh'));
        });
    "#;

    let mut worker = worker(script).await;

    assert_eq!(body_text(send(&mut worker, get(URL)).await).await, "fresh");
    assert_eq!(body_text(send(&mut worker, get(URL)).await).await, "fresh");
}

#[tokio::test]
async fn test_remove_event_listener_is_not_provided() {
    let script = "removeEventListener('fetch', () => {});";
    let reason = common::worker_err(script).await;

    assert!(exception_message(reason).contains("removeEventListener"));
}

#[tokio::test]
async fn test_the_event_carries_its_type_and_request() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response(event.type + ' ' + event.request.method));
        });
    "#;

    assert_eq!(serve_body(script).await, "fetch GET");
}

#[tokio::test]
async fn test_a_module_fetch_export_serves_the_request() {
    let script = r#"
        globalThis.default = {
            async fetch(request) {
                return new Response('module ' + new URL(request.url).pathname);
            },
        };
    "#;

    assert_eq!(common::serve_body(script).await, "module /");
}

#[tokio::test]
async fn test_a_fetch_listener_wins_over_the_module_export() {
    let script = r#"
        globalThis.default = { fetch: () => new Response('module') };

        addEventListener('fetch', (event) => event.respondWith(new Response('listener')));
    "#;

    assert_eq!(common::serve_body(script).await, "listener");
}

#[tokio::test]
async fn test_a_module_fetch_that_returns_nothing_fails() {
    let script = "globalThis.default = { fetch() {} };";

    assert!(common::exception_message(common::serve_err(script).await).contains("no response"),);
}

#[tokio::test]
async fn test_wait_until_promises_settle_before_the_response_is_delivered() {
    let script = r#"
        globalThis.default = {
            fetch(request, env, ctx) {
                const done = [];

                ctx.waitUntil(Promise.resolve().then(() => done.push('background')));
                ctx.waitUntil(Promise.reject(new Error('ignored')));

                return Promise.resolve().then(() => new Response('served ' + done.length));
            },
        };
    "#;

    assert_eq!(common::serve_body(script).await, "served 1");
}
