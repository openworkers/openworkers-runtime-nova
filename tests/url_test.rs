mod common;

use common::js;
use common::js_err;

#[tokio::test]
async fn test_url_exposes_every_component() {
    let script = r#"
        (() => {
            const url = new URL('https://user:pass@example.com:8443/a/b?x=1&y=2#frag');

            return [
                url.href,
                url.origin,
                url.protocol,
                url.username,
                url.password,
                url.host,
                url.hostname,
                url.port,
                url.pathname,
                url.search,
                url.hash,
            ].join(' | ');
        })()
    "#;

    assert_eq!(
        js(script).await,
        "https://user:pass@example.com:8443/a/b?x=1&y=2#frag | https://example.com:8443 | \
         https: | user | pass | example.com:8443 | example.com | 8443 | /a/b | ?x=1&y=2 | #frag"
    );
}

#[tokio::test]
async fn test_url_resolves_against_a_base() {
    for (input, base, expected) in [
        ("/b", "http://example.com/a/c", "http://example.com/b"),
        ("d", "http://example.com/a/c", "http://example.com/a/d"),
        ("../e", "http://example.com/a/c/d", "http://example.com/a/e"),
        (
            "//other.com/x",
            "https://example.com/a",
            "https://other.com/x",
        ),
        ("?q=1", "http://example.com/a", "http://example.com/a?q=1"),
        (
            "http://absolute/",
            "http://example.com/a",
            "http://absolute/",
        ),
    ] {
        let script = format!("new URL('{input}', '{base}').href");

        assert_eq!(js(&script).await, expected, "for {input} against {base}");
    }
}

#[tokio::test]
async fn test_url_normalizes_a_default_port_away() {
    let script =
        "new URL('http://example.com:80/a').href + ' ' + new URL('http://example.com:80/a').port";

    assert_eq!(js(script).await, "http://example.com/a ");
}

#[tokio::test]
async fn test_url_percent_encodes_the_path_and_punycodes_the_host() {
    let script = "new URL('http://\u{e9}xample.com/a b').href";

    assert_eq!(js(script).await, "http://xn--xample-9ua.com/a%20b");
}

#[tokio::test]
async fn test_an_opaque_scheme_has_a_null_origin() {
    let script = "new URL('sveltekit-internal://').origin + ':' + \
                  new URL('sveltekit-internal://').protocol";

    assert_eq!(js(script).await, "null:sveltekit-internal:");
}

#[tokio::test]
async fn test_an_unparseable_url_is_a_type_error() {
    for input in ["not a url", "http://", "/relative"] {
        let message = js_err(&format!("new URL('{input}')")).await;

        assert!(message.contains("TypeError"), "for {input}: {message}");
    }
}

#[tokio::test]
async fn test_can_parse_and_static_parse_report_failure_without_throwing() {
    let script = "URL.canParse('http://a/') + ',' + URL.canParse('nope') + ',' + \
                  URL.parse('nope') + ',' + URL.parse('http://a/').pathname";

    assert_eq!(js(script).await, "true,false,null,/");
}

#[tokio::test]
async fn test_url_setters_rewrite_the_href() {
    let script = r#"
        (() => {
            const url = new URL('http://example.com/a?x=1#h');

            url.protocol = 'https:';
            url.hostname = 'other.com';
            url.port = '8080';
            url.pathname = '/b/c';
            url.search = '?y=2';
            url.hash = 'top';
            url.username = 'u';
            url.password = 'p';

            return url.href;
        })()
    "#;

    assert_eq!(js(script).await, "https://u:p@other.com:8080/b/c?y=2#top");
}

#[tokio::test]
async fn test_the_host_setter_carries_a_port() {
    let script = "(() => { const u = new URL('http://a.com/'); u.host = 'b.com:81'; \
                  return u.host + ' ' + u.hostname + ' ' + u.port; })()";

    assert_eq!(js(script).await, "b.com:81 b.com 81");
}

#[tokio::test]
async fn test_setters_that_cannot_apply_leave_the_url_alone() {
    let script = r#"
        (() => {
            const url = new URL('http://example.com/a');

            url.hostname = 'evil.com:99';
            url.port = 'notaport';
            url.protocol = 'not a scheme';

            return url.href;
        })()
    "#;

    assert_eq!(js(script).await, "http://example.com/a");
}

#[tokio::test]
async fn test_the_href_setter_replaces_the_whole_url() {
    let script = "(() => { const u = new URL('http://a.com/x'); u.href = 'https://b.com/y?z=1'; \
                  return u.href + ' ' + u.searchParams.get('z'); })()";

    assert_eq!(js(script).await, "https://b.com/y?z=1 1");
    assert!(
        js_err("(() => { const u = new URL('http://a.com/'); u.href = 'nope'; })()")
            .await
            .contains("TypeError")
    );
}

#[tokio::test]
async fn test_empty_search_and_hash_serialize_as_empty_strings() {
    let script = "(() => { const u = new URL('http://a.com/p?x=1#h'); u.search = ''; u.hash = ''; \
                  return JSON.stringify([u.search, u.hash, u.href]); })()";

    assert_eq!(js(script).await, r#"["","","http://a.com/p"]"#);
}

#[tokio::test]
async fn test_url_stringifies_to_its_href() {
    let script = "String(new URL('http://a.com/x')) + ' ' + \
                  JSON.stringify({ u: new URL('http://a.com/x') })";

    assert_eq!(js(script).await, r#"http://a.com/x {"u":"http://a.com/x"}"#);
}

#[tokio::test]
async fn test_search_params_read_the_query() {
    let script = r#"
        (() => {
            const params = new URL('http://a.com/?a=1&b=2&a=3&flag').searchParams;

            return JSON.stringify([
                params.get('a'),
                params.getAll('a'),
                params.get('missing'),
                params.has('flag'),
                params.get('flag'),
                params.size,
            ]);
        })()
    "#;

    assert_eq!(js(script).await, r#"["1",["1","3"],null,true,"",4]"#);
}

#[tokio::test]
async fn test_search_params_decode_plus_and_percent_escapes() {
    let script = "JSON.stringify([...new URLSearchParams('a=b+c&d=%C3%A9%20f&%C3%BC=1')])";

    assert_eq!(
        js(script).await,
        "[[\"a\",\"b c\"],[\"d\",\"\u{e9} f\"],[\"\u{fc}\",\"1\"]]"
    );
}

#[tokio::test]
async fn test_search_params_serialize_with_form_encoding() {
    let script = "new URLSearchParams({ 'a b': 'c+d', 'e': '\u{e9}/*' }).toString()";

    assert_eq!(js(script).await, "a+b=c%2Bd&e=%C3%A9%2F*");
}

#[tokio::test]
async fn test_search_params_accept_every_init_shape() {
    let script = r#"
        JSON.stringify([
            new URLSearchParams('?a=1&b=2').toString(),
            new URLSearchParams([['a', '1'], ['b', '2']]).toString(),
            new URLSearchParams({ a: '1', b: '2' }).toString(),
            new URLSearchParams(new URLSearchParams('a=1&b=2')).toString(),
            new URLSearchParams().toString(),
        ])
    "#;

    assert_eq!(
        js(script).await,
        r#"["a=1&b=2","a=1&b=2","a=1&b=2","a=1&b=2",""]"#
    );
}

#[tokio::test]
async fn test_search_params_mutations() {
    let script = r#"
        (() => {
            const params = new URLSearchParams('a=1&b=2&a=3');

            params.append('c', '4');
            params.set('a', '9');
            params.delete('b');

            return params.toString() + ' | ' + params.has('a', '9') + ' | ' + params.has('a', '1');
        })()
    "#;

    assert_eq!(js(script).await, "a=9&c=4 | true | false");
}

#[tokio::test]
async fn test_search_params_sort_is_by_name() {
    let script = "(() => { const p = new URLSearchParams('c=3&a=1&b=2&a=0'); p.sort(); \
                  return p.toString(); })()";

    assert_eq!(js(script).await, "a=1&a=0&b=2&c=3");
}

#[tokio::test]
async fn test_search_params_iteration_shapes() {
    let script = r#"
        (() => {
            const params = new URLSearchParams('a=1&b=2');
            const seen = [];

            params.forEach((value, name) => seen.push(name + '=' + value));

            return JSON.stringify([
                seen,
                [...params.keys()],
                [...params.values()],
                [...params.entries()].map((pair) => pair.join(':')),
            ]);
        })()
    "#;

    assert_eq!(
        js(script).await,
        r#"[["a=1","b=2"],["a","b"],["1","2"],["a:1","b:2"]]"#
    );
}

#[tokio::test]
async fn test_search_params_write_back_to_their_url() {
    let script = r#"
        (() => {
            const url = new URL('http://a.com/p');

            url.searchParams.append('q', 'a b');
            url.searchParams.set('r', '2');

            const beforeDelete = url.href;

            url.searchParams.delete('q');

            return beforeDelete + ' | ' + url.href + ' | ' + url.search;
        })()
    "#;

    assert_eq!(
        js(script).await,
        "http://a.com/p?q=a+b&r=2 | http://a.com/p?r=2 | ?r=2"
    );
}

#[tokio::test]
async fn test_the_search_setter_refreshes_the_params() {
    let script = "(() => { const u = new URL('http://a.com/?a=1'); const p = u.searchParams; \
                  u.search = 'b=2'; return p.get('a') + ',' + p.get('b'); })()";

    assert_eq!(js(script).await, "null,2");
}

#[tokio::test]
async fn test_detached_params_do_not_touch_a_url() {
    let script = "(() => { const p = new URLSearchParams('a=1'); p.set('a', '2'); \
                  return p.toString(); })()";

    assert_eq!(js(script).await, "a=2");
}
