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

use aes_gcm::Aes256Gcm;
use aes_gcm::KeyInit;
use aes_gcm::aead::Aead;
use aes_gcm::aead::Payload;

use hmac::Hmac;
use hmac::Mac;

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

/// `__ow_native_hmac(hash, keyHex, dataHex, tagHex)`: the tag, in hex, or
/// `"true"`/`"false"` when a tag is given to check against. Verification runs
/// here so the comparison is the constant-time one the crate does.
pub fn native_hmac<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let key = args.get(1).scope(agent, gc.nogc());
    let data = args.get(2).scope(agent, gc.nogc());
    let tag = args.get(3).scope(agent, gc.nogc());

    let hash = args.get(0).to_string(agent, gc.reborrow()).unbind()?;
    let hash = hash.to_string_lossy(agent).into_owned();

    let key = key.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let key = key.to_string_lossy(agent).into_owned();

    let data = data.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let data = data.to_string_lossy(agent).into_owned();

    let tag = tag.get(agent);
    let tag = if tag.is_undefined() {
        None
    } else {
        let tag = tag.to_string(agent, gc.reborrow()).unbind()?;

        Some(tag.to_string_lossy(agent).into_owned())
    };

    let (Some(key), Some(data)) = (from_hex(&key), from_hex(&data)) else {
        return Ok(Value::Null);
    };

    macro_rules! mac {
        ($hash:ty) => {{
            let mut mac = <Hmac<$hash> as KeyInit>::new_from_slice(&key)
                .expect("HMAC takes a key of any length");

            mac.update(&data);
            mac
        }};
    }

    if let Some(tag) = tag {
        let Some(tag) = from_hex(&tag) else {
            return Ok(Value::Boolean(false));
        };

        let held = match hash.as_str() {
            "SHA-256" => mac!(Sha256).verify_slice(&tag).is_ok(),
            "SHA-384" => mac!(Sha384).verify_slice(&tag).is_ok(),
            "SHA-512" => mac!(Sha512).verify_slice(&tag).is_ok(),
            "SHA-1" => mac!(Sha1).verify_slice(&tag).is_ok(),
            _ => return Ok(Value::Null),
        };

        return Ok(Value::Boolean(held));
    }

    let answer = match hash.as_str() {
        "SHA-256" => to_hex(&mac!(Sha256).finalize().into_bytes()),
        "SHA-384" => to_hex(&mac!(Sha384).finalize().into_bytes()),
        "SHA-512" => to_hex(&mac!(Sha512).finalize().into_bytes()),
        "SHA-1" => to_hex(&mac!(Sha1).finalize().into_bytes()),
        _ => return Ok(Value::Null),
    };

    Ok(JsString::from_string(agent, answer, gc.nogc())
        .unbind()
        .into())
}

/// `__ow_native_aes_gcm(op, keyHex, ivHex, dataHex)`: the ciphertext with its
/// tag appended, or the plaintext, in hex. `null` when the key or the nonce is
/// the wrong size, and `false` when the tag does not hold.
pub fn native_aes_gcm<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let key = args.get(1).scope(agent, gc.nogc());
    let iv = args.get(2).scope(agent, gc.nogc());
    let data = args.get(3).scope(agent, gc.nogc());

    let op = args.get(0).to_string(agent, gc.reborrow()).unbind()?;
    let op = op.to_string_lossy(agent).into_owned();

    let key = key.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let key = key.to_string_lossy(agent).into_owned();

    let iv = iv.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let iv = iv.to_string_lossy(agent).into_owned();

    let data = data.get(agent).to_string(agent, gc.reborrow()).unbind()?;
    let data = data.to_string_lossy(agent).into_owned();

    let (Some(key), Some(iv), Some(data)) = (from_hex(&key), from_hex(&iv), from_hex(&data)) else {
        return Ok(Value::Null);
    };

    // A slice no longer converts on its own; the sizes are the ones AES-256-GCM
    // takes, so the conversion is the check.
    let (Ok(key), Ok(iv)) = (
        <&[u8; 32]>::try_from(key.as_slice()),
        <&[u8; 12]>::try_from(iv.as_slice()),
    ) else {
        return Ok(Value::Null);
    };

    let cipher = Aes256Gcm::new(key.into());
    let nonce = iv.into();
    let payload = Payload {
        msg: &data,
        aad: &[],
    };

    let out = match op.as_str() {
        "encrypt" => cipher.encrypt(nonce, payload).ok(),
        "decrypt" => cipher.decrypt(nonce, payload).ok(),
        _ => return Ok(Value::Null),
    };

    let Some(out) = out else {
        return Ok(Value::Boolean(false));
    };

    Ok(JsString::from_string(agent, to_hex(&out), gc.nogc())
        .unbind()
        .into())
}
