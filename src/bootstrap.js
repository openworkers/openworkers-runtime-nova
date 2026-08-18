// Nova is a pure ECMAScript engine: the service-worker globals and the
// host boundary (JSON strings via __ow_native_* builtins) are defined here.
(function () {
  'use strict';

  const handlers = [];

  function headersToPairs(headers) {
    const pairs = [];

    if (!headers) {
      return pairs;
    }

    if (typeof headers.entries === 'function') {
      for (const [key, value] of headers.entries()) {
        pairs.push([String(key), String(value)]);
      }
    } else if (Array.isArray(headers)) {
      for (const pair of headers) {
        pairs.push([String(pair[0]), String(pair[1])]);
      }
    } else {
      for (const key of Object.keys(headers)) {
        pairs.push([String(key), String(headers[key])]);
      }
    }

    return pairs;
  }

  class Request {
    constructor(url, init) {
      init = init || {};
      this.url = String(url);
      this.method = init.method ? String(init.method).toUpperCase() : 'GET';
      this.headers = init.headers || {};
      this.body = init.body === undefined ? null : init.body;
    }

    text() {
      return Promise.resolve(this.body === null ? '' : String(this.body));
    }

    json() {
      return this.text().then(JSON.parse);
    }
  }

  class Response {
    constructor(body, init) {
      init = init || {};
      this.body = body === undefined || body === null ? '' : String(body);
      this.status = init.status === undefined ? 200 : init.status | 0;
      this.headers = init.headers || {};
    }

    text() {
      return Promise.resolve(this.body);
    }

    json() {
      return this.text().then(JSON.parse);
    }
  }

  function makeLog(level) {
    return function () {
      const parts = [];

      for (const arg of arguments) {
        if (typeof arg === 'string') {
          parts.push(arg);
        } else {
          try {
            parts.push(JSON.stringify(arg));
          } catch (_e) {
            parts.push(String(arg));
          }
        }
      }

      __ow_native_log(String(level), parts.join(' '));
    };
  }

  globalThis.Request = Request;
  globalThis.Response = Response;
  globalThis.console = {
    log: makeLog('log'),
    info: makeLog('info'),
    warn: makeLog('warn'),
    error: makeLog('error'),
    debug: makeLog('debug'),
  };

  globalThis.addEventListener = function (type, handler) {
    if (type === 'fetch') {
      handlers.push(handler);
    }
  };

  globalThis.__ow_dispatch = function (requestJson) {
    const data = JSON.parse(requestJson);
    const request = new Request(data.url, {
      method: data.method,
      headers: data.headers,
      body: data.body,
    });
    const event = {
      type: 'fetch',
      request: request,
      _response: null,
      respondWith(response) {
        this._response = response;
      },
    };

    (async function () {
      if (handlers.length === 0) {
        throw new Error('no fetch handler registered');
      }

      for (const handler of handlers) {
        await handler(event);
      }

      const response = await event._response;

      if (!response) {
        throw new Error('fetch handler did not call respondWith()');
      }

      return {
        status: response.status | 0,
        headers: headersToPairs(response.headers),
        body:
          response.body === undefined || response.body === null
            ? ''
            : String(response.body),
      };
    })().then(
      function (response) {
        __ow_native_respond(JSON.stringify({ response: response }));
      },
      function (error) {
        const message =
          error instanceof Error && error.stack ? error.stack : String(error);
        __ow_native_respond(JSON.stringify({ error: message }));
      }
    );
  };
})();
