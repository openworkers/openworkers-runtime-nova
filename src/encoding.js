// Base64 and UTF-8 conversions (WHATWG Infra / Encoding).
(function () {
  'use strict';

  const BASE64 =
    'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

  function invalidCharacter(message) {
    return new DOMException(message, 'InvalidCharacterError');
  }

  function btoa(data) {
    const input = String(data);
    let output = '';

    for (let i = 0; i < input.length; i += 3) {
      const bytes = [];

      for (let j = 0; j < 3 && i + j < input.length; j++) {
        const code = input.charCodeAt(i + j);

        if (code > 0xff) {
          throw invalidCharacter('btoa: code point above U+00FF');
        }

        bytes.push(code);
      }

      const bits = (bytes[0] << 16) | ((bytes[1] || 0) << 8) | (bytes[2] || 0);

      output +=
        BASE64[(bits >> 18) & 63] +
        BASE64[(bits >> 12) & 63] +
        (bytes.length > 1 ? BASE64[(bits >> 6) & 63] : '=') +
        (bytes.length > 2 ? BASE64[bits & 63] : '=');
    }

    return output;
  }

  function atob(data) {
    let input = String(data).replace(/[ \t\n\f\r]/g, '');

    if (input.length % 4 === 0) {
      input = input.replace(/==?$/, '');
    }

    if (input.length % 4 === 1) {
      throw invalidCharacter('atob: input length is not a valid base64 length');
    }

    let output = '';
    let bits = 0;
    let width = 0;

    for (const character of input) {
      const value = BASE64.indexOf(character);

      if (value < 0) {
        throw invalidCharacter('atob: character outside the base64 alphabet');
      }

      bits = (bits << 6) | value;
      width += 6;

      if (width >= 8) {
        width -= 8;
        output += String.fromCharCode((bits >> width) & 0xff);
      }
    }

    return output;
  }

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

      for (let i = 0; i < text.length; i++) {
        let code = text.charCodeAt(i);

        if (code >= 0xd800 && code <= 0xdbff) {
          const trail = text.charCodeAt(i + 1);

          if (trail >= 0xdc00 && trail <= 0xdfff) {
            code = 0x10000 + ((code - 0xd800) << 10) + (trail - 0xdc00);
            i++;
          } else {
            code = REPLACEMENT;
          }
        } else if (code >= 0xdc00 && code <= 0xdfff) {
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

  globalThis.atob = atob;
  globalThis.btoa = btoa;
  globalThis.TextEncoder = TextEncoder;
  globalThis.TextDecoder = TextDecoder;
})();
