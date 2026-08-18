mod common;

use std::collections::HashMap;

use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::RequestBody;

use common::body_text;
use common::exception_message;
use common::fetch;
use common::serve;
use common::serve_body;
use common::serve_err;

#[tokio::test]
async fn test_hello_world() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response('Hello, World!', { status: 200 }));
        });
    "#;

    let response = serve(script).await;

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

    let response = serve(script).await;

    assert_eq!(response.status, 201);
    assert_eq!(
        response.headers,
        vec![
            (
                "content-type".to_string(),
                "text/plain;charset=UTF-8".to_string()
            ),
            ("x-test".to_string(), "1".to_string()),
        ]
    );
    assert_eq!(body_text(response).await, "async body");
}

#[tokio::test]
async fn test_request_marshaling() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const request = event.request;

            event.respondWith(new Response(JSON.stringify({
                method: request.method,
                url: request.url,
                contentType: request.headers.get('content-type'),
                body: await request.text(),
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
    let script = "addEventListener('fetch', () => { throw new Error('boom'); });";

    assert!(exception_message(serve_err(script).await).contains("boom"));
}

#[tokio::test]
async fn test_console_output_does_not_reach_the_response() {
    let script = r#"
        console.log('starting', { detail: 42 });

        addEventListener('fetch', (event) => {
            console.error('in handler');
            event.respondWith(new Response('ok'));
        });
    "#;

    assert_eq!(serve_body(script).await, "ok");
}
