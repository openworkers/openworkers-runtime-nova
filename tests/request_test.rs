mod common;

use std::collections::HashMap;

use bytes::Bytes;

use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::RequestBody;
use openworkers_core::TerminationReason;

use common::URL;
use common::body_text;
use common::fetch;
use common::get;
use common::post;
use common::worker;

const ECHO_URL: &str = r#"
    addEventListener('fetch', (event) => {
        event.respondWith(new Response(event.request.url));
    });
"#;

const ECHO_REQUEST: &str = r#"
    addEventListener('fetch', async (event) => {
        const request = event.request;

        event.respondWith(new Response(JSON.stringify({
            method: request.method,
            url: request.url,
            headers: Object.fromEntries(request.headers),
            bodyUsed: request.bodyUsed,
            text: await request.text(),
        })));
    });
"#;

async fn echo(req: HttpRequest) -> serde_json::Value {
    let body = body_text(fetch(ECHO_REQUEST, req).await).await;

    serde_json::from_str(&body).expect("echo body should be JSON")
}

#[tokio::test]
async fn test_every_http_method_reaches_the_handler() {
    let methods = [
        HttpMethod::Get,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Delete,
        HttpMethod::Patch,
        HttpMethod::Head,
        HttpMethod::Options,
    ];

    let mut worker = worker(ECHO_REQUEST).await;

    for method in methods {
        let request = HttpRequest {
            method: method.clone(),
            url: URL.to_string(),
            headers: HashMap::new(),
            body: RequestBody::None,
        };

        let response = common::send(&mut worker, request).await;
        let json: serde_json::Value =
            serde_json::from_str(&body_text(response).await).expect("echo body should be JSON");

        assert_eq!(json["method"], method.as_str());
    }
}

#[tokio::test]
async fn test_the_url_reaches_the_guest_in_its_parsed_form() {
    let urls = [
        ("http://localhost/plain", "http://localhost/plain"),
        ("http://localhost/a\"b\\c", "http://localhost/a%22b/c"),
        (
            "http://localhost/</script>",
            "http://localhost/%3C/script%3E",
        ),
        (
            "http://localhost/caf\u{e9}/\u{1f600}",
            "http://localhost/caf%C3%A9/%F0%9F%98%80",
        ),
        (
            "http://localhost/%E2%82%AC?q=a%20b&q=c#frag",
            "http://localhost/%E2%82%AC?q=a%20b&q=c#frag",
        ),
    ];

    let mut worker = worker(ECHO_URL).await;

    for (url, expected) in urls {
        let response = common::send(&mut worker, get(url)).await;

        assert_eq!(body_text(response).await, expected);
    }
}

#[tokio::test]
async fn test_a_url_the_host_cannot_parse_fails_the_request() {
    let mut worker = worker(ECHO_URL).await;

    for url in ["", "not a url at all", "/relative"] {
        let reason = common::send_err(&mut worker, get(url)).await;

        assert!(
            common::exception_message(reason).contains("Invalid URL"),
            "for {url}"
        );
    }
}

#[tokio::test]
async fn test_query_string_and_fragment_are_preserved() {
    let json = echo(get("http://localhost/search?q=1&q=2&empty=&flag#top")).await;

    assert_eq!(
        json["url"],
        "http://localhost/search?q=1&q=2&empty=&flag#top"
    );
}

#[tokio::test]
async fn test_header_names_are_lowercased_and_values_combined() {
    let mut headers = HashMap::new();
    headers.insert("X-Mixed-Case".to_string(), "Value".to_string());
    headers.insert("x-mixed-case".to_string(), "other".to_string());

    let json = echo(HttpRequest {
        method: HttpMethod::Get,
        url: URL.to_string(),
        headers,
        body: RequestBody::None,
    })
    .await;

    let combined = json["headers"]["x-mixed-case"]
        .as_str()
        .expect("string")
        .to_string();

    assert!(
        combined.contains("Value") && combined.contains("other"),
        "{combined}"
    );
    assert!(json["headers"].get("X-Mixed-Case").is_none());
}

#[tokio::test]
async fn test_unusual_but_legal_headers_survive() {
    let mut headers = HashMap::new();
    headers.insert("empty".to_string(), String::new());
    headers.insert("x-unicode".to_string(), "caf\u{e9}".to_string());

    let json = echo(HttpRequest {
        method: HttpMethod::Get,
        url: URL.to_string(),
        headers,
        body: RequestBody::None,
    })
    .await;

    assert_eq!(json["headers"]["empty"], "");
    assert_eq!(json["headers"]["x-unicode"], "caf\u{e9}");
}

#[tokio::test]
async fn test_a_header_http_forbids_fails_the_request() {
    for (name, value) in [("quote\"name", "v"), ("x-newline", "a\nb")] {
        let mut headers = HashMap::new();
        headers.insert(name.to_string(), value.to_string());

        let reason = common::fetch_err(
            ECHO_REQUEST,
            HttpRequest {
                method: HttpMethod::Get,
                url: URL.to_string(),
                headers,
                body: RequestBody::None,
            },
        )
        .await;

        assert!(
            common::exception_message(reason).contains("TypeError"),
            "for header {name}"
        );
    }
}

#[tokio::test]
async fn test_a_large_header_value_round_trips() {
    let mut headers = HashMap::new();
    headers.insert("x-big".to_string(), "v".repeat(50_000));

    let json = echo(HttpRequest {
        method: HttpMethod::Get,
        url: URL.to_string(),
        headers,
        body: RequestBody::None,
    })
    .await;

    assert_eq!(
        json["headers"]["x-big"].as_str().expect("string").len(),
        50_000
    );
}

#[tokio::test]
async fn test_the_request_headers_are_rebuilt_for_each_request() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const seen = String(event.request.headers.get('x-added'));
            event.request.headers.set('x-added', 'yes');
            event.respondWith(new Response(seen));
        });
    "#;

    let mut worker = worker(script).await;

    for _ in 0..2 {
        let response = common::send(&mut worker, get(URL)).await;

        assert_eq!(body_text(response).await, "null");
    }
}

#[tokio::test]
async fn test_fifty_headers_all_arrive() {
    let mut headers = HashMap::new();

    for i in 0..50 {
        headers.insert(format!("x-h-{i}"), format!("v{i}"));
    }

    let json = echo(HttpRequest {
        method: HttpMethod::Get,
        url: URL.to_string(),
        headers,
        body: RequestBody::None,
    })
    .await;

    assert_eq!(json["headers"].as_object().expect("object").len(), 50);
    assert_eq!(json["headers"]["x-h-49"], "v49");
}

#[tokio::test]
async fn test_missing_request_body_is_null() {
    let json = echo(get(URL)).await;

    assert_eq!(json["bodyUsed"], false);
    assert_eq!(json["text"], "");
}

#[tokio::test]
async fn test_empty_request_body_is_an_empty_string() {
    let json = echo(post(URL, Bytes::new())).await;

    assert_eq!(json["bodyUsed"], false);
    assert_eq!(json["text"], "");
}

#[tokio::test]
async fn test_request_body_non_utf8_is_lossy() {
    let json = echo(post(URL, vec![b'h', b'i', 0xff, 0xfe, b'!'])).await;

    assert_eq!(json["text"], "hi\u{fffd}\u{fffd}!");
}

#[tokio::test]
async fn test_request_body_unicode_round_trips() {
    let json = echo(post(URL, "caf\u{e9} \u{1f600} \u{2028}")).await;

    assert_eq!(json["text"], "caf\u{e9} \u{1f600} \u{2028}");
}

#[tokio::test]
async fn test_request_body_may_contain_a_nul_byte() {
    let json = echo(post(URL, vec![b'a', 0, b'b'])).await;

    assert_eq!(json["text"], "a\0b");
}

#[tokio::test]
async fn test_large_request_body_round_trips() {
    let json = echo(post(URL, "x".repeat(100_000))).await;

    assert_eq!(json["text"].as_str().expect("string").len(), 100_000);
}

#[tokio::test]
async fn test_request_body_json_is_parseable_by_the_guest() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const payload = await event.request.json();
            event.respondWith(new Response(String(payload.a + payload.b)));
        });
    "#;

    let response = fetch(script, post(URL, r#"{"a":1,"b":2}"#)).await;

    assert_eq!(body_text(response).await, "3");
}

#[tokio::test]
async fn test_a_body_that_is_not_json_rejects_the_json_call() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            try {
                await event.request.json();
                event.respondWith(new Response('parsed'));
            } catch (error) {
                event.respondWith(new Response(String(error)));
            }
        });
    "#;

    let response = fetch(script, post(URL, "not json at all")).await;

    assert!(body_text(response).await.contains("SyntaxError"));
}

#[tokio::test]
async fn test_a_head_request_keeps_the_body_it_was_given() {
    let json = echo(HttpRequest {
        method: HttpMethod::Head,
        url: URL.to_string(),
        headers: HashMap::new(),
        body: RequestBody::Bytes("head body".into()),
    })
    .await;

    assert_eq!(json["method"], "HEAD");
    assert_eq!(json["text"], "head body");
}

#[tokio::test]
async fn test_the_request_constructor_is_available_to_the_guest() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const request = new Request('http://elsewhere/', { method: 'post', body: 'b' });

            event.respondWith(new Response([
                request.url,
                request.method,
                await request.text(),
                request.headers.get('content-type'),
                new Request('http://elsewhere/path').method,
            ].join('|')));
        });
    "#;

    let response = fetch(script, get(URL)).await;

    assert_eq!(
        body_text(response).await,
        "http://elsewhere/|POST|b|text/plain;charset=UTF-8|GET"
    );
}

#[tokio::test]
async fn test_the_request_constructor_rejects_a_relative_url() {
    let script = r#"
        addEventListener('fetch', (event) => {
            try {
                new Request('u');
                event.respondWith(new Response('accepted'));
            } catch (error) {
                event.respondWith(new Response(String(error)));
            }
        });
    "#;

    assert!(common::serve_body(script).await.contains("TypeError"));
}

#[tokio::test]
async fn test_streaming_request_body_is_rejected() {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);

    let request = HttpRequest {
        method: HttpMethod::Post,
        url: URL.to_string(),
        headers: HashMap::new(),
        body: RequestBody::Stream(rx),
    };

    let reason = common::fetch_err(ECHO_REQUEST, request).await;

    match reason {
        TerminationReason::Other(message) => {
            assert!(
                message.contains("Streaming request bodies"),
                "unexpected message: {message}"
            )
        }
        other => panic!("expected Other, got {other:?}"),
    }
}
