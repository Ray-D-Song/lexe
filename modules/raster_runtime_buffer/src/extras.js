// Additional `node:buffer` exports. Evaluated once per `node:buffer` instance
// with the native helpers and the `default` export object, which it populates.
//
// References (refs/node/lib): buffer.js (transcode, isUtf8, isAscii, SlowBuffer,
// INSPECT_MAX_BYTES), internal/blob.js (resolveObjectURL), internal/util.js
// (normalizeEncoding).
(function (natives, target, maxLength, maxStringLength) {
  "use strict";

  const { transcodeNative, isUtf8Native, isAsciiNative } = natives;

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

  function addNumericalSeparator(value) {
    let result = "";
    let i = value.length;
    const start = value.startsWith("-") ? 1 : 0;
    for (; i >= start + 4; i -= 3) result = `_${value.slice(i - 3, i)}${result}`;
    return `${value.slice(0, i)}${result}`;
  }

  function describeNumber(value) {
    if (Number.isInteger(value) && Math.abs(value) > 2 ** 32) {
      return addNumericalSeparator(String(value));
    }
    return String(value);
  }

  function validateNumber(value, name, min, max) {
    if (typeof value !== "number") {
      throw makeError(
        TypeError,
        "ERR_INVALID_ARG_TYPE",
        `The "${name}" argument must be of type number. Received ${describeReceived(value)}`
      );
    }
    const hasBound = min != null || max != null;
    if ((min != null && value < min) || (max != null && value > max) || (hasBound && value !== value)) {
      const range =
        min != null && max != null ? `>= ${min} && <= ${max}` : min != null ? `>= ${min}` : `<= ${max}`;
      throw makeError(
        RangeError,
        "ERR_OUT_OF_RANGE",
        `The value of "${name}" is out of range. It must be ${range}. Received ${describeNumber(value)}`
      );
    }
  }

  function describeExpectedViews(name, value) {
    return makeError(
      TypeError,
      "ERR_INVALID_ARG_TYPE",
      `The "${name}" argument must be an instance of ArrayBuffer, Buffer, or TypedArray. Received ${describeReceived(value)}`
    );
  }

  const ENCODING_NAMES = new Map([
    ["utf8", "utf8"],
    ["utf-8", "utf8"],
    ["ucs2", "utf16le"],
    ["ucs-2", "utf16le"],
    ["utf16le", "utf16le"],
    ["utf-16le", "utf16le"],
    ["latin1", "latin1"],
    ["binary", "latin1"],
    ["base64", "base64"],
    ["base64url", "base64url"],
    ["hex", "hex"],
    ["ascii", "ascii"],
  ]);

  function normalizeEncoding(enc) {
    if (enc == null || enc === "" || enc === "utf8" || enc === "utf-8") return "utf8";
    if (typeof enc !== "string") return undefined;
    return ENCODING_NAMES.get(enc) ?? ENCODING_NAMES.get(enc.toLowerCase());
  }

  const ICU_ERROR_NAMES = new Map([
    [1, "U_ILLEGAL_ARGUMENT_ERROR"],
    [10, "U_INVALID_CHAR_FOUND"],
  ]);

  function transcode(source, fromEncoding, toEncoding) {
    if (!(source instanceof Uint8Array)) {
      throw makeError(
        TypeError,
        "ERR_INVALID_ARG_TYPE",
        `The "source" argument must be an instance of Buffer or Uint8Array. Received ${describeReceived(source)}`
      );
    }
    if (source.length === 0) return Buffer.alloc(0);

    fromEncoding = normalizeEncoding(fromEncoding) || fromEncoding;
    toEncoding = normalizeEncoding(toEncoding) || toEncoding;
    const result = transcodeNative(source, fromEncoding, toEncoding);
    if (typeof result !== "number") return result;

    const code = ICU_ERROR_NAMES.get(result);
    const error = new Error(`Unable to transcode Buffer [${code}]`);
    error.code = code;
    error.errno = result;
    throw error;
  }

  function asBytes(input) {
    if (ArrayBuffer.isView(input)) {
      if (input instanceof DataView) return undefined;
      try {
        return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      } catch {
        return new Uint8Array(0);
      }
    }
    if (input instanceof ArrayBuffer || (typeof SharedArrayBuffer === "function" && input instanceof SharedArrayBuffer)) {
      try {
        return new Uint8Array(input);
      } catch {
        return new Uint8Array(0);
      }
    }
    return undefined;
  }

  function isUtf8(input) {
    const bytes = asBytes(input);
    if (bytes === undefined) throw describeExpectedViews("input", input);
    return isUtf8Native(bytes);
  }

  function isAscii(input) {
    const bytes = asBytes(input);
    if (bytes === undefined) throw describeExpectedViews("input", input);
    return isAsciiNative(bytes);
  }

  let slowBufferWarned = false;
  function slowBuffer(size) {
    if (!slowBufferWarned) {
      slowBufferWarned = true;
      if (typeof process === "object" && typeof process.emitWarning === "function") {
        process.emitWarning(
          "SlowBuffer() is deprecated. Please use Buffer.allocUnsafeSlow()",
          "DeprecationWarning",
          "DEP0030"
        );
      }
    }
    validateNumber(size, "size", 0, maxLength);
    return Buffer.allocUnsafeSlow(size);
  }
  // Matches the wrapper Node's `deprecate()` returns.
  function deprecated(size) {
    return new.target ? Reflect.construct(slowBuffer, [size], new.target) : slowBuffer(size);
  }
  const SlowBuffer = deprecated;

  const storeKey = Symbol.for("raster.internal.blobURLStore");

  function resolveObjectURL(url) {
    url = `${url}`;
    try {
      const parsed = new URL(url);
      const split = parsed.pathname.split(":", 3);
      if (split.length !== 2) return;
      const { 0: base, 1: id } = split;
      if (base !== "nodedata") return;
      const store = globalThis[storeKey];
      const blob = store && store.get(id);
      if (blob === undefined) return;
      return blob.slice(0, blob.size, blob.type);
    } catch {
      return;
    }
  }

  let inspectMaxBytes = 50;

  target.SlowBuffer = SlowBuffer;
  target.transcode = transcode;
  target.isUtf8 = isUtf8;
  target.isAscii = isAscii;
  target.kMaxLength = maxLength;
  target.kStringMaxLength = maxStringLength;
  target.Blob = globalThis.Blob;
  target.File = globalThis.File;
  target.resolveObjectURL = resolveObjectURL;
  Object.defineProperty(target, "INSPECT_MAX_BYTES", {
    enumerable: true,
    configurable: true,
    get() {
      return inspectMaxBytes;
    },
    set(value) {
      validateNumber(value, "INSPECT_MAX_BYTES", 0);
      inspectMaxBytes = value;
    },
  });
})
