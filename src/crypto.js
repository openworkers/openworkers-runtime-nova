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
    return new Uint8Array(__ow_native_random(count));
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

      if (name === 'ECDSA') {
        // Only a public key comes in, as the uncompressed point.
        const curve = curveOf(algorithm);
        const point = toHex(bytesOf(data));

        if (__ow_native_ecdsa('check', curve, point) !== true) {
          throw new DOMException('that is not a point on ' + curve, 'DataError');
        }

        return new CryptoKey(
          'public',
          { name: 'ECDSA', namedCurve: curve },
          Boolean(extractable),
          [...usages],
          point
        );
      }

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

      if (name === 'ECDSA') {
        const curve = curveOf(algorithm);
        const [scalar, point] = __ow_native_ecdsa('generate', curve).split(':');
        const shape = { name: 'ECDSA', namedCurve: curve };
        const asked = [...usages];

        // A public key can only verify, and the pair's public half is always
        // extractable, as the standard has it.
        return {
          privateKey: new CryptoKey(
            'private',
            shape,
            Boolean(extractable),
            asked.filter((usage) => usage === 'sign'),
            scalar
          ),
          publicKey: new CryptoKey(
            'public',
            shape,
            true,
            asked.filter((usage) => usage === 'verify'),
            point
          ),
        };
      }

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

      // The raw form of an elliptic key is its public point.
      if (key.algorithm.name === 'ECDSA' && key.type !== 'public') {
        throw new DOMException('only a public key has a raw form', 'InvalidAccessError');
      }

      return fromHex(rawOf(key)).buffer;
    }

    async sign(algorithm, key, data) {
      if (algorithmName(algorithm) === 'ECDSA') {
        return ecdsa('sign', algorithm, key, data);
      }

      if (algorithmName(algorithm) !== 'HMAC') {
        throw new DOMException('only HMAC and ECDSA sign here', 'NotSupportedError');
      }

      const tag = __ow_native_hmac(hashOf(algorithm, key), rawOf(key), toHex(bytesOf(data)));

      if (tag === null) {
        throw new DOMException('that is not a hash this runtime has', 'NotSupportedError');
      }

      return fromHex(tag).buffer;
    }

    async verify(algorithm, key, signature, data) {
      if (algorithmName(algorithm) === 'ECDSA') {
        return ecdsa('verify', algorithm, key, data, signature);
      }

      if (algorithmName(algorithm) !== 'HMAC') {
        throw new DOMException('only HMAC and ECDSA verify here', 'NotSupportedError');
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

  const CURVES = ['P-256', 'P-384'];

  function curveOf(algorithm) {
    const curve = String(algorithm.namedCurve);

    if (!CURVES.includes(curve)) {
      throw new DOMException(curve + ' is not a curve this runtime has', 'NotSupportedError');
    }

    return curve;
  }

  // The digest is the guest layer's to compute, so the host signs bytes it did
  // not hash and a hash it does not know is refused here.
  function ecdsa(op, algorithm, key, data, signature) {
    const wanted = op === 'sign' ? 'private' : 'public';

    if (key.algorithm.name !== 'ECDSA' || key.type !== wanted) {
      throw new DOMException('that key cannot ' + op, 'InvalidAccessError');
    }

    const hash = String(
      algorithm.hash !== null && typeof algorithm.hash === 'object'
        ? algorithm.hash.name
        : algorithm.hash
    ).toUpperCase();

    if (!DIGESTS.includes(hash)) {
      throw new DOMException(hash + ' is not a digest this runtime has', 'NotSupportedError');
    }

    const digest = __ow_native_digest(hash, toHex(bytesOf(data)));
    const curve = key.algorithm.namedCurve;

    if (op === 'sign') {
      return fromHex(__ow_native_ecdsa('sign', curve, rawOf(key), digest)).buffer;
    }

    return __ow_native_ecdsa('verify', curve, rawOf(key), digest, toHex(bytesOf(signature))) === true;
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
