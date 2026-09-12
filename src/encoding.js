// UTF-8 conversions (WHATWG Encoding).
(function () {
  'use strict';

  // Every label the Encoding standard maps to UTF-8; no other encoding is
  // decodable here, so the rest are rejected rather than approximated.
  const UTF8_LABELS = new Set([
    'unicode-1-1-utf-8',
    'unicode11utf8',
    'unicode20utf8',
    'utf-8',
    'utf8',
    'x-unicode20utf8',
  ]);

  const REPLACEMENT = 0xfffd;

  class TextEncoder {
    get encoding() {
      return 'utf-8';
    }

    encode(input) {
      const text = input === undefined ? '' : String(input);
      const bytes = [];

      // Walk code points, not UTF-16 indices: charCodeAt aborts the process on
      // a heap string whose first character is a surrogate pair.
      for (const ch of text) {
        let code = ch.codePointAt(0);

        if (code >= 0xd800 && code <= 0xdfff) {
          code = REPLACEMENT;
        }

        if (code <= 0x7f) {
          bytes.push(code);
        } else if (code <= 0x7ff) {
          bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
        } else if (code <= 0xffff) {
          bytes.push(
            0xe0 | (code >> 12),
            0x80 | ((code >> 6) & 0x3f),
            0x80 | (code & 0x3f)
          );
        } else {
          bytes.push(
            0xf0 | (code >> 18),
            0x80 | ((code >> 12) & 0x3f),
            0x80 | ((code >> 6) & 0x3f),
            0x80 | (code & 0x3f)
          );
        }
      }

      return new Uint8Array(bytes);
    }
  }

  function toBytes(input) {
    if (input === undefined) {
      return new Uint8Array(0);
    }

    if (ArrayBuffer.isView(input)) {
      return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
    }

    if (input instanceof ArrayBuffer) {
      return new Uint8Array(input);
    }

    throw new TypeError('decode expects an ArrayBuffer or a view of one');
  }

  class TextDecoder {
    #fatal;
    #ignoreBOM;
    #bomSeen = false;
    #needed = 0;
    #seen = 0;
    #codePoint = 0;
    #lower = 0x80;
    #upper = 0xbf;

    constructor(label, options) {
      const encoding = String(label === undefined ? 'utf-8' : label)
        .trim()
        .toLowerCase();

      if (!UTF8_LABELS.has(encoding)) {
        throw new RangeError('unsupported encoding: ' + encoding);
      }

      options = options || {};
      this.#fatal = Boolean(options.fatal);
      this.#ignoreBOM = Boolean(options.ignoreBOM);
    }

    get encoding() {
      return 'utf-8';
    }

    get fatal() {
      return this.#fatal;
    }

    get ignoreBOM() {
      return this.#ignoreBOM;
    }

    decode(input, options) {
      const stream = Boolean((options || {}).stream);
      const bytes = toBytes(input);
      const points = [];

      for (let i = 0; i < bytes.length; i++) {
        const byte = bytes[i];

        if (this.#needed === 0) {
          if (byte <= 0x7f) {
            points.push(byte);
          } else if (byte >= 0xc2 && byte <= 0xdf) {
            this.#start(1, byte & 0x1f);
          } else if (byte >= 0xe0 && byte <= 0xef) {
            this.#start(2, byte & 0x0f);
            this.#lower = byte === 0xe0 ? 0xa0 : 0x80;
            this.#upper = byte === 0xed ? 0x9f : 0xbf;
          } else if (byte >= 0xf0 && byte <= 0xf4) {
            this.#start(3, byte & 0x07);
            this.#lower = byte === 0xf0 ? 0x90 : 0x80;
            this.#upper = byte === 0xf4 ? 0x8f : 0xbf;
          } else {
            points.push(this.#malformed());
          }

          continue;
        }

        if (byte < this.#lower || byte > this.#upper) {
          this.#reset();
          points.push(this.#malformed());
          // The offending byte starts a new sequence of its own.
          i--;

          continue;
        }

        this.#lower = 0x80;
        this.#upper = 0xbf;
        this.#codePoint = (this.#codePoint << 6) | (byte & 0x3f);
        this.#seen++;

        if (this.#seen === this.#needed) {
          points.push(this.#codePoint);
          this.#reset();
        }
      }

      if (!stream && this.#needed !== 0) {
        this.#reset();
        points.push(this.#malformed());
      }

      if (!this.#ignoreBOM && !this.#bomSeen && points.length > 0) {
        this.#bomSeen = true;

        if (points[0] === 0xfeff) {
          points.shift();
        }
      }

      if (!stream) {
        this.#bomSeen = false;
      }

      let text = '';

      // Spreading the whole array at once overflows the call stack.
      for (let i = 0; i < points.length; i += 1024) {
        text += String.fromCodePoint(...points.slice(i, i + 1024));
      }

      return text;
    }

    #start(needed, codePoint) {
      this.#needed = needed;
      this.#seen = 0;
      this.#codePoint = codePoint;
    }

    #reset() {
      this.#needed = 0;
      this.#seen = 0;
      this.#codePoint = 0;
      this.#lower = 0x80;
      this.#upper = 0xbf;
    }

    #malformed() {
      if (this.#fatal) {
        throw new TypeError('decode: malformed UTF-8 sequence');
      }

      return REPLACEMENT;
    }
  }

  globalThis.TextEncoder = TextEncoder;
  globalThis.TextDecoder = TextDecoder;
})();
