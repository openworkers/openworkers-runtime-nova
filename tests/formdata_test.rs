mod common;

use std::collections::HashMap;

use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::RequestBody;

use common::URL;
use common::body_text;
use common::fetch;
use common::js;

/// Reports the parsed entries of the request body, or the failure.
const READ_FORM: &str = r#"
    addEventListener('fetch', (event) => {
        event.respondWith(event.request.formData().then(
            (form) => new Response(JSON.stringify([...form])),
            (error) => new Response(String(error)),
        ));
    });
"#;

fn post(body: &str, content_type: &str) -> HttpRequest {
    let mut headers = HashMap::new();

    if !content_type.is_empty() {
        headers.insert("content-type".to_string(), content_type.to_string());
    }

    HttpRequest {
        method: HttpMethod::Post,
        url: URL.to_string(),
        headers,
        body: RequestBody::Bytes(body.to_string().into()),
    }
}

async fn read_form(body: &str, content_type: &str) -> String {
    body_text(fetch(READ_FORM, post(body, content_type)).await).await
}

#[tokio::test]
async fn test_urlencoded_body_parses_into_form_data() {
    let body = read_form(
        "a=1&b=hello+world&a=2&c=%C3%A9",
        "application/x-www-form-urlencoded",
    )
    .await;

    assert_eq!(
        body,
        "[[\"a\",\"1\"],[\"b\",\"hello world\"],[\"a\",\"2\"],[\"c\",\"\u{e9}\"]]"
    );
}

#[tokio::test]
async fn test_multipart_body_parses_into_form_data() {
    let body = read_form(
        "--X\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n1\r\n\
         --X\r\nContent-Disposition: form-data; name=\"b\"\r\n\r\nline\r\nbreak\r\n\
         --X--\r\n",
        "multipart/form-data; boundary=X",
    )
    .await;

    assert_eq!(body, r#"[["a","1"],["b","line\r\nbreak"]]"#);
}

/// Reports what an upload arrived as.
const READ_FILE: &str = r#"
    addEventListener('fetch', (event) => {
        event.respondWith(event.request.formData().then(async (form) => {
            const file = form.get('f');

            return new Response(JSON.stringify([
                file instanceof File,
                file.name,
                file.type,
                await file.text(),
            ]));
        }));
    });
"#;

#[tokio::test]
async fn test_a_multipart_file_part_arrives_as_a_file() {
    let request = post(
        "--X\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a.txt\"\r\n\
         Content-Type: text/plain\r\n\r\nhello\r\n--X--\r\n",
        "multipart/form-data; boundary=\"X\"",
    );

    let body = body_text(fetch(READ_FILE, request).await).await;

    assert_eq!(body, r#"[true,"a.txt","text/plain","hello"]"#);
}

#[tokio::test]
async fn test_a_body_of_another_type_is_refused() {
    let body = read_form("{}", "application/json").await;

    assert_eq!(body, "TypeError: Invalid content-type for formData()");
}

#[tokio::test]
async fn test_a_multipart_body_without_a_boundary_is_refused() {
    let body = read_form("--X--", "multipart/form-data").await;

    assert_eq!(body, "TypeError: Missing boundary in multipart/form-data");
}

#[tokio::test]
async fn test_form_data_keeps_insertion_order_and_repeats() {
    let entries = js(
        "(() => { const f = new FormData(); f.append('a', 1); f.append('b', 2); \
         f.append('a', 3); return JSON.stringify([...f]); })()",
    )
    .await;

    assert_eq!(entries, r#"[["a","1"],["b","2"],["a","3"]]"#);
}

#[tokio::test]
async fn test_form_data_get_and_delete_and_set() {
    let script = "(() => { const f = new FormData(); f.append('a', 1); f.append('a', 2); \
         f.append('b', 3); const before = [f.get('a'), f.getAll('a').join(','), f.has('c')]; \
         f.set('a', 9); f.delete('b'); \
         return JSON.stringify([before, [...f], f.get('c')]); })()";

    assert_eq!(js(script).await, r#"[["1","1,2",false],[["a","9"]],null]"#);
}

#[tokio::test]
async fn test_a_form_data_body_round_trips_through_a_request() {
    let script = "(async () => { const f = new FormData(); f.append('a', 'x y'); \
         f.append('b', 'z'); \
         const request = new Request('http://localhost/', { method: 'POST', body: f }); \
         const type = request.headers.get('content-type'); \
         const back = await request.formData(); \
         return type.startsWith('multipart/form-data; boundary=') + '|' + \
         JSON.stringify([...back]); })()";

    assert_eq!(js(script).await, r#"true|[["a","x y"],["b","z"]]"#);
}

#[tokio::test]
async fn test_a_response_body_reads_back_as_form_data() {
    let script = "(async () => { const response = new Response('a=1&b=2', \
         { headers: { 'content-type': 'application/x-www-form-urlencoded' } }); \
         return JSON.stringify([...(await response.formData())]); })()";

    assert_eq!(js(script).await, r#"[["a","1"],["b","2"]]"#);
}

#[tokio::test]
async fn test_reading_the_body_twice_is_refused() {
    let script = "(async () => { const response = new Response('a=1', \
         { headers: { 'content-type': 'application/x-www-form-urlencoded' } }); \
         await response.text(); \
         return response.formData().catch((error) => String(error)); })()";

    assert_eq!(
        js(script).await,
        "TypeError: Body has already been consumed"
    );
}
