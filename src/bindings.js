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

  function base64Of(bytes) {
    let binary = '';

    // Spreading the whole array at once overflows the call stack.
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    }

    return btoa(binary);
  }

  // A fetch the host may refuse: it answers whatever its operations handler
  // allows. The whole body crosses at once, so a response is never streamed.
  globalThis.fetch = async function fetch(input, init) {
    const request = new Request(input, init);

    if (request.signal.aborted) {
      throw request.signal.reason;
    }

    const body = request.body === null ? undefined : base64Of(new Uint8Array(await request.arrayBuffer()));

    const answer = await call('http', '', 'fetch', {
      url: request.url,
      method: request.method,
      headers: Object.fromEntries(request.headers),
      bodyBase64: body,
    });

    const bytes = bytesOf(answer.body);

    // These statuses have no body, and a Response refuses one.
    const bodiless = answer.status === 204 || answer.status === 205 || answer.status === 304;

    return new Response(bodiless ? null : bytes, {
      status: answer.status,
      headers: answer.headers,
    });
  };

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
