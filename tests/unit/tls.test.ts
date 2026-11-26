import defaultImport from "node:tls";
import legacyImport from "tls";

it("node:tls should be the same as tls", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

it("should export connect and createServer", () => {
  expect(typeof legacyImport.connect).toBe("function");
  expect(typeof legacyImport.createServer).toBe("function");
});

it("should export TLSSocket and Server classes", () => {
  expect(typeof legacyImport.TLSSocket).toBe("function");
  expect(typeof legacyImport.Server).toBe("function");
});

it("should export constants", () => {
  expect(legacyImport.CLIENT_RENEG_LIMIT).toBe(3);
  expect(legacyImport.CLIENT_RENEG_WINDOW).toBe(600);
  expect(typeof legacyImport.DEFAULT_CIPHERS).toBe("string");
  expect(legacyImport.DEFAULT_ECDH_CURVE).toBe("auto");
  expect(legacyImport.DEFAULT_MIN_VERSION).toBe("TLSv1.2");
  expect(legacyImport.DEFAULT_MAX_VERSION).toBe("TLSv1.3");
});

it("ESM import works", async () => {
  const tls = await import("node:tls");
  expect(typeof tls.connect).toBe("function");
});

it("TLSSocket exposes setKeepAlive, setNoDelay, and live TLS getters", () => {
  const sock = new legacyImport.TLSSocket();
  expect(sock.encrypted).toBe(true);
  expect(sock.alpnProtocol).toBe(false);
  expect(sock.authorized).toBe(false);
  expect(Array.isArray(sock.autoSelectFamilyAttemptedAddresses)).toBe(true);
  expect(sock.setNoDelay(true)).toBe(sock);
  expect(sock.setKeepAlive(true, 1000)).toBe(sock);
  sock.destroy();
});
