//! In-process timings: worker construction vs one task execution.

use std::time::Instant;

use openworkers_core::Event;
use openworkers_core::Script;
use openworkers_core::Worker as _;

use openworkers_runtime_nova::Worker;

const SCRIPT: &str =
    "globalThis.default = { task: (event) => ({ doubled: event.payload.n * 2 }) };";

const RUNS: u32 = 200;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut init = Vec::new();
    let mut exec = Vec::new();

    for _ in 0..RUNS {
        let start = Instant::now();
        let mut worker = Worker::new(Script::new(SCRIPT), None).await.unwrap();
        init.push(start.elapsed().as_secs_f64() * 1000.0);

        let payload = serde_json::json!({ "n": 21 });
        let (event, rx) = Event::invoke("bench".to_string(), Some(payload), None);

        let start = Instant::now();
        worker.exec(event).await.unwrap();
        let result = rx.await.unwrap();
        exec.push(start.elapsed().as_secs_f64() * 1000.0);

        assert_eq!(result.data.unwrap()["doubled"], 42);
    }

    report("Worker::new (bootstrap + script eval)", &mut init);
    report("exec(Event::Task) on a warm worker", &mut exec);
}

fn report(label: &str, samples: &mut [f64]) {
    samples.sort_by(f64::total_cmp);

    let mean = samples.iter().sum::<f64>() / samples.len() as f64;

    println!(
        "{label:<38} min {:7.3} ms   median {:7.3} ms   mean {mean:7.3} ms   (n={})",
        samples[0],
        samples[samples.len() / 2],
        samples.len()
    );
}
