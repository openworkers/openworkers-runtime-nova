// The WHATWG Headers class: an ordered list of name/value pairs, iterated
// sorted and combined the way the Fetch standard prescribes.
(function () {
  'use strict';

  const TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
  const FORBIDDEN = /[\x00\r\n]/;

  function normalizeName(name) {
    const text = String(name);

    if (!TOKEN.test(text)) {
      throw new TypeError('invalid header name: ' + text);
    }

    return text.toLowerCase();
  }

  function normalizeValue(value) {
    const text = String(value)
      .replace(/^[\t\n\r ]+/, '')
      .replace(/[\t\n\r ]+$/, '');

    if (FORBIDDEN.test(text)) {
      throw new TypeError('invalid header value: ' + text);
    }

    return text;
  }

  // Assigned by the class body, the only scope the private header list has.
  let toWire;

  class Headers {
    #list = [];

    constructor(init) {
      if (init === undefined || init === null) {
        return;
      }

      if (init instanceof Headers) {
        for (const pair of init.#list) {
          this.#list.push([pair[0], pair[1]]);
        }

        return;
      }

      if (typeof init[Symbol.iterator] === 'function') {
        for (const entry of init) {
          const pair = Array.from(entry);

          if (pair.length !== 2) {
            throw new TypeError('Headers: each entry needs a name and a value');
          }

          this.append(pair[0], pair[1]);
        }

        return;
      }

      for (const key of Object.keys(init)) {
        this.append(key, init[key]);
      }
    }

    append(name, value) {
      this.#list.push([normalizeName(name), normalizeValue(value)]);
    }

    delete(name) {
      const target = normalizeName(name);

      this.#list = this.#list.filter((pair) => pair[0] !== target);
    }

    get(name) {
      const target = normalizeName(name);
      const values = this.#list
        .filter((pair) => pair[0] === target)
        .map((pair) => pair[1]);

      return values.length === 0 ? null : values.join(', ');
    }

    getSetCookie() {
      return this.#list
        .filter((pair) => pair[0] === 'set-cookie')
        .map((pair) => pair[1]);
    }

    has(name) {
      const target = normalizeName(name);

      return this.#list.some((pair) => pair[0] === target);
    }

    set(name, value) {
      const target = normalizeName(name);
      const replacement = normalizeValue(value);
      let seen = false;

      this.#list = this.#list.filter((pair) => {
        if (pair[0] !== target) {
          return true;
        }

        if (seen) {
          return false;
        }

        seen = true;
        pair[1] = replacement;

        return true;
      });

      if (!seen) {
        this.#list.push([target, replacement]);
      }
    }

    forEach(callback, thisArg) {
      for (const pair of this.#combined()) {
        callback.call(thisArg, pair[1], pair[0], this);
      }
    }

    *entries() {
      yield* this.#combined();
    }

    *keys() {
      for (const pair of this.#combined()) {
        yield pair[0];
      }
    }

    *values() {
      for (const pair of this.#combined()) {
        yield pair[1];
      }
    }

    [Symbol.iterator]() {
      return this.entries();
    }

    #names() {
      return [...new Set(this.#list.map((pair) => pair[0]))];
    }

    // One entry per name, except set-cookie which the standard keeps split
    // so a client can read the cookies apart.
    #grouped(names) {
      const grouped = [];

      for (const name of names) {
        if (name === 'set-cookie') {
          for (const value of this.getSetCookie()) {
            grouped.push([name, value]);
          }

          continue;
        }

        grouped.push([name, this.get(name)]);
      }

      return grouped;
    }

    // The standard sorts what JS iterates.
    #combined() {
      return this.#grouped(this.#names().sort());
    }

    // The wire keeps the order the handler set, as the other openworkers
    // runtimes do; the standard's sort governs the JS iterator, not HTTP.
    static {
      toWire = (headers) => headers.#grouped(headers.#names());
    }
  }

  globalThis.Headers = Headers;

  // Every response shape reaches the host through here, so none can skip
  // the name and value validation the Headers constructor does.
  globalThis.__ow_headers_to_wire = function (init) {
    return toWire(init instanceof Headers ? init : new Headers(init));
  };
})();
