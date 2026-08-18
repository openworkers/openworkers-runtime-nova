// The Web Crypto entry points that need host entropy. SubtleCrypto is absent.
(function () {
  'use strict';

  const INTEGER_VIEWS = [
    Int8Array,
    Uint8Array,
    Uint8ClampedArray,
    Int16Array,
    Uint16Array,
    Int32Array,
    Uint32Array,
    BigInt64Array,
    BigUint64Array,
  ];

  function randomBytes(count) {
    const hex = __ow_native_random_hex(count);
    const bytes = new Uint8Array(count);

    for (let i = 0; i < count; i++) {
      bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    }

    return bytes;
  }

  globalThis.crypto = {
    getRandomValues(view) {
      if (!INTEGER_VIEWS.some((kind) => view instanceof kind)) {
        throw new TypeError('getRandomValues expects an integer typed array');
      }

      if (view.byteLength > 65536) {
        throw new DOMException(
          'getRandomValues accepts at most 65536 bytes',
          'QuotaExceededError'
        );
      }

      const target = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);

      target.set(randomBytes(view.byteLength));

      return view;
    },

    randomUUID() {
      const bytes = randomBytes(16);

      bytes[6] = (bytes[6] & 0x0f) | 0x40;
      bytes[8] = (bytes[8] & 0x3f) | 0x80;

      const hex = Array.from(bytes, (byte) =>
        byte.toString(16).padStart(2, '0')
      ).join('');

      return [
        hex.slice(0, 8),
        hex.slice(8, 12),
        hex.slice(12, 16),
        hex.slice(16, 20),
        hex.slice(20),
      ].join('-');
    },
  };
})();
