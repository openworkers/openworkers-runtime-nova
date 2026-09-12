// The host side of the response headers: the surface owns the interface, this
// is only the walk that puts it on the wire.
(function () {
  'use strict';

  // The wire keeps the order the handler set: the standard's sort governs the JS
  // iterator, not HTTP. The constructor is what validates a plain-object init.
  globalThis.__ow_headers_to_wire = function (init) {
    const headers = init instanceof globalThis.Headers ? init : new globalThis.Headers(init);
    const pairs = [];

    for (const [name, value] of headers._map) {
      if (Array.isArray(value)) {
        for (const one of value) {
          pairs.push([name, one]);
        }
      } else {
        pairs.push([name, value]);
      }
    }

    return pairs;
  };
})();
