mod common;

use std::time::Duration;
use std::time::Instant;

use openworkers_core::Event;
use openworkers_core::RuntimeLimits;
use openworkers_core::Script;
use openworkers_core::TerminationReason;
use openworkers_core::Worker as WorkerTrait;

use openworkers_runtime_nova::Worker;

use common::URL;
use common::get;

/// An interval nobody clears would run for as long as the worker lives, so the
/// wall clock is what ends it.
#[tokio::test]
async fn test_an_interval_that_never_clears_ends_on_the_wall_clock() {
    let script = r#"
        addEventListener('fetch', (event) => {
            setInterval(() => {}, 1);
            event.respondWith(new Response('ok'));
        });
    "#;

    let limits = RuntimeLimits {
        max_wall_clock_time_ms: 200,
        ..Default::default()
    };

    let mut worker = Worker::new(Script::new(script), Some(limits))
        .await
        .expect("worker should initialize");

    let (task, _rx) = Event::fetch(get(URL));
    let started = Instant::now();
    let outcome = worker.exec(task).await;

    assert!(
        matches!(outcome, Err(TerminationReason::WallClockTimeout)),
        "{outcome:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the wall clock should have ended it, not the test runner"
    );
}

/// A timer that is due only after the handler returned still runs, because the
/// drain does not end until the timers do.
#[tokio::test]
async fn test_a_timer_runs_after_the_handler_returned() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Promise((resolve) => {
                setTimeout(() => resolve(new Response('late')), 20);
            }));
        });
    "#;

    assert_eq!(common::serve_body(script).await, "late");
}
