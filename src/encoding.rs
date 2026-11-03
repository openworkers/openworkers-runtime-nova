//! The legacy encodings a `TextDecoder` is built for, backed by `encoding_rs`.
//!
//! UTF-8 stays in JavaScript, where it keeps its state between `stream` calls.
//! Every other label comes here, and each call decodes the bytes it is given
//! whole, so a multi-byte character split across two `stream` calls decodes as
//! U+FFFD. A single-byte encoding is exact.

use nova_vm::ecmascript::Agent;
use nova_vm::ecmascript::ArgumentsList;
use nova_vm::ecmascript::JsResult;
use nova_vm::ecmascript::String as JsString;
use nova_vm::ecmascript::Value;
use nova_vm::engine::Bindable;
use nova_vm::engine::GcScope;
use nova_vm::engine::Scopable;

use encoding_rs::Encoding;

use crate::crypto::from_hex;

/// `__ow_native_encoding(op, text, hex, fatal, ignoreBOM)`.
///
/// - `name(label)`: the standard's name for that label, or `null` for a label
///   the Encoding Standard does not list.
/// - `decode(name, hex, fatal, ignoreBOM)`: the text, or `false` when `fatal` is
///   set and the bytes are malformed. A byte order mark of the encoding's own is
///   dropped unless `ignoreBOM` is set; only the UTF-16 encodings have one here.
pub fn native_encoding<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let text = args.get(1).scope(agent, gc.nogc());
    let hex = args.get(2).scope(agent, gc.nogc());
    let fatal = args.get(3).scope(agent, gc.nogc());
    let ignore_bom = args.get(4).scope(agent, gc.nogc());

    let op = args.get(0).to_string(agent, gc.reborrow()).unbind()?;
    let op = op.to_string_lossy(agent).into_owned();

    let text = text.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let text = text.to_string_lossy(agent).into_owned();

    let Some(encoding) = Encoding::for_label(text.trim().as_bytes()) else {
        return Ok(Value::Null);
    };

    if op == "name" {
        return Ok(JsString::from_str(agent, encoding.name(), gc.nogc())
            .unbind()
            .into());
    }

    let hex = hex.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let hex = hex.to_string_lossy(agent).into_owned();
    let fatal = fatal.get(agent) == Value::Boolean(true);
    let ignore_bom = ignore_bom.get(agent) == Value::Boolean(true);

    let Some(bytes) = from_hex(&hex) else {
        return Ok(Value::Null);
    };

    // Done on the bytes, since the decoder below is told to leave BOMs alone.
    let bom: &[u8] = if encoding == encoding_rs::UTF_16LE {
        &[0xff, 0xfe]
    } else if encoding == encoding_rs::UTF_16BE {
        &[0xfe, 0xff]
    } else {
        &[]
    };

    let bytes = match bytes.strip_prefix(bom) {
        Some(rest) if !ignore_bom && !bom.is_empty() => rest.to_vec(),
        _ => bytes,
    };

    let decoded = if fatal {
        encoding
            .decode_without_bom_handling_and_without_replacement(&bytes)
            .map(|text| text.into_owned())
    } else {
        Some(encoding.decode_without_bom_handling(&bytes).0.into_owned())
    };

    Ok(match decoded {
        Some(decoded) => JsString::from_string(agent, decoded, gc.nogc())
            .unbind()
            .into(),
        None => Value::Boolean(false),
    })
}
