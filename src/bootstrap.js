// Nova is a pure ECMAScript engine: the service-worker globals and the
// host boundary (JSON strings via __ow_native_* builtins) are defined here.
(function () {
  'use strict';

  const fetchHandlers = [];
  const taskHandlers = [];

  // Lone surrogates reach the host as \uXXXX escapes that its JSON parser rejects.
  function wireText(value) {
    return String(value).toWellFormed();
  }

  // Property names never reach a JSON.stringify replacer, so rebuild objects.
  function wireJson(_key, value) {
    if (typeof value === 'string') {
      return wireText(value);
    }

    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
      return value;
    }

    const clean = {};

    for (const key of Object.keys(value)) {
      clean[wireText(key)] = value[key];
    }

    return clean;
  }

  // Checked at the wire boundary, so duck-typed responses cannot skip it.
  function wireStatus(value) {
    const status = Number(value);

    if (!Number.isInteger(status) || status < 100 || status > 599) {
      throw new RangeError('response status out of range: ' + String(value));
    }

    return status;
  }

  function headersToPairs(headers) {
    const pairs = [];

    if (!headers) {
      return pairs;
    }

    // Array check must come first: arrays also have an entries() method.
    if (Array.isArray(headers)) {
      for (const pair of headers) {
        pairs.push([wireText(pair[0]), wireText(pair[1])]);
      }
    } else if (typeof headers.entries === 'function') {
      for (const [key, value] of headers.entries()) {
        pairs.push([wireText(key), wireText(value)]);
      }
    } else {
      for (const key of Object.keys(headers)) {
        pairs.push([wireText(key), wireText(headers[key])]);
      }
    }

    return pairs;
  }

  class DOMException extends Error {
    constructor(message, name) {
      super(message);
      this.name = name === undefined ? 'Error' : String(name);
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

  globalThis.DOMException = DOMException;
  globalThis.console = {
    log: makeLog('log'),
    info: makeLog('info'),
    warn: makeLog('warn'),
    error: makeLog('error'),
    debug: makeLog('debug'),
  };

  // Nothing to fire an error event on, so a throwing callback is logged.
  globalThis.queueMicrotask = function (callback) {
    if (typeof callback !== 'function') {
      throw new TypeError('queueMicrotask requires a function');
    }

    Promise.resolve().then(function () {
      try {
        callback();
      } catch (error) {
        console.error('uncaught exception in queueMicrotask: ' + String(error));
      }
    });
  };

  globalThis.addEventListener = function (type, handler) {
    if (type === 'fetch') {
      fetchHandlers.push(handler);
    } else if (type === 'task') {
      taskHandlers.push(handler);
    }
  };

  // The host tells an event outcome from a failed dispatch by the key it gets.
  function settle(promise) {
    promise.then(
      function (value) {
        __ow_native_respond(JSON.stringify({ value: value }, wireJson));
      },
      function (error) {
        const message =
          error instanceof Error && error.stack ? error.stack : String(error);
        __ow_native_respond(JSON.stringify({ error: wireText(message) }));
      }
    );
  }

  globalThis.__ow_dispatch = function (requestJson) {
    const data = JSON.parse(requestJson);
    const request = __ow_request_from_wire(data);
    const background = [];
    const event = {
      type: 'fetch',
      request: request,
      _response: null,
      respondWith(response) {
        this._response = response;
      },
      waitUntil(promise) {
        background.push(promise);
      },
    };

    const module = globalThis.default;
    const hasModuleFetch = module && typeof module.fetch === 'function';

    settle((async function () {
      if (fetchHandlers.length === 0 && !hasModuleFetch) {
        throw new Error('no fetch handler registered');
      }

      let returned;

      if (fetchHandlers.length > 0) {
        for (const handler of fetchHandlers) {
          await handler(event);
        }
      } else {
        returned = await module.fetch(request, globalThis.env, {
          waitUntil: event.waitUntil,
          passThroughOnException() {},
        });
      }

      const response = await (event._response === null ? returned : event._response);

      if (!response) {
        throw new Error(
          fetchHandlers.length > 0
            ? 'fetch handler did not call respondWith()'
            : 'fetch handler returned no response'
        );
      }

      // A duck-typed response is still accepted, so read a body either way.
      const raw =
        typeof response.text === 'function' ? await response.text() : response.body;

      // A rejected background promise must not sink a response already produced.
      await Promise.all(background).catch(function () {});

      return {
        status: wireStatus(response.status),
        headers: headersToPairs(response.headers),
        body: raw === undefined || raw === null ? '' : wireText(raw),
      };
    })());
  };

  function toTaskResult(value) {
    if (value !== null && typeof value === 'object' && 'success' in value) {
      return {
        success: value.success !== false,
        data: value.data,
        error: value.error,
      };
    }

    return { success: true, data: value };
  }

  globalThis.__ow_dispatch_task = function (initJson) {
    const init = JSON.parse(initJson);
    const background = [];
    let responded = false;
    let responseValue;

    const event = {
      type: 'task',
      taskId: init.taskId,
      payload: init.payload,
      source: init.source,
      attempt: init.attempt,
      scheduledTime: init.scheduledTime,
      respondWith(value) {
        responded = true;
        responseValue = value;
      },
      waitUntil(promise) {
        background.push(promise);
      },
    };

    const module = globalThis.default;
    const hasModuleTask = module && typeof module.task === 'function';

    settle((async function () {
      if (taskHandlers.length === 0 && !hasModuleTask) {
        throw new Error('no task handler registered');
      }

      // A task that throws is a failed task, not a failed dispatch.
      try {
        let returned;

        if (taskHandlers.length > 0) {
          for (const handler of taskHandlers) {
            returned = await handler(event);
          }
        } else {
          returned = await module.task(event, globalThis.env, {
            waitUntil: event.waitUntil,
          });
        }

        const result = toTaskResult(responded ? await responseValue : returned);

        // A rejected background promise must not sink a result already produced.
        await Promise.all(background).catch(function () {});

        return result;
      } catch (error) {
        return {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        };
      }
    })());
  };
})();
