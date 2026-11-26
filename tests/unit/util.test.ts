import defaultImport from "node:util";
import legacyImport from "util";

import { EventEmitter } from "node:events";

it("node:util should be the same as util", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const { inherits, promisify, inspect, formatWithOptions, debuglog, toUSVString, types } =
  defaultImport;

it("format does not append trailing newlines", () => {
  expect(defaultImport.format("a", "b")).toBe("a b");
  expect(defaultImport.format("a", "b")).not.toContain("\n");
});

it("exposes inspect.custom and toUSVString", () => {
  expect(inspect.custom).toBe(Symbol.for("nodejs.util.inspect.custom"));
  expect(toUSVString("a\uD800b")).toBe("a\uFFFDb");
  expect(toUSVString("a\uD800\uDC00b")).toBe("a\uD800\uDC00b");
});

it("supports debuglog and util.types predicates", () => {
  const logger = debuglog("TEST");
  expect(typeof logger).toBe("function");
  expect(typeof logger.enabled).toBe("boolean");
  expect(typeof defaultImport.debug).toBe("function");

  expect(types.isSharedArrayBuffer()).toBe(false);
  expect(types.isUint8Array(new Uint8Array())).toBe(true);
  expect(types.isDataView(new DataView(new ArrayBuffer(1)))).toBe(true);
});

describe("util.types / util/types", () => {
  const _require = require;

  it("loads via util, util/types, and node:util/types with one shared object", () => {
    const fromUtil = defaultImport.types;
    const fromSub = _require("util/types");
    const fromNodeSub = _require("node:util/types");
    expect(fromUtil).toBe(fromSub);
    expect(fromUtil).toBe(fromNodeSub);
  });

  it("isDate covers Date, plain objects, and forged tags", () => {
    expect(types.isDate(new Date())).toBe(true);
    expect(types.isDate({})).toBe(false);
    expect(types.isDate({ [Symbol.toStringTag]: "Date" })).toBe(false);
    expect(types.isDate(null)).toBe(false);
    expect(types.isDate("2020-01-01")).toBe(false);
  });
});

describe("util.deprecate", () => {
  it("returns the same function identity and preserves call semantics", () => {
    const { deprecate } = defaultImport;
    const fn = function sample(this: { n: number }, x: number) {
      return this.n + x;
    };
    const wrapped = deprecate(fn, "use other", "DEP0000");
    expect(wrapped).toBe(fn);
    expect(wrapped.call({ n: 1 }, 2)).toBe(3);
  });

  it("throws TypeError for non-function arguments", () => {
    const { deprecate } = defaultImport;
    expect(() => deprecate(null as any)).toThrow(TypeError);
    expect(() => deprecate(1 as any)).toThrow(/fn|function/i);
  });
});

it("formatWithOptions formats multiple values", () => {
  expect(formatWithOptions({}, "a", "b")).toBe("a b");
  expect(formatWithOptions({ colors: true }, "%s:%s", "foo")).toBe("foo:%s");
});

describe("inherits", () => {
  it("should be inheritable parent classes", () => {
    function MyStream() {
      EventEmitter.call(this);
    }

    inherits(MyStream, EventEmitter);

    const stream = new MyStream();

    expect(stream instanceof EventEmitter).toBeTruthy();
    expect(MyStream.super_).toEqual(EventEmitter);
  });
});

describe("promisify", () => {
  it("should exist on named and default exports", async () => {
    const named = await import("util");
    expect(typeof promisify).toBe("function");
    expect(named.promisify).toBe(promisify);
    expect(named.default.promisify).toBe(promisify);
  });

  it("should resolve on callback success", async () => {
    const fn = (x: number, cb: (err: Error | null, value?: number) => void) => {
      cb(null, x + 1);
    };
    await expect(promisify(fn)(41)).resolves.toBe(42);
  });

  it("should reject on callback error", async () => {
    const err = new Error("boom");
    const fn = (_cb: (err: Error | null) => void) => {
      _cb(err);
    };
    await expect(promisify(fn)()).rejects.toBe(err);
  });

  it("should reject when the original throws synchronously", async () => {
    const fn = () => {
      throw new TypeError("sync fail");
    };
    await expect(promisify(fn)()).rejects.toThrow(/sync fail/);
  });

  it("should preserve this for method calls", async () => {
    const obj = {
      value: 7,
      method(cb: (err: Error | null, value?: number) => void) {
        cb(null, this.value);
      },
    };
    const bound = promisify(obj.method);
    await expect(bound.call(obj)).resolves.toBe(7);
  });

  it("should throw TypeError for non-function arguments", () => {
    expect(() => promisify(null as any)).toThrow(TypeError);
    expect(() => promisify(1 as any)).toThrow(
      /The "original" argument must be of type function/
    );
  });
});

describe("parseArgs", () => {
  const { parseArgs } = defaultImport;

  it("parses flags, short clusters, inline values, negative flags, defaults, and tokens", () => {
    const res = parseArgs({
      args: ["-fbval", "--bar=1", "--bar", "2", "--no-flag", "pos", "--", "-x"],
      options: {
        foo: { type: "boolean", short: "f" },
        bar: { type: "string", short: "b", multiple: true, default: ["0"] },
        flag: { type: "boolean", multiple: true },
        keep: { type: "boolean", default: true },
      },
      allowPositionals: true,
      allowNegative: true,
      tokens: true,
    });

    expect(Object.getPrototypeOf(res.values)).toBeNull();
    expect(res.values.foo).toBe(true);
    expect(res.values.bar).toEqual(["val", "1", "2"]);
    expect(res.values.flag).toEqual([false]);
    expect(res.values.keep).toBe(true);
    expect(res.positionals).toEqual(["pos", "-x"]);
    expect(res.tokens).toEqual([
      { kind: "option", name: "foo", rawName: "-f", index: 0, value: undefined, inlineValue: undefined },
      { kind: "option", name: "bar", rawName: "-b", index: 0, value: "val", inlineValue: true },
      { kind: "option", name: "bar", rawName: "--bar", index: 1, value: "1", inlineValue: true },
      { kind: "option", name: "bar", rawName: "--bar", index: 2, value: "2", inlineValue: false },
      { kind: "option", name: "flag", rawName: "--no-flag", index: 4, value: undefined, inlineValue: undefined },
      { kind: "positional", index: 5, value: "pos" },
      { kind: "option-terminator", index: 6 },
      { kind: "positional", index: 7, value: "-x" },
    ]);
  });

  it("supports non-strict unknown options and treats '-' as positional", () => {
    const res = parseArgs({
      args: ["--unk=1", "--flag", "-xy", "-", "pos"],
      strict: false,
      tokens: true,
    });
    expect(res.values).toEqual({ unk: "1", flag: true, x: true, y: true });
    expect(res.positionals).toEqual(["-", "pos"]);
  });

  it("throws Node-compatible errors on invalid config, unknown options, ambiguous values, and positionals", () => {
    const catchCode = (fn: () => unknown) => {
      try {
        fn();
        return null;
      } catch (e: any) {
        return e.code;
      }
    };

    expect(catchCode(() => parseArgs({ args: ["--unk"] }))).toBe("ERR_PARSE_ARGS_UNKNOWN_OPTION");
    expect(catchCode(() => parseArgs({ args: ["-x"] }))).toBe("ERR_PARSE_ARGS_UNKNOWN_OPTION");
    expect(catchCode(() => parseArgs({ args: ["--name"], options: { name: { type: "string" } } }))).toBe(
      "ERR_PARSE_ARGS_INVALID_OPTION_VALUE"
    );
    expect(
      catchCode(() =>
        parseArgs({ args: ["--name", "--other"], options: { name: { type: "string" }, other: { type: "boolean" } } })
      )
    ).toBe("ERR_PARSE_ARGS_INVALID_OPTION_VALUE");
    expect(catchCode(() => parseArgs({ args: ["--flag=1"], options: { flag: { type: "boolean" } } }))).toBe(
      "ERR_PARSE_ARGS_INVALID_OPTION_VALUE"
    );
    expect(catchCode(() => parseArgs({ args: ["pos"] }))).toBe("ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL");
    expect(
      catchCode(() => parseArgs({ args: [], options: { bad: { type: "number" as any } } }))
    ).toBe("ERR_INVALID_ARG_TYPE");
    expect(
      catchCode(() => parseArgs({ args: [], options: { bad: { type: "boolean", short: "ab" } } }))
    ).toBe("ERR_INVALID_ARG_VALUE");
  });
});
