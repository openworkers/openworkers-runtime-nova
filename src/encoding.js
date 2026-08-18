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

  globalThis.atob = atob;
  globalThis.btoa = btoa;
})();
