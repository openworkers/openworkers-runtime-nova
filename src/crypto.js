// Web Crypto. The entropy, the digests and the two key algorithms are the
// host's; a key's material lives here, as hex, and goes back down for every
// operation.
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

  // Key material, kept off the object the guest holds.
  const material = new WeakMap();

  function algorithmName(algorithm) {
    return String(
      algorithm !== null && typeof algorithm === 'object' ? algorithm.name : algorithm
    ).toUpperCase();
  }

  class CryptoKey {
    constructor(type, algorithm, extractable, usages, raw) {
      this.type = type;
      this.algorithm = algorithm;
      this.extractable = extractable;
      this.usages = usages;

      material.set(this, raw);
    }
  }

  function rawOf(key) {
    const raw = material.get(key);

    if (raw === undefined) {
      throw new DOMException('that is not a key this runtime made', 'InvalidAccessError');
    }

    return raw;
  }

  function hashOf(algorithm, key) {
    const hash = algorithm !== null && typeof algorithm === 'object' ? algorithm.hash : undefined;

    return String(hash === undefined ? key.algorithm.hash : hash).toUpperCase();
  }

  class SubtleCrypto {
    async importKey(format, data, algorithm, extractable, usages) {
      if (String(format) !== 'raw') {
        throw new DOMException(format + ' keys are not imported here', 'NotSupportedError');
      }

      const name = algorithmName(algorithm);

      if (name !== 'HMAC' && name !== 'AES-GCM') {
        throw new DOMException(name + ' is not a key algorithm here', 'NotSupportedError');
      }

      const shape =
        name === 'HMAC'
          ? { name: name, hash: String(algorithm.hash).toUpperCase() }
          : { name: name, length: bytesOf(data).byteLength * 8 };

      return new CryptoKey('secret', shape, Boolean(extractable), [...usages], toHex(bytesOf(data)));
    }

    async generateKey(algorithm, extractable, usages) {
      const name = algorithmName(algorithm);

      if (name !== 'AES-GCM') {
        throw new DOMException(name + ' pairs are not generated here', 'NotSupportedError');
      }

      const length = Number(algorithm.length) || 256;

      if (length !== 256) {
        throw new DOMException('AES-GCM here takes a 256 bit key', 'NotSupportedError');
      }

      return new CryptoKey(
        'secret',
        { name: name, length: length },
        Boolean(extractable),
        [...usages],
        toHex(randomBytes(length / 8))
      );
    }

    async exportKey(format, key) {
      if (String(format) !== 'raw') {
        throw new DOMException(format + ' keys are not exported here', 'NotSupportedError');
      }

      if (!key.extractable) {
        throw new DOMException('that key was imported as not extractable', 'InvalidAccessError');
      }

      return fromHex(rawOf(key)).buffer;
    }

    async sign(algorithm, key, data) {
      if (algorithmName(algorithm) !== 'HMAC') {
        throw new DOMException('only HMAC signs here', 'NotSupportedError');
      }

      const tag = __ow_native_hmac(hashOf(algorithm, key), rawOf(key), toHex(bytesOf(data)));

      if (tag === null) {
        throw new DOMException('that is not a hash this runtime has', 'NotSupportedError');
      }

      return fromHex(tag).buffer;
    }

    async verify(algorithm, key, signature, data) {
      if (algorithmName(algorithm) !== 'HMAC') {
        throw new DOMException('only HMAC verifies here', 'NotSupportedError');
      }

      return __ow_native_hmac(
        hashOf(algorithm, key),
        rawOf(key),
        toHex(bytesOf(data)),
        toHex(bytesOf(signature))
      ) === true;
    }

    async encrypt(algorithm, key, data) {
      return aes('encrypt', algorithm, key, data);
    }

    async decrypt(algorithm, key, data) {
      return aes('decrypt', algorithm, key, data);
    }

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

  function aes(op, algorithm, key, data) {
    if (algorithmName(algorithm) !== 'AES-GCM') {
      throw new DOMException('only AES-GCM is a cipher here', 'NotSupportedError');
    }

    const out = __ow_native_aes_gcm(
      op,
      rawOf(key),
      toHex(bytesOf(algorithm.iv)),
      toHex(bytesOf(data))
    );

    if (out === null) {
      throw new DOMException('AES-GCM here takes a 256 bit key and a 12 byte iv', 'DataError');
    }

    if (out === false) {
      throw new DOMException('the tag does not hold', 'OperationError');
    }

    return fromHex(out).buffer;
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
  globalThis.CryptoKey = CryptoKey;
  globalThis.SubtleCrypto = SubtleCrypto;
  globalThis.crypto = new Crypto();
})();
