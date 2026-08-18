#![allow(dead_code)]

use std::collections::HashMap;

use bytes::Bytes;

use openworkers_core::Event;
use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::HttpResponse;
use openworkers_core::RequestBody;
use openworkers_core::Script;
use openworkers_core::TaskResult;
use openworkers_core::TerminationReason;
use openworkers_core::Worker as WorkerTrait;

use openworkers_runtime_nova::Worker;

use serde_json::Value as JsonValue;

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

/// The string value of one expression, evaluated inside a fetch handler.
pub async fn js(expression: &str) -> String {
    serve_body(&js_script(expression)).await
}

/// The exception message from an expression expected to throw.
pub async fn js_err(expression: &str) -> String {
    exception_message(serve_err(&js_script(expression)).await)
}

fn js_script(expression: &str) -> String {
    format!(
        "addEventListener('fetch', (event) => \
         event.respondWith(new Response(String({expression}))));"
    )
}

pub async fn body_text(response: HttpResponse) -> String {
    let bytes = response.body.collect().await.unwrap_or_default();

    String::from_utf8(bytes.to_vec()).expect("body should be UTF-8")
}

pub async fn invoke(worker: &mut Worker, payload: Option<JsonValue>) -> TaskResult {
    let (event, rx) = Event::invoke("task-1".to_string(), payload, Some("test".to_string()));

    worker.exec(event).await.expect("task should execute");

    rx.await.expect("should receive a task result")
}

pub async fn invoke_err(worker: &mut Worker) -> TerminationReason {
    let (event, _rx) = Event::invoke("task-1".to_string(), None, None);

    worker.exec(event).await.expect_err("task should fail")
}

/// One invocation on a fresh worker.
pub async fn run_task(script: &str, payload: Option<JsonValue>) -> TaskResult {
    invoke(&mut worker(script).await, payload).await
}

pub async fn run_task_err(script: &str) -> TerminationReason {
    invoke_err(&mut worker(script).await).await
}

/// The data of a task expected to succeed.
pub async fn task_data(script: &str, payload: Option<JsonValue>) -> JsonValue {
    let result = run_task(script, payload).await;

    assert!(result.success, "task failed: {:?}", result.error);

    result.data.expect("task should carry data")
}

pub fn exception_message(reason: TerminationReason) -> String {
    match reason {
        TerminationReason::Exception(message) => message,
        other => panic!("expected Exception, got {other:?}"),
    }
}
