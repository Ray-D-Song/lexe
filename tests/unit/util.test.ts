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

describe("util.inspect options parity", () => {
  it("supports showProxy", () => {
    const target = { a: 1 };
    const handler = { get: () => 2 };
    const proxy = new Proxy(target, handler);
    expect(inspect(proxy)).not.toContain("Proxy [");
    const withProxy = inspect(proxy, { showProxy: true });
    expect(withProxy).toContain("Proxy [");
    expect(withProxy).toContain("a: 1");
    expect(withProxy).toContain("get: [function: get]");
  });

  it("supports getters (true, 'get', 'set', throwing getter)", () => {
    const obj = {
      get onlyGet() {
        return 42;
      },
      get both() {
        return 7;
      },
      set both(_v: number) {},
      get boom() {
        throw new Error("bad getter");
      },
    };
    const allOut = inspect(obj, { getters: true });
    expect(allOut).toContain("onlyGet: [Getter: 42]");
    expect(allOut).toContain("both: [Getter/Setter: 7]");
    expect(allOut).toContain("boom: [Getter: <Inspection threw");

    const getOut = inspect(obj, { getters: "get" });
    expect(getOut).toContain("onlyGet: [Getter: 42]");
    expect(getOut).toContain("both: [Getter/Setter]");

    const setOut = inspect(obj, { getters: "set" });
    expect(setOut).toContain("onlyGet: [Getter]");
    expect(setOut).toContain("both: [Getter/Setter: 7]");
  });

  it("supports numericSeparator", () => {
    expect(inspect(1234567, { numericSeparator: true })).toBe("1_234_567");
    expect(inspect(-1234567.8901, { numericSeparator: true })).toBe("-1_234_567.890_1");
    expect(inspect(1234567890n, { numericSeparator: true })).toBe("1_234_567_890n");
    expect(inspect(123, { numericSeparator: true })).toBe("123");
  });

  it("supports breakLength", () => {
    const obj = { alpha: "1234567890", beta: "1234567890", gamma: "1234567890" };
    const single = inspect(obj, { breakLength: Infinity });
    expect(single).not.toContain("\n");
    const wrapped = inspect(obj, { breakLength: 30 });
    expect(wrapped).toContain("\n");
    expect(wrapped).toContain("  alpha:");
  });
});

describe("transferableAbortController and transferableAbortSignal", () => {
  it("transfers signal via structuredClone and propagates abort across Worker", async () => {
    const { transferableAbortController, transferableAbortSignal } = defaultImport as any;
    const { Worker } = await import("node:worker_threads");

    expect(() => transferableAbortSignal({} as any)).toThrow();

    const ac = transferableAbortController();
    const clonedSig = structuredClone(ac.signal, { transfer: [ac.signal] });
    expect(clonedSig).toBeInstanceOf(AbortSignal);
    expect(clonedSig).not.toBe(ac.signal);
    expect(clonedSig.aborted).toBe(false);

    // Re-transferring detached signal throws DataCloneError; non-transferred AbortSignal throws DataCloneError.
    let detachedErr: any;
    try {
      structuredClone(ac.signal, { transfer: [ac.signal] });
    } catch (e) {
      detachedErr = e;
    }
    expect(detachedErr?.name).toBe("DataCloneError");

    const plain = new AbortController().signal;
    let plainErr: any;
    try {
      structuredClone(plain);
    } catch (e) {
      plainErr = e;
    }
    expect(plainErr?.name).toBe("DataCloneError");

    let fired = false;
    clonedSig.addEventListener("abort", () => {
      fired = true;
    });
    ac.abort("stop-now");
    await new Promise((r) => setTimeout(r, 15));
    expect(clonedSig.aborted).toBe(true);
    expect(clonedSig.reason).toBe("stop-now");
    expect(fired).toBe(true);

    // Cross-worker propagation with transferableAbortSignal(new AbortController().signal)
    const ac2 = new AbortController();
    const tSig = transferableAbortSignal(ac2.signal);
    const code = `
      import { parentPort, workerData } from "node:worker_threads";
      workerData.signal.addEventListener("abort", () => {
        parentPort.postMessage({ aborted: true, reason: workerData.signal.reason });
      });
    `;
    const worker = new Worker(
      new URL("data:text/javascript;base64," + Buffer.from(code).toString("base64")),
      { workerData: { signal: tSig }, transferList: [tSig] }
    );
    const workerReply = new Promise<any>((resolve) => worker.once("message", resolve));
    await new Promise<void>((resolve) => worker.once("online", resolve));
    ac2.abort("cross-worker-cancel");
    const got = await workerReply;
    expect(got).toEqual({ aborted: true, reason: "cross-worker-cancel" });
    await worker.terminate();
  });
});
