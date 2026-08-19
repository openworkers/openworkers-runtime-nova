// FormData, and the two body formats it parses from and serializes to.
// Values are strings: there is no Blob or File in this runtime, so a
// multipart part with a filename arrives as its text.
(function () {
  'use strict';

  const CRLF = '\r\n';

  class FormData {
    #entries = [];

    append(name, value) {
      this.#entries.push([String(name), String(value)]);
    }

    delete(name) {
      const target = String(name);

      this.#entries = this.#entries.filter((entry) => entry[0] !== target);
    }

    get(name) {
      const target = String(name);
      const found = this.#entries.find((entry) => entry[0] === target);

      return found === undefined ? null : found[1];
    }

    getAll(name) {
      const target = String(name);

      return this.#entries
        .filter((entry) => entry[0] === target)
        .map((entry) => entry[1]);
    }

    has(name) {
      const target = String(name);

      return this.#entries.some((entry) => entry[0] === target);
    }

    // Replaces the first entry of that name in place and drops the rest.
    set(name, value) {
      const target = String(name);
      const replacement = String(value);
      let seen = false;

      this.#entries = this.#entries.filter((entry) => {
        if (entry[0] !== target) {
          return true;
        }

        if (seen) {
          return false;
        }

        seen = true;
        entry[1] = replacement;

        return true;
      });

      if (!seen) {
        this.#entries.push([target, replacement]);
      }
    }

    forEach(callback, thisArg) {
      for (const entry of this.#entries.slice()) {
        callback.call(thisArg, entry[1], entry[0], this);
      }
    }

    *entries() {
      for (const entry of this.#entries.slice()) {
        yield [entry[0], entry[1]];
      }
    }

    *keys() {
      for (const entry of this.#entries.slice()) {
        yield entry[0];
      }
    }

    *values() {
      for (const entry of this.#entries.slice()) {
        yield entry[1];
      }
    }

    [Symbol.iterator]() {
      return this.entries();
    }
  }

  // `filename="x"` also contains `name="`, so the match has to start a parameter.
  function parameter(header, name) {
    const key = name + '="';
    const lower = header.toLowerCase();
    let search = 0;

    for (;;) {
      const at = lower.indexOf(key, search);

      if (at < 0) {
        return null;
      }

      const before = at === 0 ? ';' : header[at - 1];

      if (before === ';' || before === ' ' || before === '\t') {
        const from = at + key.length;
        const to = header.indexOf('"', from);

        return to < 0 ? null : header.slice(from, to);
      }

      search = at + key.length;
    }
  }

  function boundaryOf(contentType) {
    const at = contentType.toLowerCase().indexOf('boundary=');

    if (at < 0) {
      return null;
    }

    let value = contentType.slice(at + 'boundary='.length).trim();
    const end = value.indexOf(';');

    if (end >= 0) {
      value = value.slice(0, end).trim();
    }

    if (value.startsWith('"') && value.endsWith('"') && value.length > 1) {
      value = value.slice(1, -1);
    }

    return value === '' ? null : value;
  }

  // Where a part's headers end, and how many characters the blank line takes.
  function headerEnd(part) {
    const crlf = part.indexOf(CRLF + CRLF);
    const lf = part.indexOf('\n\n');

    if (crlf >= 0 && (lf < 0 || crlf < lf)) {
      return [crlf, 4];
    }

    return lf < 0 ? [-1, 0] : [lf, 2];
  }

  function stripTrailingBreak(text) {
    if (text.endsWith(CRLF)) {
      return text.slice(0, -2);
    }

    return text.endsWith('\n') ? text.slice(0, -1) : text;
  }

  function parseMultipart(text, boundary) {
    const form = new FormData();
    const chunks = text.split('--' + boundary);

    // Chunk 0 is the preamble; the closing delimiter is the one starting `--`.
    for (let i = 1; i < chunks.length; i++) {
      const chunk = chunks[i];

      if (chunk.startsWith('--')) {
        break;
      }

      const [end, blank] = headerEnd(chunk);

      if (end < 0) {
        continue;
      }

      const headers = chunk.slice(0, end).split('\n');
      let name = null;

      for (const header of headers) {
        if (header.toLowerCase().includes('content-disposition:')) {
          name = parameter(header, 'name');
        }
      }

      if (name === null) {
        continue;
      }

      form.append(name, stripTrailingBreak(chunk.slice(end + blank)));
    }

    return form;
  }

  function parseUrlencoded(text) {
    const form = new FormData();

    for (const [name, value] of new URLSearchParams(text)) {
      form.append(name, value);
    }

    return form;
  }

  globalThis.FormData = FormData;

  // The content type decides how to read the body; another type is refused
  // rather than guessed at.
  globalThis.__ow_formdata_parse = function (text, contentType) {
    const header = String(contentType === null ? '' : contentType);
    const type = header.toLowerCase();

    if (type.startsWith('multipart/form-data')) {
      const boundary = boundaryOf(header);

      if (boundary === null) {
        throw new TypeError('multipart/form-data body without a boundary');
      }

      return parseMultipart(text, boundary);
    }

    if (type.startsWith('application/x-www-form-urlencoded')) {
      return parseUrlencoded(text);
    }

    throw new TypeError('cannot read a ' + (type || 'typeless') + ' body as FormData');
  };

  // Multipart is what a FormData body means on the wire.
  globalThis.__ow_formdata_serialize = function (form) {
    const boundary = '----openworkers' + crypto.randomUUID();
    let body = '';

    for (const [name, value] of form) {
      body += '--' + boundary + CRLF;
      body += 'Content-Disposition: form-data; name="' + name + '"' + CRLF + CRLF;
      body += value + CRLF;
    }

    body += '--' + boundary + '--' + CRLF;

    return { body: body, type: 'multipart/form-data; boundary=' + boundary };
  };
})();
