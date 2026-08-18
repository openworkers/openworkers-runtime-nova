mod common;

use openworkers_core::HttpResponse;

use common::exception_message;
use common::serve;
use common::serve_body;
use common::serve_err;

fn respond_with(expression: &str) -> String {
    format!("addEventListener('fetch', (event) => event.respondWith({expression}));")
}

/// A string body sets content-type on its own; drop it where the test is
/// about the headers the script asked for.
fn declared_headers(response: HttpResponse) -> Vec<(String, String)> {
    response
        .headers
        .into_iter()
        .filter(|(name, _)| name != "content-type")
        .collect()
}

#[tokio::test]
async fn test_status_round_trips() {
    for status in [200, 201, 301, 404, 500, 599] {
        let script = respond_with(&format!("new Response('b', {{ status: {status} }})"));

        assert_eq!(serve(&script).await.status, status);
    }
}

#[tokio::test]
async fn test_status_outside_the_http_range_is_a_range_error() {
    for status in ["-1", "0", "99", "100", "199", "600", "70000"] {
        let script = respond_with(&format!("new Response('b', {{ status: {status} }})"));
        let message = exception_message(serve_err(&script).await);

        assert!(
            message.contains("RangeError") && message.contains(status),
            "unexpected message for status {status}: {message}"
        );
    }
}

#[tokio::test]
async fn test_numeric_string_status_is_accepted() {
    let script = respond_with("new Response('b', { status: '201' })");

    assert_eq!(serve(&script).await.status, 201);
}

#[tokio::test]
async fn test_a_status_that_is_not_an_integer_is_a_range_error() {
    for status in ["200.7", "'abc'", "NaN", "Infinity", "2 ** 32 + 200"] {
        let script = respond_with(&format!("new Response('b', {{ status: {status} }})"));
        let message = exception_message(serve_err(&script).await);

        assert!(
            message.contains("RangeError"),
            "unexpected message for status {status}: {message}"
        );
    }
}

#[tokio::test]
async fn test_a_null_body_status_refuses_a_body() {
    for status in [204, 205, 304] {
        let script = respond_with(&format!("new Response('nope', {{ status: {status} }})"));

        assert!(
            exception_message(serve_err(&script).await).contains("TypeError"),
            "status {status}"
        );
    }

    let script = respond_with("new Response(null, { status: 204 })");
    let response = serve(&script).await;

    assert_eq!(response.status, 204);
    assert_eq!(common::body_text(response).await, "");
}

#[tokio::test]
async fn test_headers_from_a_plain_object() {
    let script = respond_with("new Response('ok', { headers: { 'x-a': '1', 'x-b': '2' } })");

    assert_eq!(
        declared_headers(serve(&script).await),
        vec![
            ("x-a".to_string(), "1".to_string()),
            ("x-b".to_string(), "2".to_string()),
        ]
    );
}

#[tokio::test]
async fn test_headers_from_an_array_of_pairs() {
    let script = respond_with("new Response('ok', { headers: [['x-a', '1'], ['x-b', '2']] })");

    assert_eq!(
        declared_headers(serve(&script).await),
        vec![
            ("x-a".to_string(), "1".to_string()),
            ("x-b".to_string(), "2".to_string()),
        ]
    );
}

#[tokio::test]
async fn test_headers_from_a_map() {
    let script = respond_with("new Response('ok', { headers: new Map([['x-a', '1']]) })");

    assert_eq!(
        declared_headers(serve(&script).await),
        vec![("x-a".to_string(), "1".to_string())]
    );
}

#[tokio::test]
async fn test_duplicate_header_names_are_combined() {
    let script = respond_with("new Response('ok', { headers: [['x-a', '1'], ['x-a', '2']] })");

    assert_eq!(
        declared_headers(serve(&script).await),
        vec![("x-a".to_string(), "1, 2".to_string())]
    );
}

#[tokio::test]
async fn test_header_values_are_stringified() {
    let script =
        respond_with("new Response('ok', { headers: { a: 1, b: null, c: undefined, d: true } })");

    assert_eq!(
        declared_headers(serve(&script).await),
        vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "null".to_string()),
            ("c".to_string(), "undefined".to_string()),
            ("d".to_string(), "true".to_string()),
        ]
    );
}

#[tokio::test]
async fn test_a_header_name_that_is_not_a_token_is_rejected() {
    for name in ["''", "'a b'", "'a:b'"] {
        let script = respond_with(&format!(
            "new Response('ok', {{ headers: {{ {name}: 'v' }} }})"
        ));

        assert!(
            exception_message(serve_err(&script).await).contains("TypeError"),
            "name {name}"
        );
    }
}

#[tokio::test]
async fn test_missing_headers_leave_only_the_content_type() {
    for init in ["{}", "{ headers: null }", "{ headers: undefined }", "null"] {
        let script = respond_with(&format!("new Response('ok', {init})"));

        assert!(
            declared_headers(serve(&script).await).is_empty(),
            "init {init}"
        );
    }
}

#[tokio::test]
async fn test_a_header_pair_shorter_than_two_entries_is_rejected() {
    let script = respond_with("new Response('ok', { headers: [['x-a']] })");

    assert!(exception_message(serve_err(&script).await).contains("TypeError"));
}

#[tokio::test]
async fn test_a_header_entry_that_is_not_indexable_surfaces_as_an_exception() {
    let script = respond_with("new Response('ok', { headers: [null] })");

    assert!(exception_message(serve_err(&script).await).contains("TypeError"));
}

#[tokio::test]
async fn test_symbol_keyed_and_inherited_header_keys_are_skipped() {
    let script = respond_with(
        "new Response('ok', { headers: Object.assign(Object.create({ inherited: '1' }), \
         { own: '2', [Symbol('s')]: '3' }) })",
    );

    assert_eq!(
        declared_headers(serve(&script).await),
        vec![("own".to_string(), "2".to_string())]
    );
}

#[tokio::test]
async fn test_two_hundred_headers_all_arrive() {
    let script = respond_with(
        "new Response('ok', { headers: Array.from({ length: 200 }, (_, i) => ['x-h-' + i, String(i)]) })",
    );

    let headers = declared_headers(serve(&script).await);

    assert_eq!(headers.len(), 200);
    assert!(headers.contains(&("x-h-199".to_string(), "199".to_string())));
}

#[tokio::test]
async fn test_throwing_header_iterator_surfaces_as_an_exception() {
    let script = respond_with(
        "new Response('ok', { headers: { [Symbol.iterator]() { throw new Error('nope'); } } })",
    );

    assert!(exception_message(serve_err(&script).await).contains("nope"));
}

#[tokio::test]
async fn test_missing_body_is_empty() {
    for expression in [
        "new Response()",
        "new Response(undefined)",
        "new Response(null)",
    ] {
        let script = respond_with(expression);

        assert_eq!(serve_body(&script).await, "", "expression {expression}");
    }
}

#[tokio::test]
async fn test_non_string_body_values_are_stringified() {
    let cases = [
        ("42", "42"),
        ("true", "true"),
        ("({ a: 1 })", "[object Object]"),
        ("[1, 2]", "1,2"),
    ];

    for (expression, expected) in cases {
        let script = respond_with(&format!("new Response({expression})"));

        assert_eq!(
            serve_body(&script).await,
            expected,
            "expression {expression}"
        );
    }
}

#[tokio::test]
async fn test_body_unicode_round_trips() {
    let script = respond_with("new Response('caf\\u00e9 \\u{1F600} \\u2028 \\u0000')");

    assert_eq!(serve_body(&script).await, "caf\u{e9} \u{1f600} \u{2028} \0");
}

#[tokio::test]
async fn test_body_lone_surrogate_is_replaced() {
    let script = respond_with("new Response('a\\uD800b')");

    assert_eq!(serve_body(&script).await, "a\u{fffd}b");
}

#[tokio::test]
async fn test_header_lone_surrogate_is_replaced() {
    let script = respond_with("new Response('ok', { headers: { 'x-s': '\\uDC00' } })");

    assert_eq!(
        declared_headers(serve(&script).await),
        vec![("x-s".to_string(), "\u{fffd}".to_string())]
    );
}

#[tokio::test]
async fn test_large_response_body_round_trips() {
    let script = respond_with("new Response('y'.repeat(100000))");

    assert_eq!(serve_body(&script).await.len(), 100_000);
}

#[tokio::test]
async fn test_a_plain_object_shaped_like_a_response_is_accepted() {
    let script = respond_with("({ status: 201, headers: { a: 'b' }, body: 'duck' })");
    let response = serve(&script).await;

    assert_eq!(response.status, 201);
    assert_eq!(response.headers, vec![("a".to_string(), "b".to_string())]);
    assert_eq!(common::body_text(response).await, "duck");
}

/// A hand-rolled response cannot smuggle past Headers' validation.
#[tokio::test]
async fn test_a_response_shaped_object_still_validates_its_headers() {
    let cases = [
        r"{ 'x-a': 'ok\r\nSet-Cookie: injected=1' }",
        "{ 'bad name': 'v' }",
        "{ 'x-a': 'nul\\0byte' }",
    ];

    for headers in cases {
        let script = respond_with(&format!(
            "({{ status: 200, headers: {headers}, body: 'b' }})"
        ));

        assert!(
            exception_message(serve_err(&script).await).contains("TypeError"),
            "headers {headers}"
        );
    }
}

#[tokio::test]
async fn test_the_wire_keeps_the_order_the_headers_were_set() {
    let script = respond_with(
        "new Response(null, { headers: [['x-z', '1'], ['x-a', '2'], ['set-cookie', 'a=1'], \
         ['x-z', '3'], ['set-cookie', 'b=2']] })",
    );

    assert_eq!(
        serve(&script).await.headers,
        vec![
            ("x-z".to_string(), "1, 3".to_string()),
            ("x-a".to_string(), "2".to_string()),
            ("set-cookie".to_string(), "a=1".to_string()),
            ("set-cookie".to_string(), "b=2".to_string()),
        ]
    );
}

#[tokio::test]
async fn test_a_value_that_is_not_a_response_is_rejected() {
    for expression in ["'plain string'", "42", "({})", "Promise.resolve('str')"] {
        let message = exception_message(serve_err(&respond_with(expression)).await);

        assert!(
            message.contains("RangeError"),
            "expression {expression} gave {message}"
        );
    }
}

#[tokio::test]
async fn test_a_response_shaped_object_may_omit_headers_and_body() {
    let script = respond_with("({ status: 204 })");
    let response = serve(&script).await;

    assert_eq!(response.status, 204);
    assert!(response.headers.is_empty());
    assert_eq!(common::body_text(response).await, "");
}

#[tokio::test]
async fn test_a_guest_defined_response_class_is_accepted() {
    let script = r#"
        globalThis.Response = class {
            constructor(body) {
                this.status = 200;
                this.headers = { 'x-c': 'custom' };
                this.body = body;
            }
        };

        addEventListener('fetch', (event) => event.respondWith(new Response('mine')));
    "#;

    let response = serve(script).await;

    assert_eq!(
        response.headers,
        vec![("x-c".to_string(), "custom".to_string())]
    );
    assert_eq!(common::body_text(response).await, "mine");
}

#[tokio::test]
async fn test_a_throwing_accessor_on_the_response_surfaces_as_an_exception() {
    let cases = [
        ("({ get status() { throw new Error('s'); } })", "s"),
        (
            "({ status: 200, get headers() { throw new Error('h'); } })",
            "h",
        ),
        (
            "({ status: 200, get body() { throw new Error('b'); } })",
            "b",
        ),
        (
            "({ status: 200, body: { toString() { throw new Error('t'); } } })",
            "t",
        ),
    ];

    for (expression, expected) in cases {
        let message = exception_message(serve_err(&respond_with(expression)).await);

        assert!(
            message.contains(expected),
            "expression {expression} gave {message}"
        );
    }
}
