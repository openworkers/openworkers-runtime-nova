//! SSR bench: parse, compile and render a bundled SvelteKit worker.
//!
//! `cargo run --release --example ssr -- <bundle.js> [expected.html]`
//! The bundle must already be a classic script (openworkers-transform).

use std::time::Instant;

use openworkers_core::Event;
use openworkers_core::Script;
use openworkers_core::TerminationReason;
use openworkers_core::Worker as _;

use openworkers_runtime_nova::Worker;

mod common;

use common::get;
use common::report;
use common::rss_kb;

/// Every route of the openworkers-website fixture is prerendered except this
/// one, so it is the only one that reaches SvelteKit's renderer.
const BENCH_URL: &str = "http://localhost/ssr-bench";

const WARM_RUNS: usize = 20;
const COLD_RUNS: usize = 10;
const RESIDENT_WORKERS: usize = 10;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: ssr <bundle.js> [expected.html]");
    let source = std::fs::read_to_string(&path).expect("bundle should be readable");
    let expected = args
        .next()
        .map(|path| std::fs::read_to_string(path).expect("oracle should be readable"));

    println!("bundle: {path} ({} bytes)", source.len());
    println!("baseline RSS: {} kB", rss_kb());

    measure_parse(&source).await;

    let start = Instant::now();
    let worker = Worker::new(Script::new(source.as_str()), None).await;
    println!(
        "\nWorker::new (parse + compile + top-level eval): {:.3} ms",
        elapsed_ms(start)
    );

    match worker {
        Ok(mut worker) => {
            if render_once(&mut worker, expected.as_deref()).await {
                measure_renders(source.as_str(), &mut worker).await;
            }
        }
        Err(reason) => println!("worker init failed: {}", describe(reason)),
    }

    measure_residency(source.as_str()).await;
}

/// Wrapping the bundle in a branch nothing takes leaves parse and compile
/// costs without the top-level evaluation.
async fn measure_parse(source: &str) {
    let parse_only = format!("if (globalThis.__ow_never) {{\n{source}\n}}");

    let mut empty = Vec::new();
    let mut parse = Vec::new();

    for _ in 0..COLD_RUNS {
        let start = Instant::now();
        let worker = Worker::new(Script::new(""), None).await;
        empty.push(elapsed_ms(start));
        drop(worker);

        let start = Instant::now();
        let worker = Worker::new(Script::new(parse_only.as_str()), None).await;
        parse.push(elapsed_ms(start));

        if let Err(reason) = worker {
            println!("parse failed: {}", describe(reason));

            return;
        }
    }

    report("bootstrap only (empty guest script)", &mut empty);
    report("bootstrap + parse/compile bundle", &mut parse);
}

async fn render_once(worker: &mut Worker, expected: Option<&str>) -> bool {
    let start = Instant::now();
    let outcome = render(worker).await;
    let elapsed = elapsed_ms(start);

    let (status, headers, body) = match outcome {
        Ok(response) => response,
        Err(reason) => {
            println!(
                "first render failed after {elapsed:.3} ms: {}",
                describe(reason)
            );

            return false;
        }
    };

    println!(
        "first render: {elapsed:.3} ms, status {status}, {} bytes",
        body.len()
    );

    for (name, value) in headers {
        println!("  {name}: {value}");
    }

    println!("body[0..200]: {}", &body[..body.len().min(200)]);

    if let Some(expected) = expected {
        let verdict = if body == expected {
            "yes".to_string()
        } else {
            format!("no ({} bytes expected)", expected.len())
        };

        println!("matches the reference render: {verdict}");
    }

    true
}

async fn measure_renders(source: &str, worker: &mut Worker) {
    let mut warm = Vec::new();

    for _ in 0..WARM_RUNS {
        let start = Instant::now();
        render(worker).await.expect("warm render should succeed");
        warm.push(elapsed_ms(start));
    }

    report("warm render", &mut warm);

    let mut cold = Vec::new();

    for _ in 0..COLD_RUNS {
        let start = Instant::now();
        let mut worker = Worker::new(Script::new(source), None)
            .await
            .expect("cold worker should initialize");
        render(&mut worker)
            .await
            .expect("cold render should succeed");
        cold.push(elapsed_ms(start));
    }

    report("cold cycle (new worker + 1 render)", &mut cold);
}

/// What a sleeping worker costs: the workers stay alive while RSS is sampled.
async fn measure_residency(source: &str) {
    let before = rss_kb();
    let mut workers = Vec::new();

    for _ in 0..RESIDENT_WORKERS {
        workers.push(
            Worker::new(Script::new(source), None)
                .await
                .expect("worker should initialize"),
        );
    }

    let after = rss_kb();

    println!(
        "\nRSS with {RESIDENT_WORKERS} resident workers: {after} kB (+{} kB, {} kB each)",
        after - before,
        (after - before) / RESIDENT_WORKERS as u64
    );
}

type Rendered = (u16, Vec<(String, String)>, String);

async fn render(worker: &mut Worker) -> Result<Rendered, TerminationReason> {
    let (event, rx) = Event::fetch(get(BENCH_URL));

    worker.exec(event).await?;

    let response = rx.await.expect("response channel should deliver");
    let body = response
        .body
        .collect()
        .await
        .expect("should read body")
        .unwrap_or_default();

    Ok((
        response.status,
        response.headers,
        String::from_utf8_lossy(&body).into_owned(),
    ))
}

fn describe(reason: TerminationReason) -> String {
    match reason {
        TerminationReason::Exception(message) => message,
        TerminationReason::InitializationError(message) => message,
        TerminationReason::Other(message) => message,
        other => format!("{other:?}"),
    }
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
