//! The entropy source behind the guest's `crypto`, and the digests behind its
//! `subtle`.

use nova_vm::ecmascript::Agent;
use nova_vm::ecmascript::ArgumentsList;
use nova_vm::ecmascript::JsResult;
use nova_vm::ecmascript::String as JsString;
use nova_vm::ecmascript::Value;
use nova_vm::engine::Bindable;
use nova_vm::engine::GcScope;
use nova_vm::engine::Scopable;

use sha1::Sha1;
use sha2::Digest;
use sha2::Sha256;
use sha2::Sha384;
use sha2::Sha512;

/// The largest request `crypto.getRandomValues` accepts, so a guest reaching
/// past the JS layer cannot ask for an unbounded allocation.
const MAX_BYTES: i32 = 65_536;

/// `__ow_native_random_hex(count)`: hex-encoded random bytes.
pub fn native_random_hex<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let count = args.get(0).to_int32(agent, gc.reborrow()).unbind()?;
    let count = count.clamp(0, MAX_BYTES) as usize;

    let mut bytes = vec![0u8; count];

    getrandom::fill(&mut bytes).expect("the system random source is always available");

    let mut hex = String::with_capacity(count * 2);

    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }

    Ok(JsString::from_string(agent, hex, gc.nogc()).unbind().into())
}

/// `__ow_native_digest(algorithm, hex)`: the digest of those bytes, in hex, or
/// `null` for an algorithm this runtime does not have. Hex both ways because
/// nova_vm gives an embedder no way to build or read a typed array.
pub fn native_digest<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let input = args.get(1).scope(agent, gc.nogc());

    let algorithm = args.get(0).to_string(agent, gc.reborrow()).unbind()?;
    let algorithm = algorithm.to_string_lossy(agent).into_owned();

    let input = input.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let input = input.to_string_lossy(agent).into_owned();

    let Some(bytes) = from_hex(&input) else {
        return Ok(Value::Null);
    };

    let digest = match algorithm.as_str() {
        "SHA-1" => to_hex(&Sha1::digest(&bytes)),
        "SHA-256" => to_hex(&Sha256::digest(&bytes)),
        "SHA-384" => to_hex(&Sha384::digest(&bytes)),
        "SHA-512" => to_hex(&Sha512::digest(&bytes)),
        _ => return Ok(Value::Null),
    };

    Ok(JsString::from_string(agent, digest, gc.nogc())
        .unbind()
        .into())
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }

    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
