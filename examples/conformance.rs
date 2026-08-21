//! Runs the SvelteKit conformance fixture and diffs against the V8 oracle.
//!
//!   cargo run --release --example conformance -- [--only NAME] [--fixture DIR]
//!
//! The fixture lives in the openworkers-conformance checkout; point at it with
//! --fixture or OW_CONFORMANCE_FIXTURE. A scenario that panics takes the
//! process with it, so --only exists to run them one per process.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use openworkers_core::Event;
use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::RequestBody;
use openworkers_core::Script;
use openworkers_core::TerminationReason;
use openworkers_core::Worker as _;
use openworkers_transform::CodeLanguage;
use openworkers_transform::parse_worker_code;
use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;

use openworkers_runtime_nova::Worker;

/// Bindings never reach the guest on this backend, so the fixture's ASSETS
/// binding is a JS shim answering 404, like the other runtimes' runners do.
const ENV_SHIM: &str = "globalThis.env = { ASSETS: { fetch() { \
     return Promise.resolve(new Response(null, { status: 404 })); } } };\n";

/// The fixture README's own caveat: the oracle recorded a `+` that the WHATWG
/// urlencoded parser decodes to a space, so matching it here would be a bug.
const ORACLE_IS_WRONG: &str = "urlencoded-plus";

#[derive(Deserialize)]
struct Spec {
    bundle: String,
    base_url: String,
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
struct Scenario {
    name: String,
    request: RequestSpec,
}

#[derive(Deserialize)]
struct RequestSpec {
    method: String,
    path: String,
    headers: Vec<String>,
    body: Option<String>,
}

#[derive(Deserialize)]
struct Oracle {
    lowered: Lowered,
    scenarios: Vec<Recorded>,
}

#[derive(Deserialize)]
struct Lowered {
    bytes: usize,
    sha256: String,
}

#[derive(Deserialize)]
struct Recorded {
    name: String,
    response: RecordedResponse,
}

#[derive(Deserialize)]
struct RecordedResponse {
    status: u16,
    headers: Vec<String>,
    body_file: String,
    warm_identical: bool,
}

struct Answer {
    status: u16,
    headers: Vec<String>,
    body: Vec<u8>,
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn fixture_dir(arg: Option<String>) -> PathBuf {
    if let Some(path) = arg {
        return PathBuf::from(path);
    }

    if let Ok(path) = std::env::var("OW_CONFORMANCE_FIXTURE") {
        return PathBuf::from(path);
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../openworkers-conformance/fixtures/sveltekit-app")
}

async fn dispatch(
    worker: &mut Worker,
    base_url: &str,
    spec: &RequestSpec,
) -> Result<Answer, TerminationReason> {
    let headers: HashMap<String, String> = spec
        .headers
        .iter()
        .map(|header| {
            let (name, value) = header
                .split_once(": ")
                .expect("header must be `name: value`");

            (name.to_string(), value.to_string())
        })
        .collect();

    let request = HttpRequest {
        method: spec.method.parse::<HttpMethod>().expect("bad method"),
        url: format!("{base_url}{}", spec.path),
        headers,
        body: match &spec.body {
            Some(body) => RequestBody::Bytes(body.clone().into_bytes().into()),
            None => RequestBody::None,
        },
    };

    let (event, rx) = Event::fetch(request);

    worker.exec(event).await?;

    let response = rx.await.expect("response channel should deliver");
    let body = response
        .body
        .collect()
        .await
        .expect("should read body")
        .unwrap_or_default();

    Ok(Answer {
        status: response.status,
        headers: response
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect(),
        body: body.to_vec(),
    })
}

/// The first place two header lists disagree, as `index: expected / got`.
fn header_diff(expected: &[String], got: &[String]) -> Option<String> {
    for (index, want) in expected.iter().enumerate() {
        match got.get(index) {
            Some(have) if have == want => continue,
            Some(have) => return Some(format!("header {index}: want `{want}`, got `{have}`")),
            None => return Some(format!("header {index}: want `{want}`, got nothing")),
        }
    }

    got.get(expected.len())
        .map(|extra| format!("header {}: unexpected `{extra}`", expected.len()))
}

/// The first byte where two bodies diverge, with the bytes around it.
fn body_diff(expected: &[u8], got: &[u8]) -> Option<String> {
    if expected == got {
        return None;
    }

    let at = expected
        .iter()
        .zip(got)
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(got.len()));

    let from = at.saturating_sub(40);

    Some(format!(
        "byte {at} of {}/{} (expected/got)\n      expected: ...{}\n      got:      ...{}",
        expected.len(),
        got.len(),
        String::from_utf8_lossy(&expected[from..(at + 60).min(expected.len())]).escape_debug(),
        String::from_utf8_lossy(&got[from..(at + 60).min(got.len())]).escape_debug(),
    ))
}

async fn run(
    scenario: &Scenario,
    oracle: &Recorded,
    root: &Path,
    base_url: &str,
    code: &str,
) -> &'static str {
    let expected = &oracle.response;
    let expected_body = std::fs::read(root.join(&expected.body_file)).expect("no oracle body");

    let mut worker = match Worker::new(Script::new(code), None).await {
        Ok(worker) => worker,
        Err(reason) => {
            println!("{:<24} FAIL  worker creation: {reason:?}", scenario.name);

            return "FAIL";
        }
    };

    let answer = match dispatch(&mut worker, base_url, &scenario.request).await {
        Ok(answer) => answer,
        Err(reason) => {
            println!("{:<24} FAIL  {reason:?}", scenario.name);

            return "FAIL";
        }
    };

    let status_ok = answer.status == expected.status;
    let headers = header_diff(&expected.headers, &answer.headers);
    let body = body_diff(&expected_body, &answer.body);

    // The oracle's own warm check: the same request twice on one worker.
    let warm = dispatch(&mut worker, base_url, &scenario.request).await;
    let warm_identical = match &warm {
        Ok(second) => {
            second.status == answer.status
                && second.headers == answer.headers
                && second.body == answer.body
        }
        Err(_) => false,
    };

    let verdict = match (status_ok, &headers, &body) {
        (true, None, None) => "PASS",
        (true, Some(_), None) => "PARTIAL",
        _ => "FAIL",
    };

    println!(
        "{:<24} {verdict:<8} status {} (want {}), {} bytes (want {})",
        scenario.name,
        answer.status,
        expected.status,
        answer.body.len(),
        expected_body.len()
    );

    if verdict != "PASS" && scenario.name == ORACLE_IS_WRONG {
        println!("    note: the oracle keeps `+` here, WHATWG decodes it to a space");
    }

    if let Some(diff) = headers {
        println!("    headers: {diff}");
    }

    if let Some(diff) = body {
        println!("    body: {diff}");
    }

    if warm_identical != expected.warm_identical {
        match warm {
            Ok(_) => println!("    warm: differs from the cold answer, oracle says identical"),
            Err(reason) => println!("    warm: second request failed: {reason:?}"),
        }
    }

    verdict
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut only = None;
    let mut fixture = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => only = args.next(),
            "--fixture" => fixture = args.next(),
            other => panic!("unknown argument: {other}"),
        }
    }

    let root = fixture_dir(fixture);

    let spec: Spec =
        serde_json::from_slice(&std::fs::read(root.join("scenarios.json")).expect("no scenarios"))
            .expect("bad scenarios.json");
    let oracle: Oracle =
        serde_json::from_slice(&std::fs::read(root.join("oracle.json")).expect("no oracle"))
            .expect("bad oracle.json");

    let bundle = std::fs::read(root.join(&spec.bundle)).expect("no bundle");
    let lowered = parse_worker_code(&bundle, CodeLanguage::JavaScript).expect("transform failed");
    let digest = sha256(lowered.as_bytes());

    println!(
        "lowered {} bytes, sha256 {digest} ({})",
        lowered.len(),
        if digest == oracle.lowered.sha256 && lowered.len() == oracle.lowered.bytes {
            "same bytes as the oracle"
        } else {
            "DIFFERENT from the oracle"
        }
    );

    let code = format!("{ENV_SHIM}{lowered}");
    let mut tally: Vec<&str> = Vec::new();

    for scenario in &spec.scenarios {
        if only.as_ref().is_some_and(|name| name != &scenario.name) {
            continue;
        }

        let recorded = oracle
            .scenarios
            .iter()
            .find(|recorded| recorded.name == scenario.name)
            .expect("scenario missing from the oracle");

        tally.push(run(scenario, recorded, &root, &spec.base_url, &code).await);
    }

    println!(
        "\n{} of {} scenarios match the oracle, {} partially",
        tally.iter().filter(|verdict| **verdict == "PASS").count(),
        tally.len(),
        tally
            .iter()
            .filter(|verdict| **verdict == "PARTIAL")
            .count()
    );
}
