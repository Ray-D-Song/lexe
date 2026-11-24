import defaultImport from "node:crypto";
import legacyImport from "crypto";

it("node:crypto should be the same as crypto", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const {
  createHash,
  createHmac,
  randomBytes,
  randomInt,
  randomUUID,
  randomFillSync,
  randomFill,
  getRandomValues,
  webcrypto,
  publicEncrypt,
  constants,
} = defaultImport;

describe("crypto object/module", () => {
  it("should have a createHash()", () => {
    expect(crypto.createHash).toBeDefined();
    expect(createHash).toBeDefined();
  });

  it("should export publicEncrypt and constants", () => {
    expect(typeof publicEncrypt).toEqual("function");
    expect(typeof defaultImport.publicEncrypt).toEqual("function");
    expect(publicEncrypt).toBe(defaultImport.publicEncrypt);
    expect(legacyImport.publicEncrypt).toBe(defaultImport.publicEncrypt);
    expect(constants.RSA_PKCS1_OAEP_PADDING).toEqual(4);
    expect(defaultImport.constants.RSA_PKCS1_OAEP_PADDING).toEqual(4);
  });
  it("should have a createHmac()", () => {
    expect(globalThis.crypto.createHmac).toBeDefined();
    expect(createHmac).toBeDefined();
  });
  it("should have a randomBytes()", () => {
    expect(globalThis.crypto.randomBytes).toBeDefined();
    expect(randomBytes).toBeDefined();
  });
  it("should have a randomInt()", () => {
    expect(globalThis.crypto.randomInt).toBeDefined();
    expect(randomInt).toBeDefined();
  });
  it("should have a randomUUID()", () => {
    expect(globalThis.crypto.randomUUID).toBeDefined();
    expect(randomUUID).toBeDefined();
  });
  it("should have a randomFillSync()", () => {
    expect(globalThis.crypto.randomFillSync).toBeDefined();
    expect(randomFillSync).toBeDefined();
  });
  it("should have a randomFill()", () => {
    expect(globalThis.crypto.randomFill).toBeDefined();
    expect(randomFill).toBeDefined();
  });
  it("should have a webcrypto and should be equal to globalThis.crypto", () => {
    expect(webcrypto).toBeDefined();
    expect(webcrypto === globalThis.crypto).toBeTruthy();
    expect(webcrypto).toStrictEqual(globalThis.crypto);
  });
});

describe("Hashing", () => {
  it("should hash to sha256 with b64 encoding", () => {
    let hash = createHash("sha256").update("message").digest("base64");
    expect(hash).toEqual("q1MKE+RZFJgrefm34/uplM/R8/si9xzqGvvwK0YMbR0=");
  });

  it("should hash to sha256 with hex encoding", () => {
    let hash = createHash("sha256").update("message").digest("hex");
    expect(hash).toEqual(
      "ab530a13e45914982b79f9b7e3fba994cfd1f3fb22f71cea1afbf02b460c6d1d"
    );
  });

  it("should hash to hmac-sha256 with b64 encoding", () => {
    let hash = createHmac("sha256", "key").update("message").digest("base64");
    expect(hash).toEqual("bp7ym3X//Ft6uuUn1Y/a2y/kLnIZARl2kXNDBl9Y7Uo=");
  });

  it("should hash to hmac-sha256 with hex encoding", () => {
    let hash = createHmac("sha256", "key").update("message").digest("hex");
    expect(hash).toEqual(
      "6e9ef29b75fffc5b7abae527d58fdadb2fe42e7219011976917343065f58ed4a"
    );
  });
});

describe("publicEncrypt", () => {
  const LIMITED_CRYPTO = process.env.LEXE_LIMITED_CRYPTO === "1";

  // Static 2048-bit RSA public key (SPKI PEM).
  const SPKI_PEM = `-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAu1SU1LfVLPHCozMxH2Mo
4lgOEePzNm0tRgeLezV6ffAt0gunVTLw7onLRnrq0/IzW7yWR7QkrmBL7jTKEn5u
+qKhbwKfBstIs+bMY2Zkp18gnTxKLxoS2tFczGkPLPgizskuemMghRniWaoLcyeh
kd3qqGElvW/VDL5AaWTg0nLVkjRo9z+40RQzuVaE8AkAFmxZzow3x+VJYKdjykkJ
0iT9wCS0DRTXu269V264Vf/3jvredZiKRkgwlL9xNAwxXFg0x/XFw005UWVRIkdg
cKWTjpBP2dPwVZ4WWC+9aGVd+Gyn1o0CLelf4rEjGoXbAAEgAqeGUxrcIlbjXfbc
mwIDAQAB
-----END PUBLIC KEY-----`;

  it("module shape: default has publicEncrypt and Node-compatible subtle", () => {
    expect(typeof defaultImport.publicEncrypt).toEqual("function");
    expect(typeof defaultImport.randomUUID).toEqual("function");
    expect(defaultImport.crypto.subtle).toBeDefined();
    expect(defaultImport.webcrypto.subtle).toBeDefined();
    expect(defaultImport.crypto).toBe(defaultImport.webcrypto);
    // Node: top-level subtle === webcrypto.subtle
    expect(defaultImport.subtle).toBe(defaultImport.webcrypto.subtle);
  });

  it("should accept mysql2-style options object via module export", () => {
    if (LIMITED_CRYPTO) {
      expect(() =>
        publicEncrypt(
          {
            key: SPKI_PEM,
            oaepHash: "sha1",
            padding: constants.RSA_PKCS1_OAEP_PADDING,
          },
          Buffer.from("password")
        )
      ).toThrow(/not supported by the active crypto provider/i);
      return;
    }
    const out = publicEncrypt(
      {
        key: SPKI_PEM,
        oaepHash: "sha1",
        padding: constants.RSA_PKCS1_OAEP_PADDING,
      },
      Buffer.from("password")
    );
    expect(Buffer.isBuffer(out)).toBeTruthy();
    expect(out.length).toEqual(256);
  });

  it("should encrypt only DataView view range for data and label", () => {
    if (LIMITED_CRYPTO) return;
    // Plaintext with sentinels outside the view: only middle "pw" is encrypted.
    const dataBacking = new ArrayBuffer(6);
    const dataBytes = new Uint8Array(dataBacking);
    dataBytes.set([0xaa, 0xbb, 0x70, 0x77, 0xcc, 0xdd]); // .. 'p','w' ..
    const dataView = new DataView(dataBacking, 2, 2);
    // Prove the view bytes are 0x70 0x77 before encrypting.
    expect([...Buffer.from(dataView.buffer, dataView.byteOffset, dataView.byteLength)]).toEqual([
      0x70, 0x77,
    ]);
    expect([...new Uint8Array(dataView.buffer, dataView.byteOffset, dataView.byteLength)]).toEqual([
      0x70, 0x77,
    ]);

    const out = publicEncrypt(
      {
        key: SPKI_PEM,
        oaepHash: "sha1",
        padding: constants.RSA_PKCS1_OAEP_PADDING,
      },
      dataView
    );
    expect(out.length).toEqual(256);

    // Label with sentinels: only the view range is used.
    const labelBacking = new ArrayBuffer(4);
    new Uint8Array(labelBacking).set([0x11, 0x22, 0x33, 0x44]);
    const labelView = new DataView(labelBacking, 1, 2);
    const out2 = publicEncrypt(
      {
        key: SPKI_PEM,
        oaepHash: "sha1",
        padding: constants.RSA_PKCS1_OAEP_PADDING,
        oaepLabel: labelView,
      },
      Buffer.from("x")
    );
    expect(out2.length).toEqual(256);
  });

  it("should ignore poisoned global DataView and shadowed own properties", () => {
    if (LIMITED_CRYPTO) return;
    const backing = new ArrayBuffer(6);
    new Uint8Array(backing).set([0xaa, 0xbb, 0x70, 0x77, 0xcc, 0xdd]);
    const dataView = new DataView(backing, 2, 2); // "pw"

    // Shadow own properties (must not change internal-slot reads).
    Object.defineProperty(dataView, "byteOffset", { value: 0 });
    Object.defineProperty(dataView, "byteLength", { value: 6 });
    Object.defineProperty(dataView, "buffer", {
      value: new Uint8Array([9, 9, 9, 9]).buffer,
    });

    // Replace global constructor (must not coerce DataView via ToString).
    const RealDataView = globalThis.DataView;
    // @ts-expect-error intentional pollution
    globalThis.DataView = function () {
      throw new Error("global DataView should not be used");
    };

    try {
      const out = publicEncrypt(
        {
          key: SPKI_PEM,
          oaepHash: "sha1",
          padding: constants.RSA_PKCS1_OAEP_PADDING,
        },
        dataView
      );
      expect(out.length).toEqual(256);
    } finally {
      globalThis.DataView = RealDataView;
    }

    // Forged plain object is not a view.
    const fake = {
      buffer: backing,
      byteOffset: 2,
      byteLength: 2,
    };
    expect(() =>
      publicEncrypt(
        {
          key: SPKI_PEM,
          oaepHash: "sha1",
          padding: constants.RSA_PKCS1_OAEP_PADDING,
        },
        fake as unknown as DataView
      )
    ).toThrow();
  });

  it("should reject unsupported padding", () => {
    expect(() =>
      publicEncrypt(
        {
          key: SPKI_PEM,
          padding: 1,
        },
        Buffer.from("x")
      )
    ).toThrow();
  });
});

describe("random", () => {
  it("should generate a random buffer synchronously using randomFillSync", () => {
    const buffer = randomFillSync(Buffer.alloc(16));
    expect(buffer.length).toEqual(16);
  });

  it("should generate a random buffer asynchronously using randomFill", (done) => {
    randomFill(Buffer.alloc(16), (err, buffer) => {
      expect(err).toBeNull();
      expect(buffer.length).toEqual(16);
      done();
    });
  });

  it("should generate random bytes synchronously into a Uint8Array using randomFillSync", () => {
    const uint8Array = new Uint8Array(16);
    randomFillSync(uint8Array);
    expect(uint8Array.length).toEqual(16);
    for (const byte of uint8Array) {
      expect(byte >= 0 && byte <= 255).toBeTruthy();
    }
  });

  it("should generate random bytes asynchronously into a DataView using randomFill", (done) => {
    const dataView = new DataView(new ArrayBuffer(32));
    randomFill(dataView, (err, buffer) => {
      expect(err).toBeNull();
      expect(buffer.buffer).toEqual(dataView.buffer);
      expect(dataView.byteLength).toEqual(32);
      for (let i = 0; i < 32; i++) {
        expect(
          dataView.getUint8(i) >= 0 && dataView.getUint8(i) <= 255
        ).toBeTruthy();
      }
      done();
    });
  });

  it("should generate a random UUID using randomUUID", () => {
    const uuid = randomUUID();
    expect(uuid.length).toEqual(36);
    const uuidRegex =
      /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
    expect(uuid).toMatch(uuidRegex);
  });

  it("should generate a random bytes buffer using randomBytes", () => {
    const buffer = randomBytes(16);
    expect(buffer).toBeInstanceOf(Buffer);
    expect(buffer.length).toEqual(16);
  });

  it("should generate a random int using randomInt", () => {
    // Do it 10 times, to make sure we respect min and max
    for (const number of [...Array(10).keys()]) {
      const randomInteger = randomInt(
        Number.MAX_SAFE_INTEGER - 1,
        Number.MAX_SAFE_INTEGER
      );
      expect(typeof randomInteger).toEqual("number");
      expect(Number.MAX_SAFE_INTEGER - 1).toEqual(randomInteger);
      expect(typeof randomInteger).toEqual("number");
    }

    // Do it 20 times to make sure we never get values outside the range
    for (const number of [...Array(20).keys()]) {
      const randomInteger = randomInt(0, 5);
      expect(randomInteger).toBeLessThan(5);
      expect(randomInteger).toBeGreaterThanOrEqual(0);
    }
  });

  it("should generate random bytes synchronously into a Int8Array using getRandomValues", () => {
    const int8Array = new Int8Array(10);
    getRandomValues(int8Array);
    expect(int8Array.length).toEqual(10);
    for (const byte of int8Array) {
      expect(byte >= -0x80 && byte <= 0x7f).toBeTruthy();
    }
  });

  it("should generate random bytes synchronously into a Uint8Array using getRandomValues", () => {
    const uint8Array = new Uint8Array(10);
    getRandomValues(uint8Array);
    expect(uint8Array.length).toEqual(10);
    for (const byte of uint8Array) {
      expect(byte >= 0x00 && byte <= 0xff).toBeTruthy();
    }
  });

  it("should generate random bytes synchronously into a Uint8ClampedArray using getRandomValues", () => {
    const uint8ClampedArray = new Uint8ClampedArray(10);
    getRandomValues(uint8ClampedArray);
    expect(uint8ClampedArray.length).toEqual(10);
    for (const byte of uint8ClampedArray) {
      expect(byte >= 0x00 && byte <= 0xff).toBeTruthy();
    }
  });

  it("should generate random bytes synchronously into a Int16Array using getRandomValues", () => {
    const int16Array = new Int16Array(10);
    getRandomValues(int16Array);
    expect(int16Array.length).toEqual(10);
    for (const byte of int16Array) {
      expect(byte >= -0x8000 && byte <= 0x7fff).toBeTruthy();
    }
  });

  it("should generate random bytes synchronously into a Uint16Array using getRandomValues", () => {
    const uint16Array = new Uint16Array(10);
    getRandomValues(uint16Array);
    expect(uint16Array.length).toEqual(10);
    for (const byte of uint16Array) {
      expect(byte >= 0x0000 && byte <= 0xffff).toBeTruthy();
    }
  });

  it("should generate random bytes synchronously into a Int32Array using getRandomValues", () => {
    const int32Array = new Int32Array(10);
    getRandomValues(int32Array);
    expect(int32Array.length).toEqual(10);
    for (const byte of int32Array) {
      expect(byte >= -0x80000000 && byte <= 0x7fffffff).toBeTruthy();
    }
  });

  it("should generate random bytes synchronously into a Uint32Array using getRandomValues", () => {
    const uint32Array = new Uint32Array(10);
    getRandomValues(uint32Array);
    expect(uint32Array.length).toEqual(10);
    for (const byte of uint32Array) {
      expect(byte >= 0x00000000 && byte <= 0xffffffff).toBeTruthy();
    }
  });

  it("should be an error, if it exceeds 65536 bytes", () => {
    const int8Array = new BigInt64Array(65536 / 8 + 1);
    let errorMessage = "";
    try {
      getRandomValues(int8Array);
    } catch (ex: any) {
      errorMessage = ex.message;
    }
    expect(errorMessage).toEqual(
      "QuotaExceededError: The requested length exceeds 65,536 bytes"
    );
  });
});
