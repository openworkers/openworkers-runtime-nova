mod common;

use common::exception_message;
use common::serve_body;
use common::serve_err;

#[tokio::test]
async fn test_atomics_wait_throws_instead_of_blocking() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const cell = new Int32Array(new SharedArrayBuffer(8));
            let outcome;

            try {
                outcome = 'returned ' + Atomics.wait(cell, 0, 0, 1);
            } catch (error) {
                outcome = String(error);
            }

            event.respondWith(new Response(outcome));
        });
    "#;

    assert!(serve_body(script).await.contains("TypeError"));
}

#[tokio::test]
async fn test_console_accepts_any_argument_shape() {
    let script = r#"
        const circular = {};
        circular.self = circular;

        console.log('at eval time');

        addEventListener('fetch', (event) => {
            console.log();
            console.info(circular);
            console.warn(1, 'two', [3], null, undefined);
            console.error(new Error('logged not thrown'));
            console.debug(Symbol('s'), 10n);
            event.respondWith(new Response('survived'));
        });
    "#;

    assert_eq!(serve_body(script).await, "survived");
}

#[tokio::test]
async fn test_native_log_survives_a_hostile_first_argument() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const noisy = {
                toString() {
                    let text = '';

                    for (let i = 0; i < 5000; i++) {
                        text += String(i);
                    }

                    return text.slice(0, 4);
                },
            };

            __ow_native_log(noisy, 'second' + 'argument');
            event.respondWith(new Response('survived'));
        });
    "#;

    assert_eq!(serve_body(script).await, "survived");
}

#[tokio::test]
async fn test_json_round_trip_of_awkward_values() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response(JSON.stringify({
                nan: NaN,
                infinity: Infinity,
                negativeZero: -0,
                huge: 1e308,
                undefined: undefined,
                epoch: new Date(0),
                nested: { deep: [1, [2, [3]]] },
            })));
        });
    "#;

    let body = serve_body(script).await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("body should be JSON");

    assert!(json["nan"].is_null());
    assert!(json["infinity"].is_null());
    assert_eq!(json["negativeZero"], 0);
    assert_eq!(json["huge"], 1e308);
    assert!(json.get("undefined").is_none());
    assert_eq!(json["epoch"], "1970-01-01T00:00:00.000Z");
    assert_eq!(json["nested"]["deep"][1][1][0], 3);
}

#[tokio::test]
async fn test_web_platform_globals_are_absent() {
    let missing = [
        "setTimeout",
        "setInterval",
        "queueMicrotask",
        "fetch",
        "URL",
        "TextEncoder",
        "TextDecoder",
        "Headers",
        "crypto",
        "structuredClone",
        "atob",
        "Intl",
    ];

    let script = format!(
        "addEventListener('fetch', (event) => event.respondWith(new Response([{}].join(','))));",
        missing
            .iter()
            .map(|name| format!("typeof {name}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let expected = vec!["undefined"; missing.len()].join(",");

    assert_eq!(serve_body(&script).await, expected);
}

#[tokio::test]
async fn test_regexp_lookaround_is_unsupported() {
    let script = r#"
        addEventListener('fetch', (event) => {
            let outcome;

            try {
                outcome = 'matched ' + /a(?=b)/.test('ab');
            } catch (error) {
                outcome = String(error);
            }

            event.respondWith(new Response(outcome));
        });
    "#;

    assert!(serve_body(script).await.contains("SyntaxError"));
}

#[tokio::test]
async fn test_the_host_boundary_is_reachable_from_guest_code() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response([
                typeof __ow_dispatch,
                typeof __ow_native_respond,
                typeof __ow_native_log,
            ].join(',')));
        });
    "#;

    assert_eq!(serve_body(script).await, "function,function,function");
}

#[tokio::test]
async fn test_the_native_builtins_cannot_be_replaced_by_the_guest() {
    let script = r#"
        addEventListener('fetch', (event) => {
            __ow_native_respond = 1;
            __ow_native_log = 1;

            event.respondWith(new Response([
                typeof __ow_native_respond,
                typeof __ow_native_log,
            ].join(',')));
        });
    "#;

    assert_eq!(serve_body(script).await, "function,function");
}

#[tokio::test]
async fn test_replacing_json_stringify_breaks_the_dispatch() {
    let script = r#"
        JSON.stringify = function () { throw new Error('hijacked'); };

        addEventListener('fetch', (event) => event.respondWith(new Response('ok')));
    "#;

    assert!(exception_message(serve_err(script).await).contains("did not settle"));
}

#[tokio::test]
async fn test_a_guest_supplied_dispatch_payload_is_reported_as_invalid() {
    let script = r#"
        globalThis.__ow_dispatch = function () { __ow_native_respond('not json'); };
    "#;

    assert!(matches!(
        serve_err(script).await,
        openworkers_core::TerminationReason::Other(_)
    ));
}
