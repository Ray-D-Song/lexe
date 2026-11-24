// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
pub mod text_decoder;
pub mod text_encoder;

use raster_runtime_logging::format_plain;
use raster_runtime_logging::format_values;
use raster_runtime_utils::{
    class::CUSTOM_INSPECT_SYMBOL_DESCRIPTION,
    module::{export_default, ModuleInfo},
};
use rquickjs::{
    atom::PredefinedAtom,
    function::Func,
    module::{Declarations, Exports, ModuleDef},
    prelude::Rest,
    ArrayBuffer, Class, Ctx, Exception, Function, Object, Result, Symbol, Value,
};
use std::collections::HashMap;
use text_decoder::TextDecoder;
use text_encoder::TextEncoder;

fn inherits<'js>(ctor: Function<'js>, super_ctor: Function<'js>) -> Result<()> {
    let super_proto: Object<'js> = super_ctor.get("prototype")?;
    let proto: Object<'js> = ctor.get("prototype")?;
    proto.set_prototype(Some(&super_proto))?;
    ctor.set("super_", super_ctor)?;
    Ok(())
}

fn create_promisify<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    ctx.eval(
        r#"(function () {
  const kCustomPromisifiedSymbol = Symbol.for("nodejs.util.promisify.custom");

  function promisify(original) {
    if (typeof original !== "function") {
      throw new TypeError('The "original" argument must be of type function');
    }

    const custom = original[kCustomPromisifiedSymbol];
    if (custom !== undefined) {
      if (typeof custom !== "function") {
        throw new TypeError('The "util.promisify.custom" argument must be of type function');
      }
      return custom;
    }

    return function (...args) {
      return new Promise((resolve, reject) => {
        original.call(this, ...args, (error, value) => {
          if (error) reject(error);
          else resolve(value);
        });
      });
    };
  }
  promisify.custom = kCustomPromisifiedSymbol;
  return promisify;
})()"#,
    )
}

fn create_to_usv_string<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    ctx.eval(
        r#"(function () {
  return function toUSVString(value) {
    const string = String(value);
    let result = "";
    for (let i = 0; i < string.length; i++) {
      const codeUnit = string.charCodeAt(i);
      if (codeUnit >= 0xd800 && codeUnit <= 0xdbff) {
        if (i + 1 < string.length) {
          const next = string.charCodeAt(i + 1);
          if (next >= 0xdc00 && next <= 0xdfff) {
            result += string[i] + string[i + 1];
            i += 1;
            continue;
          }
        }
        result += "\uFFFD";
        continue;
      }
      if (codeUnit >= 0xdc00 && codeUnit <= 0xdfff) {
        result += "\uFFFD";
        continue;
      }
      result += string[i];
    }
    return result;
  };
})()"#,
    )
}

fn create_debuglog<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    ctx.eval(
        r#"(function () {
  const cache = new Map();

  function matchesPattern(set, pattern) {
    if (pattern === set) return true;
    if (pattern.endsWith("*")) {
      return set.startsWith(pattern.slice(0, -1));
    }
    return false;
  }

  function isEnabled(set) {
    const env = (globalThis.process && process.env && process.env.NODE_DEBUG) || "";
    for (const part of env.split(/[,\s]+/)) {
      if (!part) continue;
      if (matchesPattern(set, part.toUpperCase())) {
        return true;
      }
    }
    return false;
  }

  return function debuglog(set) {
    set = String(set).toUpperCase();
    if (cache.has(set)) {
      return cache.get(set);
    }

    const fn = function (...args) {
      if (!fn.enabled) return;
      const prefix = set + " " + (globalThis.process && process.pid != null ? process.pid : "") + " ";
      console.error(prefix + require("util").format(...args));
    };
    Object.defineProperty(fn, "enabled", {
      configurable: true,
      enumerable: true,
      get() {
        return isEnabled(set);
      },
    });
    cache.set(set, fn);
    return fn;
  };
})()"#,
    )
}

/// Shared `util.types` / `util/types` object, cached once per isolate via a
/// non-enumerable `Symbol.for` key so all entry points return the same object.
fn get_or_create_types_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    install_type_predicates(ctx)?;
    ctx.eval(
        r#"(function () {
  const KEY = Symbol.for("nodejs.util.types");
  if (Object.prototype.hasOwnProperty.call(globalThis, KEY) && globalThis[KEY]) {
    return globalThis[KEY];
  }

  const objectToString = Object.prototype.toString;

  // `[object X]` → `"X"`, the internal-slot tag the engine reports. Unlike a
  // user-assigned `Symbol.toStringTag`, this is not forgeable in QuickJS for the
  // built-ins below, so it is what the predicates key on.
  const tagOf = (value) => {
    if (value === null || value === undefined) return "";
    const tag = objectToString.call(value);
    return tag.slice(8, tag.length - 1);
  };

  // A real Error has its own `stack`/`message` slots; `Object.create(Error.prototype)`
  // does not, and Node reports `false` for it.
  const hasOwnErrorSlot = (value) => {
    try {
      return typeof value.stack === "string" && typeof value.name === "string";
    } catch {
      return false;
    }
  };

  const types = {
    isProxy(value) {
      return typeof value === "object" && value !== null && globalThis.__rasterIsProxy
        ? globalThis.__rasterIsProxy(value)
        : false;
    },
    isPromise(value) {
      return typeof value === "object" && value !== null && globalThis.__rasterIsPromise
        ? globalThis.__rasterIsPromise(value)
        : false;
    },
    isArrayBuffer(value) {
      // A SharedArrayBuffer is a distinct type in Node, so exclude it here.
      return typeof value === "object" && value !== null
        && !!globalThis.__rasterIsArrayBuffer
        && globalThis.__rasterIsArrayBuffer(value)
        && !globalThis.__rasterIsSharedArrayBuffer(value);
    },
    isSharedArrayBuffer(value) {
      return typeof value === "object" && value !== null
        && !!globalThis.__rasterIsSharedArrayBuffer
        && globalThis.__rasterIsSharedArrayBuffer(value);
    },
    isAnyArrayBuffer(value) {
      // Matches Node: either kind of ArrayBuffer.
      return types.isArrayBuffer(value) || types.isSharedArrayBuffer(value);
    },

    // --- ArrayBufferView / TypedArray family ------------------------------
    isArrayBufferView(value) {
      return ArrayBuffer.isView(value);
    },
    isTypedArray(value) {
      return types.isArrayBufferView(value) && !types.isDataView(value);
    },
    isDataView(value) {
      return tagOf(value) === "DataView";
    },
    isUint8Array(value) {
      return tagOf(value) === "Uint8Array";
    },
    isUint8ClampedArray(value) {
      return tagOf(value) === "Uint8ClampedArray";
    },
    isUint16Array(value) {
      return tagOf(value) === "Uint16Array";
    },
    isUint32Array(value) {
      return tagOf(value) === "Uint32Array";
    },
    isInt8Array(value) {
      return tagOf(value) === "Int8Array";
    },
    isInt16Array(value) {
      return tagOf(value) === "Int16Array";
    },
    isInt32Array(value) {
      return tagOf(value) === "Int32Array";
    },
    isFloat16Array(value) {
      return tagOf(value) === "Float16Array";
    },
    isFloat32Array(value) {
      return tagOf(value) === "Float32Array";
    },
    isFloat64Array(value) {
      return tagOf(value) === "Float64Array";
    },
    isBigInt64Array(value) {
      return tagOf(value) === "BigInt64Array";
    },
    isBigUint64Array(value) {
      return tagOf(value) === "BigUint64Array";
    },

    // --- collections and iterators ---------------------------------------
    isMap(value) {
      return tagOf(value) === "Map";
    },
    isSet(value) {
      return tagOf(value) === "Set";
    },
    isWeakMap(value) {
      return tagOf(value) === "WeakMap";
    },
    isWeakSet(value) {
      return tagOf(value) === "WeakSet";
    },
    isMapIterator(value) {
      // Only an actual map iterator has this tag; a plain object cannot fake
      // it because the tag is not a property.
      return tagOf(value) === "Map Iterator";
    },
    isSetIterator(value) {
      return tagOf(value) === "Set Iterator";
    },

    // --- other built-ins --------------------------------------------------
    isDate(value) {
      // Brand check: works across realms and rejects a Symbol.toStringTag
      // forgery, which `Object.prototype.toString` alone would accept.
      if (typeof value !== "object" || value === null) {
        return false;
      }
      try {
        Date.prototype.getTime.call(value);
        return true;
      } catch {
        return false;
      }
    },
    isRegExp(value) {
      if (typeof value !== "object" || value === null) {
        return false;
      }
      try {
        Object.getOwnPropertyDescriptor(RegExp.prototype, "source").get.call(value);
        return true;
      } catch {
        return false;
      }
    },
    isNativeError(value) {
      if (typeof value !== "object" || value === null) {
        return false;
      }
      // `Object.create(Error.prototype)` is not a native error; only objects
      // created by the Error constructors carry the internal slots.
      return Error.prototype.isPrototypeOf(value)
        && Object.prototype.toString.call(value) === "[object Error]"
        && typeof value.message === "string"
        && hasOwnErrorSlot(value);
    },
    isArgumentsObject(value) {
      return tagOf(value) === "Arguments";
    },
    isGeneratorFunction(value) {
      return tagOf(value) === "GeneratorFunction";
    },
    isGeneratorObject(value) {
      return tagOf(value) === "Generator";
    },
    isAsyncFunction(value) {
      return tagOf(value) === "AsyncFunction";
    },
    isModuleNamespaceObject(value) {
      // Module namespace objects are the only ones whose toStringTag is
      // "Module" and are not constructible.
      return tagOf(value) === "Module";
    },

    // --- boxed primitives -------------------------------------------------
    isStringObject(value) {
      // A primitive also tags as `[object String]`; only an object wrapper is
      // an object here.
      return typeof value === "object" && value !== null && tagOf(value) === "String";
    },
    isNumberObject(value) {
      // A primitive also tags as `[object Number]`; only an object wrapper is
      // an object here.
      return typeof value === "object" && value !== null && tagOf(value) === "Number";
    },
    isBooleanObject(value) {
      // A primitive also tags as `[object Boolean]`; only an object wrapper is
      // an object here.
      return typeof value === "object" && value !== null && tagOf(value) === "Boolean";
    },
    isBigIntObject(value) {
      // A primitive also tags as `[object BigInt]`; only an object wrapper is
      // an object here.
      return typeof value === "object" && value !== null && tagOf(value) === "BigInt";
    },
    isSymbolObject(value) {
      // A primitive also tags as `[object Symbol]`; only an object wrapper is
      // an object here.
      return typeof value === "object" && value !== null && tagOf(value) === "Symbol";
    },
    isBoxedPrimitive(value) {
      return types.isStringObject(value) || types.isNumberObject(value)
        || types.isBooleanObject(value) || types.isBigIntObject(value)
        || types.isSymbolObject(value);
    },

    // --- platform objects that cannot exist here --------------------------
    // Node registers these only when the corresponding subsystem is present;
    // reporting `false` is the correct answer for every value.
    isExternal(value) {
      return typeof value === "object" && value !== null
        && !!globalThis.__rasterIsExternal
        ? globalThis.__rasterIsExternal(value)
        : false;
    },
    isKeyObject(value) {
      return typeof value === "object" && value !== null
        && !!globalThis.__rasterIsKeyObject
        ? globalThis.__rasterIsKeyObject(value)
        : false;
    },
    isCryptoKey(value) {
      return typeof value === "object" && value !== null
        && !!globalThis.__rasterIsCryptoKey
        ? globalThis.__rasterIsCryptoKey(value)
        : false;
    },
  };

  Object.defineProperty(globalThis, KEY, {
    value: types,
    enumerable: false,
    configurable: true,
    writable: true,
  });
  return types;
})()"#,
    )
}

/// `MIMEType`/`MIMEParams` and the utility functions `util` was missing.
///
/// Behaviours follow Node 24, obtained by probing it rather than from the docs:
/// `getSystemErrorName`/`getSystemErrorMessage` reject non-negative codes with
/// `ERR_OUT_OF_RANGE`; `MIMEType` lowercases type/subtype but keeps parameter
/// case, and `toString()` drops the space after `;`.
const UTIL_EXTRAS_JS: &str = r#"(function () {
  "use strict";

  // --- MIME ---------------------------------------------------------------
  const TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

  class MIMEParams {
    #entries = new Map();
    constructor() {}
    get(name) {
      const key = String(name).toLowerCase();
      const entry = this.#entries.get(key);
      return entry === undefined ? null : entry;
    }
    set(name, value) {
      if (arguments.length < 2) {
        throw new TypeError('The "value" argument must be specified');
      }
      const key = String(name).toLowerCase();
      if (!TOKEN.test(key)) {
        throw new TypeError("Invalid MIME parameter name: " + key);
      }
      const text = String(value);
      if (text.length > 0 && !TOKEN.test(text) && !/^"[^"]*"$/.test(text)) {
        // A value with separators must be quoted, as in Node.
        if (!/^[\x20-\x7e]*$/.test(text) || /[;,"\\]/.test(text)) {
          throw new TypeError("Invalid MIME parameter value: " + text);
        }
      }
      this.#entries.set(key, text);
      return this;
    }
    delete(name) {
      this.#entries.delete(String(name).toLowerCase());
    }
    has(name) {
      return this.#entries.has(String(name).toLowerCase());
    }
    *entries() {
      // Insertion order, as in Node.
      for (const entry of this.#entries.entries()) yield entry;
    }
    *keys() {
      for (const [key] of this.entries()) yield key;
    }
    *values() {
      for (const [, value] of this.entries()) yield value;
    }
    forEach(fn, thisArg) {
      for (const [key, value] of this.entries()) fn.call(thisArg, value, key, this);
    }
    toString() {
      const parts = [];
      for (const [key, value] of this.entries()) {
        parts.push(";" + key + "=" + value);
      }
      return parts.join("");
    }
    [Symbol.iterator]() {
      return this.entries();
    }
  }

  class MIMEType {
    #type = "";
    #subtype = "";
    #params = new MIMEParams();
    constructor(input) {
      const text = String(input);
      const semi = text.indexOf(";");
      const essence = semi === -1 ? text : text.slice(0, semi);
      const slash = essence.indexOf("/");
      if (slash === -1) {
        throw new TypeError("Invalid MIME type: " + text);
      }
      const type = essence.slice(0, slash).trim().toLowerCase();
      const subtype = essence.slice(slash + 1).trim().toLowerCase();
      if (!TOKEN.test(type) || !TOKEN.test(subtype)) {
        throw new TypeError("Invalid MIME type: " + text);
      }
      this.#type = type;
      this.#subtype = subtype;
      if (semi !== -1) {
        for (const raw of text.slice(semi + 1).split(";")) {
          const part = raw.trim();
          if (!part) continue;
          const eq = part.indexOf("=");
          if (eq === -1) continue;
          this.#params.set(part.slice(0, eq), part.slice(eq + 1));
        }
      }
    }
    get type() { return this.#type; }
    set type(value) {
      const text = String(value).toLowerCase();
      if (!TOKEN.test(text)) throw new TypeError("Invalid MIME type: " + text);
      this.#type = text;
    }
    get subtype() { return this.#subtype; }
    set subtype(value) {
      const text = String(value).toLowerCase();
      if (!TOKEN.test(text)) throw new TypeError("Invalid MIME subtype: " + text);
      this.#subtype = text;
    }
    get essence() { return this.#type + "/" + this.#subtype; }
    get params() { return this.#params; }
    toString() { return this.essence + this.#params.toString(); }
    toJSON() { return this.toString(); }
  }

  // --- system errors ------------------------------------------------------
  // A representative slice of the C errno table. `code: -1` is EPERM on the
  // platforms Raster targets; the lookup tolerates absence.
  const SYSTEM_ERRORS = new Map([
    [-1, ["EPERM", "operation not permitted"]],
    [-2, ["ENOENT", "no such file or directory"]],
    [-3, ["ESRCH", "no such process"]],
    [-4, ["EINTR", "interrupted system call"]],
    [-5, ["EIO", "i/o error"]],
    [-6, ["ENXIO", "no such device or address"]],
    [-7, ["E2BIG", "argument list too long"]],
    [-9, ["EBADF", "bad file descriptor"]],
    [-11, ["EAGAIN", "resource temporarily unavailable"]],
    [-12, ["ENOMEM", "not enough memory"]],
    [-13, ["EACCES", "permission denied"]],
    [-14, ["EFAULT", "bad address"]],
    [-16, ["EBUSY", "resource busy or locked"]],
    [-17, ["EEXIST", "file already exists"]],
    [-18, ["EXDEV", "cross-device link not permitted"]],
    [-19, ["ENODEV", "no such device"]],
    [-20, ["ENOTDIR", "not a directory"]],
    [-21, ["EISDIR", "illegal operation on a directory"]],
    [-22, ["EINVAL", "invalid argument"]],
    [-23, ["ENFILE", "file table overflow"]],
    [-24, ["EMFILE", "too many open files"]],
    [-25, ["ENOTTY", "inappropriate ioctl for device"]],
    [-27, ["EFBIG", "file too large"]],
    [-28, ["ENOSPC", "no space left on device"]],
    [-29, ["ESPIPE", "invalid seek"]],
    [-30, ["EROFS", "read-only file system"]],
    [-31, ["EMLINK", "too many links"]],
    [-32, ["EPIPE", "broken pipe"]],
    [-34, ["ERANGE", "result out of range"]],
    [-36, ["ENAMETOOLONG", "name too long"]],
    [-39, ["ENOTEMPTY", "directory not empty"]],
    [-40, ["ELOOP", "too many symbolic links encountered"]],
    // Network family (Node reports these as negative errno too).
    [-97, ["EAFNOSUPPORT", "address family not supported by protocol"]],
    [-98, ["EADDRINUSE", "address already in use"]],
    [-99, ["EADDRNOTAVAIL", "address not available"]],
    [-100, ["ENETDOWN", "network is down"]],
    [-101, ["ENETUNREACH", "network is unreachable"]],
    [-103, ["ECONNABORTED", "software caused connection abort"]],
    [-104, ["ECONNRESET", "connection reset by peer"]],
    [-105, ["ENOBUFS", "no buffer space available"]],
    [-106, ["EISCONN", "socket is already connected"]],
    [-107, ["ENOTCONN", "socket is not connected"]],
    [-110, ["ETIMEDOUT", "connection timed out"]],
    [-111, ["ECONNREFUSED", "connection refused"]],
    [-113, ["EHOSTUNREACH", "host is unreachable"]],
  ]);

  const getSystemErrorMap = () => SYSTEM_ERRORS;

  function getSystemErrorName(err) {
    // Node rejects a non-negative code here too (`ERR_OUT_OF_RANGE`).
    if (typeof err !== "number" || !Number.isInteger(err) || err >= 0) {
      const error = new RangeError(
        'The value of "err" is out of range. It must be a negative integer. Received ' + err
      );
      error.code = "ERR_OUT_OF_RANGE";
      throw error;
    }
    const entry = SYSTEM_ERRORS.get(err);
    if (entry) return entry[0];
    return "Unknown system error " + err;
  }

  function getSystemErrorMessage(err) {
    if (typeof err !== "number" || !Number.isInteger(err) || err >= 0) {
      const error = new RangeError(
        'The value of "err" is out of range. It must be a negative integer. Received ' + err
      );
      error.code = "ERR_OUT_OF_RANGE";
      throw error;
    }
    const entry = SYSTEM_ERRORS.get(err);
    return entry ? entry[1] : "Unknown system error " + err;
  }

  // --- signals ------------------------------------------------------------
  const SIGNAL_NUMBERS = {
    SIGINT: 2, SIGILL: 4, SIGABRT: 6, SIGFPE: 8, SIGKILL: 9, SIGSEGV: 11,
    SIGPIPE: 13, SIGALRM: 14, SIGTERM: 15, SIGHUP: 1, SIGQUIT: 3, SIGTRAP: 5,
    SIGBUS: 7, SIGUSR1: 10, SIGUSR2: 12, SIGCHLD: 17, SIGCONT: 18, SIGSTOP: 19,
    SIGTSTP: 20, SIGTTIN: 21, SIGTTOU: 22, SIGURG: 23, SIGXCPU: 24,
    SIGXFSZ: 25, SIGVTALRM: 26, SIGPROF: 27, SIGWINCH: 28, SIGIO: 29,
    SIGPWR: 30, SIGSYS: 31,
  };

  function convertProcessSignalToExitCode(signal) {
    if (typeof signal !== "string" || !(signal in SIGNAL_NUMBERS)) {
      const error = new TypeError('Unknown signal: ' + signal);
      error.code = "ERR_UNKNOWN_SIGNAL";
      throw error;
    }
    return 128 + SIGNAL_NUMBERS[signal];
  }

  // --- diff ---------------------------------------------------------------
  // Port of Node's `internal/util/inspect.diff`: a Myers-free simple LCS walk
  // that emits `[operation, text]` pairs, which is the documented shape.
  function diff(actual, expected) {
    if (actual === expected) return [[0, actual]];
    const actualLines = String(actual).split(/(\n)/);
    const expectedLines = String(expected).split(/(\n)/);

    // Longest common subsequence table over lines.
    const rows = actualLines.length + 1;
    const cols = expectedLines.length + 1;
    const lcs = new Array(rows);
    for (let i = 0; i < rows; i++) lcs[i] = new Int32Array(cols);
    for (let i = actualLines.length - 1; i >= 0; i--) {
      for (let j = expectedLines.length - 1; j >= 0; j--) {
        lcs[i][j] = actualLines[i] === expectedLines[j]
          ? lcs[i + 1][j + 1] + 1
          : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
      }
    }

    const result = [];
    let i = 0;
    let j = 0;
    while (i < actualLines.length && j < expectedLines.length) {
      if (actualLines[i] === expectedLines[j]) {
        result.push([0, actualLines[i]]);
        i++;
        j++;
      } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
        result.push([1, actualLines[i]]);
        i++;
      } else {
        result.push([-1, expectedLines[j]]);
        j++;
      }
    }
    while (i < actualLines.length) result.push([1, actualLines[i++]]);
    while (j < expectedLines.length) result.push([-1, expectedLines[j++]]);
    return result;
  }

  // --- call sites ---------------------------------------------------------
  function getCallSites(frames) {
    const limit = frames === undefined ? 10 : frames;
    const previousLimit = Error.stackTraceLimit;
    Error.stackTraceLimit = limit + 1;
    // Capture a plain stack rather than using `captureStackTrace(holder, fn)`:
    // the filtered form trims everything up to `fn`, and since this helper is
    // the innermost frame it would leave nothing at all.
    const stack = new Error().stack || "";
    Error.stackTraceLimit = previousLimit;
    const sites = [];
    for (const line of stack.split("\n").slice(1)) {
      const match = /^\s*at\s+(?:(.*?)\s+\()?(.*?):(\d+):(\d+)\)?$/.exec(line);
      if (!match) continue;
      // Skip this helper's own frame.
      if (match[1] === "getCallSites") continue;
      const scriptName = match[2];
      const lineNumber = Number(match[3]);
      const column = Number(match[4]);
      sites.push({
        functionName: match[1] || "<anonymous>",
        scriptName,
        scriptId: "0",
        lineNumber,
        column,
        columnNumber: column,
      });
      if (sites.length >= limit) break;
    }
    return sites;
  }

  // --- aborted ------------------------------------------------------------
  function aborted(signal, resource) {
    if (typeof signal !== "object" || signal === null || typeof signal.aborted !== "boolean") {
      throw new TypeError('The "signal" argument must be an AbortSignal');
    }
    const target = resource && typeof resource === "object" ? resource : null;
    return new Promise((resolve, reject) => {
      if (signal.aborted) {
        const error = signal.reason !== undefined
          ? signal.reason
          : new Error("The operation was aborted");
        if (target && typeof target.destroy === "function") target.destroy(error);
        reject(error);
        return;
      }
      const onAbort = () => {
        const error = signal.reason !== undefined
          ? signal.reason
          : new Error("The operation was aborted");
        if (target && typeof target.destroy === "function") target.destroy(error);
        reject(error);
      };
      signal.addEventListener("abort", onAbort, { once: true });
      resolve(() => signal.removeEventListener("abort", onAbort));
    });
  }

  // --- callbackify --------------------------------------------------------
  function callbackify(original) {
    if (typeof original !== "function") {
      throw new TypeError('The "original" argument must be of type function');
    }
    return function (...args) {
      const callback = args.pop();
      if (typeof callback !== "function") {
        throw new TypeError('The last argument must be of type function');
      }
      Promise.resolve().then(() => original.apply(this, args)).then(
        (value) => queueMicrotask(() => callback(null, value)),
        (error) => queueMicrotask(() => callback(error))
      );
    };
  }

  // --- _extend ------------------------------------------------------------
  function _extend(target, source) {
    for (const key of Object.keys(source)) target[key] = source[key];
    return target;
  }

  return {
    MIMEType,
    MIMEParams,
    getSystemErrorMap,
    getSystemErrorName,
    getSystemErrorMessage,
    convertProcessSignalToExitCode,
    diff,
    getCallSites,
    aborted,
    callbackify,
    _extend,
  };
})()"#;

/// Add `inspect.colors`, `inspect.styles` and `inspect.defaultOptions`.
///
/// Node hangs these off the `inspect` function object, and code reads them
/// directly (`util.inspect.colors.green[0]`, `defaultOptions.depth`).
fn attach_inspect_metadata<'js>(ctx: &Ctx<'js>, default: &Object<'js>) -> Result<()> {
    let inspect: Function = default.get("inspect")?;
    let metadata: Object = ctx.eval(
        r#"(function () {
  // ANSI open/close pairs, matching Node's names and codes.
  const colors = {
    bold: [1, 22], italic: [3, 23], underline: [4, 24], inverse: [7, 27],
    white: [37, 39], grey: [90, 39], black: [30, 39], blue: [34, 39],
    cyan: [36, 39], green: [32, 39], magenta: [35, 39], red: [31, 39],
    yellow: [33, 39],
  };
  // Node's default type-to-colour mapping.
  const styles = {
    special: "cyan", number: "yellow", bigint: "yellow", boolean: "yellow",
    undefined: "grey", null: "bold", string: "green", symbol: "green",
    date: "magenta", regexp: "red",
  };
  const defaultOptions = {
    indentationLvl: 0,
    currentDepth: 0,
    stylize: (str) => str,
    showHidden: false,
    depth: 2,
    colors: false,
    customInspect: true,
    showProxy: false,
    maxArrayLength: 100,
    maxStringLength: 10000,
    breakLength: 128,
    compact: 3,
    sorted: false,
    getters: false,
    numericSeparator: false,
  };
  return { colors, styles, defaultOptions };
})()"#,
    )?;
    inspect.set("colors", metadata.get::<_, Object>("colors")?)?;
    inspect.set("styles", metadata.get::<_, Object>("styles")?)?;
    inspect.set(
        "defaultOptions",
        metadata.get::<_, Object>("defaultOptions")?,
    )?;
    Ok(())
}

/// `MIMEType` / `MIMEParams`, plus the `util` entries that were missing.
fn install_util_extras<'js>(ctx: &Ctx<'js>, default: &Object<'js>) -> Result<()> {
    let extras: Object = ctx.eval(UTIL_EXTRAS_JS)?;
    for name in [
        "MIMEType",
        "MIMEParams",
        "getSystemErrorMap",
        "getSystemErrorName",
        "getSystemErrorMessage",
        "convertProcessSignalToExitCode",
        "diff",
        "getCallSites",
        "aborted",
        "callbackify",
        "_extend",
    ] {
        let value: Value = extras.get(name)?;
        default.set(name, value)?;
    }
    Ok(())
}

fn create_deprecate<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    // Phase 1: identity wrapper with Node-compatible TypeError on bad args.
    // Runtime deprecation warnings are intentionally not emitted yet.
    ctx.eval(
        r#"(function () {
  return function deprecate(fn, message, code) {
    if (typeof fn !== "function") {
      throw new TypeError('The "fn" argument must be of type function. Received type ' + typeof fn);
    }
    // Keep this, arguments, return value, and function identity unchanged.
    return fn;
  };
})()"#,
    )
}

/// States for the dotenv / `util.parseEnv` state machine (Node 24.3).
///
/// Quoted variants document the machine; quote bodies are resolved via
/// lookahead in `BeforeValue` (closed → multi-line; unclosed → until NL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum ParseEnvState {
    LineStart,
    Key,
    BeforeValue,
    UnquotedValue,
    SingleQuotedValue,
    DoubleQuotedValue,
    BacktickValue,
    AfterQuotedValue,
    Comment,
}

fn is_env_ws(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\u{0c}'
}

/// Find the relative index of a closing quote in `rest`.
///
/// Node 24.3 does **not** treat `\"` as an escaped closer — the first matching
/// quote character ends the value (so `A="a\"b"` yields value `a\`).
fn find_closing_quote(rest: &[char], opener: char) -> Option<usize> {
    rest.iter().position(|&ch| ch == opener)
}

/// Node 24.3 `util.parseEnv` double-quote expansion: only `\n` is converted
/// to a newline. All other escape sequences (including `\r`, `\t`, `\"`, `\\`)
/// are left as literal two-character sequences in the value.
fn expand_double_quoted_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                },
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn normalize_env_key(raw_key: &str) -> Option<String> {
    let mut k = raw_key.trim().to_string();
    if let Some(rest) = k.strip_prefix("export ") {
        k = rest.trim().to_string();
    }
    if k.is_empty() {
        None
    } else {
        Some(k)
    }
}

fn store_pair(store: &mut HashMap<String, String>, raw_key: &str, value: String) {
    if let Some(k) = normalize_env_key(raw_key) {
        store.insert(k, value);
    }
}

/// Parse dotenv-style content into key/value pairs (later keys win).
///
/// Behavior matches Node 24.3 `util.parseEnv` / `Dotenv::ParseContent`:
/// UTF-8 BOM, LF/CRLF, optional `export` prefix, quoted multi-line values
/// (when a closing quote exists later), `#` comments outside quotes, invalid
/// lines ignored. Unclosed quotes take only until the first newline (including
/// the opening quote), matching Node fixtures.
pub fn parse_env_content(content: &str) -> HashMap<String, String> {
    let mut store: HashMap<String, String> = HashMap::new();

    // Strip UTF-8 BOM and normalize CRLF → LF (Node strips all `\r`).
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let normalized: String = content.chars().filter(|&c| c != '\r').collect();
    let bytes = normalized.as_str();
    let chars: Vec<char> = bytes.chars().collect();
    let len = chars.len();
    let mut i = 0usize;
    let mut state = ParseEnvState::LineStart;
    let mut key = String::new();
    let mut value = String::new();

    while i < len {
        let c = chars[i];
        match state {
            ParseEnvState::LineStart => {
                if c == '\n' || is_env_ws(c) {
                    i += 1;
                    continue;
                }
                if c == '#' {
                    state = ParseEnvState::Comment;
                    i += 1;
                    continue;
                }
                key.clear();
                value.clear();
                key.push(c);
                state = ParseEnvState::Key;
                i += 1;
            },
            ParseEnvState::Key => {
                if c == '\n' {
                    // Invalid line (no '=').
                    key.clear();
                    state = ParseEnvState::LineStart;
                    i += 1;
                } else if c == '=' {
                    state = ParseEnvState::BeforeValue;
                    i += 1;
                } else {
                    key.push(c);
                    i += 1;
                }
            },
            ParseEnvState::BeforeValue => {
                if c == '\n' {
                    store_pair(&mut store, &key, String::new());
                    key.clear();
                    value.clear();
                    state = ParseEnvState::LineStart;
                    i += 1;
                } else if is_env_ws(c) {
                    i += 1;
                } else if c == '\'' || c == '"' || c == '`' {
                    // Lookahead for closing quote (may span lines). Double quotes
                    // treat `\"` as escaped so the quote does not close.
                    // If no closer exists in the remaining content, take only until
                    // first newline including the opening quote (Node MULTI_NOT_VALID_QUOTE).
                    let opener = c;
                    let rest = &chars[i + 1..];
                    let close_rel = find_closing_quote(rest, opener);
                    if let Some(rel) = close_rel {
                        // Closed quote found (possibly multi-line).
                        let inner: String = rest[..rel].iter().collect();
                        let v = if opener == '"' {
                            expand_double_quoted_escapes(&inner)
                        } else {
                            inner
                        };
                        store_pair(&mut store, &key, v);
                        key.clear();
                        value.clear();
                        // Advance past opening quote + inner + closing quote.
                        i = i + 1 + rel + 1;
                        state = ParseEnvState::AfterQuotedValue;
                    } else {
                        // Unclosed: value is opening quote + content until newline/EOF.
                        let mut incomplete = String::new();
                        incomplete.push(opener);
                        i += 1; // skip opener
                        while i < len && chars[i] != '\n' {
                            incomplete.push(chars[i]);
                            i += 1;
                        }
                        store_pair(&mut store, &key, incomplete);
                        key.clear();
                        value.clear();
                        if i < len && chars[i] == '\n' {
                            i += 1;
                        }
                        state = ParseEnvState::LineStart;
                    }
                } else if c == '#' {
                    store_pair(&mut store, &key, String::new());
                    key.clear();
                    value.clear();
                    state = ParseEnvState::Comment;
                    i += 1;
                } else {
                    value.push(c);
                    state = ParseEnvState::UnquotedValue;
                    i += 1;
                }
            },
            ParseEnvState::UnquotedValue => {
                if c == '\n' {
                    let v = value.trim_end_matches(|ch: char| is_env_ws(ch)).to_string();
                    store_pair(&mut store, &key, v);
                    key.clear();
                    value.clear();
                    state = ParseEnvState::LineStart;
                    i += 1;
                } else if c == '#' {
                    let v = value.trim_end_matches(|ch: char| is_env_ws(ch)).to_string();
                    store_pair(&mut store, &key, v);
                    key.clear();
                    value.clear();
                    state = ParseEnvState::Comment;
                    i += 1;
                } else {
                    value.push(c);
                    i += 1;
                }
            },
            // Quoted states are entered only via BeforeValue lookahead; kept for
            // documentation of the machine. AfterQuotedValue handles the tail.
            ParseEnvState::SingleQuotedValue
            | ParseEnvState::DoubleQuotedValue
            | ParseEnvState::BacktickValue => {
                // Unreachable in normal flow — BeforeValue handles quotes.
                i += 1;
            },
            ParseEnvState::AfterQuotedValue => {
                // After closing quote: whitespace and `#` comments (and other junk) ignored until NL.
                if c == '\n' {
                    state = ParseEnvState::LineStart;
                    i += 1;
                } else if c == '#' {
                    state = ParseEnvState::Comment;
                    i += 1;
                } else {
                    i += 1;
                }
            },
            ParseEnvState::Comment => {
                if c == '\n' {
                    state = ParseEnvState::LineStart;
                }
                i += 1;
            },
        }
    }

    // EOF flush for unfinished unquoted / empty values.
    match state {
        ParseEnvState::BeforeValue => {
            store_pair(&mut store, &key, String::new());
        },
        ParseEnvState::UnquotedValue => {
            let v = value.trim_end_matches(|ch: char| is_env_ws(ch)).to_string();
            store_pair(&mut store, &key, v);
        },
        _ => {},
    }

    store
}

fn parse_env_js<'js>(ctx: Ctx<'js>, content: Value<'js>) -> Result<Object<'js>> {
    let content = content.as_string().ok_or_else(|| {
        let ty = if content.is_null() {
            "null".to_string()
        } else if content.is_undefined() {
            "undefined".to_string()
        } else if content.is_bool() {
            "boolean".to_string()
        } else if content.as_number().is_some() {
            "number".to_string()
        } else if content.is_object() {
            "object".to_string()
        } else {
            "unknown".to_string()
        };
        Exception::throw_type(
            &ctx,
            &format!("The \"content\" argument must be of type string. Received type {ty}"),
        )
    })?;
    let content = content.to_string()?;
    let pairs = parse_env_content(&content);

    // Null-prototype object (Object.create(null)).
    let obj = Object::new(ctx.clone())?;
    obj.set_prototype(None)?;
    for (k, v) in pairs {
        obj.set(k, v)?;
    }
    Ok(obj)
}

fn create_strip_vt_control_characters<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    // ANSI/VT stripper aligned with Node's inspect.stripVTControlCharacters.
    ctx.eval(
        r#"(function () {
  // CSI / OSC / ESC sequences (practical subset used by Node).
  const VT = /[\u001b\u009b][[\]()#;?]*(?:(?:(?:(?:;[-a-zA-Z\d\/#&.:=?%@~_]+)*|[a-zA-Z\d]+(?:;[-a-zA-Z\d\/#&.:=?%@~_]*)*)?\u0007)|(?:(?:\d{1,4}(?:;\d{0,4})*)?[\dA-PR-TZcf-nq-uy=><~]))/g;
  return function stripVTControlCharacters(str) {
    if (typeof str !== "string") {
      throw new TypeError('The "str" argument must be of type string. Received type ' + typeof str);
    }
    if (str.indexOf("\u001B") === -1 && str.indexOf("\u009B") === -1) return str;
    return str.replace(VT, "");
  };
})()"#,
    )
}

fn create_style_text<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    // Node 24.3 util.styleText — named styles only, strict text/options/stream checks.
    // Color enable reuses stream.getColorDepth / hasColors / isTTY (no duplicate env logic).
    ctx.eval(
        r##"(function () {
  const styles = {
    reset: [0, 0],
    bold: [1, 22],
    dim: [2, 22],
    italic: [3, 23],
    underline: [4, 24],
    blink: [5, 25],
    inverse: [7, 27],
    hidden: [8, 28],
    strikethrough: [9, 29],
    doubleunderline: [21, 24],
    black: [30, 39],
    red: [31, 39],
    green: [32, 39],
    yellow: [33, 39],
    blue: [34, 39],
    magenta: [35, 39],
    cyan: [36, 39],
    white: [37, 39],
    bgBlack: [40, 49],
    bgRed: [41, 49],
    bgGreen: [42, 49],
    bgYellow: [43, 49],
    bgBlue: [44, 49],
    bgMagenta: [45, 49],
    bgCyan: [46, 49],
    bgWhite: [47, 49],
    framed: [51, 54],
    overlined: [53, 55],
    gray: [90, 39],
    grey: [90, 39],
    redBright: [91, 39],
    greenBright: [92, 39],
    yellowBright: [93, 39],
    blueBright: [94, 39],
    magentaBright: [95, 39],
    cyanBright: [96, 39],
    whiteBright: [97, 39],
    bgGray: [100, 49],
    bgGrey: [100, 49],
    bgRedBright: [101, 49],
    bgGreenBright: [102, 49],
    bgYellowBright: [103, 49],
    bgBlueBright: [104, 49],
    bgMagentaBright: [105, 49],
    bgCyanBright: [106, 49],
    bgWhiteBright: [107, 49],
  };

  function openClose(codes) {
    return {
      open: "\u001b[" + codes[0] + "m",
      close: "\u001b[" + codes[1] + "m",
    };
  }

  function replaceClose(str, close, open) {
    let i = str.indexOf(close);
    if (i === -1) return str;
    let result = "";
    let cursor = 0;
    while (i !== -1) {
      result += str.substring(cursor, i) + close + open;
      cursor = i + close.length;
      i = str.indexOf(close, cursor);
    }
    return result + str.substring(cursor);
  }

  // Accept real Node/Raster streams; reject plain {isTTY:true} objects.
  function isStreamLike(obj) {
    if (!obj || typeof obj !== "object") return false;
    // Classic Node streams / Raster tty WriteStream (write+on) / Readable (pipe+on)
    if (typeof obj.write === "function" && typeof obj.on === "function") return true;
    if (typeof obj.pipe === "function" && typeof obj.on === "function") return true;
    // Web streams
    if (typeof obj.getReader === "function" && typeof obj.cancel === "function") return true;
    if (typeof obj.getWriter === "function" && typeof obj.abort === "function") return true;
    return false;
  }

  // Reuse stream.getColorDepth / hasColors / isTTY — mirrors internal/util/colors.shouldColorize.
  function shouldColorize(stream) {
    const env = (globalThis.process && process.env) || {};
    if (env.FORCE_COLOR !== undefined) {
      if (typeof stream?.getColorDepth === "function") {
        return stream.getColorDepth() > 2;
      }
      if (typeof stream?.hasColors === "function") {
        return stream.hasColors();
      }
      // FORCE_COLOR set: colorize unless TERM=dumb and no depth helper.
      return env.TERM !== "dumb";
    }
    if (stream && typeof stream.hasColors === "function") {
      return stream.hasColors();
    }
    return !!(stream && stream.isTTY && (
      typeof stream.getColorDepth === "function" ? stream.getColorDepth() > 2 : true
    ));
  }

  function styleText(format, text, options) {
    // text MUST be a string — never coerce with String(text).
    if (typeof text !== "string") {
      throw new TypeError(
        'The "text" argument must be of type string. Received type ' + typeof text
      );
    }

    // Node 24.3: null options is TypeError (not coerced to {}).
    if (options === null) {
      throw new TypeError(
        'The "options" argument must be of type object. Received null'
      );
    }
    if (options === undefined) {
      options = {};
    } else if (typeof options !== "object" || Array.isArray(options)) {
      throw new TypeError(
        'The "options" argument must be of type object. Received type ' + typeof options
      );
    }

    const validateStream = options.validateStream !== undefined ? options.validateStream : true;
    if (typeof validateStream !== "boolean") {
      throw new TypeError(
        'The "options.validateStream" property must be of type boolean. Received type ' +
          typeof validateStream
      );
    }

    let skipColorize = false;
    if (validateStream) {
      const stream =
        options.stream !== undefined && options.stream !== null
          ? options.stream
          : globalThis.process && process.stdout;
      if (!isStreamLike(stream)) {
        throw new TypeError(
          'The "options.stream" property must be an instance of Readable, Writable, or Stream'
        );
      }
      skipColorize = !shouldColorize(stream);
    }

    const formats = Array.isArray(format) ? format : [format];
    let openCodes = "";
    let closeCodes = "";
    let processed = text;

    // Only named styles from the table (no #hex / "none" extensions).
    for (let i = 0; i < formats.length; i++) {
      const key = formats[i];
      if (typeof key !== "string") {
        throw new TypeError(
          'The "format" argument must be of type string or an array of strings'
        );
      }
      if (key.charAt(0) === "#") {
        throw new TypeError(
          'The "format" argument must be a valid named style (hex colors are not supported). Received \'' +
            key +
            "'"
        );
      }
      const codes = styles[key];
      if (!codes) {
        throw new TypeError(
          'The "format" argument must be one of: ' +
            Object.keys(styles).join(", ") +
            ". Received '" +
            key +
            "'"
        );
      }
      if (skipColorize) continue;
      const sc = openClose(codes);
      openCodes += sc.open;
      closeCodes = sc.close + closeCodes;
      processed = replaceClose(processed, sc.close, sc.open);
    }

    if (skipColorize) return text;
    return openCodes + processed + closeCodes;
  }

  return styleText;
})()"##,
    )
}

fn install_type_predicates(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    if globals.contains_key("__rasterIsProxy")? {
        return Ok(());
    }

    globals.set(
        "__rasterIsProxy",
        Func::from(|value: Value| value.is_proxy()),
    )?;
    globals.set(
        "__rasterIsPromise",
        Func::from(|value: Value| value.is_promise()),
    )?;
    globals.set(
        "__rasterIsArrayBuffer",
        Func::from(|value: Value| ArrayBuffer::from_value(value).is_some()),
    )?;
    // `ArrayBuffer::from_value` succeeds for a SharedArrayBuffer too, so the
    // two have to be told apart by constructor name — the same way structured
    // clone does it.
    globals.set(
        "__rasterIsSharedArrayBuffer",
        Func::from(|ctx: Ctx<'_>, value: Value<'_>| -> Result<bool> {
            let Some(obj) = value.as_object() else {
                return Ok(false);
            };
            if ArrayBuffer::from_value(value.clone()).is_none() {
                return Ok(false);
            }
            let Some(ctor) = obj.get::<_, Option<Object>>(PredefinedAtom::Constructor)? else {
                return Ok(false);
            };
            let name: String = ctor.get(PredefinedAtom::Name)?;
            let _ = ctx;
            Ok(name == "SharedArrayBuffer")
        }),
    )?;
    Ok(())
}

fn inspect_value<'js>(ctx: Ctx<'js>, value: Value<'js>) -> Result<String> {
    // Compact form: short objects print on one line (`{ a: 1, b: 2 }`), which is
    // what `util.inspect` does. `write_sep` handles the separators, so this no
    // longer emits a carriage return either.
    format_plain(ctx, false, rquickjs::prelude::Rest(vec![value]))
}

fn format_with_options<'js>(
    ctx: Ctx<'js>,
    options: Object<'js>,
    args: rquickjs::prelude::Rest<Value<'js>>,
) -> Result<String> {
    let colors = options
        .get::<_, Option<bool>>("colors")
        .ok()
        .flatten()
        .unwrap_or(false);
    format_values(&ctx, args, colors, false)
}

fn format_export<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    let mut formatted = format_plain(ctx, true, args)?;
    if formatted.ends_with('\n') {
        formatted.pop();
    }
    if formatted.ends_with('\r') {
        formatted.pop();
    }
    Ok(formatted)
}

/// Install `util.inspect.custom` on the five built-ins that define one in Node.
const INSPECT_CUSTOM_JS: &str = include_str!("inspect_custom.js");

fn install_custom_inspect<'js>(
    ctx: &Ctx<'js>,
    symbol: Symbol<'js>,
    inspect_fn: Value<'js>,
) -> Result<()> {
    let factory: Function<'js> = ctx.eval(INSPECT_CUSTOM_JS)?;
    let installer: Function<'js> = factory.call((symbol,))?;
    installer.call::<_, ()>((inspect_fn,))?;
    Ok(())
}

pub fn define_text_encoding_constructors(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    if globals.contains_key("TextEncoder")? && globals.contains_key("TextDecoder")? {
        return Ok(());
    }

    Class::<TextEncoder>::define(&globals)?;
    Class::<TextDecoder>::define(&globals)?;
    Ok(())
}

pub struct UtilModule;

impl ModuleDef for UtilModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare(stringify!(TextDecoder))?;
        declare.declare(stringify!(TextEncoder))?;
        declare.declare("format")?;
        declare.declare("inherits")?;
        declare.declare("promisify")?;
        declare.declare("inspect")?;
        declare.declare("formatWithOptions")?;
        declare.declare("MIMEType")?;
        declare.declare("MIMEParams")?;
        declare.declare("getSystemErrorMap")?;
        declare.declare("getSystemErrorName")?;
        declare.declare("getSystemErrorMessage")?;
        declare.declare("convertProcessSignalToExitCode")?;
        declare.declare("diff")?;
        declare.declare("getCallSites")?;
        declare.declare("aborted")?;
        declare.declare("callbackify")?;
        declare.declare("_extend")?;
        declare.declare("debuglog")?;
        declare.declare("debug")?;
        declare.declare("toUSVString")?;
        declare.declare("types")?;
        declare.declare("deprecate")?;
        declare.declare("styleText")?;
        declare.declare("stripVTControlCharacters")?;
        declare.declare("parseEnv")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            install_type_predicates(ctx)?;
            let globals = ctx.globals();

            let encoder: Function = globals.get(stringify!(TextEncoder))?;
            let decoder: Function = globals.get(stringify!(TextDecoder))?;
            let promisify = create_promisify(ctx)?;
            let to_usv_string = create_to_usv_string(ctx)?;
            let debuglog = create_debuglog(ctx)?;
            let types = get_or_create_types_object(ctx)?;
            let deprecate = create_deprecate(ctx)?;
            let style_text = create_style_text(ctx)?;
            let strip_vt = create_strip_vt_control_characters(ctx)?;

            default.set(stringify!(TextEncoder), encoder)?;
            default.set(stringify!(TextDecoder), decoder)?;
            default.set("format", Func::from(format_export))?;
            default.set("inherits", Func::from(inherits))?;
            default.set("promisify", promisify)?;
            default.set("inspect", Func::from(inspect_value))?;
            // Node attaches these to the `inspect` function itself; callers read
            // `util.inspect.colors.red[0]` and `inspect.defaultOptions.depth`.
            attach_inspect_metadata(ctx, default)?;
            default.set("formatWithOptions", Func::from(format_with_options))?;
            install_util_extras(ctx, default)?;
            default.set("debuglog", debuglog.clone())?;
            default.set("debug", debuglog)?;
            default.set("toUSVString", to_usv_string)?;
            default.set("types", types)?;
            default.set("deprecate", deprecate)?;
            default.set("styleText", style_text)?;
            default.set("stripVTControlCharacters", strip_vt)?;
            default.set("parseEnv", Func::from(parse_env_js))?;
            let inspect_symbol =
                Symbol::new_global(ctx.clone(), CUSTOM_INSPECT_SYMBOL_DESCRIPTION)?;
            let inspect_value: Value = default.get("inspect")?;
            inspect_value
                .as_object()
                .expect("inspect export")
                .set("custom", inspect_symbol.clone())?;

            // Give the five built-ins that have `util.inspect.custom` in Node a
            // function-form implementation. See `inspect_custom.js`.
            install_custom_inspect(ctx, inspect_symbol, inspect_value)?;

            Ok(())
        })
    }
}

impl From<UtilModule> for ModuleInfo<UtilModule> {
    fn from(val: UtilModule) -> Self {
        ModuleInfo {
            name: "util",
            module: val,
        }
    }
}

/// `require("util/types")` / `require("node:util/types")` — same object as `util.types`.
pub struct UtilTypesModule;

impl ModuleDef for UtilTypesModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("isProxy")?;
        declare.declare("isPromise")?;
        declare.declare("isArrayBuffer")?;
        declare.declare("isAnyArrayBuffer")?;
        declare.declare("isSharedArrayBuffer")?;
        declare.declare("isTypedArray")?;
        declare.declare("isDataView")?;
        declare.declare("isArrayBufferView")?;
        declare.declare("isUint8Array")?;
        declare.declare("isUint8ClampedArray")?;
        declare.declare("isUint16Array")?;
        declare.declare("isUint32Array")?;
        declare.declare("isInt8Array")?;
        declare.declare("isInt16Array")?;
        declare.declare("isInt32Array")?;
        declare.declare("isFloat16Array")?;
        declare.declare("isFloat32Array")?;
        declare.declare("isFloat64Array")?;
        declare.declare("isBigInt64Array")?;
        declare.declare("isBigUint64Array")?;
        declare.declare("isDate")?;
        declare.declare("isRegExp")?;
        declare.declare("isMap")?;
        declare.declare("isSet")?;
        declare.declare("isWeakMap")?;
        declare.declare("isWeakSet")?;
        declare.declare("isMapIterator")?;
        declare.declare("isSetIterator")?;
        declare.declare("isArgumentsObject")?;
        declare.declare("isNativeError")?;
        declare.declare("isStringObject")?;
        declare.declare("isNumberObject")?;
        declare.declare("isBooleanObject")?;
        declare.declare("isBigIntObject")?;
        declare.declare("isSymbolObject")?;
        declare.declare("isBoxedPrimitive")?;
        declare.declare("isAsyncFunction")?;
        declare.declare("isGeneratorFunction")?;
        declare.declare("isGeneratorObject")?;
        declare.declare("isExternal")?;
        declare.declare("isKeyObject")?;
        declare.declare("isCryptoKey")?;
        declare.declare("isModuleNamespaceObject")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        // Export the shared types object as `default` so CJS `require("util/types")`
        // returns the same object as `require("util").types` (builtin interop merges
        // named exports onto the default object when it is an object).
        let types = get_or_create_types_object(ctx)?;
        for key in types.keys::<String>() {
            let key = key?;
            let value: Value = types.get(&key)?;
            exports.export(key.as_str(), value)?;
        }
        exports.export("default", types)?;
        Ok(())
    }
}

impl From<UtilTypesModule> for ModuleInfo<UtilTypesModule> {
    fn from(val: UtilTypesModule) -> Self {
        ModuleInfo {
            name: "util/types",
            module: val,
        }
    }
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    define_text_encoding_constructors(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_basic_and_comments() {
        let m = parse_env_content(
            r#"
A="x#y" # tail
B="one
two"
C=plain # comment
D='a#b'
export E=value
DUP=first
DUP=second
# full line comment
INVALID_LINE
=emptykey
"#,
        );
        assert_eq!(m.get("A").map(String::as_str), Some("x#y"));
        assert_eq!(m.get("B").map(String::as_str), Some("one\ntwo"));
        assert_eq!(m.get("C").map(String::as_str), Some("plain"));
        assert_eq!(m.get("D").map(String::as_str), Some("a#b"));
        assert_eq!(m.get("E").map(String::as_str), Some("value"));
        assert_eq!(m.get("DUP").map(String::as_str), Some("second"));
        assert!(!m.contains_key("INVALID_LINE"));
    }

    #[test]
    fn parse_env_double_quote_escapes() {
        // Node 24.3: only `\n` expands inside double quotes.
        // A="a\tb" → "a\\tb", A="a\rb" → "a\\rb", A="a\\b" → "a\\\\b",
        // A="a\"b" → value "a\\" (quote ends value early when unescaped).
        assert_eq!(
            parse_env_content("A=\"a\\nb\"")
                .get("A")
                .map(String::as_str),
            Some("a\nb")
        );
        assert_eq!(
            parse_env_content("A=\"a\\tb\"")
                .get("A")
                .map(String::as_str),
            Some(r"a\tb")
        );
        assert_eq!(
            parse_env_content("A=\"a\\rb\"")
                .get("A")
                .map(String::as_str),
            Some(r"a\rb")
        );
        assert_eq!(
            parse_env_content("A=\"a\\\\b\"")
                .get("A")
                .map(String::as_str),
            Some(r"a\\b")
        );
        // `\"` is NOT an escape in Node: the quote closes the string.
        // A="a\"b" → key A value "a\\" then junk `b"` ignored / separate.
        let m_quote = parse_env_content("A=\"a\\\"b\"");
        assert_eq!(m_quote.get("A").map(String::as_str), Some(r"a\"));
        let m2 = parse_env_content("S='a\\nb'");
        assert_eq!(m2.get("S").map(String::as_str), Some(r"a\nb"));
    }

    #[test]
    fn parse_env_export_and_empty() {
        let m = parse_env_content("export FOO = bar\nEMPTY=\nKEY_ONLY_COMMENT=# c\n");
        assert_eq!(m.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(m.get("EMPTY").map(String::as_str), Some(""));
        assert_eq!(m.get("KEY_ONLY_COMMENT").map(String::as_str), Some(""));
    }

    #[test]
    fn parse_env_crlf_and_bom() {
        let m = parse_env_content("\u{feff}A=1\r\nB=2\r\n");
        assert_eq!(m.get("A").map(String::as_str), Some("1"));
        assert_eq!(m.get("B").map(String::as_str), Some("2"));
    }

    #[test]
    fn parse_env_unclosed_quote_until_newline() {
        let m =
            parse_env_content("MULTI_NOT_VALID_QUOTE=\"\nMULTI_NOT_VALID=THIS\nIS NOT MULTILINE\n");
        assert_eq!(
            m.get("MULTI_NOT_VALID_QUOTE").map(String::as_str),
            Some("\"")
        );
        assert_eq!(m.get("MULTI_NOT_VALID").map(String::as_str), Some("THIS"));
    }

    #[test]
    fn parse_env_equals_in_value() {
        let m = parse_env_content("A=\"B=C\"\nB=C=D\n");
        assert_eq!(m.get("A").map(String::as_str), Some("B=C"));
        assert_eq!(m.get("B").map(String::as_str), Some("C=D"));
    }

    #[test]
    fn parse_env_backticks_and_spaced() {
        let m = parse_env_content(
            "BACKTICKS=`backticks`\nSPACED_KEY = parsed\nTRIM=    some spaced out string\n",
        );
        assert_eq!(m.get("BACKTICKS").map(String::as_str), Some("backticks"));
        assert_eq!(m.get("SPACED_KEY").map(String::as_str), Some("parsed"));
        assert_eq!(
            m.get("TRIM").map(String::as_str),
            Some("some spaced out string")
        );
    }

    #[tokio::test]
    async fn style_text_strict_validation() {
        use raster_runtime_test::test_async_with;
        use rquickjs::CatchResultExt;

        test_async_with(|ctx| {
            Box::pin(async move {
                let style_text = create_style_text(&ctx).unwrap();

                // Single style, validateStream=false
                let styled: String = style_text
                    .call(("red", "hi", rquickjs::Object::new(ctx.clone()).and_then(
                        |o| {
                            o.set("validateStream", false)?;
                            Ok(o)
                        },
                    )
                    .unwrap()))
                    .unwrap();
                assert!(styled.contains("hi"));
                assert!(styled.contains('\u{001b}'));

                // Style array
                let opts = rquickjs::Object::new(ctx.clone()).unwrap();
                opts.set("validateStream", false).unwrap();
                let multi: String = style_text
                    .call((vec!["bold", "red"], "x", opts))
                    .unwrap();
                assert!(multi.contains("x"));
                assert!(multi.contains('\u{001b}'));

                // Non-string text (number) must throw — no String(text) coercion
                let opts = rquickjs::Object::new(ctx.clone()).unwrap();
                opts.set("validateStream", false).unwrap();
                let err = style_text
                    .call::<_, String>(("red", 42, opts))
                    .catch(&ctx)
                    .unwrap_err();
                assert!(
                    err.to_string().contains("text") || err.to_string().contains("string"),
                    "got: {err}"
                );

                // Illegal style
                let opts = rquickjs::Object::new(ctx.clone()).unwrap();
                opts.set("validateStream", false).unwrap();
                let err = style_text
                    .call::<_, String>(("not-a-style", "t", opts))
                    .catch(&ctx)
                    .unwrap_err();
                assert!(err.to_string().contains("format"), "got: {err}");

                // Hex style rejected
                let opts = rquickjs::Object::new(ctx.clone()).unwrap();
                opts.set("validateStream", false).unwrap();
                let err = style_text
                    .call::<_, String>(("#ff0000", "t", opts))
                    .catch(&ctx)
                    .unwrap_err();
                assert!(
                    err.to_string().contains("format") || err.to_string().contains("hex"),
                    "got: {err}"
                );

                // Fake stream {isTTY:true} rejected when validateStream=true
                let opts: rquickjs::Object = ctx
                    .eval(
                        r#"(function(){ return { stream: { isTTY: true } }; })()"#,
                    )
                    .unwrap();
                let err = style_text
                    .call::<_, String>(("red", "t", opts))
                    .catch(&ctx)
                    .unwrap_err();
                assert!(
                    err.to_string().contains("stream") || err.to_string().contains("Stream"),
                    "got: {err}"
                );

                // Real stream-like object (write+on) accepted; isTTY false → plain text
                let opts: rquickjs::Object = ctx
                    .eval(
                        r#"(function(){
                      const stream = {
                        isTTY: false,
                        write() {},
                        on() {},
                        hasColors() { return false; },
                        getColorDepth() { return 1; },
                      };
                      return { stream, validateStream: true };
                    })()"#,
                    )
                    .unwrap();
                let plain: String = style_text.call(("red", "plain", opts)).unwrap();
                assert_eq!(plain, "plain");

                // validateStream=false ignores fake stream
                let opts: rquickjs::Object = ctx
                    .eval(
                        r#"(function(){ return { stream: { isTTY: true }, validateStream: false }; })()"#,
                    )
                    .unwrap();
                let ok: String = style_text.call(("green", "ok", opts)).unwrap();
                assert!(ok.contains("ok"));
                assert!(ok.contains('\u{001b}'));
            })
        })
        .await;
    }
}
