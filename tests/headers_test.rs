mod common;

use common::js;
use common::js_err;

#[tokio::test]
async fn test_headers_are_case_insensitive() {
    let script = "(() => { const h = new Headers({ 'Content-Type': 'text/html' }); \
                  return h.get('content-type') + ',' + h.has('CONTENT-TYPE') + ',' + h.get('nope'); })()";

    assert_eq!(js(script).await, "text/html,true,null");
}

#[tokio::test]
async fn test_append_joins_repeated_values_with_a_comma_and_space() {
    let script = r#"
        (() => {
            const headers = new Headers();

            headers.append('link', '</a>; rel=preload');
            headers.append('link', '</b>; rel=preload');

            return headers.get('link');
        })()
    "#;

    assert_eq!(js(script).await, "</a>; rel=preload, </b>; rel=preload");
}

#[tokio::test]
async fn test_set_replaces_every_value() {
    let script = "(() => { const h = new Headers(); h.append('a', '1'); h.append('a', '2'); \
                  h.set('A', '3'); return h.get('a'); })()";

    assert_eq!(js(script).await, "3");
}

#[tokio::test]
async fn test_delete_removes_every_value() {
    let script = "(() => { const h = new Headers([['a', '1'], ['a', '2'], ['b', '3']]); \
                  h.delete('a'); return h.get('a') + ',' + h.get('b'); })()";

    assert_eq!(js(script).await, "null,3");
}

#[tokio::test]
async fn test_values_are_trimmed_of_http_whitespace() {
    assert_eq!(js("new Headers({ a: '  1  ' }).get('a')").await, "1");
}

#[tokio::test]
async fn test_an_invalid_name_or_value_is_a_type_error() {
    for expression in [
        "new Headers({ 'a b': '1' })",
        "new Headers().append('a\\n', '1')",
        "new Headers().append('a', 'b\\r\\nc: d')",
    ] {
        assert!(
            js_err(expression).await.contains("TypeError"),
            "{expression}"
        );
    }
}

#[tokio::test]
async fn test_iteration_is_sorted_and_combined() {
    let script = r#"
        (() => {
            const headers = new Headers();

            headers.append('x-b', '2');
            headers.append('x-a', '1');
            headers.append('x-a', '3');

            return JSON.stringify([...headers]);
        })()
    "#;

    assert_eq!(js(script).await, r#"[["x-a","1, 3"],["x-b","2"]]"#);
}

#[tokio::test]
async fn test_set_cookie_stays_split() {
    let script = r#"
        (() => {
            const headers = new Headers();

            headers.append('set-cookie', 'a=1');
            headers.append('set-cookie', 'b=2');

            return JSON.stringify([headers.getSetCookie(), [...headers.entries()]]);
        })()
    "#;

    assert_eq!(
        js(script).await,
        r#"[["a=1","b=2"],[["set-cookie","a=1"],["set-cookie","b=2"]]]"#
    );
}

#[tokio::test]
async fn test_every_init_shape_is_accepted() {
    let script = r#"
        JSON.stringify([
            [...new Headers({ a: '1' })],
            [...new Headers([['a', '1']])],
            [...new Headers(new Headers({ a: '1' }))],
            [...new Headers()],
        ])
    "#;

    assert_eq!(
        js(script).await,
        r#"[[["a","1"]],[["a","1"]],[["a","1"]],[]]"#
    );
}

#[tokio::test]
async fn test_keys_values_and_for_each_agree_with_entries() {
    let script = r#"
        (() => {
            const headers = new Headers({ b: '2', a: '1' });
            const seen = [];

            headers.forEach((value, name) => seen.push(name + '=' + value));

            return JSON.stringify([[...headers.keys()], [...headers.values()], seen]);
        })()
    "#;

    assert_eq!(js(script).await, r#"[["a","b"],["1","2"],["a=1","b=2"]]"#);
}
