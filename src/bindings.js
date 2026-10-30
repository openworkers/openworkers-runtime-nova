// The guest side of `env`: a binding call is a promise the host settles later.
(function () {
  'use strict';

  const pending = new Map();
  let next = 1;

  function call(kind, name, method, params) {
    return new Promise((resolve, reject) => {
      const id = next++;

      pending.set(id, { resolve: resolve, reject: reject });
      __ow_native_binding(id, kind, name, method, JSON.stringify(params || {}));
    });
  }

  globalThis.__ow_settle_binding = function (id, ok, payload) {
    const call = pending.get(id);

    if (call === undefined) {
      return;
    }

    pending.delete(id);

    if (ok) {
      call.resolve(JSON.parse(payload));
    } else {
      call.reject(new Error(payload));
    }
  };

  function bytesOf(base64) {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);

    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }

    return bytes;
  }

  globalThis.__ow_assets_binding = function (name) {
    return {
      fetch(input, init) {
        // A bare path is a path on the host of the binding, which new Request would
        // refuse. Resolving it against a base also removes dot segments, so the
        // host never gets a `..` from the guest.
        const request =
          input instanceof Request
            ? input
            : {
                url: new URL(String(input), 'http://localhost/').href,
                method: (init && init.method) || 'GET',
                headers: new Headers(init && init.headers),
              };

        return call('fetch', name, 'fetch', {
          url: request.url,
          method: request.method,
          headers: Object.fromEntries(request.headers),
        }).then(
          (answer) =>
            new Response(bytesOf(answer.body), {
              status: answer.status,
              headers: answer.headers,
            })
        );
      },
    };
  };

  globalThis.__ow_database_binding = function (name) {
    return {
      query(sql, params) {
        return call('query', name, 'query', { sql: String(sql), params: params || [] }).then(
          (answer) => answer.rows
        );
      },
    };
  };

  globalThis.__ow_invoke_binding = function (name) {
    return {
      invoke(command, args) {
        if (typeof command !== 'string') {
          return Promise.reject(new TypeError('invoke: the command is not a string'));
        }

        return call('invoke', name, 'invoke', { command: command, args: args });
      },
    };
  };

  globalThis.__ow_absent_binding = function (name) {
    const refuse = () => {
      throw new Error(name + ' is a binding this runtime does not serve');
    };

    return { fetch: refuse, query: refuse, get: refuse, put: refuse, delete: refuse };
  };
})();
