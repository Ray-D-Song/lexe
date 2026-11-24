import qs, { unescapeBuffer as namedUnescapeBuffer } from "node:querystring";
import legacy from "querystring";
import Module from "node:module";

const { parse, stringify, escape, unescape, unescapeBuffer, encode, decode } = qs;

it("registers querystring as a builtin module", () => {
  expect(Module.isBuiltin("querystring")).toBe(true);
  expect(require("querystring")).toBe(qs);
  expect(require("node:querystring")).toBe(qs);
  expect(legacy).toBe(qs);
});

it("stringifies scalars, arrays, and repeated keys", () => {
  expect(stringify({ a: 1, b: ["x", "y"] })).toBe("a=1&b=x&b=y");
  expect(stringify({ empty: [] })).toBe("");
  expect(parse("a=1&b=x&b=y")).toEqual({ a: "1", b: ["x", "y"] });
});

it("supports custom separators and encoders", () => {
  expect(stringify({ a: "b" }, ";", ":")).toBe("a:b");
  expect(stringify({ a: 1, b: 2 }, "", ":")).toBe("a:1&b:2");
  expect(parse("a:b", ";", ":", { maxKeys: 0 })).toEqual({ a: "b" });
});

it("aliases encode/decode to stringify/parse", () => {
  expect(encode).toBe(stringify);
  expect(decode).toBe(parse);
  expect(encode({ a: 1 })).toBe("a=1");
  expect(decode("a=1")).toEqual({ a: "1" });
});

it("handles escape, unescape, and malformed percent sequences", () => {
  expect(escape("a b!")).toBe("a%20b!");
  expect(unescape("a+b")).toBe("a+b");
  expect(unescape("a+b%2Zc")).toBe("a+b%2Zc");
  expect(unescape("a+b%2Zc", true)).toBe("a b%2Zc");
  expect(unescape("%E2%28")).toBe("\uFFFD(");
  expect(parse("a+b=1%2Zc")).toEqual({ "a b": "1%2Zc" });
  expect(() => escape("\uD800")).toThrow(URIError);
});

it("exports unescapeBuffer returning a Buffer matching Node.js semantics", () => {
  expect(unescapeBuffer).toBe(namedUnescapeBuffer);

  const b1 = unescapeBuffer("a+b%20c");
  expect(Buffer.isBuffer(b1)).toBe(true);
  expect(b1.toString("utf8")).toBe("a+b c");

  const b2 = unescapeBuffer("a+b%20c", true);
  expect(Buffer.isBuffer(b2)).toBe(true);
  expect(b2.toString("utf8")).toBe("a b c");

  const b3 = unescapeBuffer("%E4%B8%AD");
  expect(Array.from(b3)).toEqual([0xe4, 0xb8, 0xad]);

  const b4 = unescapeBuffer("%ZZ%20a+b", true);
  expect(b4.toString("utf8")).toBe("%ZZ a b");

  expect(Array.from(unescapeBuffer(""))).toEqual([]);
  expect(() => unescapeBuffer(123 as any)).toThrow(TypeError);
});
