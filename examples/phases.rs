//! Where a request's time goes on a real bundle: parse and compile, top-level
//! evaluation, and the dispatch itself.

use std::time::Instant;

use openworkers_core::Event;
use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::RequestBody;
use openworkers_core::Script;
use openworkers_core::Worker as _;

use openworkers_runtime_nova::Worker;

use openworkers_transform::CodeLanguage;
use openworkers_transform::parse_worker_code;

fn request(url: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: std::collections::HashMap::new(),
        body: RequestBody::None,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = std::env::args().nth(1).expect("usage: phases <bundle.js>");
    let source = std::fs::read_to_string(&path).expect("bundle should be readable");
    let lowered = Instant::now();
    let code = parse_worker_code(source.as_bytes(), CodeLanguage::JavaScript)
        .expect("bundle should lower to a classic script");

    println!(
        "bundle: {} ({} bytes, lowered to {} in {:.1} ms)",
        path,
        source.len(),
        code.len(),
        lowered.elapsed().as_secs_f64() * 1000.0
    );

    for round in 1..=3 {
        let started = Instant::now();
        let mut worker = Worker::new(Script::new(code.clone()), None)
            .await
            .expect("worker should initialize");
        let built = started.elapsed();

        let dispatch = Instant::now();
        let (task, rx) = Event::fetch(request("http://localhost/"));
        let outcome = worker.exec(task).await;
        let dispatched = dispatch.elapsed();

        let status = match outcome {
            Ok(()) => match rx.await {
                Ok(response) => format!("{}", response.status),
                Err(_) => "no response".to_string(),
            },
            Err(reason) => format!("{reason:?}"),
        };

        println!(
            "round {round}: Worker::new {:.1} ms, exec {:.1} ms, status {status}",
            built.as_secs_f64() * 1000.0,
            dispatched.as_secs_f64() * 1000.0
        );

        // A warm worker takes the same request again, which is what a pool would
        // hand back.
        let warm = Instant::now();
        let (task, rx) = Event::fetch(request("http://localhost/"));
        let _ = worker.exec(task).await;
        let _ = rx.await;

        println!("         exec on the warm worker {:.1} ms", warm.elapsed().as_secs_f64() * 1000.0);
    }
}
