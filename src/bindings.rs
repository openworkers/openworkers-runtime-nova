//! The host side of `env`: a binding call the guest makes, answered later.
//!
//! The guest holds the promise; the host keeps only the call id and the future
//! it is waiting on, and settles through `__ow_settle_binding` when the answer
//! comes. It is the shape the timers use, for the same reason: nova_vm gives an
//! embedder no way to root a JavaScript function across a drain.

use std::future::Future;
use std::pin::Pin;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use nova_vm::ecmascript::Agent;
use nova_vm::ecmascript::ArgumentsList;
use nova_vm::ecmascript::JsResult;
use nova_vm::ecmascript::Value;
use nova_vm::engine::Bindable;
use nova_vm::engine::GcScope;
use nova_vm::engine::Scopable;

use openworkers_core::BindingInfo;
use openworkers_core::BindingType;
use openworkers_core::DatabaseOp;
use openworkers_core::DatabaseResult;
use openworkers_core::HttpRequest;
use openworkers_core::OperationsHandle;
use openworkers_core::RequestBody;
use openworkers_core::SqlParam;

use serde::Deserialize;

/// A call the guest is waiting on. The answer is JSON either way: what the
/// binding returned, or the message to reject with.
pub struct Pending {
    pub id: f64,
    pub answer: Pin<Box<dyn Future<Output = Result<String, String>> + Send>>,
}

/// Binding types this runtime answers, which is what the runner checks a
/// worker's declarations against.
pub const SUPPORTED: &[BindingType] = &[BindingType::Assets, BindingType::Database];

#[derive(Deserialize)]
struct FetchParams {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Deserialize)]
struct QueryParams {
    sql: String,
    #[serde(default)]
    params: Vec<SqlParam>,
}

/// `__ow_native_binding(id, kind, name, method, paramsJson)`: starts the call
/// and returns at once. The guest's promise settles when the drain gets to it.
pub fn native_binding<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let kind = args.get(1).scope(agent, gc.nogc());
    let name = args.get(2).scope(agent, gc.nogc());
    let params = args.get(4).scope(agent, gc.nogc());

    let id = args.get(0).to_number(agent, gc.reborrow()).unbind()?;
    let id = id.into_f64(agent);

    let kind = kind.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let kind = kind.to_string_lossy(agent).into_owned();

    let name = name.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let name = name.to_string_lossy(agent).into_owned();

    let params = params.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let params = params.to_string_lossy(agent).into_owned();

    let slots = crate::worker::host_slots(agent);

    let Some(ops) = slots.ops.clone() else {
        slots.bindings.borrow_mut().push(Pending {
            id,
            answer: Box::pin(async { Err("no operations handler is wired".to_string()) }),
        });

        return Ok(Value::Undefined);
    };

    slots.bindings.borrow_mut().push(Pending {
        id,
        answer: answer(ops, kind, name, params),
    });

    Ok(Value::Undefined)
}

fn answer(
    ops: OperationsHandle,
    kind: String,
    name: String,
    params: String,
) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
    match kind.as_str() {
        "fetch" => Box::pin(async move { binding_fetch(ops, name, params).await }),
        "query" => Box::pin(async move { binding_query(ops, name, params).await }),
        other => {
            let message = format!("'{other}' is not a binding call this runtime makes");

            Box::pin(async move { Err(message) })
        }
    }
}

async fn binding_fetch(
    ops: OperationsHandle,
    name: String,
    params: String,
) -> Result<String, String> {
    let params: FetchParams =
        serde_json::from_str(&params).map_err(|e| format!("bad fetch parameters: {e}"))?;

    let method = params.method.as_deref().unwrap_or("GET");
    let request = HttpRequest {
        method: method.parse().unwrap_or_default(),
        url: params.url,
        headers: params.headers.unwrap_or_default(),
        body: match params.body {
            Some(body) => RequestBody::Bytes(body.into()),
            None => RequestBody::None,
        },
    };

    let response = ops.handle_binding_fetch(&name, request).await?;
    let body = collect(response.body).await?;

    serde_json::to_string(&serde_json::json!({
        "status": response.status,
        "headers": response.headers,
        // An asset is arbitrary bytes and only text crosses here.
        "body": BASE64.encode(&body),
    }))
    .map_err(|e| e.to_string())
}

async fn binding_query(
    ops: OperationsHandle,
    name: String,
    params: String,
) -> Result<String, String> {
    let params: QueryParams =
        serde_json::from_str(&params).map_err(|e| format!("bad query parameters: {e}"))?;

    let op = DatabaseOp::Query {
        sql: params.sql,
        params: params.params,
    };

    match ops.handle_binding_database(&name, op).await {
        DatabaseResult::Rows(rows) => Ok(format!("{{\"rows\":{rows}}}")),
        // The typed shape is the one that carries bytes; it becomes the same
        // array of objects the JSON shape already is.
        DatabaseResult::Table { columns, rows } => {
            let rows: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|row| {
                    columns
                        .iter()
                        .cloned()
                        .zip(row)
                        .map(|(column, value)| {
                            (column, serde_json::to_value(value).unwrap_or_default())
                        })
                        .collect::<serde_json::Map<_, _>>()
                        .into()
                })
                .collect();

            serde_json::to_string(&serde_json::json!({ "rows": rows })).map_err(|e| e.to_string())
        }
        DatabaseResult::Error(message) => Err(message),
    }
}

async fn collect(body: openworkers_core::ResponseBody) -> Result<Vec<u8>, String> {
    match body {
        openworkers_core::ResponseBody::None => Ok(Vec::new()),
        openworkers_core::ResponseBody::Bytes(bytes) => Ok(bytes.to_vec()),
        openworkers_core::ResponseBody::Stream(mut stream) => {
            let mut out = Vec::new();

            while let Some(chunk) = stream.recv().await {
                out.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
            }

            Ok(out)
        }
    }
}

/// The `env` object a script sees: its variables, and one object per binding.
pub fn env_source(
    env: &Option<std::collections::HashMap<String, String>>,
    bindings: &[BindingInfo],
) -> String {
    let variables = env
        .as_ref()
        .map(|map| serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string()))
        .unwrap_or_else(|| "{}".to_string());

    let objects: Vec<String> = bindings
        .iter()
        .map(|binding| {
            let name = serde_json::to_string(&binding.name).unwrap_or_else(|_| "\"\"".to_string());

            match binding.binding_type {
                BindingType::Assets => format!("{name}: __ow_assets_binding({name})"),
                BindingType::Database => format!("{name}: __ow_database_binding({name})"),
                // The runner refuses these before a worker gets here, so what is
                // left is a binding a newer runner allowed through.
                _ => format!("{name}: __ow_absent_binding({name})"),
            }
        })
        .collect();

    let objects = if objects.is_empty() {
        String::new()
    } else {
        format!(", {{{}}}", objects.join(","))
    };

    format!(
        "Object.defineProperty(globalThis, 'env', {{ \
         value: Object.freeze(Object.assign({variables}{objects})), \
         writable: false, enumerable: true, configurable: false }});"
    )
}
