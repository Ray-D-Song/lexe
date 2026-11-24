// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    Ctx, Object, Result, Value,
};

const QUERYSTRING_FACTORY: &str = r#"(function () {
  const hexTable = new Array(256);
  for (let i = 0; i < 256; ++i) {
    hexTable[i] = "%" + (i < 16 ? "0" : "") + i.toString(16).toUpperCase();
  }

  const isHexTable = new Int8Array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
  ]);

  const unhexTable = new Int8Array([
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, -1, -1, -1, -1, -1, -1,
    -1, 10, 11, 12, 13, 14, 15, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, 10, 11, 12, 13, 14, 15, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
  ]);

  const noEscape = new Int8Array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 1,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 0,
  ]);

  function encodeStr(str, noEscapeTable, hexTable) {
    const len = str.length;
    if (len === 0) return "";

    let out = "";
    let lastPos = 0;
    let i = 0;

    outer: for (; i < len; i++) {
      let c = str.charCodeAt(i);

      while (c < 0x80) {
        if (noEscapeTable[c] !== 1) {
          if (lastPos < i) out += str.slice(lastPos, i);
          lastPos = i + 1;
          out += hexTable[c];
        }

        if (++i === len) break outer;

        c = str.charCodeAt(i);
      }

      if (lastPos < i) out += str.slice(lastPos, i);

      if (c < 0x800) {
        lastPos = i + 1;
        out += hexTable[0xc0 | (c >> 6)] + hexTable[0x80 | (c & 0x3f)];
      } else if (c < 0xd800 || c >= 0xe000) {
        lastPos = i + 1;
        out +=
          hexTable[0xe0 | (c >> 12)] +
          hexTable[0x80 | ((c >> 6) & 0x3f)] +
          hexTable[0x80 | (c & 0x3f)];
      } else {
        if (i + 1 === len) {
          throw new URIError("URI malformed");
        }
        i++;
        c = 0x10000 + (((c & 0x3ff) << 10) | (str.charCodeAt(i) & 0x3ff));
        lastPos = i + 1;
        out +=
          hexTable[0xf0 | (c >> 18)] +
          hexTable[0x80 | ((c >> 12) & 0x3f)] +
          hexTable[0x80 | ((c >> 6) & 0x3f)] +
          hexTable[0x80 | (c & 0x3f)];
      }
    }

    if (lastPos < len) out += str.slice(lastPos);

    return out;
  }

  function unescapeBuffer(s, decodeSpaces) {
    const out = new Uint8Array(s.length);
    let index = 0;
    let outIndex = 0;
    const maxLength = s.length - 2;
    let hasHex = false;
    while (index < s.length) {
      let currentChar = s.charCodeAt(index);
      if (currentChar === 43 && decodeSpaces) {
        out[outIndex++] = 32;
        index++;
        continue;
      }
      if (currentChar === 37 && index < maxLength) {
        currentChar = s.charCodeAt(++index);
        const hexHigh = unhexTable[currentChar];
        if (!(hexHigh >= 0)) {
          out[outIndex++] = 37;
          continue;
        }
        const nextChar = s.charCodeAt(++index);
        const hexLow = unhexTable[nextChar];
        if (!(hexLow >= 0)) {
          out[outIndex++] = 37;
          index--;
        } else {
          hasHex = true;
          currentChar = hexHigh * 16 + hexLow;
        }
      }
      out[outIndex++] = currentChar;
      index++;
    }
    const bytes = hasHex ? out.subarray(0, outIndex) : out.subarray(0, outIndex);
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  }

  function qsUnescape(s, decodeSpaces) {
    try {
      return decodeURIComponent(s);
    } catch {
      return unescapeBuffer(s, decodeSpaces !== false);
    }
  }

  function qsEscape(str) {
    if (typeof str !== "string") {
      if (typeof str === "object") str = String(str);
      else str += "";
    }
    return encodeStr(str, noEscape, hexTable);
  }

  function stringifyPrimitive(v) {
    if (typeof v === "string") return v;
    if (typeof v === "number" && Number.isFinite(v)) return "" + v;
    if (typeof v === "bigint") return "" + v;
    if (typeof v === "boolean") return v ? "true" : "false";
    return "";
  }

  function encodeStringified(v, encode) {
    if (typeof v === "string") return v.length ? encode(v) : "";
    if (typeof v === "number" && Number.isFinite(v)) {
      return Math.abs(v) < 1e21 ? "" + v : encode("" + v);
    }
    if (typeof v === "bigint") return "" + v;
    if (typeof v === "boolean") return v ? "true" : "false";
    return "";
  }

  function encodeStringifiedCustom(v, encode) {
    return encode(stringifyPrimitive(v));
  }

  function stringify(obj, sep, eq, options) {
    sep = sep || "&";
    eq = eq || "=";

    let encode = qsEscape;
    if (options && typeof options.encodeURIComponent === "function") {
      encode = options.encodeURIComponent;
    }
    const convert = encode === qsEscape ? encodeStringified : encodeStringifiedCustom;

    if (obj !== null && typeof obj === "object") {
      const keys = Object.keys(obj);
      const len = keys.length;
      let fields = "";
      for (let i = 0; i < len; ++i) {
        const k = keys[i];
        const v = obj[k];
        let ks = convert(k, encode);
        ks += eq;

        if (Array.isArray(v)) {
          const vlen = v.length;
          if (vlen === 0) continue;
          if (fields) fields += sep;
          for (let j = 0; j < vlen; ++j) {
            if (j) fields += sep;
            fields += ks;
            fields += convert(v[j], encode);
          }
        } else {
          if (fields) fields += sep;
          fields += ks;
          fields += convert(v, encode);
        }
      }
      return fields;
    }
    return "";
  }

  function charCodes(str) {
    if (str.length === 0) return [];
    if (str.length === 1) return [str.charCodeAt(0)];
    const ret = new Array(str.length);
    for (let i = 0; i < str.length; ++i) ret[i] = str.charCodeAt(i);
    return ret;
  }

  const defSepCodes = [38];
  const defEqCodes = [61];

  function decodeStr(s, decoder) {
    try {
      return decoder(s);
    } catch {
      return qsUnescape(s, true);
    }
  }

  function addKeyVal(obj, key, value, keyEncoded, valEncoded, decode) {
    if (key.length > 0 && keyEncoded) key = decodeStr(key, decode);
    if (value.length > 0 && valEncoded) value = decodeStr(value, decode);

    if (obj[key] === undefined) {
      obj[key] = value;
    } else {
      const curValue = obj[key];
      if (Array.isArray(curValue)) curValue.push(value);
      else obj[key] = [curValue, value];
    }
  }

  function parse(qs, sep, eq, options) {
    const obj = Object.create(null);

    if (typeof qs !== "string" || qs.length === 0) {
      return obj;
    }

    const sepCodes = !sep ? defSepCodes : charCodes(String(sep));
    const eqCodes = !eq ? defEqCodes : charCodes(String(eq));
    const sepLen = sepCodes.length;
    const eqLen = eqCodes.length;

    let pairs = 1000;
    if (options && typeof options.maxKeys === "number") {
      pairs = options.maxKeys > 0 ? options.maxKeys : -1;
    }

    let decode = qsUnescape;
    if (options && typeof options.decodeURIComponent === "function") {
      decode = options.decodeURIComponent;
    }
    const customDecode = decode !== qsUnescape;

    let lastPos = 0;
    let sepIdx = 0;
    let eqIdx = 0;
    let key = "";
    let value = "";
    let keyEncoded = customDecode;
    let valEncoded = customDecode;
    const plusChar = customDecode ? "%20" : " ";
    let encodeCheck = 0;

    for (let i = 0; i < qs.length; ++i) {
      const code = qs.charCodeAt(i);

      if (code === sepCodes[sepIdx]) {
        if (++sepIdx === sepLen) {
          const end = i - sepIdx + 1;
          if (eqIdx < eqLen) {
            if (lastPos < end) {
              key += qs.slice(lastPos, end);
            } else if (key.length === 0) {
              if (--pairs === 0) return obj;
              lastPos = i + 1;
              sepIdx = eqIdx = 0;
              continue;
            }
          } else if (lastPos < end) {
            value += qs.slice(lastPos, end);
          }

          addKeyVal(obj, key, value, keyEncoded, valEncoded, decode);

          if (--pairs === 0) return obj;
          keyEncoded = valEncoded = customDecode;
          key = value = "";
          encodeCheck = 0;
          lastPos = i + 1;
          sepIdx = eqIdx = 0;
        }
      } else {
        sepIdx = 0;
        if (eqIdx < eqLen) {
          if (code === eqCodes[eqIdx]) {
            if (++eqIdx === eqLen) {
              const end = i - eqIdx + 1;
              if (lastPos < end) key += qs.slice(lastPos, end);
              encodeCheck = 0;
              lastPos = i + 1;
            }
            continue;
          } else {
            eqIdx = 0;
            if (!keyEncoded) {
              if (code === 37) {
                encodeCheck = 1;
                continue;
              } else if (encodeCheck > 0) {
                if (isHexTable[code] === 1) {
                  if (++encodeCheck === 3) keyEncoded = true;
                  continue;
                } else {
                  encodeCheck = 0;
                }
              }
            }
          }
          if (code === 43) {
            if (lastPos < i) key += qs.slice(lastPos, i);
            key += plusChar;
            lastPos = i + 1;
            continue;
          }
        }
        if (code === 43) {
          if (lastPos < i) value += qs.slice(lastPos, i);
          value += plusChar;
          lastPos = i + 1;
        } else if (!valEncoded) {
          if (code === 37) {
            encodeCheck = 1;
          } else if (encodeCheck > 0) {
            if (isHexTable[code] === 1) {
              if (++encodeCheck === 3) valEncoded = true;
            } else {
              encodeCheck = 0;
            }
          }
        }
      }
    }

    if (lastPos < qs.length) {
      if (eqIdx < eqLen) key += qs.slice(lastPos);
      else if (sepIdx < sepLen) value += qs.slice(lastPos);
    } else if (eqIdx === 0 && key.length === 0) {
      return obj;
    }

    addKeyVal(obj, key, value, keyEncoded, valEncoded, decode);

    return obj;
  }

  return {
    stringify,
    parse,
    escape: qsEscape,
    unescape: qsUnescape,
    encode: stringify,
    decode: parse,
  };
})()"#;

fn create_querystring<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let module_value: Value = ctx.eval(QUERYSTRING_FACTORY)?;
    Ok(module_value
        .as_object()
        .expect("querystring module")
        .clone())
}

pub struct QuerystringModule;

impl ModuleDef for QuerystringModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("stringify")?;
        declare.declare("parse")?;
        declare.declare("escape")?;
        declare.declare("unescape")?;
        declare.declare("encode")?;
        declare.declare("decode")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let querystring = create_querystring(ctx)?;

        export_default(ctx, exports, |default| {
            for name in querystring.keys::<String>() {
                let name = name?;
                let value: Value = querystring.get(&name)?;
                default.set(name, value)?;
            }
            Ok(())
        })
    }
}

impl From<QuerystringModule> for ModuleInfo<QuerystringModule> {
    fn from(val: QuerystringModule) -> Self {
        ModuleInfo {
            name: "querystring",
            module: val,
        }
    }
}
