// The host side of the incoming request: the surface owns the interface, this
// is only the lift from the wire.
(function () {
  'use strict';

  // A request HTTP already delivered keeps its body whatever its method is,
  // which is what `_fromHost` tells the surface.
  globalThis.__ow_request_from_wire = function (data) {
    const init = { method: data.method, headers: data.headers, _fromHost: true };

    if (data.body !== null && data.body !== undefined) {
      init.body = String(data.body);
    }

    return new globalThis.Request(data.url, init);
  };
})();
