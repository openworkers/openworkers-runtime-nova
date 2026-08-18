//! The parsing half of the guest's `URL`, backed by the `url` crate.

use nova_vm::ecmascript::Agent;
use nova_vm::ecmascript::ArgumentsList;
use nova_vm::ecmascript::JsResult;
use nova_vm::ecmascript::String as JsString;
use nova_vm::ecmascript::Value;
use nova_vm::engine::Bindable;
use nova_vm::engine::GcScope;
use nova_vm::engine::Scopable;

use url::Position;
use url::Url;

/// `__ow_native_url(op, input, value)`: parses (`op` = "parse", `value` = base)
/// or applies one WHATWG setter, and answers with the components as JSON.
/// A parse failure answers `null`; a failed setter leaves the URL unchanged.
pub fn native_url<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let input = args.get(1).scope(agent, gc.nogc());
    let value = args.get(2).scope(agent, gc.nogc());

    let op = args.get(0).to_string(agent, gc.reborrow()).unbind()?;
    let op = op.to_string_lossy(agent).into_owned();

    let input = input.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let input = input.to_string_lossy(agent).into_owned();

    let value = value.get(agent);
    let value = if value.is_undefined() {
        None
    } else {
        let value = value.to_string(agent, gc.reborrow()).unbind()?;

        Some(value.to_string_lossy(agent).into_owned())
    };

    let components = apply(&op, &input, value.as_deref())
        .map(|url| components(&url))
        .unwrap_or_else(|| "null".to_string());

    Ok(JsString::from_string(agent, components, gc.nogc())
        .unbind()
        .into())
}

fn apply(op: &str, input: &str, value: Option<&str>) -> Option<Url> {
    if op == "parse" {
        return match value {
            Some(base) => Url::options()
                .base_url(Some(&Url::parse(base).ok()?))
                .parse(input)
                .ok(),
            None => Url::parse(input).ok(),
        };
    }

    // Setters run against an href this module produced, so it always parses.
    let mut url = Url::parse(input).ok()?;
    let value = value.unwrap_or_default();

    if op == "href" {
        return Url::parse(value).ok();
    }

    match op {
        "protocol" => {
            let scheme = value.split(':').next().unwrap_or_default();
            let _ = url.set_scheme(scheme);
        }
        "username" => {
            let _ = url.set_username(value);
        }
        "password" => {
            let _ = url.set_password(Some(value));
        }
        "host" => set_host(&mut url, value),
        "hostname" => {
            // The spec's hostname setter drops a value carrying a port; the
            // url crate would silently keep the part before the colon.
            if !value.contains(':') {
                let _ = url.set_host(Some(value));
            }
        }
        "port" => set_port(&mut url, value),
        "pathname" => url.set_path(value),
        "search" => {
            let query = value.strip_prefix('?').unwrap_or(value);

            url.set_query(if query.is_empty() { None } else { Some(query) });
        }
        "hash" => {
            let fragment = value.strip_prefix('#').unwrap_or(value);

            url.set_fragment(if fragment.is_empty() {
                None
            } else {
                Some(fragment)
            });
        }
        _ => return None,
    }

    Some(url)
}

/// The host setter carries an optional port, which `Url::set_host` ignores.
fn set_host(url: &mut Url, value: &str) {
    let (host, port) = match value.rfind(':') {
        Some(colon) if !value.ends_with(']') => (&value[..colon], Some(&value[colon + 1..])),
        _ => (value, None),
    };

    if url.set_host(Some(host)).is_err() {
        return;
    }

    if let Some(port) = port {
        set_port(url, port);
    }
}

fn set_port(url: &mut Url, value: &str) {
    if value.is_empty() {
        let _ = url.set_port(None);

        return;
    }

    let Ok(port) = value.parse::<u16>() else {
        return;
    };

    let _ = url.set_port(Some(port));
}

fn components(url: &Url) -> String {
    let search = match url.query() {
        Some(query) if !query.is_empty() => format!("?{query}"),
        _ => String::new(),
    };
    let hash = match url.fragment() {
        Some(fragment) if !fragment.is_empty() => format!("#{fragment}"),
        _ => String::new(),
    };

    serde_json::json!({
        "href": url.as_str(),
        "origin": url.origin().ascii_serialization(),
        "protocol": format!("{}:", url.scheme()),
        "username": url.username(),
        "password": url.password().unwrap_or_default(),
        "host": &url[Position::BeforeHost..Position::AfterPort],
        "hostname": url.host_str().unwrap_or_default(),
        "port": url.port().map(|port| port.to_string()).unwrap_or_default(),
        "pathname": url.path(),
        "search": search,
        "hash": hash,
    })
    .to_string()
}
