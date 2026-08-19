// Request and Response. Bodies are buffered: a string or bytes, never a
// stream, since the runtime has no ReadableStream yet.
(function () {
  'use strict';

  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  const METHOD_TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
  const NORMALIZED_METHODS = new Set([
    'DELETE',
    'GET',
    'HEAD',
    'OPTIONS',
    'POST',
    'PUT',
  ]);
  const REDIRECT_STATUSES = new Set([301, 302, 303, 307, 308]);
  const NULL_BODY_STATUSES = new Set([204, 205, 304]);

  function normalizeMethod(method) {
    const text = String(method);

    if (!METHOD_TOKEN.test(text)) {
      throw new TypeError('invalid request method: ' + text);
    }

    const upper = text.toUpperCase();

    return NORMALIZED_METHODS.has(upper) ? upper : text;
  }

  // The body plus the content type it implies; an explicit header wins.
  function extractBody(source) {
    if (source === null || source === undefined) {
      return { body: null, type: null };
    }

    if (typeof source === 'string') {
      return { body: source, type: 'text/plain;charset=UTF-8' };
    }

    if (source instanceof URLSearchParams) {
      return {
        body: source.toString(),
        type: 'application/x-www-form-urlencoded;charset=UTF-8',
      };
    }

    if (source instanceof FormData) {
      return __ow_formdata_serialize(source);
    }

    if (ArrayBuffer.isView(source)) {
      const bytes = new Uint8Array(
        source.buffer.slice(source.byteOffset, source.byteOffset + source.byteLength)
      );

      return { body: bytes, type: null };
    }

    if (source instanceof ArrayBuffer) {
      return { body: new Uint8Array(source.slice(0)), type: null };
    }

    if (typeof source.getReader === 'function') {
      throw new TypeError('streaming bodies are not supported yet');
    }

    return { body: String(source), type: 'text/plain;charset=UTF-8' };
  }

  function bodyToText(body) {
    if (body === null) {
      return '';
    }

    return typeof body === 'string' ? body : decoder.decode(body);
  }

  function bodyToBytes(body) {
    if (body === null) {
      return new Uint8Array(0);
    }

    return typeof body === 'string' ? encoder.encode(body) : body;
  }

  // Assigned by the class body so the wire path can seed a body the public
  // constructor would refuse.
  let setRequestBody;

  class Request {
    #url;
    #method = 'GET';
    #headers;
    #body = null;
    #bodyUsed = false;

    constructor(input, init) {
      init = init || {};

      let headers = null;

      if (input instanceof Request) {
        this.#url = input.#url;
        this.#method = input.#method;
        this.#body = input.#body;
        headers = new Headers(input.#headers);
      } else {
        this.#url = new URL(String(input)).href;
      }

      if (init.method !== undefined) {
        this.#method = normalizeMethod(init.method);
      }

      if (init.headers !== undefined) {
        headers = new Headers(init.headers);
      }

      this.#headers = headers === null ? new Headers() : headers;

      if (init.body !== undefined && init.body !== null) {
        if (this.#method === 'GET' || this.#method === 'HEAD') {
          throw new TypeError('a GET or HEAD request cannot carry a body');
        }

        const extracted = extractBody(init.body);

        this.#body = extracted.body;

        if (extracted.type !== null && !this.#headers.has('content-type')) {
          this.#headers.append('content-type', extracted.type);
        }
      }
    }

    get url() {
      return this.#url;
    }

    get method() {
      return this.#method;
    }

    get headers() {
      return this.#headers;
    }

    get bodyUsed() {
      return this.#bodyUsed;
    }

    clone() {
      if (this.#bodyUsed) {
        throw new TypeError('a request whose body was read cannot be cloned');
      }

      return new Request(this);
    }

    text() {
      return this.#consume().then(bodyToText);
    }

    json() {
      return this.text().then(JSON.parse);
    }

    formData() {
      const type = this.#headers.get('content-type');

      return this.text().then((text) => __ow_formdata_parse(text, type));
    }

    bytes() {
      return this.#consume().then(bodyToBytes);
    }

    arrayBuffer() {
      return this.bytes().then((bytes) => bytes.buffer);
    }

    #consume() {
      if (this.#bodyUsed) {
        return Promise.reject(new TypeError('body already read'));
      }

      this.#bodyUsed = this.#body !== null;

      return Promise.resolve(this.#body);
    }

    static {
      setRequestBody = (request, body) => {
        request.#body = body;
      };
    }
  }

  class Response {
    #status;
    #statusText;
    #headers;
    #body = null;
    #bodyUsed = false;

    constructor(body, init) {
      init = init || {};

      const status = init.status === undefined ? 200 : Number(init.status);

      if (!Number.isInteger(status) || status < 200 || status > 599) {
        throw new RangeError('response status out of range: ' + String(init.status));
      }

      this.#status = status;
      this.#statusText = init.statusText === undefined ? '' : String(init.statusText);
      this.#headers = new Headers(init.headers);

      if (body === undefined || body === null) {
        return;
      }

      if (NULL_BODY_STATUSES.has(status)) {
        throw new TypeError('a ' + status + ' response cannot carry a body');
      }

      const extracted = extractBody(body);

      this.#body = extracted.body;

      if (extracted.type !== null && !this.#headers.has('content-type')) {
        this.#headers.append('content-type', extracted.type);
      }
    }

    static json(data, init) {
      init = init || {};

      // Only a default, and it has to beat the text/plain a string body implies.
      const headers = new Headers(init.headers);

      if (!headers.has('content-type')) {
        headers.set('content-type', 'application/json');
      }

      return new Response(JSON.stringify(data), { ...init, headers: headers });
    }

    static redirect(url, status) {
      const code = status === undefined ? 302 : Number(status);

      if (!REDIRECT_STATUSES.has(code)) {
        throw new RangeError('not a redirect status: ' + String(status));
      }

      return new Response(null, {
        status: code,
        headers: { location: new URL(String(url)).href },
      });
    }

    get status() {
      return this.#status;
    }

    get statusText() {
      return this.#statusText;
    }

    get ok() {
      return this.#status >= 200 && this.#status < 300;
    }

    get headers() {
      return this.#headers;
    }

    get bodyUsed() {
      return this.#bodyUsed;
    }

    clone() {
      if (this.#bodyUsed) {
        throw new TypeError('a response whose body was read cannot be cloned');
      }

      const clone = new Response(null, {
        status: this.#status,
        statusText: this.#statusText,
        headers: this.#headers,
      });

      clone.#body = this.#body;

      return clone;
    }

    text() {
      return this.#consume().then(bodyToText);
    }

    json() {
      return this.text().then(JSON.parse);
    }

    formData() {
      const type = this.#headers.get('content-type');

      return this.text().then((text) => __ow_formdata_parse(text, type));
    }

    bytes() {
      return this.#consume().then(bodyToBytes);
    }

    arrayBuffer() {
      return this.bytes().then((bytes) => bytes.buffer);
    }

    #consume() {
      if (this.#bodyUsed) {
        return Promise.reject(new TypeError('body already read'));
      }

      this.#bodyUsed = this.#body !== null;

      return Promise.resolve(this.#body);
    }
  }

  // A request HTTP already delivered keeps its body whatever its method is.
  globalThis.__ow_request_from_wire = function (data) {
    const request = new Request(data.url, {
      method: data.method,
      headers: data.headers,
    });

    if (data.body !== null && data.body !== undefined) {
      setRequestBody(request, String(data.body));
    }

    return request;
  };

  globalThis.Request = Request;
  globalThis.Response = Response;
})();
