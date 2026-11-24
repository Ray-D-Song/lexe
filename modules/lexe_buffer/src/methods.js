// Buffer instance and static methods ported from refs/node/lib/buffer.js and
// refs/node/lib/internal/buffer.js. The byte-level work (compare, search,
// fill, slice, write, swap) is done by `natives`; argument handling and
// error behavior follow Node.
(function (natives, Buffer) {
  "use strict";

  const kMaxLength = 2 ** 53 - 1;
  const proto = Buffer.prototype;
  const TypedArrayProto = Object.getPrototypeOf(Uint8Array.prototype);
  const typedArrayTag = Object.getOwnPropertyDescriptor(TypedArrayProto, Symbol.toStringTag).get;
  const isUint8Array = (v) => typedArrayTag.call(v) === "Uint8Array";
  const isArrayBufferView = (v) => ArrayBuffer.isView(v);
  const isNumber = (v) => typeof v === "number";

  // ---- errors (subset of lib/internal/errors.js) ----

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

  const inspect = (value, options) => globalThis.require("util").inspect(value, options);

  function determineSpecificType(value) {
    if (value === null) return "null";
    if (value === undefined) return "undefined";
    const type = typeof value;
    switch (type) {
      case "bigint":
        return `type bigint (${value}n)`;
      case "number":
        if (value === 0) return 1 / value === -Infinity ? "type number (-0)" : "type number (0)";
        if (value !== value) return "type number (NaN)";
        if (value === Infinity) return "type number (Infinity)";
        if (value === -Infinity) return "type number (-Infinity)";
        return `type number (${value})`;
      case "boolean":
        return value ? "type boolean (true)" : "type boolean (false)";
      case "symbol":
        return `type symbol (${String(value)})`;
      case "function":
        return `function ${value.name}`;
      case "object":
        if (value.constructor && "name" in value.constructor) {
          return `an instance of ${value.constructor.name}`;
        }
        return inspect(value, { depth: -1 });
      case "string":
        value.length > 28 && (value = `${value.slice(0, 25)}...`);
        if (value.indexOf("'") === -1) return `type string ('${value}')`;
        return `type string (${JSON.stringify(value)})`;
      default:
        value = inspect(value, { colors: false });
        if (value.length > 28) value = `${value.slice(0, 25)}...`;
        return `type ${type} (${value})`;
    }
  }

  const kTypes = ["string", "function", "number", "object", "Function", "Object", "boolean", "bigint", "symbol"];
  const classRegExp = /^([A-Z][a-z0-9]*)+$/;

  function formatList(array, type = "and") {
    return array.length < 3
      ? array.join(` ${type} `)
      : `${array.slice(0, -1).join(", ")}, ${type} ${array[array.length - 1]}`;
  }

  function invalidArgType(name, expected, actual) {
    if (!Array.isArray(expected)) expected = [expected];
    let msg = "The ";
    if (name.endsWith(" argument")) {
      msg += `${name} `;
    } else {
      msg += `"${name}" ${name.includes(".") ? "property" : "argument"} `;
    }
    msg += "must be ";
    const types = [];
    const instances = [];
    const other = [];
    for (const value of expected) {
      if (kTypes.includes(value)) types.push(value.toLowerCase());
      else if (classRegExp.exec(value) !== null) instances.push(value);
      else other.push(value);
    }
    if (instances.length > 0) {
      const pos = types.indexOf("object");
      if (pos !== -1) {
        types.splice(pos, 1);
        instances.push("Object");
      }
    }
    if (types.length > 0) {
      msg += `${types.length > 1 ? "one of type" : "of type"} ${formatList(types, "or")}`;
      if (instances.length > 0 || other.length > 0) msg += " or ";
    }
    if (instances.length > 0) {
      msg += `an instance of ${formatList(instances, "or")}`;
      if (other.length > 0) msg += " or ";
    }
    if (other.length > 0) {
      if (other.length > 1) {
        msg += `one of ${formatList(other, "or")}`;
      } else {
        if (other[0].toLowerCase() !== other[0]) msg += "an ";
        msg += `${other[0]}`;
      }
    }
    msg += `. Received ${determineSpecificType(actual)}`;
    return makeError(TypeError, "ERR_INVALID_ARG_TYPE", msg);
  }

  function addNumericalSeparator(val) {
    let res = "";
    let i = val.length;
    const start = val[0] === "-" ? 1 : 0;
    for (; i >= start + 4; i -= 3) res = `_${val.slice(i - 3, i)}${res}`;
    return `${val.slice(0, i)}${res}`;
  }

  function outOfRange(str, range, input) {
    let received;
    if (Number.isInteger(input) && Math.abs(input) > 2 ** 32) {
      received = addNumericalSeparator(String(input));
    } else if (typeof input === "bigint") {
      received = String(input);
      if (input > 2n ** 32n || input < -(2n ** 32n)) received = addNumericalSeparator(received);
      received += "n";
    } else {
      received = inspect(input);
    }
    return makeError(
      RangeError,
      "ERR_OUT_OF_RANGE",
      `The value of "${str}" is out of range. It must be ${range}. Received ${received}`
    );
  }

  function bufferOutOfBounds(name) {
    return makeError(
      RangeError,
      "ERR_BUFFER_OUT_OF_BOUNDS",
      name ? `"${name}" is outside of buffer bounds` : "Attempt to access memory outside buffer bounds"
    );
  }

  function unknownEncoding(encoding) {
    return makeError(TypeError, "ERR_UNKNOWN_ENCODING", `Unknown encoding: ${encoding}`);
  }

  function invalidArgValue(name, value, reason = "is invalid") {
    let inspected = inspect(value);
    if (inspected.length > 128) inspected = `${inspected.slice(0, 128)}...`;
    const type = name.includes(".") ? "property" : "argument";
    return makeError(TypeError, "ERR_INVALID_ARG_VALUE", `The ${type} '${name}' ${reason}. Received ${inspected}`);
  }

  function invalidBufferSize(bits) {
    return makeError(RangeError, "ERR_INVALID_BUFFER_SIZE", `Buffer size must be a multiple of ${bits}`);
  }

  // ---- validators (lib/internal/validators.js) ----

  function validateNumber(value, name, min = undefined, max) {
    if (typeof value !== "number") throw invalidArgType(name, "number", value);
    if (
      (min != null && value < min) ||
      (max != null && value > max) ||
      ((min != null || max != null) && Number.isNaN(value))
    ) {
      throw outOfRange(
        name,
        `${min != null ? `>= ${min}` : ""}${min != null && max != null ? " && " : ""}${max != null ? `<= ${max}` : ""}`,
        value
      );
    }
  }

  function validateInteger(value, name, min = Number.MIN_SAFE_INTEGER, max = Number.MAX_SAFE_INTEGER) {
    if (typeof value !== "number") throw invalidArgType(name, "number", value);
    if (!Number.isInteger(value)) throw outOfRange(name, "an integer", value);
    if (value < min || value > max) throw outOfRange(name, `>= ${min} && <= ${max}`, value);
  }

  const validateOffset = (value, name, min = 0, max = kMaxLength) => validateInteger(value, name, min, max);

  // ---- fill ----

  function normalizeEncoding(enc) {
    if (enc == null || enc === "utf8" || enc === "utf-8") return "utf8";
    return slowCases(enc);
  }

  function slowCases(enc) {
    switch (enc.length) {
      case 4:
        if (enc === "UTF8") return "utf8";
        if (enc === "ucs2" || enc === "UCS2") return "utf16le";
        enc = `${enc}`.toLowerCase();
        if (enc === "utf8") return "utf8";
        if (enc === "ucs2") return "utf16le";
        break;
      case 3:
        if (enc === "hex" || enc === "HEX" || `${enc}`.toLowerCase() === "hex") return "hex";
        break;
      case 5:
        if (enc === "ascii") return "ascii";
        if (enc === "ucs-2") return "utf16le";
        if (enc === "UTF-8") return "utf8";
        if (enc === "ASCII") return "ascii";
        if (enc === "UCS-2") return "utf16le";
        enc = `${enc}`.toLowerCase();
        if (enc === "utf-8") return "utf8";
        if (enc === "ascii") return "ascii";
        if (enc === "ucs-2") return "utf16le";
        break;
      case 6:
        if (enc === "base64") return "base64";
        if (enc === "latin1" || enc === "binary") return "latin1";
        if (enc === "BASE64") return "base64";
        if (enc === "LATIN1" || enc === "BINARY") return "latin1";
        enc = `${enc}`.toLowerCase();
        if (enc === "base64") return "base64";
        if (enc === "latin1" || enc === "binary") return "latin1";
        break;
      case 7:
        if (enc === "utf16le" || enc === "UTF16LE" || `${enc}`.toLowerCase() === "utf16le") return "utf16le";
        break;
      case 8:
        if (enc === "utf-16le" || enc === "UTF-16LE" || `${enc}`.toLowerCase() === "utf-16le") return "utf16le";
        break;
      case 9:
        if (enc === "base64url" || enc === "BASE64URL" || `${enc}`.toLowerCase() === "base64url") return "base64url";
        break;
      default:
        if (enc === "") return "utf8";
    }
    return undefined;
  }

  function validateString(value, name) {
    if (typeof value !== "string") throw invalidArgType(name, "string", value);
  }

  function _fill(buf, value, offset, end, encoding) {
    if (typeof value === "string") {
      if (offset === undefined || typeof offset === "string") {
        encoding = offset;
        offset = 0;
        end = buf.length;
      } else if (typeof end === "string") {
        encoding = end;
        end = buf.length;
      }

      const normalizedEncoding = normalizeEncoding(encoding);
      if (normalizedEncoding === undefined) {
        validateString(encoding, "encoding");
        throw unknownEncoding(encoding);
      }

      if (value.length === 0) {
        value = 0;
      } else if (value.length === 1) {
        if (normalizedEncoding === "utf8") {
          const code = value.charCodeAt(0);
          if (code < 128) value = code;
        } else if (normalizedEncoding === "latin1") {
          value = value.charCodeAt(0);
        }
      }
    } else {
      encoding = undefined;
    }

    if (offset === undefined) {
      offset = 0;
      end = buf.length;
    } else {
      validateOffset(offset, "offset");
      if (end === undefined) {
        end = buf.length;
      } else {
        validateOffset(end, "end", 0, buf.length);
      }
      if (offset >= end) return buf;
    }

    if (typeof value === "number") {
      const byteLen = buf.byteLength;
      const fillLength = end - offset;
      if (offset > end || fillLength + offset > byteLen) throw bufferOutOfBounds();
      TypedArrayProto.fill.call(buf, value, offset, end);
    } else {
      let source = value;
      if (typeof value !== "string" && !isArrayBufferView(value)) {
        source = (+value >>> 0) & 255;
        TypedArrayProto.fill.call(buf, source, offset, end);
        return buf;
      }
      const res = natives.fill(buf, source, offset, end, encoding);
      if (res < 0) {
        if (res === -1) throw invalidArgValue("value", value);
        throw bufferOutOfBounds();
      }
    }

    return buf;
  }

  proto.fill = function fill(value, offset, end, encoding) {
    return _fill(this, value, offset, end, encoding);
  };

  // ---- indexOf / lastIndexOf / includes ----

  const encodingOps = {
    utf8: { encoding: "utf8", encodingVal: "utf8", native: true },
    ucs2: { encoding: "ucs2", encodingVal: "utf16le", native: true },
    utf16le: { encoding: "utf16le", encodingVal: "utf16le", native: true },
    latin1: { encoding: "latin1", encodingVal: "latin1", native: true },
    ascii: { encoding: "ascii", encodingVal: "ascii", native: false },
    base64: { encoding: "base64", encodingVal: "base64", native: false },
    base64url: { encoding: "base64url", encodingVal: "base64url", native: false },
    hex: { encoding: "hex", encodingVal: "hex", native: false },
  };

  function getEncodingOps(encoding) {
    encoding += "";
    switch (encoding.length) {
      case 4:
        if (encoding === "utf8") return encodingOps.utf8;
        if (encoding === "ucs2") return encodingOps.ucs2;
        encoding = encoding.toLowerCase();
        if (encoding === "utf8") return encodingOps.utf8;
        if (encoding === "ucs2") return encodingOps.ucs2;
        break;
      case 5:
        if (encoding === "utf-8") return encodingOps.utf8;
        if (encoding === "ascii") return encodingOps.ascii;
        if (encoding === "ucs-2") return encodingOps.ucs2;
        encoding = encoding.toLowerCase();
        if (encoding === "utf-8") return encodingOps.utf8;
        if (encoding === "ascii") return encodingOps.ascii;
        if (encoding === "ucs-2") return encodingOps.ucs2;
        break;
      case 7:
        if (encoding === "utf16le" || encoding.toLowerCase() === "utf16le") return encodingOps.utf16le;
        break;
      case 8:
        if (encoding === "utf-16le" || encoding.toLowerCase() === "utf-16le") return encodingOps.utf16le;
        break;
      case 6:
        if (encoding === "latin1" || encoding === "binary") return encodingOps.latin1;
        if (encoding === "base64") return encodingOps.base64;
        encoding = encoding.toLowerCase();
        if (encoding === "latin1" || encoding === "binary") return encodingOps.latin1;
        if (encoding === "base64") return encodingOps.base64;
        break;
      case 3:
        if (encoding === "hex" || encoding.toLowerCase() === "hex") return encodingOps.hex;
        break;
      case 9:
        if (encoding === "base64url" || encoding.toLowerCase() === "base64url") return encodingOps.base64url;
        break;
    }
    return undefined;
  }

  function validateBuffer(buffer, name = "buffer") {
    if (!isArrayBufferView(buffer)) {
      throw invalidArgType(name, ["Buffer", "TypedArray", "DataView"], buffer);
    }
  }

  function bidirectionalIndexOf(buffer, val, byteOffset, encoding, dir) {
    validateBuffer(buffer);

    if (typeof byteOffset === "string") {
      encoding = byteOffset;
      byteOffset = undefined;
    } else if (byteOffset > 0x7fffffff) {
      byteOffset = 0x7fffffff;
    } else if (byteOffset < -0x80000000) {
      byteOffset = -0x80000000;
    }
    byteOffset = +byteOffset;
    if (Number.isNaN(byteOffset)) {
      byteOffset = dir ? 0 : buffer.length || buffer.byteLength;
    }
    dir = !!dir;

    if (typeof val === "number") {
      return natives.indexOfNumber(buffer, val >>> 0, byteOffset, dir);
    }

    let ops;
    if (encoding === undefined) ops = encodingOps.utf8;
    else ops = getEncodingOps(encoding);

    if (typeof val === "string") {
      if (ops === undefined) throw unknownEncoding(encoding);
      if (ops.native) return natives.indexOfString(buffer, val, byteOffset, ops.encodingVal, dir);
      return natives.indexOfBuffer(buffer, Buffer.from(val, ops.encoding), byteOffset, ops.encodingVal, dir);
    }

    if (isUint8Array(val)) {
      const encodingVal = ops === undefined ? "utf8" : ops.encodingVal;
      return natives.indexOfBuffer(buffer, val, byteOffset, encodingVal, dir);
    }

    throw invalidArgType("value", ["number", "string", "Buffer", "Uint8Array"], val);
  }

  proto.indexOf = function indexOf(val, byteOffset, encoding, _reserved) {
    return bidirectionalIndexOf(this, val, byteOffset, encoding, true);
  };

  proto.lastIndexOf = function lastIndexOf(val, byteOffset, encoding, _reserved) {
    return bidirectionalIndexOf(this, val, byteOffset, encoding, false);
  };

  proto.includes = function includes(val, byteOffset, encoding, _reserved) {
    return bidirectionalIndexOf(this, val, byteOffset, encoding, true) !== -1;
  };

  // ---- compare / equals ----

  function compare(buf1, buf2) {
    if (!isUint8Array(buf1)) throw invalidArgType("buf1", ["Buffer", "Uint8Array"], buf1);
    if (!isUint8Array(buf2)) throw invalidArgType("buf2", ["Buffer", "Uint8Array"], buf2);
    if (buf1 === buf2) return 0;
    return natives.compare(buf1, buf2);
  }
  Buffer.compare = compare;

  proto.equals = function equals(otherBuffer) {
    if (!isUint8Array(otherBuffer)) {
      throw invalidArgType("otherBuffer", ["Buffer", "Uint8Array"], otherBuffer);
    }
    if (this === otherBuffer) return true;
    const len = this.byteLength;
    if (len !== otherBuffer.byteLength) return false;
    return len === 0 || natives.compare(this, otherBuffer) === 0;
  };

  proto.compare = function compare(target, targetStart, targetEnd, sourceStart, sourceEnd) {
    if (!isUint8Array(target)) throw invalidArgType("target", ["Buffer", "Uint8Array"], target);
    if (arguments.length === 1) return natives.compare(this, target);

    if (targetStart === undefined) targetStart = 0;
    else validateOffset(targetStart, "targetStart");

    if (targetEnd === undefined) targetEnd = target.length;
    else validateOffset(targetEnd, "targetEnd", 0, target.length);

    if (sourceStart === undefined) sourceStart = 0;
    else validateOffset(sourceStart, "sourceStart");

    if (sourceEnd === undefined) sourceEnd = this.length;
    else validateOffset(sourceEnd, "sourceEnd", 0, this.length);

    if (sourceStart >= sourceEnd) return targetStart >= targetEnd ? 0 : -1;
    if (targetStart >= targetEnd) return 1;

    return natives.compareOffset(this, target, targetStart, sourceStart, targetEnd, sourceEnd);
  };

  const tByteLength = Object.getOwnPropertyDescriptor(TypedArrayProto, "byteLength").get;
  const dvByteLength = Object.getOwnPropertyDescriptor(DataView.prototype, "byteLength").get;
  function viewLength(buf) {
    if (!isArrayBufferView(buf)) throw makeError(TypeError, "ERR_INVALID_ARG_TYPE", "argument must be a buffer");
    return buf instanceof DataView ? dvByteLength.call(buf) : tByteLength.call(buf);
  }
  function parseIndex(value, def) {
    if (value === undefined) return def;
    let n = +value;
    n = n !== n ? 0 : Math.trunc(n);
    if (n < 0) throw makeError(RangeError, "ERR_OUT_OF_RANGE", "Index out of range");
    return n;
  }
  const sliceEncodings = [
    ["utf8", "utf8"],
    ["ucs2", "utf16le"],
    ["latin1", "latin1"],
    ["ascii", "ascii"],
    ["base64", "base64"],
    ["base64url", "base64url"],
    ["hex", "hex"],
  ];
  for (const [name, enc] of sliceEncodings) {
    const slice = { [name + "Slice"](start, end) {
      const len = viewLength(this);
      if (len === 0) return "";
      start = parseIndex(start, 0);
      end = parseIndex(end, len);
      if (end > len) throw makeError(RangeError, "ERR_OUT_OF_RANGE", "Index out of range");
      if (start >= end) return "";
      return natives.slice(this, enc, start, end);
    } }[name + "Slice"];
    Object.defineProperty(slice, "name", { value: "" });
    proto[name + "Slice"] = slice;
  }

  function checkString(value) {
    if (typeof value !== "string") throw makeError(TypeError, "ERR_INVALID_ARG_TYPE", "argument must be a string");
  }
  const jsWrapped = new Set(["utf8", "latin1", "ascii"]);
  const toStaticIndex = (v) => {
    const n = typeof v === "bigint" ? Number(v) : +v;
    return n !== n ? 0 : Math.trunc(n);
  };
  for (const [name, enc] of sliceEncodings) {
    let fn;
    if (jsWrapped.has(name)) {
      fn = { [name + "Write"](string, offset, length) {
        if (offset === undefined) offset = 0;
        if (length === undefined) length = this.byteLength - (typeof offset === "bigint" ? Number(offset) : offset);
        if (offset < 0 || offset > this.byteLength) throw bufferOutOfBounds("offset");
        if (length < 0 || length > this.byteLength - (typeof offset === "bigint" ? Number(offset) : offset)) {
          throw bufferOutOfBounds("length");
        }
        const len = viewLength(this);
        checkString(string);
        offset = toStaticIndex(offset);
        length = Math.min(toStaticIndex(length), len - offset);
        if (length <= 0) return 0;
        return natives.write(this, enc, string, offset, length);
      } }[name + "Write"];
    } else {
      fn = { [name + "Write"](string, offset, length) {
        const len = viewLength(this);
        checkString(string);
        offset = parseIndex(offset, 0);
        if (offset > len) throw bufferOutOfBounds("offset");
        length = Math.min(parseIndex(length, len - offset), len - offset);
        if (length === 0) return 0;
        return natives.write(this, enc, string, offset, length);
      } }[name + "Write"];
    }
    Object.defineProperty(fn, "name", { value: "" });
    proto[name + "Write"] = fn;
  }
  Object.defineProperty(proto.toString, "name", { value: "toString" });
  Object.defineProperty(proto.toString, "length", { value: 3 });
  Object.defineProperty(proto.write, "name", { value: "write" });
  Object.defineProperty(proto.write, "length", { value: 4 });
  proto.toLocaleString = proto.toString;

  for (const bits of [16, 32, 64]) {
    const width = bits / 8;
    const name = "swap" + bits;
    proto[name] = { [name]() {
      const len = this.length;
      if (len % width !== 0) throw invalidBufferSize(`${bits}-bits`);
      if (len > 0) natives.swap(this, width);
      return this;
    } }[name];
  }

  const tLength = Object.getOwnPropertyDescriptor(TypedArrayProto, "length").get;
  proto.toJSON = function toJSON() {
    const len = tLength.call(this);
    if (len > 0) {
      const data = new Array(len);
      for (let i = 0; i < len; ++i) data[i] = this[i];
      return { type: "Buffer", data };
    }
    return { type: "Buffer", data: [] };
  };

  function adjustOffset(offset, length) {
    offset = Math.trunc(offset);
    if (offset === 0) return 0;
    if (offset < 0) {
      offset += length;
      return offset > 0 ? offset : 0;
    }
    if (offset < length) return offset;
    return offset !== offset ? 0 : length;
  }
  const tBuffer = Object.getOwnPropertyDescriptor(TypedArrayProto, "buffer").get;
  const tByteOffset = Object.getOwnPropertyDescriptor(TypedArrayProto, "byteOffset").get;
  const dvBuffer = Object.getOwnPropertyDescriptor(DataView.prototype, "buffer").get;
  const dvByteOffset = Object.getOwnPropertyDescriptor(DataView.prototype, "byteOffset").get;
  const viewBytes = (v, start, count) => {
    const dv = v instanceof DataView;
    const buf = dv ? dvBuffer.call(v) : tBuffer.call(v);
    const off = dv ? dvByteOffset.call(v) : tByteOffset.call(v);
    return new Uint8Array(buf, off + start, count === undefined ? viewLength(v) - start : count);
  };
  proto.subarray = function subarray(start, end) {
    const srcLength = tLength.call(this);
    start = adjustOffset(start, srcLength);
    end = end !== undefined ? adjustOffset(end, srcLength) : srcLength;
    const newLength = end > start ? end - start : 0;
    return Reflect.construct(Uint8Array, [tBuffer.call(this), tByteOffset.call(this) + start, newLength], Buffer);
  };
  proto.slice = function slice(start, end) {
    return this.subarray(start, end);
  };

  function toInteger(n, defaultVal) {
    n = +n;
    if (n === n && n >= Number.MIN_SAFE_INTEGER && n <= Number.MAX_SAFE_INTEGER) {
      return n % 1 === 0 ? n : Math.floor(n);
    }
    return defaultVal;
  }
  proto.copy = function copy(target, targetStart, sourceStart, sourceEnd) {
    const source = this;
    if (!isArrayBufferView(source)) throw invalidArgType("source", ["Buffer", "Uint8Array"], source);
    if (!isArrayBufferView(target)) throw invalidArgType("target", ["Buffer", "Uint8Array"], target);
    if (targetStart === undefined) {
      targetStart = 0;
    } else {
      targetStart = Number.isInteger(targetStart) ? targetStart : toInteger(targetStart, 0);
      if (targetStart < 0) throw outOfRange("targetStart", ">= 0", targetStart);
    }
    if (sourceStart === undefined) {
      sourceStart = 0;
    } else {
      sourceStart = Number.isInteger(sourceStart) ? sourceStart : toInteger(sourceStart, 0);
      if (sourceStart < 0 || sourceStart > viewLength(source)) {
        throw outOfRange("sourceStart", `>= 0 && <= ${viewLength(source)}`, sourceStart);
      }
    }
    if (sourceEnd === undefined) {
      sourceEnd = viewLength(source);
    } else {
      sourceEnd = Number.isInteger(sourceEnd) ? sourceEnd : toInteger(sourceEnd, 0);
      if (sourceEnd < 0) throw outOfRange("sourceEnd", ">= 0", sourceEnd);
    }
    if (targetStart >= viewLength(target) || sourceStart >= sourceEnd) return 0;
    if (sourceEnd - sourceStart > viewLength(target) - targetStart) {
      sourceEnd = sourceStart + viewLength(target) - targetStart;
    }
    let nb = sourceEnd - sourceStart;
    const sourceLen = viewLength(source) - sourceStart;
    if (nb > sourceLen) nb = sourceLen;
    if (nb <= 0) return 0;
    viewBytes(target, 0).set(viewBytes(source, sourceStart, nb), targetStart);
    return nb;
  };
  Object.defineProperty(proto.copy, "length", { value: 4 });

  const typedArraySlice = TypedArrayProto.slice;
  Buffer.copyBytesFrom = function copyBytesFrom(view, offset, length) {
    if (typedArrayTag.call(view) === undefined) throw invalidArgType("view", ["TypedArray"], view);
    const viewLength = tLength.call(view);
    if (viewLength === 0) return Buffer.alloc(0);
    if (offset !== undefined || length !== undefined) {
      if (offset !== undefined) {
        validateInteger(offset, "offset", 0);
        if (offset >= viewLength) return Buffer.alloc(0);
      } else {
        offset = 0;
      }
      let end;
      if (length !== undefined) {
        validateInteger(length, "length", 0);
        end = offset + length;
      } else {
        end = viewLength;
      }
      view = typedArraySlice.call(view, offset, end);
    }
    const out = Buffer.alloc(view.byteLength);
    out.set(new Uint8Array(view.buffer, view.byteOffset, view.byteLength));
    return out;
  };

  const of = (...items) => {
    const b = Buffer.alloc(items.length);
    for (let k = 0; k < items.length; k++) b[k] = items[k];
    return b;
  };
  Buffer.of = of;

  Buffer.poolSize = 64 * 1024;
  for (const [name, key] of [["parent", "buffer"], ["offset", "byteOffset"]]) {
    Object.defineProperty(proto, name, {
      enumerable: true,
      get() {
        if (!(this instanceof Buffer)) return undefined;
        return this[key];
      },
    });
  }

  // Temporary buffers to convert numbers.
  const float32Array = new Float32Array(1);
  const uInt8Float32Array = new Uint8Array(float32Array.buffer);
  const float64Array = new Float64Array(1);
  const uInt8Float64Array = new Uint8Array(float64Array.buffer);

  // Check endianness.
  float32Array[0] = -1; // 0xBF800000
  // Either it is [0, 0, 128, 191] or [191, 128, 0, 0]. It is not possible to
  // check this with `os.endianness()` because that is determined at compile time.
  const bigEndian = uInt8Float32Array[3] === 0;

  function checkBounds(buf, offset, byteLength) {
    validateNumber(offset, 'offset');
    if (buf[offset] === undefined || buf[offset + byteLength] === undefined)
      boundsError(offset, buf.length - (byteLength + 1));
  }

  function checkInt(value, min, max, buf, offset, byteLength) {
    if (value > max || value < min) {
      const n = typeof min === 'bigint' ? 'n' : '';
      let range;
      if (byteLength > 3) {
        if (min === 0 || min === 0n) {
          range = `>= 0${n} and < 2${n} ** ${(byteLength + 1) * 8}${n}`;
        } else {
          range = `>= -(2${n} ** ${(byteLength + 1) * 8 - 1}${n}) and ` +
                  `< 2${n} ** ${(byteLength + 1) * 8 - 1}${n}`;
        }
      } else {
        range = `>= ${min}${n} and <= ${max}${n}`;
      }
      throw outOfRange('value', range, value);
    }
    checkBounds(buf, offset, byteLength);
  }

  function boundsError(value, length, type) {
    if (Math.floor(value) !== value) {
      validateNumber(value, type);
      throw outOfRange(type || 'offset', 'an integer', value);
    }

    if (length < 0)
      throw bufferOutOfBounds();

    throw outOfRange(type || 'offset',
                               `>= ${type ? 1 : 0} and <= ${length}`,
                               value);
  }

  // Read integers.
  function readBigUInt64LE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 7];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 8);

    const lo = first +
      this[++offset] * 2 ** 8 +
      this[++offset] * 2 ** 16 +
      this[++offset] * 2 ** 24;

    const hi = this[++offset] +
      this[++offset] * 2 ** 8 +
      this[++offset] * 2 ** 16 +
      last * 2 ** 24;

    return BigInt(lo) + (BigInt(hi) << 32n);
  }

  function readBigUInt64BE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 7];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 8);

    const hi = first * 2 ** 24 +
      this[++offset] * 2 ** 16 +
      this[++offset] * 2 ** 8 +
      this[++offset];

    const lo = this[++offset] * 2 ** 24 +
      this[++offset] * 2 ** 16 +
      this[++offset] * 2 ** 8 +
      last;

    return (BigInt(hi) << 32n) + BigInt(lo);
  }

  function readBigInt64LE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 7];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 8);

    const val = this[offset + 4] +
      this[offset + 5] * 2 ** 8 +
      this[offset + 6] * 2 ** 16 +
      (last << 24); // Overflow
    return (BigInt(val) << 32n) +
      BigInt(first +
      this[++offset] * 2 ** 8 +
      this[++offset] * 2 ** 16 +
      this[++offset] * 2 ** 24);
  }

  function readBigInt64BE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 7];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 8);

    const val = (first << 24) + // Overflow
      this[++offset] * 2 ** 16 +
      this[++offset] * 2 ** 8 +
      this[++offset];
    return (BigInt(val) << 32n) +
      BigInt(this[++offset] * 2 ** 24 +
      this[++offset] * 2 ** 16 +
      this[++offset] * 2 ** 8 +
      last);
  }

  function readUIntLE(offset, byteLength) {
    if (offset === undefined)
      throw invalidArgType('offset', 'number', offset);
    if (byteLength === 6)
      return readUInt48LE(this, offset);
    if (byteLength === 5)
      return readUInt40LE(this, offset);
    if (byteLength === 3)
      return readUInt24LE(this, offset);
    if (byteLength === 4)
      return this.readUInt32LE(offset);
    if (byteLength === 2)
      return this.readUInt16LE(offset);
    if (byteLength === 1)
      return this.readUInt8(offset);

    boundsError(byteLength, 6, 'byteLength');
  }

  function readUInt48LE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 5];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 6);

    return first +
      buf[++offset] * 2 ** 8 +
      buf[++offset] * 2 ** 16 +
      buf[++offset] * 2 ** 24 +
      (buf[++offset] + last * 2 ** 8) * 2 ** 32;
  }

  function readUInt40LE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 4];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 5);

    return first +
      buf[++offset] * 2 ** 8 +
      buf[++offset] * 2 ** 16 +
      buf[++offset] * 2 ** 24 +
      last * 2 ** 32;
  }

  function readUInt32LE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 3];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 4);

    return first +
      this[++offset] * 2 ** 8 +
      this[++offset] * 2 ** 16 +
      last * 2 ** 24;
  }

  function readUInt24LE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 2];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 3);

    return first + buf[++offset] * 2 ** 8 + last * 2 ** 16;
  }

  function readUInt16LE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 1];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 2);

    return first + last * 2 ** 8;
  }

  function readUInt8(offset = 0) {
    validateNumber(offset, 'offset');
    const val = this[offset];
    if (val === undefined)
      boundsError(offset, this.length - 1);

    return val;
  }

  function readUIntBE(offset, byteLength) {
    if (offset === undefined)
      throw invalidArgType('offset', 'number', offset);
    if (byteLength === 6)
      return readUInt48BE(this, offset);
    if (byteLength === 5)
      return readUInt40BE(this, offset);
    if (byteLength === 3)
      return readUInt24BE(this, offset);
    if (byteLength === 4)
      return this.readUInt32BE(offset);
    if (byteLength === 2)
      return this.readUInt16BE(offset);
    if (byteLength === 1)
      return this.readUInt8(offset);

    boundsError(byteLength, 6, 'byteLength');
  }

  function readUInt48BE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 5];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 6);

    return (first * 2 ** 8 + buf[++offset]) * 2 ** 32 +
      buf[++offset] * 2 ** 24 +
      buf[++offset] * 2 ** 16 +
      buf[++offset] * 2 ** 8 +
      last;
  }

  function readUInt40BE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 4];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 5);

    return first * 2 ** 32 +
      buf[++offset] * 2 ** 24 +
      buf[++offset] * 2 ** 16 +
      buf[++offset] * 2 ** 8 +
      last;
  }

  function readUInt32BE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 3];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 4);

    return first * 2 ** 24 +
      this[++offset] * 2 ** 16 +
      this[++offset] * 2 ** 8 +
      last;
  }

  function readUInt24BE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 2];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 3);

    return first * 2 ** 16 + buf[++offset] * 2 ** 8 + last;
  }

  function readUInt16BE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 1];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 2);

    return first * 2 ** 8 + last;
  }

  function readIntLE(offset, byteLength) {
    if (offset === undefined)
      throw invalidArgType('offset', 'number', offset);
    if (byteLength === 6)
      return readInt48LE(this, offset);
    if (byteLength === 5)
      return readInt40LE(this, offset);
    if (byteLength === 3)
      return readInt24LE(this, offset);
    if (byteLength === 4)
      return this.readInt32LE(offset);
    if (byteLength === 2)
      return this.readInt16LE(offset);
    if (byteLength === 1)
      return this.readInt8(offset);

    boundsError(byteLength, 6, 'byteLength');
  }

  function readInt48LE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 5];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 6);

    const val = buf[offset + 4] + last * 2 ** 8;
    return (val | (val & 2 ** 15) * 0x1fffe) * 2 ** 32 +
      first +
      buf[++offset] * 2 ** 8 +
      buf[++offset] * 2 ** 16 +
      buf[++offset] * 2 ** 24;
  }

  function readInt40LE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 4];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 5);

    return (last | (last & 2 ** 7) * 0x1fffffe) * 2 ** 32 +
      first +
      buf[++offset] * 2 ** 8 +
      buf[++offset] * 2 ** 16 +
      buf[++offset] * 2 ** 24;
  }

  function readInt32LE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 3];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 4);

    return first +
      this[++offset] * 2 ** 8 +
      this[++offset] * 2 ** 16 +
      (last << 24); // Overflow
  }

  function readInt24LE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 2];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 3);

    const val = first + buf[++offset] * 2 ** 8 + last * 2 ** 16;
    return val | (val & 2 ** 23) * 0x1fe;
  }

  function readInt16LE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 1];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 2);

    const val = first + last * 2 ** 8;
    return val | (val & 2 ** 15) * 0x1fffe;
  }

  function readInt8(offset = 0) {
    validateNumber(offset, 'offset');
    const val = this[offset];
    if (val === undefined)
      boundsError(offset, this.length - 1);

    return val | (val & 2 ** 7) * 0x1fffffe;
  }

  function readIntBE(offset, byteLength) {
    if (offset === undefined)
      throw invalidArgType('offset', 'number', offset);
    if (byteLength === 6)
      return readInt48BE(this, offset);
    if (byteLength === 5)
      return readInt40BE(this, offset);
    if (byteLength === 3)
      return readInt24BE(this, offset);
    if (byteLength === 4)
      return this.readInt32BE(offset);
    if (byteLength === 2)
      return this.readInt16BE(offset);
    if (byteLength === 1)
      return this.readInt8(offset);

    boundsError(byteLength, 6, 'byteLength');
  }

  function readInt48BE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 5];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 6);

    const val = buf[++offset] + first * 2 ** 8;
    return (val | (val & 2 ** 15) * 0x1fffe) * 2 ** 32 +
      buf[++offset] * 2 ** 24 +
      buf[++offset] * 2 ** 16 +
      buf[++offset] * 2 ** 8 +
      last;
  }

  function readInt40BE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 4];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 5);

    return (first | (first & 2 ** 7) * 0x1fffffe) * 2 ** 32 +
      buf[++offset] * 2 ** 24 +
      buf[++offset] * 2 ** 16 +
      buf[++offset] * 2 ** 8 +
      last;
  }

  function readInt32BE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 3];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 4);

    return (first << 24) + // Overflow
      this[++offset] * 2 ** 16 +
      this[++offset] * 2 ** 8 +
      last;
  }

  function readInt24BE(buf, offset = 0) {
    validateNumber(offset, 'offset');
    const first = buf[offset];
    const last = buf[offset + 2];
    if (first === undefined || last === undefined)
      boundsError(offset, buf.length - 3);

    const val = first * 2 ** 16 + buf[++offset] * 2 ** 8 + last;
    return val | (val & 2 ** 23) * 0x1fe;
  }

  function readInt16BE(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 1];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 2);

    const val = first * 2 ** 8 + last;
    return val | (val & 2 ** 15) * 0x1fffe;
  }

  // Read floats
  function readFloatBackwards(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 3];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 4);

    uInt8Float32Array[3] = first;
    uInt8Float32Array[2] = this[++offset];
    uInt8Float32Array[1] = this[++offset];
    uInt8Float32Array[0] = last;
    return float32Array[0];
  }

  function readFloatForwards(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 3];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 4);

    uInt8Float32Array[0] = first;
    uInt8Float32Array[1] = this[++offset];
    uInt8Float32Array[2] = this[++offset];
    uInt8Float32Array[3] = last;
    return float32Array[0];
  }

  function readDoubleBackwards(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 7];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 8);

    uInt8Float64Array[7] = first;
    uInt8Float64Array[6] = this[++offset];
    uInt8Float64Array[5] = this[++offset];
    uInt8Float64Array[4] = this[++offset];
    uInt8Float64Array[3] = this[++offset];
    uInt8Float64Array[2] = this[++offset];
    uInt8Float64Array[1] = this[++offset];
    uInt8Float64Array[0] = last;
    return float64Array[0];
  }

  function readDoubleForwards(offset = 0) {
    validateNumber(offset, 'offset');
    const first = this[offset];
    const last = this[offset + 7];
    if (first === undefined || last === undefined)
      boundsError(offset, this.length - 8);

    uInt8Float64Array[0] = first;
    uInt8Float64Array[1] = this[++offset];
    uInt8Float64Array[2] = this[++offset];
    uInt8Float64Array[3] = this[++offset];
    uInt8Float64Array[4] = this[++offset];
    uInt8Float64Array[5] = this[++offset];
    uInt8Float64Array[6] = this[++offset];
    uInt8Float64Array[7] = last;
    return float64Array[0];
  }

  // Write integers.
  function writeBigU_Int64LE(buf, value, offset, min, max) {
    checkInt(value, min, max, buf, offset, 7);

    let lo = Number(value & 0xffffffffn);
    buf[offset++] = lo;
    lo = lo >> 8;
    buf[offset++] = lo;
    lo = lo >> 8;
    buf[offset++] = lo;
    lo = lo >> 8;
    buf[offset++] = lo;
    let hi = Number(value >> 32n & 0xffffffffn);
    buf[offset++] = hi;
    hi = hi >> 8;
    buf[offset++] = hi;
    hi = hi >> 8;
    buf[offset++] = hi;
    hi = hi >> 8;
    buf[offset++] = hi;
    return offset;
  }

  function writeBigUInt64LE(value, offset = 0) {
    return writeBigU_Int64LE(this, value, offset, 0n, 0xffffffffffffffffn);
  }

  function writeBigU_Int64BE(buf, value, offset, min, max) {
    checkInt(value, min, max, buf, offset, 7);

    let lo = Number(value & 0xffffffffn);
    buf[offset + 7] = lo;
    lo = lo >> 8;
    buf[offset + 6] = lo;
    lo = lo >> 8;
    buf[offset + 5] = lo;
    lo = lo >> 8;
    buf[offset + 4] = lo;
    let hi = Number(value >> 32n & 0xffffffffn);
    buf[offset + 3] = hi;
    hi = hi >> 8;
    buf[offset + 2] = hi;
    hi = hi >> 8;
    buf[offset + 1] = hi;
    hi = hi >> 8;
    buf[offset] = hi;
    return offset + 8;
  }

  function writeBigUInt64BE(value, offset = 0) {
    return writeBigU_Int64BE(this, value, offset, 0n, 0xffffffffffffffffn);
  }

  function writeBigInt64LE(value, offset = 0) {
    return writeBigU_Int64LE(
      this, value, offset, -0x8000000000000000n, 0x7fffffffffffffffn);
  }

  function writeBigInt64BE(value, offset = 0) {
    return writeBigU_Int64BE(
      this, value, offset, -0x8000000000000000n, 0x7fffffffffffffffn);
  }

  function writeUIntLE(value, offset, byteLength) {
    if (byteLength === 6)
      return writeU_Int48LE(this, value, offset, 0, 0xffffffffffff);
    if (byteLength === 5)
      return writeU_Int40LE(this, value, offset, 0, 0xffffffffff);
    if (byteLength === 3)
      return writeU_Int24LE(this, value, offset, 0, 0xffffff);
    if (byteLength === 4)
      return writeU_Int32LE(this, value, offset, 0, 0xffffffff);
    if (byteLength === 2)
      return writeU_Int16LE(this, value, offset, 0, 0xffff);
    if (byteLength === 1)
      return writeU_Int8(this, value, offset, 0, 0xff);

    boundsError(byteLength, 6, 'byteLength');
  }

  function writeU_Int48LE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 5);

    const newVal = Math.floor(value * 2 ** -32);
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    buf[offset++] = newVal;
    buf[offset++] = (newVal >>> 8);
    return offset;
  }

  function writeU_Int40LE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 4);

    const newVal = value;
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    buf[offset++] = Math.floor(newVal * 2 ** -32);
    return offset;
  }

  function writeU_Int32LE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 3);

    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    return offset;
  }

  function writeUInt32LE(value, offset = 0) {
    return writeU_Int32LE(this, value, offset, 0, 0xffffffff);
  }

  function writeU_Int24LE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 2);

    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    value = value >>> 8;
    buf[offset++] = value;
    return offset;
  }

  function writeU_Int16LE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 1);

    buf[offset++] = value;
    buf[offset++] = (value >>> 8);
    return offset;
  }

  function writeUInt16LE(value, offset = 0) {
    return writeU_Int16LE(this, value, offset, 0, 0xffff);
  }

  function writeU_Int8(buf, value, offset, min, max) {
    value = +value;
    // `checkInt()` can not be used here because it checks two entries.
    validateNumber(offset, 'offset');
    if (value > max || value < min) {
      throw outOfRange('value', `>= ${min} and <= ${max}`, value);
    }
    if (buf[offset] === undefined)
      boundsError(offset, buf.length - 1);

    buf[offset] = value;
    return offset + 1;
  }

  function writeUInt8(value, offset = 0) {
    return writeU_Int8(this, value, offset, 0, 0xff);
  }

  function writeUIntBE(value, offset, byteLength) {
    if (byteLength === 6)
      return writeU_Int48BE(this, value, offset, 0, 0xffffffffffff);
    if (byteLength === 5)
      return writeU_Int40BE(this, value, offset, 0, 0xffffffffff);
    if (byteLength === 3)
      return writeU_Int24BE(this, value, offset, 0, 0xffffff);
    if (byteLength === 4)
      return writeU_Int32BE(this, value, offset, 0, 0xffffffff);
    if (byteLength === 2)
      return writeU_Int16BE(this, value, offset, 0, 0xffff);
    if (byteLength === 1)
      return writeU_Int8(this, value, offset, 0, 0xff);

    boundsError(byteLength, 6, 'byteLength');
  }

  function writeU_Int48BE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 5);

    const newVal = Math.floor(value * 2 ** -32);
    buf[offset++] = (newVal >>> 8);
    buf[offset++] = newVal;
    buf[offset + 3] = value;
    value = value >>> 8;
    buf[offset + 2] = value;
    value = value >>> 8;
    buf[offset + 1] = value;
    value = value >>> 8;
    buf[offset] = value;
    return offset + 4;
  }

  function writeU_Int40BE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 4);

    buf[offset++] = Math.floor(value * 2 ** -32);
    buf[offset + 3] = value;
    value = value >>> 8;
    buf[offset + 2] = value;
    value = value >>> 8;
    buf[offset + 1] = value;
    value = value >>> 8;
    buf[offset] = value;
    return offset + 4;
  }

  function writeU_Int32BE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 3);

    buf[offset + 3] = value;
    value = value >>> 8;
    buf[offset + 2] = value;
    value = value >>> 8;
    buf[offset + 1] = value;
    value = value >>> 8;
    buf[offset] = value;
    return offset + 4;
  }

  function writeUInt32BE(value, offset = 0) {
    return writeU_Int32BE(this, value, offset, 0, 0xffffffff);
  }

  function writeU_Int24BE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 2);

    buf[offset + 2] = value;
    value = value >>> 8;
    buf[offset + 1] = value;
    value = value >>> 8;
    buf[offset] = value;
    return offset + 3;
  }

  function writeU_Int16BE(buf, value, offset, min, max) {
    value = +value;
    checkInt(value, min, max, buf, offset, 1);

    buf[offset++] = (value >>> 8);
    buf[offset++] = value;
    return offset;
  }

  function writeUInt16BE(value, offset = 0) {
    return writeU_Int16BE(this, value, offset, 0, 0xffff);
  }

  function writeIntLE(value, offset, byteLength) {
    if (byteLength === 6)
      return writeU_Int48LE(this, value, offset, -0x800000000000, 0x7fffffffffff);
    if (byteLength === 5)
      return writeU_Int40LE(this, value, offset, -0x8000000000, 0x7fffffffff);
    if (byteLength === 3)
      return writeU_Int24LE(this, value, offset, -0x800000, 0x7fffff);
    if (byteLength === 4)
      return writeU_Int32LE(this, value, offset, -0x80000000, 0x7fffffff);
    if (byteLength === 2)
      return writeU_Int16LE(this, value, offset, -0x8000, 0x7fff);
    if (byteLength === 1)
      return writeU_Int8(this, value, offset, -0x80, 0x7f);

    boundsError(byteLength, 6, 'byteLength');
  }

  function writeInt32LE(value, offset = 0) {
    return writeU_Int32LE(this, value, offset, -0x80000000, 0x7fffffff);
  }

  function writeInt16LE(value, offset = 0) {
    return writeU_Int16LE(this, value, offset, -0x8000, 0x7fff);
  }

  function writeInt8(value, offset = 0) {
    return writeU_Int8(this, value, offset, -0x80, 0x7f);
  }

  function writeIntBE(value, offset, byteLength) {
    if (byteLength === 6)
      return writeU_Int48BE(this, value, offset, -0x800000000000, 0x7fffffffffff);
    if (byteLength === 5)
      return writeU_Int40BE(this, value, offset, -0x8000000000, 0x7fffffffff);
    if (byteLength === 3)
      return writeU_Int24BE(this, value, offset, -0x800000, 0x7fffff);
    if (byteLength === 4)
      return writeU_Int32BE(this, value, offset, -0x80000000, 0x7fffffff);
    if (byteLength === 2)
      return writeU_Int16BE(this, value, offset, -0x8000, 0x7fff);
    if (byteLength === 1)
      return writeU_Int8(this, value, offset, -0x80, 0x7f);

    boundsError(byteLength, 6, 'byteLength');
  }

  function writeInt32BE(value, offset = 0) {
    return writeU_Int32BE(this, value, offset, -0x80000000, 0x7fffffff);
  }

  function writeInt16BE(value, offset = 0) {
    return writeU_Int16BE(this, value, offset, -0x8000, 0x7fff);
  }

  // Write floats.
  function writeDoubleForwards(val, offset = 0) {
    val = +val;
    checkBounds(this, offset, 7);

    float64Array[0] = val;
    this[offset++] = uInt8Float64Array[0];
    this[offset++] = uInt8Float64Array[1];
    this[offset++] = uInt8Float64Array[2];
    this[offset++] = uInt8Float64Array[3];
    this[offset++] = uInt8Float64Array[4];
    this[offset++] = uInt8Float64Array[5];
    this[offset++] = uInt8Float64Array[6];
    this[offset++] = uInt8Float64Array[7];
    return offset;
  }

  function writeDoubleBackwards(val, offset = 0) {
    val = +val;
    checkBounds(this, offset, 7);

    float64Array[0] = val;
    this[offset++] = uInt8Float64Array[7];
    this[offset++] = uInt8Float64Array[6];
    this[offset++] = uInt8Float64Array[5];
    this[offset++] = uInt8Float64Array[4];
    this[offset++] = uInt8Float64Array[3];
    this[offset++] = uInt8Float64Array[2];
    this[offset++] = uInt8Float64Array[1];
    this[offset++] = uInt8Float64Array[0];
    return offset;
  }

  function writeFloatForwards(val, offset = 0) {
    val = +val;
    checkBounds(this, offset, 3);

    float32Array[0] = val;
    this[offset++] = uInt8Float32Array[0];
    this[offset++] = uInt8Float32Array[1];
    this[offset++] = uInt8Float32Array[2];
    this[offset++] = uInt8Float32Array[3];
    return offset;
  }

  function writeFloatBackwards(val, offset = 0) {
    val = +val;
    checkBounds(this, offset, 3);

    float32Array[0] = val;
    this[offset++] = uInt8Float32Array[3];
    this[offset++] = uInt8Float32Array[2];
    this[offset++] = uInt8Float32Array[1];
    this[offset++] = uInt8Float32Array[0];
    return offset;
  }

    proto.readBigUInt64LE = readBigUInt64LE;
    proto.readBigUInt64BE = readBigUInt64BE;
    proto.readBigUint64LE = readBigUInt64LE;
    proto.readBigUint64BE = readBigUInt64BE;
    proto.readBigInt64LE = readBigInt64LE;
    proto.readBigInt64BE = readBigInt64BE;
    proto.writeBigUInt64LE = writeBigUInt64LE;
    proto.writeBigUInt64BE = writeBigUInt64BE;
    proto.writeBigUint64LE = writeBigUInt64LE;
    proto.writeBigUint64BE = writeBigUInt64BE;
    proto.writeBigInt64LE = writeBigInt64LE;
    proto.writeBigInt64BE = writeBigInt64BE;

    proto.readUIntLE = readUIntLE;
    proto.readUInt32LE = readUInt32LE;
    proto.readUInt16LE = readUInt16LE;
    proto.readUInt8 = readUInt8;
    proto.readUIntBE = readUIntBE;
    proto.readUInt32BE = readUInt32BE;
    proto.readUInt16BE = readUInt16BE;
    proto.readUintLE = readUIntLE;
    proto.readUint32LE = readUInt32LE;
    proto.readUint16LE = readUInt16LE;
    proto.readUint8 = readUInt8;
    proto.readUintBE = readUIntBE;
    proto.readUint32BE = readUInt32BE;
    proto.readUint16BE = readUInt16BE;
    proto.readIntLE = readIntLE;
    proto.readInt32LE = readInt32LE;
    proto.readInt16LE = readInt16LE;
    proto.readInt8 = readInt8;
    proto.readIntBE = readIntBE;
    proto.readInt32BE = readInt32BE;
    proto.readInt16BE = readInt16BE;

    proto.writeUIntLE = writeUIntLE;
    proto.writeUInt32LE = writeUInt32LE;
    proto.writeUInt16LE = writeUInt16LE;
    proto.writeUInt8 = writeUInt8;
    proto.writeUIntBE = writeUIntBE;
    proto.writeUInt32BE = writeUInt32BE;
    proto.writeUInt16BE = writeUInt16BE;
    proto.writeUintLE = writeUIntLE;
    proto.writeUint32LE = writeUInt32LE;
    proto.writeUint16LE = writeUInt16LE;
    proto.writeUint8 = writeUInt8;
    proto.writeUintBE = writeUIntBE;
    proto.writeUint32BE = writeUInt32BE;
    proto.writeUint16BE = writeUInt16BE;
    proto.writeIntLE = writeIntLE;
    proto.writeInt32LE = writeInt32LE;
    proto.writeInt16LE = writeInt16LE;
    proto.writeInt8 = writeInt8;
    proto.writeIntBE = writeIntBE;
    proto.writeInt32BE = writeInt32BE;
    proto.writeInt16BE = writeInt16BE;

    proto.readFloatLE = bigEndian ? readFloatBackwards : readFloatForwards;
    proto.readFloatBE = bigEndian ? readFloatForwards : readFloatBackwards;
    proto.readDoubleLE = bigEndian ? readDoubleBackwards : readDoubleForwards;
    proto.readDoubleBE = bigEndian ? readDoubleForwards : readDoubleBackwards;
    proto.writeFloatLE = bigEndian ? writeFloatBackwards : writeFloatForwards;
    proto.writeFloatBE = bigEndian ? writeFloatForwards : writeFloatBackwards;
    proto.writeDoubleLE = bigEndian ? writeDoubleBackwards : writeDoubleForwards;
    proto.writeDoubleBE = bigEndian ? writeDoubleForwards : writeDoubleBackwards;


  for (const n of ["UInt32LE", "UInt16LE", "UInt8", "UInt32BE", "UInt16BE", "Int32LE", "Int16LE", "Int8", "Int32BE", "Int16BE"]) {
    const fn = proto["read" + n];
    Object.defineProperty(fn, "length", { value: 1 });
    Object.defineProperty(fn, "name", { value: "" });
  }
});
