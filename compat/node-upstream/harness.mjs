import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

const [testFile] = process.argv.slice(2);
if (!testFile) {
  console.error("Usage: harness.mjs <test-file>");
  process.exit(2);
}

const resolvedTest = path.resolve(testFile);
process.env.NODE_SKIP_FLAG_CHECK = "1";
process.env.NODE_TEST_KNOWN_GLOBALS ??= "0";
process.argv = [process.execPath, resolvedTest];

if (!process.config || typeof process.config !== "object") {
  Object.defineProperty(process, "config", {
    value: { target_defaults: {}, variables: {} },
    configurable: true,
    writable: true,
  });
}
if (!process.config.variables || typeof process.config.variables !== "object") {
  process.config.variables = {};
}
if (process.config.variables.v8_enable_i18n_support === undefined) {
  process.config.variables.v8_enable_i18n_support = true;
}

if (typeof process.features !== "object" || process.features === null) {
  Object.defineProperty(process, "features", {
    value: { inspector: false, debug: false, uv: true, ipv6: true, tls_alpn: true, tls_sni: true, tls_ocsp: false, tls: true },
    configurable: true,
    writable: true,
  });
}

// Node's test/common gates TLS/http2 suites on Boolean(process.versions.openssl).
// Expose a test-runner-only marker when TLS/crypto is enabled so those suites run rather than skipping.
if (process.env.LEXE_UPSTREAM_ASSUME_TLS !== "0" && !process.versions.openssl) {
  try {
    process.versions.openssl = "3.5.0";
  } catch {
    Object.defineProperty(process, "versions", {
      value: { ...process.versions, openssl: "3.5.0" },
      configurable: true,
    });
  }
}

const isEsm =
  resolvedTest.endsWith(".mjs") ||
  (() => {
    try {
      const head = fs.readFileSync(resolvedTest, "utf8").slice(0, 2048);
      return /^\s*(import|export)\s/m.test(head);
    } catch {
      return false;
    }
  })();

if (isEsm) {
  await import(pathToFileURL(resolvedTest).href);
} else {
  const require = createRequire(resolvedTest);
  require(resolvedTest);
}
