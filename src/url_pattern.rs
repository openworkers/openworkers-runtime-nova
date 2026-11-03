//! The `URLPattern` grammar, answered by `rust-urlpattern`.
//!
//! Only the parsing crosses: a pattern comes back as its components, each with
//! a matcher and an ECMAScript regexp source, and the surface matches with the
//! engine's own `RegExp`. Nothing is held on this side between calls. Both
//! directions are JSON, as for `URL`.

use nova_vm::ecmascript::Agent;
use nova_vm::ecmascript::ArgumentsList;
use nova_vm::ecmascript::JsResult;
use nova_vm::ecmascript::String as JsString;
use nova_vm::ecmascript::Value;
use nova_vm::engine::Bindable;
use nova_vm::engine::GcScope;
use nova_vm::engine::Scopable;

use urlpattern::quirks;
use urlpattern::quirks::StringOrInit;

/// `__ow_native_url_pattern(op, inputJson, base)`: `op` is `"parse"` or
/// `"input"`. Answers JSON, or `"null"` when the pattern does not parse or the
/// input is not a url any pattern could match.
pub fn native_url_pattern<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let input = args.get(1).scope(agent, gc.nogc());
    let base = args.get(2).scope(agent, gc.nogc());

    let op = args.get(0).to_string(agent, gc.reborrow()).unbind()?;
    let op = op.to_string_lossy(agent).into_owned();

    let input = input.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let input = input.to_string_lossy(agent).into_owned();

    let base = base.get(agent);
    let base = if base.is_undefined() {
        None
    } else {
        let base = base.to_string(agent, gc.reborrow()).unbind()?;

        Some(base.to_string_lossy(agent).into_owned())
    };

    let answer = answer(&op, &input, base.as_deref()).unwrap_or_else(|| "null".to_string());

    Ok(JsString::from_string(agent, answer, gc.nogc())
        .unbind()
        .into())
}

fn answer(op: &str, input: &str, base: Option<&str>) -> Option<String> {
    let input: StringOrInit = serde_json::from_str(input).ok()?;

    match op {
        "parse" => {
            let init = quirks::process_construct_pattern_input(input, base).ok()?;

            // EcmaRegexp rather than the regex crate: what crosses is the
            // pattern's source, and the surface matches with the engine's own
            // RegExp.
            let pattern =
                quirks::parse_pattern::<quirks::EcmaRegexp>(init, Default::default()).ok()?;

            serde_json::to_string(&pattern).ok()
        }
        "input" => {
            let (match_input, _) = quirks::process_match_input(input, base).ok()??;
            let match_input = quirks::parse_match_input(match_input)?;

            serde_json::to_string(&match_input).ok()
        }
        _ => None,
    }
}
