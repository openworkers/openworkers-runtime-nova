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

    fn handle_binding_database(
        &self,
        _binding: &str,
        op: DatabaseOp,
    ) -> OpFuture<'_, DatabaseResult> {
        let DatabaseOp::Query { sql, params } = op;
        let rows = serde_json::json!([{ "sql": sql, "params": params.len() }]);

        Box::pin(async move { DatabaseResult::Rows(rows.to_string()) })
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
