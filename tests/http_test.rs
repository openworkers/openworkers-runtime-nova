mod common;

use common::js;
use common::js_err;

#[tokio::test]
async fn test_a_string_body_declares_its_content_type() {
    let script = "new Response('hi').headers.get('content-type')";

    assert_eq!(js(script).await, "text/plain;charset=UTF-8");
}

#[tokio::test]
async fn test_an_explicit_content_type_wins() {
    let script = "new Response('<p>', { headers: { 'content-type': 'text/html' } })\
                  .headers.get('content-type')";

    assert_eq!(js(script).await, "text/html");
}

#[tokio::test]
async fn test_bytes_bodies_round_trip() {
    let script = r#"
        (async () => {
            const response = new Response(new TextEncoder().encode('caf\u00e9'));

            return [
                response.headers.get('content-type'),
                await response.text(),
                (await new Response('ab').bytes()).join(','),
                (await new Response('ab').arrayBuffer()).byteLength,
            ].join('|');
        })()
    "#;

    assert_eq!(js(script).await, "|caf\u{e9}|97,98|2");
}

#[tokio::test]
async fn test_a_body_can_only_be_read_once() {
    let script = r#"
        (async () => {
            const response = new Response('once');

            await response.text();

            try {
                await response.text();

                return 'read twice';
            } catch (error) {
                return response.bodyUsed + ':' + error.name;
            }
        })()
    "#;

    assert_eq!(js(script).await, "true:TypeError");
}

#[tokio::test]
async fn test_a_null_body_stays_readable() {
    let script = "(async () => { const r = new Response(); \
                  return (await r.text()) + (await r.text()) + ':' + r.bodyUsed; })()";

    assert_eq!(js(script).await, ":false");
}

#[tokio::test]
async fn test_clone_gives_each_copy_its_own_body() {
    let script = r#"
        (async () => {
            const response = new Response('shared', { status: 201, headers: { 'x-a': '1' } });
            const clone = response.clone();

            return [
                await response.text(),
                await clone.text(),
                clone.status,
                clone.headers.get('x-a'),
            ].join('|');
        })()
    "#;

    assert_eq!(js(script).await, "shared|shared|201|1");
}

#[tokio::test]
async fn test_a_read_body_cannot_be_cloned() {
    let script = "(async () => { const r = new Response('b'); await r.text(); r.clone(); })()";

    assert!(js_err(script).await.contains("TypeError"));
}

#[tokio::test]
async fn test_response_json_sets_the_json_content_type() {
    let script = "(async () => { const r = Response.json({ a: 1 }); \
                  return r.headers.get('content-type') + '|' + (await r.text()); })()";

    assert_eq!(js(script).await, "application/json|{\"a\":1}");
}

#[tokio::test]
async fn test_response_redirect_carries_a_location() {
    let script = "(() => { const r = Response.redirect('http://a.com/x', 301); \
                  return r.status + '|' + r.headers.get('location'); })()";

    assert_eq!(js(script).await, "301|http://a.com/x");
    assert!(
        js_err("Response.redirect('http://a.com/', 200)")
            .await
            .contains("RangeError")
    );
}

#[tokio::test]
async fn test_ok_follows_the_status() {
    let script =
        "[199 + 1, 299, 300, 404].map((s) => new Response(null, { status: s }).ok).join(',')";

    assert_eq!(js(script).await, "true,true,false,false");
}

#[tokio::test]
async fn test_a_stream_body_is_refused_rather_than_stringified() {
    let script = "new Response({ getReader() {} })";

    assert!(js_err(script).await.contains("streaming"));
}

#[tokio::test]
async fn test_a_request_from_another_request_copies_it() {
    let script = r#"
        (async () => {
            const first = new Request('http://a.com/x', {
                method: 'POST',
                body: 'payload',
                headers: { 'x-a': '1' },
            });
            const second = new Request(first);

            return [
                second.url,
                second.method,
                second.headers.get('x-a'),
                await second.text(),
                await first.text(),
            ].join('|');
        })()
    "#;

    assert_eq!(js(script).await, "http://a.com/x|POST|1|payload|payload");
}

#[tokio::test]
async fn test_a_get_request_cannot_carry_a_body() {
    let script = "new Request('http://a.com/', { body: 'x' })";

    assert!(js_err(script).await.contains("TypeError"));
}

#[tokio::test]
async fn test_known_methods_are_uppercased_and_others_kept() {
    let script = "['get', 'post', 'patch', 'query'].map((m) => \
                  new Request('http://a.com/', { method: m }).method).join(',')";

    assert_eq!(js(script).await, "GET,POST,patch,query");
}

#[tokio::test]
async fn test_a_method_that_is_not_a_token_is_rejected() {
    assert!(
        js_err("new Request('http://a.com/', { method: 'a b' })")
            .await
            .contains("TypeError")
    );
}

#[tokio::test]
async fn test_url_search_params_bodies_declare_form_encoding() {
    let script = r#"
        (async () => {
            const request = new Request('http://a.com/', {
                method: 'POST',
                body: new URLSearchParams({ a: '1 2' }),
            });

            return request.headers.get('content-type') + '|' + (await request.text());
        })()
    "#;

    assert_eq!(
        js(script).await,
        "application/x-www-form-urlencoded;charset=UTF-8|a=1+2"
    );
}
