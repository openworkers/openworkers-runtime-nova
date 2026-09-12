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
})();
