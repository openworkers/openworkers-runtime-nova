mod common;

use std::collections::HashMap;

use bytes::Bytes;

use openworkers_core::BindingInfo;
use openworkers_core::BindingType;
use openworkers_core::DatabaseOp;
use openworkers_core::DatabaseResult;
use openworkers_core::Event;
use openworkers_core::HttpRequest;
use openworkers_core::HttpResponse;
#[cfg(feature = "invoke")]
use openworkers_core::InvokeResult;
use openworkers_core::LogLevel;
use openworkers_core::OpFuture;
use openworkers_core::OperationsHandler;
use openworkers_core::ResponseBody;
use openworkers_core::Script;

use openworkers_runtime_nova::Worker;

use common::URL;
use common::body_text;
use common::get;

/// Answers the two binding kinds this runtime serves, with what it was asked.
struct Bindings;

impl OperationsHandler for Bindings {
    fn handle_log(&self, _level: LogLevel, _message: String) {}

    fn handle_binding_fetch(
        &self,
        binding: &str,
        request: HttpRequest,
    ) -> OpFuture<'_, Result<HttpResponse, String>> {
        let body = format!("{binding} served {}", request.url);

        Box::pin(async move {
            Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".to_string(), "text/plain".to_string())],
                body: ResponseBody::Bytes(Bytes::from(body)),
            })
        })
    }

    fn handle_fetch(&self, request: HttpRequest) -> OpFuture<'_, Result<HttpResponse, String>> {
        if request.url.ends_with("/refused") {
            return Box::pin(async { Err("the host does not reach that".to_string()) });
        }

        if request.url.ends_with("/empty") {
            return Box::pin(async {
                Ok(HttpResponse {
                    status: 204,
                    headers: Vec::new(),
                    body: ResponseBody::None,
                })
            });
        }

        let body = match request.body {
            openworkers_core::RequestBody::Bytes(bytes) => {
                String::from_utf8_lossy(&bytes).into_owned()
            }
            _ => String::new(),
        };

        let echoed = format!(
            "{} {} [{}] {}",
            request.method,
            request.url,
            request.headers.get("x-probe").cloned().unwrap_or_default(),
            body
        );

        Box::pin(async move {
            Ok(HttpResponse {
                status: 201,
                headers: vec![("x-answer".to_string(), "yes".to_string())],
                body: ResponseBody::Bytes(Bytes::from(echoed)),
            })
        })
    }

    fn handle_binding_database(
        &self,
        _binding: &str,
        op: DatabaseOp,
    ) -> OpFuture<'_, DatabaseResult> {
        let DatabaseOp::Query { sql, params } = op;
        let rows = serde_json::json!([{ "sql": sql, "params": params.len() }]);

        Box::pin(async move { DatabaseResult::Rows(rows.to_string()) })
    }

    #[cfg(feature = "invoke")]
    fn handle_binding_invoke(
        &self,
        binding: &str,
        name: &str,
        args: serde_json::Value,
    ) -> OpFuture<'_, InvokeResult> {
        let answer = match name {
            "echo" => InvokeResult::Value(serde_json::json!({ "binding": binding, "args": args })),
            "fail" => InvokeResult::Error("the disk is full".to_string()),
            other => InvokeResult::Error(format!("no function named '{other}'")),
        };

        Box::pin(async move { answer })
    }
}

async fn serve(script: &str, bindings: Vec<BindingInfo>) -> String {
    let mut env = HashMap::new();

    env.insert("GREETING".to_string(), "bonjour".to_string());

    let script = Script {
        code: script.into(),
        env: Some(env),
        bindings,
    };

    let mut worker = Worker::new_with_ops(script, None, std::sync::Arc::new(Bindings))
        .await
        .expect("worker should initialize");

    let (task, rx) = Event::fetch(get(URL));

    worker.exec(task).await.expect("task should execute");

    body_text(rx.await.expect("should receive response")).await
}

#[tokio::test]
async fn test_an_asset_binding_answers_with_a_response() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                env.ASSETS.fetch('http://localhost/app.js').then((r) =>
                    r.text().then((body) => new Response(r.status + '|' + body))
                )
            );
        });
    "#;

    let body = serve(
        script,
        vec![BindingInfo::new("ASSETS", BindingType::Assets)],
    )
    .await;

    assert_eq!(body, "200|ASSETS served http://localhost/app.js");
}

#[tokio::test]
async fn test_a_database_binding_answers_with_rows() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                env.DB.query('select $1::int', [7]).then(
                    (rows) => new Response(JSON.stringify(rows))
                )
            );
        });
    "#;

    let body = serve(script, vec![BindingInfo::new("DB", BindingType::Database)]).await;

    assert_eq!(body, r#"[{"params":1,"sql":"select $1::int"}]"#);
}

/// A guest script is not strict, so writing to the frozen `env` is ignored
/// rather than thrown; what matters is that the value does not move.
#[tokio::test]
async fn test_env_carries_the_variables_and_does_not_take_a_write() {
    let script = r#"
        addEventListener('fetch', (event) => {
            env.GREETING = 'autre';
            delete env.GREETING;

            event.respondWith(new Response(env.GREETING + '|' + Object.isFrozen(env)));
        });
    "#;

    assert_eq!(serve(script, Vec::new()).await, "bonjour|true");
}

#[tokio::test]
async fn test_two_binding_calls_both_settle() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                Promise.all([
                    env.ASSETS.fetch('http://localhost/a'),
                    env.ASSETS.fetch('http://localhost/b'),
                ]).then((responses) => new Response(responses.map((r) => r.status).join(',')))
            );
        });
    "#;

    let body = serve(
        script,
        vec![BindingInfo::new("ASSETS", BindingType::Assets)],
    )
    .await;

    assert_eq!(body, "200,200");
}

/// Runs `body`, the inside of an async function that returns a string, with
/// one invoke binding named RUST.
#[cfg(feature = "invoke")]
async fn invoke(body: &str) -> String {
    let script = format!(
        "addEventListener('fetch', (event) => {{
            event.respondWith((async () => {{ {body} }})().then((text) => new Response(String(text))));
        }});"
    );

    serve(&script, vec![BindingInfo::new("RUST", BindingType::Invoke)]).await
}

#[cfg(feature = "invoke")]
#[tokio::test]
async fn test_invoke_returns_what_the_host_answers() {
    let body = invoke("return JSON.stringify(await env.RUST.invoke('echo', { a: 2 }));").await;

    assert_eq!(body, r#"{"args":{"a":2},"binding":"RUST"}"#);
}

#[cfg(feature = "invoke")]
#[tokio::test]
async fn test_invoke_keeps_arguments_that_are_falsy() {
    let body = invoke(
        "const answers = [];
         for (const value of [0, false, '', null]) {
             answers.push((await env.RUST.invoke('echo', value)).args);
         }
         return JSON.stringify(answers);",
    )
    .await;

    assert_eq!(body, r#"[0,false,"",null]"#);
}

#[cfg(feature = "invoke")]
#[tokio::test]
async fn test_invoke_without_arguments_sends_null() {
    let body = invoke("return JSON.stringify((await env.RUST.invoke('echo')).args);").await;

    assert_eq!(body, "null");
}

#[cfg(feature = "invoke")]
#[tokio::test]
async fn test_invoke_error_rejects_with_its_message() {
    let body =
        invoke("try { await env.RUST.invoke('fail'); } catch (error) { return error.message; }")
            .await;

    assert_eq!(body, "the disk is full");
}

#[cfg(feature = "invoke")]
#[tokio::test]
async fn test_invoke_command_that_is_not_a_string_is_a_type_error() {
    let body = invoke(
        "try { await env.RUST.invoke(42); } catch (error) { return error.name + ': ' + error.message; }",
    )
    .await;

    assert_eq!(body, "TypeError: invoke: the command is not a string");
}

#[cfg(feature = "invoke")]
#[tokio::test]
async fn test_invoke_arguments_that_cannot_be_json_are_a_type_error() {
    let body = invoke(
        "const names = [];
         const circular = {}; circular.self = circular;
         for (const value of [circular, { a: 1n }]) {
             try { await env.RUST.invoke('echo', value); names.push('sent'); }
             catch (error) { names.push(error.name); }
         }
         return names.join(',');",
    )
    .await;

    assert_eq!(body, "TypeError,TypeError");
}

#[cfg(feature = "invoke")]
#[test]
fn test_the_runtime_says_it_serves_invoke() {
    assert!(openworkers_runtime_nova::SUPPORTED_BINDINGS.contains(&BindingType::Invoke));
}

#[tokio::test]
async fn test_an_asset_binding_takes_a_bare_path() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                env.ASSETS.fetch('/_app/version.json').then((r) =>
                    r.text().then((body) => new Response(r.status + '|' + body))
                )
            );
        });
    "#;

    let body = serve(
        script,
        vec![BindingInfo::new("ASSETS", BindingType::Assets)],
    )
    .await;

    assert_eq!(body, "200|ASSETS served http://localhost/_app/version.json");
}

#[tokio::test]
async fn test_an_asset_binding_takes_a_url_object_and_a_request() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                Promise.all([
                    env.ASSETS.fetch(new URL('http://localhost/a.js')),
                    env.ASSETS.fetch(new Request('http://localhost/b.js')),
                ])
                    .then((responses) => Promise.all(responses.map((r) => r.text())))
                    .then((bodies) => new Response(bodies.join(',')))
            );
        });
    "#;

    let body = serve(
        script,
        vec![BindingInfo::new("ASSETS", BindingType::Assets)],
    )
    .await;

    assert_eq!(
        body,
        "ASSETS served http://localhost/a.js,ASSETS served http://localhost/b.js"
    );
}

#[tokio::test]
async fn test_an_asset_binding_never_hands_dot_segments_to_the_host() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                Promise.all([
                    env.ASSETS.fetch('/a/../../secret.txt'),
                    env.ASSETS.fetch('http://localhost/b/../../secret.txt'),
                    env.ASSETS.fetch('/%2e%2e/secret.txt'),
                ])
                    .then((responses) => Promise.all(responses.map((r) => r.text())))
                    .then((bodies) => new Response(bodies.join(',')))
            );
        });
    "#;

    let body = serve(
        script,
        vec![BindingInfo::new("ASSETS", BindingType::Assets)],
    )
    .await;

    // The URL parser clamps all three at the root, the encoded one included
    let served = "ASSETS served http://localhost/secret.txt";

    assert_eq!(body, [served, served, served].join(","));
}

#[tokio::test]
async fn test_the_global_fetch_goes_through_the_host_and_carries_a_body() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                fetch('http://example.test/echo', {
                    method: 'POST',
                    headers: { 'x-probe': 'on' },
                    body: 'h\u00e9llo',
                }).then(async (r) =>
                    new Response(r.status + '|' + r.headers.get('x-answer') + '|' + (await r.text()))
                )
            );
        });
    "#;

    let body = serve(script, Vec::new()).await;

    assert_eq!(
        body,
        "201|yes|POST http://example.test/echo [on] h\u{e9}llo"
    );
}

#[tokio::test]
async fn test_the_global_fetch_takes_a_request_and_a_bodiless_status() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(
                fetch(new Request('http://example.test/empty')).then(async (r) =>
                    new Response(r.status + '|' + (await r.text()).length)
                )
            );
        });
    "#;

    assert_eq!(serve(script, Vec::new()).await, "204|0");
}

#[tokio::test]
async fn test_the_global_fetch_rejects_when_the_host_refuses_or_the_signal_is_aborted() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith((async () => {
                const out = [];

                try {
                    await fetch('http://example.test/refused');
                    out.push('accepted');
                } catch (error) {
                    out.push(error.message);
                }

                try {
                    await fetch('http://example.test/x', { signal: AbortSignal.abort() });
                    out.push('accepted');
                } catch (error) {
                    out.push(error.name);
                }

                return new Response(out.join('|'));
            })());
        });
    "#;

    assert_eq!(
        serve(script, Vec::new()).await,
        "the host does not reach that|AbortError"
    );
}
