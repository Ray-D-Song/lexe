import Module, { createRequire } from "node:module";
import * as constantsNs from "node:constants";
import constants, {
  F_OK,
  R_OK,
  W_OK,
  X_OK,
  O_RDONLY,
  O_CREAT,
  S_IFMT,
  S_IFREG,
  S_IFDIR,
  COPYFILE_EXCL,
  UV_DIRENT_FILE,
  ENOENT,
  EINVAL,
  SIGINT,
  SIGTERM,
  PRIORITY_LOW,
  PRIORITY_HIGHEST,
  RSA_PKCS1_PADDING,
  RSA_PKCS1_OAEP_PADDING,
  SSL_OP_NO_TLSv1_2,
  SSL_OP_CRYPTOPRO_TLSEXT_BUG,
  TLS1_3_VERSION,
  defaultCoreCipherList,
} from "node:constants";
import legacy from "constants";
import fs from "node:fs";
import os from "node:os";

const require = createRequire(import.meta.url);

it("node:constants should match constants default", () => {
  expect(constants.F_OK).toBe(legacy.F_OK);
  expect(constants.R_OK).toBe(legacy.R_OK);
  expect(constants.W_OK).toBe(legacy.W_OK);
  expect(constants.X_OK).toBe(legacy.X_OK);
  expect(constants.ENOENT).toBe(legacy.ENOENT);
  expect(constants.SIGINT).toBe(legacy.SIGINT);
  expect(constants.O_RDONLY).toBe(legacy.O_RDONLY);
  expect(constants.RSA_PKCS1_OAEP_PADDING).toBe(legacy.RSA_PKCS1_OAEP_PADDING);
});

it("Module.isBuiltin recognizes constants", () => {
  expect(Module.isBuiltin("constants")).toBe(true);
  expect(Module.isBuiltin("node:constants")).toBe(true);
  expect(Module.builtinModules).toContain("constants");
});

it("matches fs.constants, os.constants, and crypto constants", () => {
  expect(F_OK).toBe(0);
  expect(R_OK).toBe(4);
  expect(W_OK).toBe(2);
  expect(X_OK).toBe(1);
  expect(O_RDONLY).toBe(0);
  expect(O_CREAT).toBe(fs.constants.O_CREAT);
  expect(S_IFMT).toBe(0xf000);
  expect(S_IFREG).toBe(0x8000);
  expect(S_IFDIR).toBe(0x4000);
  expect(COPYFILE_EXCL).toBe(1);
  expect(UV_DIRENT_FILE).toBe(1);
  expect(Object.getPrototypeOf(fs.constants)).toBeNull();
  expect(fs.constants).toEqual(fs.promises.constants);

  for (const [k, v] of Object.entries(fs.constants)) {
    expect(constants[k]).toBe(v);
  }

  for (const group of [
    os.constants.dlopen,
    os.constants.errno,
    os.constants.signals,
    os.constants.priority,
  ]) {
    for (const [k, v] of Object.entries(group)) {
      expect(constants[k]).toBe(v);
    }
  }
  expect("UV_UDP_REUSEADDR" in constants).toBe(false);

  expect(ENOENT).toBe(os.constants.errno.ENOENT);
  expect(EINVAL).toBe(os.constants.errno.EINVAL);
  expect(SIGINT).toBe(os.constants.signals.SIGINT);
  expect(SIGTERM).toBe(os.constants.signals.SIGTERM);
  expect(PRIORITY_LOW).toBe(19);
  expect(PRIORITY_HIGHEST).toBe(-20);

  expect(RSA_PKCS1_PADDING).toBe(1);
  expect(RSA_PKCS1_OAEP_PADDING).toBe(4);
  expect(SSL_OP_NO_TLSv1_2).toBe(134217728);
  expect(SSL_OP_CRYPTOPRO_TLSEXT_BUG).toBe(0x80000000);
  expect(TLS1_3_VERSION).toBe(772);
  expect(typeof defaultCoreCipherList).toBe("string");
  expect(defaultCoreCipherList.length).toBeGreaterThan(0);

  const cjsKeys = Object.keys(require("node:constants")).sort();
  const esmKeys = Object.keys(constantsNs)
    .filter((k) => k !== "default" && k !== "module.exports")
    .sort();
  expect(esmKeys).toEqual(cjsKeys);
});

it("require('constants') is frozen flat object with platform-accurate O_SYMLINK", () => {
  const c = require("constants");
  expect(Object.getPrototypeOf(c)).toBe(Object.prototype);
  expect(c.F_OK).toBe(fs.constants.F_OK);
  expect(Object.isFrozen(c)).toBe(true);
  if (process.platform === "darwin") {
    expect(typeof c.O_SYMLINK).toBe("number");
  } else {
    expect("O_SYMLINK" in c).toBe(false);
  }
  expect(c.default).toBeUndefined();
  expect(Object.prototype.hasOwnProperty.call(c, "default")).toBe(false);
  expect(Object.prototype.hasOwnProperty.call(c, "__esModule")).toBe(false);
  try {
    c.F_OK = 99;
  } catch {
    // freeze may throw
  }
  expect(c.F_OK).toBe(0);

  const nodeC = require("node:constants");
  expect(nodeC.F_OK).toBe(0);
});

it("require('constants') does not depend on user-overridable Object.isExtensible", () => {
  const original = Object.isExtensible;
  try {
    // @ts-expect-error intentional polyfill/user override
    Object.isExtensible = undefined;
    expect(() => require("constants")).not.toThrow();
    expect(require("constants").F_OK).toBe(0);
  } finally {
    Object.isExtensible = original;
  }
});
