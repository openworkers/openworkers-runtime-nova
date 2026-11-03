// Ops the shared surface reads, answered from JavaScript over the native
// builtins this runtime already had.
(function () {
  'use strict';

  const ops = globalThis.__ow;

  // The parser answers JSON: nova_vm gives an embedder no way to build an
  // object, so the shape crosses the boundary as text.
  ops.urlParse = function (input, base) {
    return JSON.parse(
      __ow_native_url('parse', input, base === null ? undefined : base)
    );
  };

  ops.urlUpdate = function (href, part, value) {
    return JSON.parse(__ow_native_url(part, href, value));
  };

  // The pattern grammar answers JSON for the same reason: the components cross
  // as text and the surface builds its own objects from them.
  ops.urlPatternParse = function (input, base) {
    return JSON.parse(
      __ow_native_url_pattern('parse', JSON.stringify(input), base === null ? undefined : base)
    );
  };

  ops.urlPatternProcessInput = function (input, base) {
    return JSON.parse(
      __ow_native_url_pattern('input', JSON.stringify(input), base === null ? undefined : base)
    );
  };

  // Compression Streams. The host answers an ArrayBuffer, the one binary shape
  // it can build, and the typed array over it is made here.
  ops.compressionStart = function (format, decompress) {
    const id = __ow_native_compression('start', format, decompress);

    if (id === null) {
      throw new TypeError("Unsupported compression format: '" + format + "'");
    }

    return id;
  };

  ops.compressionPush = function (id, chunk) {
    const bytes = ArrayBuffer.isView(chunk) ? chunk : new Uint8Array(chunk);
    const out = __ow_native_compression('push', id, bytes.buffer, bytes.byteOffset, bytes.byteLength);

    if (out === null) {
      throw new TypeError('The compressed data is not valid');
    }

    return new Uint8Array(out);
  };

  ops.compressionFinish = function (id) {
    const out = __ow_native_compression('finish', id);

    if (out === null) {
      throw new TypeError('The compressed data is not valid');
    }

    return new Uint8Array(out);
  };

  ops.compressionDrop = function (id) {
    __ow_native_compression('drop', id);
  };
})();
