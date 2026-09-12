// Web Crypto. The entropy and the digests are the host's; `subtle` has nothing
// beyond them, so no key ever crosses this boundary.
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

  const DIGESTS = ['SHA-1', 'SHA-256', 'SHA-384', 'SHA-512'];

  function bytesOf(source) {
    if (source instanceof ArrayBuffer) {
      return new Uint8Array(source);
    }

    if (ArrayBuffer.isView(source)) {
      return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
    }

    throw new TypeError('the data to digest has to be a buffer or a view of one');
  }

  function toHex(bytes) {
    let hex = '';

    for (const byte of bytes) {
      hex += byte.toString(16).padStart(2, '0');
    }

    return hex;
  }

  function fromHex(hex) {
    const bytes = new Uint8Array(hex.length / 2);

    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    }

    return bytes;
  }

  class SubtleCrypto {
    async digest(algorithm, data) {
      const name = String(
        algorithm !== null && typeof algorithm === 'object' ? algorithm.name : algorithm
      ).toUpperCase();

      if (!DIGESTS.includes(name)) {
        throw new DOMException(name + ' is not a digest this runtime has', 'NotSupportedError');
      }

      return fromHex(__ow_native_digest(name, toHex(bytesOf(data)))).buffer;
    }
  }

  class Crypto {
    getRandomValues(view) {
      if (!INTEGER_VIEWS.some((kind) => view instanceof kind)) {
        throw new DOMException(
          'getRandomValues expects an integer typed array',
          'TypeMismatchError'
        );
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
    }

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
    }

    get subtle() {
      return subtle;
    }
  }

  const subtle = new SubtleCrypto();

  globalThis.Crypto = Crypto;
  globalThis.SubtleCrypto = SubtleCrypto;
  globalThis.crypto = new Crypto();
})();
