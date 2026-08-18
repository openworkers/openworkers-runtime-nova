use std::collections::HashMap;

use openworkers_core::Event;
use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::HttpResponse;
use openworkers_core::RequestBody;
use openworkers_core::Script;
use openworkers_core::TerminationReason;
use openworkers_core::Worker as WorkerTrait;

use openworkers_runtime_nova::Worker;

fn get(url: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    }
}

async fn fetch(script: &str, req: HttpRequest) -> HttpResponse {
    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("worker should initialize");

    let (task, rx) = Event::fetch(req);
    worker.exec(task).await.expect("task should execute");

    rx.await.expect("should receive response")
}

async fn body_text(response: HttpResponse) -> String {
    let bytes = response.body.collect().await.expect("should have a body");

    String::from_utf8(bytes.to_vec()).expect("body should be UTF-8")
}

#[tokio::test]
async fn test_hello_world() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response('Hello, World!', { status: 200 }));
        });
    "#;

    let response = fetch(script, get("http://localhost/")).await;

    assert_eq!(response.status, 200);
    assert_eq!(body_text(response).await, "Hello, World!");
}

#[tokio::test]
async fn test_async_handler() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const body = await Promise.resolve('async body');
            event.respondWith(new Response(body, {
                status: 201,
                headers: { 'x-test': '1' },
            }));
        });
    "#;

    let response = fetch(script, get("http://localhost/")).await;

    assert_eq!(response.status, 201);
    assert!(
        response
            .headers
            .iter()
            .any(|(k, v)| k == "x-test" && v == "1")
    );
    assert_eq!(body_text(response).await, "async body");
}

#[tokio::test]
async fn test_respond_with_promise() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(Promise.resolve(new Response('promised')));
        });
    "#;

    let response = fetch(script, get("http://localhost/")).await;

    assert_eq!(response.status, 200);
    assert_eq!(body_text(response).await, "promised");
}

#[tokio::test]
async fn test_request_marshaling() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const request = event.request;
            const body = await request.text();
            event.respondWith(new Response(JSON.stringify({
                method: request.method,
                url: request.url,
                contentType: request.headers['content-type'],
                body: body,
            })));
        });
    "#;

    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "text/plain".to_string());

    let request = HttpRequest {
        method: HttpMethod::Post,
        url: "http://localhost/echo?q=1".to_string(),
        headers,
        body: RequestBody::Bytes("payload".into()),
    };

    let response = fetch(script, request).await;

    assert_eq!(response.status, 200);

    let json: serde_json::Value =
        serde_json::from_str(&body_text(response).await).expect("body should be JSON");

    assert_eq!(json["method"], "POST");
    assert_eq!(json["url"], "http://localhost/echo?q=1");
    assert_eq!(json["contentType"], "text/plain");
    assert_eq!(json["body"], "payload");
}

#[tokio::test]
async fn test_handler_exception() {
    let script = r#"
        addEventListener('fetch', () => {
            throw new Error('boom');
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("worker should initialize");

    let (task, _rx) = Event::fetch(get("http://localhost/"));
    let error = worker.exec(task).await.expect_err("exec should fail");

    match error {
        TerminationReason::Exception(message) => assert!(message.contains("boom")),
        other => panic!("expected Exception, got {other:?}"),
    }
}

#[tokio::test]
async fn test_no_fetch_handler() {
    let mut worker = Worker::new(Script::new("1 + 1;"), None)
        .await
        .expect("worker should initialize");

    let (task, _rx) = Event::fetch(get("http://localhost/"));
    let error = worker.exec(task).await.expect_err("exec should fail");

    match error {
        TerminationReason::Exception(message) => {
            assert!(message.contains("no fetch handler registered"))
        }
        other => panic!("expected Exception, got {other:?}"),
    }
}

#[tokio::test]
async fn test_invalid_script() {
    let error = Worker::new(Script::new("this is not javascript"), None)
        .await
        .err()
        .expect("worker init should fail");

    assert!(matches!(error, TerminationReason::Exception(_)));
}

#[tokio::test]
async fn test_console_log() {
    let script = r#"
        console.log('starting', { detail: 42 });

        addEventListener('fetch', (event) => {
            console.error('in handler');
            event.respondWith(new Response('ok'));
        });
    "#;

    let response = fetch(script, get("http://localhost/")).await;

    assert_eq!(response.status, 200);
    assert_eq!(body_text(response).await, "ok");
}

#[tokio::test]
async fn test_headers_as_pairs() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response('ok', {
                headers: [['x-a', '1'], ['x-b', '2']],
            }));
        });
    "#;

    let response = fetch(script, get("http://localhost/")).await;

    assert_eq!(
        response.headers,
        vec![
            ("x-a".to_string(), "1".to_string()),
            ("x-b".to_string(), "2".to_string()),
        ]
    );
}

#[tokio::test]
async fn test_pending_response_fails() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Promise(() => {}));
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("worker should initialize");

    let (task, _rx) = Event::fetch(get("http://localhost/"));
    let error = worker.exec(task).await.expect_err("exec should fail");

    match error {
        TerminationReason::Exception(message) => assert!(message.contains("did not settle")),
        other => panic!("expected Exception, got {other:?}"),
    }
}

#[tokio::test]
async fn test_worker_survives_handler_exception() {
    let script = r#"
        addEventListener('fetch', (event) => {
            if (event.request.url.endsWith('/boom')) {
                throw new Error('boom');
            }

            event.respondWith(new Response('recovered'));
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("worker should initialize");

    let (task, _rx) = Event::fetch(get("http://localhost/boom"));
    worker.exec(task).await.expect_err("first exec should fail");

    let (task, rx) = Event::fetch(get("http://localhost/ok"));
    worker.exec(task).await.expect("second exec should succeed");

    let response = rx.await.expect("should receive response");
    assert_eq!(body_text(response).await, "recovered");
}

#[tokio::test]
async fn test_abort_rejects_later_exec() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response('ok'));
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("worker should initialize");

    worker.abort();

    let (task, _rx) = Event::fetch(get("http://localhost/"));
    let error = worker.exec(task).await.expect_err("exec should fail");

    assert_eq!(error, TerminationReason::Aborted);
}

#[tokio::test]
async fn test_task_event_is_rejected() {
    let mut worker = Worker::new(Script::new("1;"), None)
        .await
        .expect("worker should initialize");

    let (task, rx) = Event::invoke("task-1".to_string(), None, None);
    let error = worker.exec(task).await.expect_err("exec should fail");

    assert!(matches!(error, TerminationReason::Other(_)));

    let result = rx.await.expect("should receive task result");
    assert!(!result.success);
    assert!(
        result
            .error
            .expect("should carry an error")
            .contains("task")
    );
}

#[tokio::test]
async fn test_multiple_requests_reuse_worker() {
    let script = r#"
        let counter = 0;

        addEventListener('fetch', (event) => {
            counter += 1;
            event.respondWith(new Response(String(counter)));
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("worker should initialize");

    for expected in ["1", "2", "3"] {
        let (task, rx) = Event::fetch(get("http://localhost/"));
        worker.exec(task).await.expect("task should execute");

        let response = rx.await.expect("should receive response");
        assert_eq!(body_text(response).await, expected);
    }
}
