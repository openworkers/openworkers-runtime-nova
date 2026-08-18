//! The entropy source behind the guest's `crypto`.

use nova_vm::ecmascript::Agent;
use nova_vm::ecmascript::ArgumentsList;
use nova_vm::ecmascript::JsResult;
use nova_vm::ecmascript::String as JsString;
use nova_vm::ecmascript::Value;
use nova_vm::engine::Bindable;
use nova_vm::engine::GcScope;

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
