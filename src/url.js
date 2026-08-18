// URL and URLSearchParams. Parsing and the setters run in the host, so the
// WHATWG corner cases live in the url crate rather than in this file.
(function () {
  'use strict';

  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  // application/x-www-form-urlencoded keeps these bytes as-is.
  const SAFE_BYTES = new Set(
    Array.from('*-._0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz')
      .map((character) => character.charCodeAt(0))
  );

  const HEX = '0123456789ABCDEF';

  function hexValue(byte) {
    if (byte >= 0x30 && byte <= 0x39) {
      return byte - 0x30;
    }

    if (byte >= 0x41 && byte <= 0x46) {
      return byte - 0x41 + 10;
    }

    if (byte >= 0x61 && byte <= 0x66) {
      return byte - 0x61 + 10;
    }

    return -1;
  }

  function decodeComponent(text) {
    const bytes = encoder.encode(text);
    const decoded = [];

    for (let i = 0; i < bytes.length; i++) {
      if (bytes[i] === 0x2b) {
        decoded.push(0x20);

        continue;
      }

      const high = i + 2 < bytes.length ? hexValue(bytes[i + 1]) : -1;
      const low = high < 0 ? -1 : hexValue(bytes[i + 2]);

      if (bytes[i] === 0x25 && low >= 0) {
        decoded.push(high * 16 + low);
        i += 2;

        continue;
      }

      decoded.push(bytes[i]);
    }

    return decoder.decode(new Uint8Array(decoded));
  }

  function encodeComponent(text) {
    const bytes = encoder.encode(text);
    let encoded = '';

    for (const byte of bytes) {
      if (SAFE_BYTES.has(byte)) {
        encoded += String.fromCharCode(byte);
      } else if (byte === 0x20) {
        encoded += '+';
      } else {
        encoded += '%' + HEX[byte >> 4] + HEX[byte & 15];
      }
    }

    return encoded;
  }

  function parseQuery(query) {
    const text = query.startsWith('?') ? query.slice(1) : query;
    const pairs = [];

    for (const segment of text.split('&')) {
      if (segment === '') {
        continue;
      }

      const separator = segment.indexOf('=');
      const name = separator < 0 ? segment : segment.slice(0, separator);
      const value = separator < 0 ? '' : segment.slice(separator + 1);

      pairs.push([decodeComponent(name), decodeComponent(value)]);
    }

    return pairs;
  }

  function callNative(op, input, value) {
    return JSON.parse(__ow_native_url(op, input, value));
  }

  // Set by the class bodies below, so URL and URLSearchParams can reach each
  // other's private state without exposing it to the guest.
  let attachToUrl;
  let replacePairs;
  let assignSearch;

  class URLSearchParams {
    #pairs = [];
    #url = null;

    static {
      attachToUrl = (params, url) => {
        params.#url = url;
      };
      replacePairs = (params, query) => {
        params.#pairs = parseQuery(query);
      };
    }

    constructor(init) {
      if (init === undefined || init === null) {
        return;
      }

      if (typeof init === 'string') {
        this.#pairs = parseQuery(init);

        return;
      }

      if (init instanceof URLSearchParams) {
        this.#pairs = init.#pairs.map((pair) => [pair[0], pair[1]]);

        return;
      }

      if (typeof init[Symbol.iterator] === 'function') {
        for (const entry of init) {
          const pair = Array.from(entry);

          if (pair.length !== 2) {
            throw new TypeError('URLSearchParams: each entry needs two values');
          }

          this.#pairs.push([String(pair[0]), String(pair[1])]);
        }

        return;
      }

      for (const key of Object.keys(init)) {
        this.#pairs.push([String(key), String(init[key])]);
      }
    }

    get size() {
      return this.#pairs.length;
    }

    append(name, value) {
      this.#pairs.push([String(name), String(value)]);
      this.#notify();
    }

    delete(name, value) {
      const target = String(name);
      const wanted = value === undefined ? undefined : String(value);

      this.#pairs = this.#pairs.filter(
        (pair) => pair[0] !== target || (wanted !== undefined && pair[1] !== wanted)
      );
      this.#notify();
    }

    get(name) {
      const target = String(name);

      for (const pair of this.#pairs) {
        if (pair[0] === target) {
          return pair[1];
        }
      }

      return null;
    }

    getAll(name) {
      const target = String(name);

      return this.#pairs.filter((pair) => pair[0] === target).map((pair) => pair[1]);
    }

    has(name, value) {
      const target = String(name);
      const wanted = value === undefined ? undefined : String(value);

      return this.#pairs.some(
        (pair) => pair[0] === target && (wanted === undefined || pair[1] === wanted)
      );
    }

    set(name, value) {
      const target = String(name);
      const replacement = String(value);
      let seen = false;

      this.#pairs = this.#pairs.filter((pair) => {
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
        this.#pairs.push([target, replacement]);
      }

      this.#notify();
    }

    sort() {
      this.#pairs.sort((left, right) => {
        if (left[0] === right[0]) {
          return 0;
        }

        return left[0] < right[0] ? -1 : 1;
      });
      this.#notify();
    }

    forEach(callback, thisArg) {
      for (const pair of this.#pairs.slice()) {
        callback.call(thisArg, pair[1], pair[0], this);
      }
    }

    *entries() {
      for (const pair of this.#pairs.slice()) {
        yield [pair[0], pair[1]];
      }
    }

    *keys() {
      for (const pair of this.#pairs.slice()) {
        yield pair[0];
      }
    }

    *values() {
      for (const pair of this.#pairs.slice()) {
        yield pair[1];
      }
    }

    [Symbol.iterator]() {
      return this.entries();
    }

    toString() {
      return this.#pairs
        .map((pair) => encodeComponent(pair[0]) + '=' + encodeComponent(pair[1]))
        .join('&');
    }

    #notify() {
      if (this.#url !== null) {
        assignSearch(this.#url, this.toString());
      }
    }
  }

  class URL {
    #parts;
    #params = null;

    static {
      assignSearch = (url, query) => {
        url.#parts = callNative('search', url.#parts.href, query);
      };
    }

    constructor(input, base) {
      const parts = callNative(
        'parse',
        String(input),
        base === undefined ? undefined : String(base)
      );

      if (parts === null) {
        throw new TypeError('Invalid URL: ' + String(input));
      }

      this.#parts = parts;
    }

    static canParse(input, base) {
      return (
        callNative('parse', String(input), base === undefined ? undefined : String(base)) !==
        null
      );
    }

    static parse(input, base) {
      try {
        return new URL(input, base);
      } catch (error) {
        return null;
      }
    }

    get href() {
      return this.#parts.href;
    }

    set href(value) {
      const parts = callNative('href', this.#parts.href, String(value));

      if (parts === null) {
        throw new TypeError('Invalid URL: ' + String(value));
      }

      this.#parts = parts;
      this.#refreshParams();
    }

    get origin() {
      return this.#parts.origin;
    }

    get protocol() {
      return this.#parts.protocol;
    }

    set protocol(value) {
      this.#update('protocol', value);
    }

    get username() {
      return this.#parts.username;
    }

    set username(value) {
      this.#update('username', value);
    }

    get password() {
      return this.#parts.password;
    }

    set password(value) {
      this.#update('password', value);
    }

    get host() {
      return this.#parts.host;
    }

    set host(value) {
      this.#update('host', value);
    }

    get hostname() {
      return this.#parts.hostname;
    }

    set hostname(value) {
      this.#update('hostname', value);
    }

    get port() {
      return this.#parts.port;
    }

    set port(value) {
      this.#update('port', value);
    }

    get pathname() {
      return this.#parts.pathname;
    }

    set pathname(value) {
      this.#update('pathname', value);
    }

    get search() {
      return this.#parts.search;
    }

    set search(value) {
      this.#update('search', value);
      this.#refreshParams();
    }

    get searchParams() {
      if (this.#params === null) {
        this.#params = new URLSearchParams(this.#parts.search);
        attachToUrl(this.#params, this);
      }

      return this.#params;
    }

    get hash() {
      return this.#parts.hash;
    }

    set hash(value) {
      this.#update('hash', value);
    }

    toString() {
      return this.#parts.href;
    }

    toJSON() {
      return this.#parts.href;
    }

    // A setter the spec cannot apply leaves the URL as it was.
    #update(part, value) {
      const parts = callNative(part, this.#parts.href, String(value));

      if (parts !== null) {
        this.#parts = parts;
      }
    }

    #refreshParams() {
      if (this.#params !== null) {
        replacePairs(this.#params, this.#parts.search);
      }
    }
  }

  globalThis.URL = URL;
  globalThis.URLSearchParams = URLSearchParams;
})();
