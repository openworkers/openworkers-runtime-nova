//! Compression Streams, the codecs the surface drives.
//!
//! A codec carries state between chunks, so it lives in the worker's slots and
//! the surface holds only its id: an id from one worker cannot name another's
//! codec. `finish` and `drop` both release it, or a worker that abandons a
//! stream leaves its codec behind until the worker ends.
//!
//! Bytes cross as an `ArrayBuffer`, which nova_vm lets an embedder build and
//! read; the typed array over it is made on the JavaScript side.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;

use flate2::Compression;
use flate2::write::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::write::GzDecoder;
use flate2::write::GzEncoder;
use flate2::write::ZlibDecoder;
use flate2::write::ZlibEncoder;

use nova_vm::ecmascript::Agent;
use nova_vm::ecmascript::ArgumentsList;
use nova_vm::ecmascript::ArrayBuffer;
use nova_vm::ecmascript::JsResult;
use nova_vm::ecmascript::Value;
use nova_vm::engine::Bindable;
use nova_vm::engine::GcScope;
use nova_vm::engine::Scopable;

enum Codec {
    ZlibEncode(ZlibEncoder<Vec<u8>>),
    RawEncode(DeflateEncoder<Vec<u8>>),
    GzipEncode(GzEncoder<Vec<u8>>),
    ZlibDecode(ZlibDecoder<Vec<u8>>),
    RawDecode(DeflateDecoder<Vec<u8>>),
    GzipDecode(GzDecoder<Vec<u8>>),
}

impl Codec {
    /// The three formats the standard names, in both directions. `deflate` is
    /// the zlib wrapper and `deflate-raw` the bare stream.
    fn new(format: &str, decompress: bool) -> Option<Self> {
        let level = Compression::default();

        Some(match (format, decompress) {
            ("deflate", false) => Self::ZlibEncode(ZlibEncoder::new(Vec::new(), level)),
            ("deflate-raw", false) => Self::RawEncode(DeflateEncoder::new(Vec::new(), level)),
            ("gzip", false) => Self::GzipEncode(GzEncoder::new(Vec::new(), level)),
            ("deflate", true) => Self::ZlibDecode(ZlibDecoder::new(Vec::new())),
            ("deflate-raw", true) => Self::RawDecode(DeflateDecoder::new(Vec::new())),
            ("gzip", true) => Self::GzipDecode(GzDecoder::new(Vec::new())),
            _ => return None,
        })
    }

    /// Whatever the codec can emit from `bytes`, which for a first small chunk
    /// is usually nothing.
    fn push(&mut self, bytes: &[u8]) -> std::io::Result<Vec<u8>> {
        macro_rules! push_into {
            ($writer:expr) => {{
                $writer.write_all(bytes)?;

                Ok(std::mem::take($writer.get_mut()))
            }};
        }

        match self {
            Self::ZlibEncode(w) => push_into!(w),
            Self::RawEncode(w) => push_into!(w),
            Self::GzipEncode(w) => push_into!(w),
            Self::ZlibDecode(w) => push_into!(w),
            Self::RawDecode(w) => push_into!(w),
            Self::GzipDecode(w) => push_into!(w),
        }
    }

    fn finish(self) -> std::io::Result<Vec<u8>> {
        match self {
            Self::ZlibEncode(w) => w.finish(),
            Self::RawEncode(w) => w.finish(),
            Self::GzipEncode(w) => w.finish(),
            Self::ZlibDecode(w) => w.finish(),
            Self::RawDecode(w) => w.finish(),
            Self::GzipDecode(w) => w.finish(),
        }
    }
}

/// The codecs one worker has open.
#[derive(Default)]
pub(crate) struct Codecs {
    open: RefCell<HashMap<u32, Codec>>,
    next: Cell<u32>,
}

/// The bytes of `value` from `offset` for `length`, or `None` when it is not an
/// `ArrayBuffer` or the range is not inside it.
fn bytes_of(agent: &Agent, value: Value, offset: usize, length: usize) -> Option<Vec<u8>> {
    let Value::ArrayBuffer(buffer) = value else {
        return None;
    };

    let end = offset.checked_add(length)?;

    buffer.as_slice(agent).get(offset..end).map(<[u8]>::to_vec)
}

/// A fresh `ArrayBuffer` holding `bytes`.
pub(crate) fn buffer_of<'gc>(
    agent: &mut Agent,
    bytes: &[u8],
    gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let buffer = ArrayBuffer::new(agent, bytes.len(), gc.nogc()).unbind()?;

    buffer.as_mut_slice(agent).copy_from_slice(bytes);

    Ok(buffer.into())
}

/// `__ow_native_compression(op, a, b, c, d)`.
///
/// - `start(format, decompress)`: a stream id, or `null` for a format the
///   standard does not name.
/// - `push(id, buffer, offset, length)`: the `ArrayBuffer` the codec could emit,
///   or `null` when the input is not valid for it.
/// - `finish(id)`: the rest, and the codec is released; `null` on a bad end.
/// - `drop(id)`: releases the codec.
pub fn native_compression<'gc>(
    agent: &mut Agent,
    _this: Value,
    args: ArgumentsList,
    mut gc: GcScope<'gc, '_>,
) -> JsResult<'gc, Value<'gc>> {
    let a = args.get(1).scope(agent, gc.nogc());
    let payload = args.get(2).scope(agent, gc.nogc());
    let offset = args.get(3).scope(agent, gc.nogc());
    let length = args.get(4).scope(agent, gc.nogc());

    let op = args.get(0).to_string(agent, gc.reborrow()).unbind()?;
    let op = op.to_string_lossy(agent).into_owned();

    match op.as_str() {
        "start" => {
            let format = a.get(agent).to_string(agent, gc.reborrow()).unbind()?;
            let format = format.to_string_lossy(agent).into_owned();
            let decompress = payload.get(agent) == Value::Boolean(true);

            let Some(codec) = Codec::new(&format, decompress) else {
                return Ok(Value::Null);
            };

            let codecs = &crate::worker::host_slots(agent).codecs;
            let id = codecs.next.get() + 1;

            codecs.next.set(id);
            codecs.open.borrow_mut().insert(id, codec);

            Ok(Value::from_f64(agent, f64::from(id), gc.nogc()).unbind())
        }
        "push" => {
            let id = a.get(agent).to_uint32(agent, gc.reborrow()).unbind()?;
            let offset = offset.get(agent).to_uint32(agent, gc.reborrow()).unbind()?;
            let length = length.get(agent).to_uint32(agent, gc.reborrow()).unbind()?;

            let Some(input) = bytes_of(agent, payload.get(agent), offset as usize, length as usize)
            else {
                return Ok(Value::Null);
            };

            let codecs = &crate::worker::host_slots(agent).codecs;
            let pushed = codecs
                .open
                .borrow_mut()
                .get_mut(&id)
                .map(|codec| codec.push(&input));

            // A codec that failed is of no further use, and a stream that
            // does not name one has nothing to push into.
            match pushed {
                Some(Ok(out)) => buffer_of(agent, &out, gc),
                Some(Err(_)) => {
                    codecs.open.borrow_mut().remove(&id);

                    Ok(Value::Null)
                }
                None => Ok(Value::Null),
            }
        }
        "finish" => {
            let id = a.get(agent).to_uint32(agent, gc.reborrow()).unbind()?;
            let codecs = &crate::worker::host_slots(agent).codecs;
            let codec = codecs.open.borrow_mut().remove(&id);

            match codec.map(Codec::finish) {
                Some(Ok(out)) => buffer_of(agent, &out, gc),
                _ => Ok(Value::Null),
            }
        }
        "drop" => {
            let id = a.get(agent).to_uint32(agent, gc.reborrow()).unbind()?;

            crate::worker::host_slots(agent)
                .codecs
                .open
                .borrow_mut()
                .remove(&id);

            Ok(Value::Undefined)
        }
        _ => Ok(Value::Null),
    }
}
