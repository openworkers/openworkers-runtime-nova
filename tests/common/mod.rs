#![allow(dead_code)]

use std::collections::HashMap;

use bytes::Bytes;

use openworkers_core::Event;
use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::HttpResponse;
use openworkers_core::RequestBody;
use openworkers_core::Script;
use openworkers_core::TerminationReason;
use openworkers_core::Worker as WorkerTrait;

use openworkers_runtime_nova::Worker;

pub const URL: &str = "http://localhost/";

pub fn get(url: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    }
}

pub fn post(url: &str, body: impl Into<Bytes>) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url: url.to_string(),
        headers: HashMap::new(),
        body: RequestBody::Bytes(body.into()),
    }
}

pub async fn worker(script: &str) -> Worker {
    Worker::new(Script::new(script), None)
        .await
        .expect("worker should initialize")
}

pub async fn worker_err(script: &str) -> TerminationReason {
    Worker::new(Script::new(script), None)
        .await
        .err()
        .expect("worker init should fail")
}

pub async fn send(worker: &mut Worker, req: HttpRequest) -> HttpResponse {
    let (task, rx) = Event::fetch(req);
    worker.exec(task).await.expect("task should execute");

    rx.await.expect("should receive response")
}

pub async fn send_err(worker: &mut Worker, req: HttpRequest) -> TerminationReason {
    let (task, _rx) = Event::fetch(req);

    worker.exec(task).await.expect_err("task should fail")
}

pub async fn fetch(script: &str, req: HttpRequest) -> HttpResponse {
    send(&mut worker(script).await, req).await
}

pub async fn fetch_err(script: &str, req: HttpRequest) -> TerminationReason {
    send_err(&mut worker(script).await, req).await
}

/// One GET on a fresh worker.
pub async fn serve(script: &str) -> HttpResponse {
    fetch(script, get(URL)).await
}

pub async fn serve_err(script: &str) -> TerminationReason {
    fetch_err(script, get(URL)).await
}

pub async fn serve_body(script: &str) -> String {
    body_text(serve(script).await).await
}

pub async fn body_text(response: HttpResponse) -> String {
    let bytes = response.body.collect().await.unwrap_or_default();

    String::from_utf8(bytes.to_vec()).expect("body should be UTF-8")
}

pub fn exception_message(reason: TerminationReason) -> String {
    match reason {
        TerminationReason::Exception(message) => message,
        other => panic!("expected Exception, got {other:?}"),
    }
}
