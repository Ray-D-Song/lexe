import defaultCrypto, {
  DiffieHellman,
  DiffieHellmanGroup,
  Hash,
  Hmac,
  KeyObject,
  Sign,
  Verify,
  X509Certificate,
  checkPrime,
  checkPrimeSync,
  constants,
  createCipheriv,
  createDecipheriv,
  createDiffieHellman,
  createECDH,
  createHash,
  createHmac,
  createPrivateKey,
  createPublicKey,
  createSecretKey,
  createSign,
  createVerify,
  diffieHellman,
  fips,
  generateKey,
  generateKeyPair,
  generateKeyPairSync,
  generateKeySync,
  generatePrime,
  generatePrimeSync,
  getCipherInfo,
  getCiphers,
  getCurves,
  getDiffieHellman,
  getFips,
  getHashes,
  hash,
  hkdf,
  hkdfSync,
  pbkdf2,
  pbkdf2Sync,
  privateDecrypt,
  privateEncrypt,
  prng,
  pseudoRandomBytes,
  publicDecrypt,
  publicEncrypt,
  rng,
  scrypt,
  scryptSync,
  sign,
  timingSafeEqual,
  verify,
} from "node:crypto";

const RSA_PRIV_PEM = `-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQC1T4wUmOjE+7gY
aKUW3UspZd7nckJM3FJLvts80hvLauND7XsHJcVThVaQCbgxqm+DN6k6mIWNCE/o
wxdq8whWo65UUzHt1YBxFoRLS7QJpb3nT2AaZ5gUiOb1uOCfGXsbT0594PYIPhEZ
tNDuFOH96ZkerYRfnUfwgQMiJXqy9PUYL+4TCy57ZwV3oNJqGaYfkWCBhe3vF+hs
G9IQcaBTqJQsE7FYAuEm+hFdNOb9ByzETeqfYAcD1z5MY9M+NAKP6OHUq2ftuTBF
s6p52XAM1tln/ssFDrLSgtDksPSAOrdkBkJ4QjpaTKy9YdJ1Ascgdtp5zNAqIt7t
GH7rdKLbAgMBAAECggEAIEIC3jdGUqWmX1Fe7e55TOg+/FarzRj121wdjXikUVC3
FoLN/LuPnAiBGidgihWUuXfRuAci27C65uKu6+xJc4Czp3UV0QbHT1d+OhdB9OV0
IA9TPd9og17IGLfH3wadjIJTqLm5jEXerQAqm/FDM/W3tGmSv6AwCG4rrIuWhPgb
PUpv/ajsS5psCCTqxHixSZq8XmeLep6Ngs+J/jAA8rQDbjufIM9sfttTu5nIoKFx
Be3EqvxSTZM32YJQk9Im2zx95g+xpXrDVq7XfJj2yHCzfsPQCY50zEP5+BXS7pp5
/Ot/mmAU6zi2GT4NKHPR4AFwTSVTUxeDdFyyPFCYIQKBgQDmbo7svLljMRH39zQ7
B3u5MHe6XhT+b5A359TPioav2AvybAnXXJwDB25bVtKaJG/qIzpVzH8GQHrkv+C8
DZm69JS532o4YxYQroeu4+lDzo0rKou63LBh0nW6iugUcHL5e2apPExf0rNKQjWH
xPkJ3+uh0+GN/EKGR31yzelnDwKBgQDJbbKhFSLEQMmwF0fn4xydDb/aGpK0nDP+
Hv76flI8kJSBYeoeNb9GfjipqmbGKZrTHA2j6ZpduX3HT1c4x1Ua9oB7KCf8wUDV
Qv2nM5FtSc3ECBGx+5zLMCkNp6/eHsRNxhOv8FpWSR9InmLdNPp+uhqgknmE6hqE
Hw9EH5bndQKBgGvusyFuEIPGkz0LXDqd+bS3YA06TikTRoD5JPQFVar9zjLBqiZm
6Wh9zZIY69JpaMZJs+RGQeVCxI1q1rAeyrgIIbWiURWSS96KcWFYvYUWNYdXiVWA
U1mug5hZwfT46tGynFZ+uONVRMdulbY/a8rVSIBnR0oDM4HD8J2al02/AoGAGNyS
VLGAJgr9Gd/1vxhYGsZsrKXk/6fb/vyiahaXXeRWgBWuyOXHYxMHOWHP55uRAYeP
43Bc3j6b2LLhSB/KUFamiwzsypSrxObM2gtSYtPeMtjwHjC5LJcYKhhNa7QOdK8D
FQ8sh0PlOZhbCYSVenjL+o+5sf8XIMGYp8biYFUCgYAXI5mUNTHbAC0IFiuILnsd
Q9yfkx0/8e4BZdhuD4RIcIGx23mgiwuUDhOU1pzqQIg/NC70+gPym5uUVunDIjXE
MLmDWlCOsG0Rpu7JjZoHfB7iXyWiF4xMix1yMC3/1xh3AquD5sZF0bM80a4XNN3u
r/vDd4HQV7URDulOqhBmhw==
-----END PRIVATE KEY-----
`;

const RSA_PUB_PEM = `-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAtU+MFJjoxPu4GGilFt1L
KWXe53JCTNxSS77bPNIby2rjQ+17ByXFU4VWkAm4MapvgzepOpiFjQhP6MMXavMI
VqOuVFMx7dWAcRaES0u0CaW9509gGmeYFIjm9bjgnxl7G09OfeD2CD4RGbTQ7hTh
/emZHq2EX51H8IEDIiV6svT1GC/uEwsue2cFd6DSahmmH5FggYXt7xfobBvSEHGg
U6iULBOxWALhJvoRXTTm/QcsxE3qn2AHA9c+TGPTPjQCj+jh1Ktn7bkwRbOqedlw
DNbZZ/7LBQ6y0oLQ5LD0gDq3ZAZCeEI6WkysvWHSdQLHIHbaeczQKiLe7Rh+63Si
2wIDAQAB
-----END PUBLIC KEY-----
`;

const EC_PRIV_SEC1_PEM = `-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIHJKuO2IxVY2KHcPuQF/CJD7u8h0bCB81d7wpwDZ4lXFoAoGCCqGSM49
AwEHoUQDQgAEX3TW7olt6SQbsfXAtem+QoXb+TNSQ6sHYaXQVOV/QUAbugxyg/zS
d5qxAkcQAYY4DJYOsBwx8pmLmOXCGWyShQ==
-----END EC PRIVATE KEY-----
`;

const EC_PUB_SPKI_PEM = `-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEX3TW7olt6SQbsfXAtem+QoXb+TNS
Q6sHYaXQVOV/QUAbugxyg/zSd5qxAkcQAYY4DJYOsBwx8pmLmOXCGWyShQ==
-----END PUBLIC KEY-----
`;

const ED_PRIV_PKCS8_HEX =
  "302e020100300506032b657004220420449f67d1881030b61016cbe2e902556d26a07797aaa2529de1dfc9d96fb26ffb";
const X25519_PRIV_PKCS8_HEX =
  "302e020100300506032b656e0422042020dc818e862d1060421d4217ce161c57e4873dc2e406a7990276c99b62a86075";

const CERT_PEM = `-----BEGIN CERTIFICATE-----
MIIDHDCCAgSgAwIBAgIJAME+EnqOcCBnMA0GCSqGSIb3DQEBCwUAMDUxCzAJBgNV
BAYTAlVTMRAwDgYDVQQKDAdMZXhlT3JnMRQwEgYDVQQDDAtleGFtcGxlLmNvbTAe
Fw0yNjExMjUxMzU0NDVaFw0yNzExMjUxMzU0NDVaMDUxCzAJBgNVBAYTAlVTMRAw
DgYDVQQKDAdMZXhlT3JnMRQwEgYDVQQDDAtleGFtcGxlLmNvbTCCASIwDQYJKoZI
hvcNAQEBBQADggEPADCCAQoCggEBANdfUcqI9WM7TbxzUuwhstw2z0iQgX9BxHvN
2ASCkENC9hv5OFhw68b/7vrOfUSaiF+28mVQVaoi9GBZbZK2s1ROzE00Hw7lPDCr
OdulUU1aizBpU7X/b3Od7Lm5ugGmyrO7mK0v857Fdbwq38i0bcAAlBRkRhudLVW+
i0IdqI92WeYcsffY0bQMARlFxoZ433+pVDonPSZeK9nOh03RWjCLlGBoG0zgI5mf
j3CGNvFeRMKw9g3m/JSnvKErLL3CQU9jtnW9lr9ydBCuNjaEI1zYGyoMcPng4na7
+UamVNhMRCYNgMhc7U60bXldaZhc/RHES1At6pBnZ2SIQctnf/UCAwEAAaMvMC0w
KwYDVR0RBCQwIoILZXhhbXBsZS5jb22CDSouZXhhbXBsZS5jb22HBH8AAAEwDQYJ
KoZIhvcNAQELBQADggEBAHFd4MDwVCEge5X/bYLlDJ88YpeANbl6DQXSUAr53zbc
TMd+hys9x00gHGmiEFbpGaNB8J1WBoHoz8Atlb+LRdwn3/yfMqW7DPJXo8tPlcBt
RKeFJ03PaDn3YfapQh7dcjHF/4UPckQZwuzu4g4xRSvXgDJzy8OkRUPqpJBDzDfX
xTp3GXYK9tWkSMLW/k+Afd5Q+i0+potfobWtjAMON5FsWm9E+Px/Cesg3V5NkRBn
Km5La29Rg9ok7k3UpQHkltJkHGWf2v/ub61mxgmat1LifiFNdVjXChwOSByRZArd
6qD6ovuhEw/LoJhiSx1+cBCKC5Q8F7WydPp+EuR+DxE=
-----END CERTIFICATE-----
`;

describe("crypto classic & KeyObject & X509", () => {
  it("exports classic symbols on both named and default exports", () => {
    expect(defaultCrypto.createCipheriv).toBe(createCipheriv);
    expect(defaultCrypto.createDecipheriv).toBe(createDecipheriv);
    expect(defaultCrypto.createECDH).toBe(createECDH);
    expect(defaultCrypto.diffieHellman).toBe(diffieHellman);
    expect(defaultCrypto.createPrivateKey).toBe(createPrivateKey);
    expect(defaultCrypto.createPublicKey).toBe(createPublicKey);
    expect(defaultCrypto.createSecretKey).toBe(createSecretKey);
    expect(defaultCrypto.KeyObject).toBe(KeyObject);
    expect(defaultCrypto.timingSafeEqual).toBe(timingSafeEqual);
    expect(defaultCrypto.hkdfSync).toBe(hkdfSync);
    expect(defaultCrypto.sign).toBe(sign);
    expect(defaultCrypto.verify).toBe(verify);
    expect(defaultCrypto.X509Certificate).toBe(X509Certificate);
    expect(constants.RSA_PKCS1_PADDING).toBe(1);
    expect(constants.RSA_PKCS1_PSS_PADDING).toBe(6);
  });

  it("supports timingSafeEqual and throws RangeError with ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH", () => {
    expect(timingSafeEqual(Buffer.from("abc"), Buffer.from("abc"))).toBe(true);
    expect(timingSafeEqual(Buffer.from("abc"), Buffer.from("abd"))).toBe(false);
    expect(
      timingSafeEqual(new Uint8Array([1, 2, 3]), new DataView(new Uint8Array([1, 2, 3]).buffer))
    ).toBe(true);

    let thrown: any;
    try {
      timingSafeEqual(Buffer.from("a"), Buffer.from("ab"));
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(RangeError);
    expect(thrown.code).toBe("ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH");
  });

  it("supports hkdfSync and async hkdf matching Node output", async () => {
    const ab = hkdfSync("sha256", "ikm-secret", "salt-val", "info-ctx", 32);
    expect(ab).toBeInstanceOf(ArrayBuffer);
    expect(Buffer.from(ab).toString("hex")).toBe(
      "7e0558d87249ba2007370e71a5d97746bf18d2fb6759825b03d1a74c486c8d75"
    );

    const asyncBuf = await new Promise<ArrayBuffer>((resolve, reject) => {
      hkdf("sha256", "ikm-secret", "salt-val", "info-ctx", 32, (err, derivedKey) => {
        if (err) reject(err);
        else resolve(derivedKey);
      });
    });
    expect(asyncBuf).toBeInstanceOf(ArrayBuffer);
    expect(Buffer.from(asyncBuf).toString("hex")).toBe(
      "7e0558d87249ba2007370e71a5d97746bf18d2fb6759825b03d1a74c486c8d75"
    );
  });

  it("supports streaming AES-256-GCM, AES-256-CBC, AES-256-CTR matching Node output", () => {
    const key256 = Buffer.from(
      "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
      "hex"
    );
    const iv12 = Buffer.from("cafebabefacedbaddecaf888", "hex");
    const iv16 = Buffer.from("00112233445566778899aabbccddeeff", "hex");
    const aad = Buffer.from("additional-auth-data");
    const msg = Buffer.from("Hello Lexe Crypto Classic Streaming Parity!");

    const gcm = createCipheriv("aes-256-gcm", key256, iv12);
    gcm.setAAD(aad);
    const gcmChunk1 = gcm.update(msg.subarray(0, 10));
    const gcmChunk2 = gcm.update(msg.subarray(10));
    expect(gcmChunk1.length).toBe(10);
    expect(gcmChunk2.length).toBe(msg.length - 10);
    const gcmCt = Buffer.concat([gcmChunk1, gcmChunk2, gcm.final()]);
    const gcmTag = gcm.getAuthTag();
    expect(gcmCt.toString("hex")).toBe(
      "c2c6cc4ac55a037e3e6e7d9e0964f94b6200833dbe6a191d2dff5705ccec0492c43ea66691b53b0b522209"
    );
    expect(gcmTag.toString("hex")).toBe("2f577915333aec5e96b4f499c41d4cc6");

    const dgcm = createDecipheriv("aes-256-gcm", createSecretKey(key256), iv12);
    dgcm.setAAD(aad);
    dgcm.setAuthTag(gcmTag);
    const gcmPt = Buffer.concat([dgcm.update(gcmCt.subarray(0, 15)), dgcm.update(gcmCt.subarray(15)), dgcm.final()]);
    expect(gcmPt.toString("utf8")).toBe(msg.toString("utf8"));

    // Tampered tag must fail final()
    const badDecipher = createDecipheriv("aes-256-gcm", key256, iv12);
    badDecipher.setAAD(aad);
    const badTag = Buffer.from(gcmTag);
    badTag[0] ^= 0xff;
    badDecipher.setAuthTag(badTag);
    badDecipher.update(gcmCt);
    expect(() => badDecipher.final()).toThrow();

    // CBC
    const cbc = createCipheriv("aes-256-cbc", key256, iv16);
    const cbcCt = Buffer.concat([cbc.update(msg.subarray(0, 7)), cbc.update(msg.subarray(7)), cbc.final()]);
    expect(cbcCt.toString("hex")).toBe(
      "3e7e9dd5be439a69ab787c8828e03ee0657cf16e21eb44d52d797767e81a6da7affe8300577c464c19bf4ec0b55f3fb1"
    );
    const dcbc = createDecipheriv("aes-256-cbc", key256, iv16);
    const cbcPt = Buffer.concat([dcbc.update(cbcCt.subarray(0, 20)), dcbc.update(cbcCt.subarray(20)), dcbc.final()]);
    expect(cbcPt.toString("utf8")).toBe(msg.toString("utf8"));

    // CTR
    const ctr = createCipheriv("aes-256-ctr", key256, iv16);
    const ctrCt = Buffer.concat([ctr.update(msg.subarray(0, 19)), ctr.update(msg.subarray(19)), ctr.final()]);
    expect(ctrCt.toString("hex")).toBe(
      "c6c7dba63e4709da929969d3393010fd995fcd9e2d82ffa31a2dfe263c38d81649d561337051ae1c340f98"
    );
    const dctr = createDecipheriv("aes-256-ctr", key256, iv16);
    expect(Buffer.concat([dctr.update(ctrCt), dctr.final()]).toString("utf8")).toBe(
      msg.toString("utf8")
    );
  });

  it("supports KeyObject, derive public keys, export, sign/verify (Ed25519, ECDSA, RSA-PSS/PKCS1), and DH/ECDH", () => {
    // SecretKey
    const sec = createSecretKey(Buffer.from("0123456789abcdef"));
    expect(sec).toBeInstanceOf(KeyObject);
    expect(sec.type).toBe("secret");
    expect(sec.symmetricKeySize).toBe(16);
    expect(sec.asymmetricKeyType).toBeUndefined();
    expect(sec.export().toString()).toBe("0123456789abcdef");

    // RSA
    const rsaPriv = createPrivateKey(RSA_PRIV_PEM);
    const rsaPub = createPublicKey(rsaPriv);
    expect(rsaPriv.type).toBe("private");
    expect(rsaPriv.asymmetricKeyType).toBe("rsa");
    expect(rsaPriv.asymmetricKeyDetails.modulusLength).toBe(2048);
    expect(rsaPriv.asymmetricKeyDetails.publicExponent).toBe(65537n);
    expect(rsaPub.equals(createPublicKey(RSA_PUB_PEM))).toBe(true);

    const payload = Buffer.from("sign this payload");
    const rsaSig = sign("sha256", payload, rsaPriv);
    expect(verify("sha256", payload, rsaPub, rsaSig)).toBe(true);
    expect(verify("sha256", Buffer.from("tampered"), rsaPub, rsaSig)).toBe(false);

    const pssSig = sign(
      "sha256",
      payload,
      { key: rsaPriv, padding: constants.RSA_PKCS1_PSS_PADDING, saltLength: 32 }
    );
    expect(
      verify("sha256", payload, { key: rsaPub, padding: constants.RSA_PKCS1_PSS_PADDING }, pssSig)
    ).toBe(true);

    // EC P-256 (SEC1 + SPKI + JWK + ieee-p1363 + diffieHellman)
    const ecPriv = createPrivateKey(EC_PRIV_SEC1_PEM);
    const ecPub = createPublicKey(EC_PUB_SPKI_PEM);
    expect(ecPriv.asymmetricKeyType).toBe("ec");
    expect(ecPriv.asymmetricKeyDetails.namedCurve).toBe("prime256v1");
    expect(createPublicKey(ecPriv).equals(ecPub)).toBe(true);
    const ecJwk = ecPub.export({ format: "jwk" }) as any;
    expect(createPublicKey({ key: ecJwk, format: "jwk" }).equals(ecPub)).toBe(true);

    const ecSigDer = sign("sha256", payload, ecPriv);
    expect(verify("sha256", payload, ecPub, ecSigDer)).toBe(true);
    const ecSigRaw = sign("sha256", payload, { key: ecPriv, dsaEncoding: "ieee-p1363" });
    expect(ecSigRaw.length).toBe(64);
    expect(verify("sha256", payload, { key: ecPub, dsaEncoding: "ieee-p1363" }, ecSigRaw)).toBe(true);

    // ECDH
    const e1 = createECDH("prime256v1");
    const e2 = createECDH("prime256v1");
    const p1 = e1.generateKeys();
    const p2 = e2.generateKeys(null, "compressed");
    expect(p1.length).toBe(65);
    expect(p2.length).toBe(33);
    expect(e1.getPublicKey(null, "hybrid").length).toBe(65);
    expect(e1.computeSecret(p2).equals(e2.computeSecret(p1))).toBe(true);

    // Ed25519
    const edPriv = createPrivateKey({
      key: Buffer.from(ED_PRIV_PKCS8_HEX, "hex"),
      format: "der",
      type: "pkcs8",
    });
    const edPub = createPublicKey(edPriv);
    expect(edPriv.asymmetricKeyType).toBe("ed25519");
    expect((edPub.export({ format: "jwk" }) as any).crv).toBe("Ed25519");
    const edSig = sign(null, payload, edPriv);
    expect(edSig.length).toBe(64);
    expect(verify(null, payload, edPub, edSig)).toBe(true);

    // X25519 + diffieHellman
    const xPriv = createPrivateKey({
      key: Buffer.from(X25519_PRIV_PKCS8_HEX, "hex"),
      format: "der",
      type: "pkcs8",
    });
    const xPub = createPublicKey(xPriv);
    expect(xPriv.asymmetricKeyType).toBe("x25519");
    const dhSelf = diffieHellman({ privateKey: xPriv, publicKey: xPub });
    expect(dhSelf.length).toBe(32);
    expect(diffieHellman({ privateKey: ecPriv, publicKey: ecPub }).length).toBe(32);
  });

  it("supports X509Certificate parsing, metadata, host check, and signature verification", () => {
    const cert = new X509Certificate(CERT_PEM);
    expect(cert.subject).toBe("C=US\nO=LexeOrg\nCN=example.com");
    expect(cert.issuer).toBe("C=US\nO=LexeOrg\nCN=example.com");
    expect(cert.validFrom).toBe("Nov 25 13:54:45 2026 GMT");
    expect(cert.validTo).toBe("Nov 25 13:54:45 2027 GMT");
    expect(cert.validFromDate).toBeInstanceOf(Date);
    expect(cert.validFromDate.toISOString()).toBe("2026-11-25T13:54:45.000Z");
    expect(cert.fingerprint).toBe(
      "4E:5C:B9:02:AE:6D:37:C7:14:FC:42:CF:58:ED:AC:98:CE:0C:C2:92"
    );
    expect(cert.fingerprint256).toBe(
      "84:E2:71:A8:72:3F:D7:05:76:66:9E:BC:29:86:55:0C:AB:1A:18:BF:F7:B4:C7:A5:56:74:F8:F4:43:74:FC:4A"
    );
    expect(cert.ca).toBe(false);
    expect(cert.subjectAltName).toBe("DNS:example.com, DNS:*.example.com, IP Address:127.0.0.1");
    expect(cert.checkHost("example.com")).toBe("example.com");
    expect(cert.checkHost("api.example.com")).toBe("*.example.com");
    expect(cert.checkHost("a.b.example.com")).toBeUndefined();
    expect(cert.checkHost("other.org")).toBeUndefined();
    expect(cert.publicKey.asymmetricKeyType).toBe("rsa");
    expect(cert.verify(cert.publicKey)).toBe(true);
    expect(cert.verify(createPublicKey(RSA_PUB_PEM))).toBe(false);
    expect(cert.serialNumber).toBe("C13E127A8E702067");
  });

  it("exposes getHashes, getCiphers, and getCurves matching supported algorithms", () => {
    const hashes = getHashes();
    const ciphers = getCiphers();
    const curves = getCurves();
    expect(defaultCrypto.getHashes()).toEqual(hashes);
    expect(defaultCrypto.getCiphers()).toEqual(ciphers);
    expect(defaultCrypto.getCurves()).toEqual(curves);
    expect(Array.isArray(hashes)).toBe(true);
    expect(hashes).toContain("sha256");
    expect(hashes).toContain("sha512");
    expect(hashes).toContain("md5");
    expect(Array.isArray(ciphers)).toBe(true);
    expect(ciphers).toContain("aes-256-gcm");
    expect(ciphers).toContain("chacha20-poly1305");
    expect(Array.isArray(curves)).toBe(true);
    expect(curves).toContain("prime256v1");
  });

  it("supports crypto.hash, Hash/Hmac classes, createSign/createVerify, and PRNG aliases", () => {
    expect(hash("sha256", "abc")).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    expect(hash("sha256", Buffer.from("abc"), "buffer")).toEqual(
      Buffer.from("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", "hex")
    );
    expect(createHash("sha256")).toBeInstanceOf(Hash);
    expect(createHmac("sha256", "secret")).toBeInstanceOf(Hmac);
    expect(fips).toBe(0);
    expect(getFips()).toBe(0);
    expect(pseudoRandomBytes(16).length).toBe(16);
    expect(prng(16).length).toBe(16);
    expect(rng(16).length).toBe(16);

    const s = createSign("SHA256");
    expect(s).toBeInstanceOf(Sign);
    const sig = s.update("hello ").update("world").sign(RSA_PRIV_PEM, "base64");
    const v = createVerify("SHA256");
    expect(v).toBeInstanceOf(Verify);
    expect(v.update("hello world").verify(RSA_PUB_PEM, sig, "base64")).toBe(true);
  });

  it("supports pbkdf2/pbkdf2Sync and scrypt/scryptSync", async () => {
    // RFC 6070 vector 1
    const derived = pbkdf2Sync("password", "salt", 1, 20, "sha1");
    expect(derived.toString("hex")).toBe("0c60c80f961f0e71f3a9b524af6012062fe037a6");

    const asyncPbkdf2 = await new Promise<Buffer>((resolve, reject) => {
      pbkdf2("password", "salt", 2, 20, "sha1", (err, key) => {
        if (err) reject(err);
        else resolve(key);
      });
    });
    expect(asyncPbkdf2.toString("hex")).toBe("ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957");

    // RFC 7914 empty vector
    const sc = scryptSync("", "", 64, { N: 16, r: 1, p: 1 });
    expect(sc.slice(0, 16).toString("hex")).toBe("77d6576238657b203b19ca42c18a0497");

    const asyncScrypt = await new Promise<Buffer>((resolve, reject) => {
      scrypt("password", "NaCl", 16, { cost: 16, blockSize: 1, parallelization: 1 }, (err, key) => {
        if (err) reject(err);
        else resolve(key);
      });
    });
    expect(asyncScrypt.length).toBe(16);
  });

  it("supports generateKey*, generateKeyPair* (including Ed448/X448 & passphrase export)", async () => {
    const aesKey = generateKeySync("aes", { length: 256 });
    expect(aesKey).toBeInstanceOf(KeyObject);
    expect(aesKey.symmetricKeySize).toBe(32);

    const hmacKey = await new Promise<KeyObject>((resolve, reject) => {
      generateKey("hmac", { length: 128 }, (err, k) => (err ? reject(err) : resolve(k)));
    });
    expect(hmacKey.symmetricKeySize).toBe(16);

    const { publicKey: ed448Pub, privateKey: ed448Priv } = generateKeyPairSync("ed448");
    expect(ed448Pub.asymmetricKeyType).toBe("ed448");
    const sig448 = sign(null, Buffer.from("ed448 message"), ed448Priv);
    expect(sig448.length).toBe(114);
    expect(verify(null, Buffer.from("ed448 message"), ed448Pub, sig448)).toBe(true);

    const a448 = generateKeyPairSync("x448");
    const b448 = await new Promise<{ publicKey: KeyObject; privateKey: KeyObject }>(
      (resolve, reject) => {
        generateKeyPair("x448", (err, publicKey, privateKey) =>
          err ? reject(err) : resolve({ publicKey, privateKey })
        );
      }
    );
    const s1 = diffieHellman({ privateKey: a448.privateKey, publicKey: b448.publicKey });
    const s2 = diffieHellman({ privateKey: b448.privateKey, publicKey: a448.publicKey });
    expect(s1.length).toBe(56);
    expect(s1.equals(s2)).toBe(true);

    const { publicKey: rsaPubPem, privateKey: rsaEncPem } = generateKeyPairSync("rsa", {
      modulusLength: 1024,
      publicKeyEncoding: { type: "spki", format: "pem" },
      privateKeyEncoding: {
        type: "pkcs8",
        format: "pem",
        cipher: "aes-256-cbc",
        passphrase: "top-secret-passphrase",
      },
    });
    expect(rsaEncPem).toContain("BEGIN ENCRYPTED PRIVATE KEY");
    const importedPriv = createPrivateKey({
      key: rsaEncPem,
      passphrase: "top-secret-passphrase",
    });
    expect(importedPriv.asymmetricKeyType).toBe("rsa");
    const importedPub = createPublicKey(rsaPubPem);
    expect(importedPub.asymmetricKeyType).toBe("rsa");
  });

  it("supports RSA publicEncrypt, privateDecrypt, privateEncrypt, and publicDecrypt", () => {
    const ctOaep = publicEncrypt(
      { key: RSA_PUB_PEM, padding: constants.RSA_PKCS1_OAEP_PADDING, oaepHash: "sha256" },
      Buffer.from("secret oaep")
    );
    const ptOaep = privateDecrypt(
      { key: RSA_PRIV_PEM, padding: constants.RSA_PKCS1_OAEP_PADDING, oaepHash: "sha256" },
      ctOaep
    );
    expect(ptOaep.toString()).toBe("secret oaep");

    // Node 24 rejects RSA_PKCS1_PADDING on privateDecrypt
    const ctPkcs1 = publicEncrypt(
      { key: RSA_PUB_PEM, padding: constants.RSA_PKCS1_PADDING },
      Buffer.from("legacy pkcs1")
    );
    expect(() =>
      privateDecrypt({ key: RSA_PRIV_PEM, padding: constants.RSA_PKCS1_PADDING }, ctPkcs1)
    ).toThrow(/no longer supported/);

    // Reverse RSA (privateEncrypt -> publicDecrypt)
    const privEnc = privateEncrypt(RSA_PRIV_PEM, Buffer.from("signed block"));
    expect(publicDecrypt(RSA_PUB_PEM, privEnc).toString()).toBe("signed block");

    // Raw RSA_NO_PADDING (256-byte modulus)
    const rawBlock = Buffer.alloc(256, 0);
    Buffer.from("raw-rsa").copy(rawBlock, 240);
    const encRaw = publicEncrypt(
      { key: RSA_PUB_PEM, padding: constants.RSA_NO_PADDING },
      rawBlock
    );
    const decRaw = privateDecrypt(
      { key: RSA_PRIV_PEM, padding: constants.RSA_NO_PADDING },
      encRaw
    );
    expect(decRaw.equals(rawBlock)).toBe(true);
  });

  it("supports DiffieHellman, DiffieHellmanGroup, primes, ChaCha20-Poly1305, and getCipherInfo", async () => {
    const dh1 = getDiffieHellman("modp14");
    const dh2 = getDiffieHellman("modp14");
    expect(dh1).toBeInstanceOf(DiffieHellmanGroup);
    const pub1 = dh1.generateKeys();
    const pub2 = dh2.generateKeys();
    expect(dh1.computeSecret(pub2).equals(dh2.computeSecret(pub1))).toBe(true);

    const custom = createDiffieHellman(256);
    expect(custom).toBeInstanceOf(DiffieHellman);
    expect(custom.verifyError).toBe(0);
    expect(custom.generateKeys("hex").length).toBeGreaterThan(0);

    const pBuf = generatePrimeSync(128, { safe: true });
    expect(pBuf).toBeInstanceOf(ArrayBuffer);
    expect(pBuf.byteLength).toBe(16);
    expect(checkPrimeSync(pBuf)).toBe(true);
    const pBig = await new Promise<bigint>((resolve, reject) => {
      generatePrime(64, { bigint: true }, (err, p) => (err ? reject(err) : resolve(p)));
    });
    expect(typeof pBig).toBe("bigint");
    expect(
      await new Promise<boolean>((resolve, reject) => {
        checkPrime(pBig, (err, ok) => (err ? reject(err) : resolve(ok)));
      })
    ).toBe(true);
    expect(checkPrimeSync(15n)).toBe(false);

    const key = Buffer.alloc(32, 0x11);
    const iv = Buffer.alloc(12, 0x22);
    const aad = Buffer.from("aad-chacha");
    const c = createCipheriv("chacha20-poly1305", key, iv);
    c.setAAD(aad);
    const ct = Buffer.concat([c.update("streamed "), c.update("chacha"), c.final()]);
    const tag = c.getAuthTag();
    const d = createDecipheriv("chacha20-poly1305", key, iv);
    d.setAAD(aad);
    d.setAuthTag(tag);
    const pt = Buffer.concat([d.update(ct), d.final()]);
    expect(pt.toString()).toBe("streamed chacha");

    expect(getCipherInfo("chacha20-poly1305")?.nid).toBe(1018);
    expect(getCipherInfo(901)?.name).toBe("id-aes256-gcm");
  });
});
