// `URL.createObjectURL` / `URL.revokeObjectURL`, paired with
// `buffer.resolveObjectURL` through a shared store keyed by Symbol.for(...).
//
// References (refs/node): lib/internal/url.js (installObjectURLMethods),
// src/node_blob.cc (RevokeObjectURL).
(function (URLCtor) {
  "use strict";

  const storeKey = Symbol.for("raster.internal.blobURLStore");

  function makeError(Base, code, message) {
    const error = new Base(message);
    Object.defineProperty(error, "code", {
      value: code,
      enumerable: true,
      configurable: true,
      writable: true,
    });
    return error;
  }

  function describeReceived(value) {
    if (value === undefined) return "undefined";
    if (value === null) return "null";
    if (typeof value === "function") {
      return value.name ? `function ${value.name}` : "function ";
    }
    if (typeof value === "object") {
      const name = value.constructor && value.constructor.name;
      return name ? `an instance of ${name}` : "[Object: null prototype]";
    }
    let shown = String(value);
    if (typeof value === "string") {
      if (shown.length > 28) shown = `${shown.slice(0, 25)}...`;
      shown = `'${shown}'`;
    }
    return `type ${typeof value} (${shown})`;
  }

  function store() {
    let map = globalThis[storeKey];
    if (map === undefined) {
      map = new Map();
      Object.defineProperty(globalThis, storeKey, {
        value: map,
        enumerable: false,
        configurable: true,
        writable: true,
      });
    }
    return map;
  }

  function createObjectURL(obj) {
    const crypto = globalThis.crypto;
    if (crypto === undefined || typeof crypto.randomUUID !== "function") {
      throw makeError(Error, "ERR_NO_CRYPTO", "Node.js is not compiled with OpenSSL crypto support");
    }
    if (typeof Blob !== "function" || !(obj instanceof Blob)) {
      throw makeError(
        TypeError,
        "ERR_INVALID_ARG_TYPE",
        `The "obj" argument must be an instance of Blob. Received ${describeReceived(obj)}`
      );
    }
    const id = crypto.randomUUID();
    store().set(id, obj.slice(0, obj.size, obj.type));
    return `blob:nodedata:${id}`;
  }

  function revokeObjectURL(url) {
    if (arguments.length === 0) {
      throw makeError(TypeError, "ERR_MISSING_ARGS", 'The "url" argument must be specified');
    }
    let pathname;
    try {
      pathname = new URLCtor(`${url}`).pathname;
    } catch {
      return;
    }
    const first = pathname.indexOf(":");
    if (first === -1 || first === pathname.length) return;
    if (pathname.indexOf(":", first + 1) !== -1) return;
    const map = globalThis[storeKey];
    if (map !== undefined) map.delete(pathname.slice(first + 1));
  }

  for (const [name, value] of [
    ["createObjectURL", createObjectURL],
    ["revokeObjectURL", revokeObjectURL],
  ]) {
    Object.defineProperty(URLCtor, name, {
      value,
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
})
