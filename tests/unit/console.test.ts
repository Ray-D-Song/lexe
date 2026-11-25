import defaultImport from "node:console";
import legacyImport from "console";

import * as timers from "node:timers";
import util from "node:util";

it("node:console should be the same as console", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const { Console } = defaultImport;

it("should format strings correctly", () => {
  expect(util.format("%s:%s", "foo", "bar")).toEqual("foo:bar");
  expect(util.format("█", "foo")).toEqual("█ foo");
  expect(util.format(1, 2, 3)).toEqual("1 2 3");
  expect(util.format("%% %s")).toEqual("%% %s");
  expect(util.format("%s:%s", "foo")).toEqual("foo:%s");
  expect(util.format("Hello %%, %s! How are you, %s?", "Alice", "Bob")).toEqual(
    "Hello %, Alice! How are you, Bob?"
  );
  expect(util.format("The %s %d %f. %i", "quick", "42", "3.14", "abc")).toEqual(
    "The quick 42 3.14. NaN"
  );
  expect(
    util.format("Unmatched placeholders: %s %x %% %q", "one", "two")
  ).toEqual("Unmatched placeholders: one %x % %q two");
  expect(
    util.format("Unmatched placeholders: %s", "one", "two", "three")
  ).toEqual("Unmatched placeholders: one two three");

  // Should not throw any exceptions
  console.log("%s:%s", "foo", "bar");
});

it("should log module", () => {
  let module = util.format(timers);

  expect(module).toEqual(
    `
{
  clearImmediate: [function: (anonymous)],
  clearInterval: [function: (anonymous)],
  clearTimeout: [function: (anonymous)],
  default: {
    setTimeout: [function: setTimeout],
    clearTimeout: [function: (anonymous)],
    setInterval: [function: setInterval],
    clearInterval: [function: (anonymous)],
    setImmediate: [function: setImmediate],
    clearImmediate: [function: (anonymous)],
    queueMicrotask: [function: (anonymous)],
    promises: { setTimeout: [function: setTimeout], setInterval: [function: setInterval], setImmediate: [function: setImmediate], scheduler: { wait: [function: wait], yield: [function: yield] } }
  },
  promises: [Circular],
  queueMicrotask: [function: (anonymous)],
  setImmediate: [function: setImmediate],
  setInterval: [function: setInterval],
  setTimeout: [function: setTimeout]
}
`.trim()
  );
});
it("should log using console object", () => {
  const consoleObj = new Console({
    stdout: process.stdout,
    stderr: process.stderr,
  });

  // we check if the log does not throw an exception when called
  consoleObj.log("log");
  consoleObj.debug("debug");
  consoleObj.info("info");
  consoleObj.assert(false, "text for assertion should display");
  consoleObj.assert(true, "This text should not be seen");

  consoleObj.warn("warn");
  consoleObj.error("error");
  consoleObj.trace("trace");
});

it("supports bound console timer and count methods", () => {
  expect(typeof console.count).toBe("function");
  expect(typeof console.time).toBe("function");
  expect(typeof console.timeLog).toBe("function");
  expect(typeof console.timeEnd).toBe("function");
  expect(typeof console.dir).toBe("function");

  const count = console.count.bind(console);
  const time = console.time.bind(console);
  const timeLog = console.timeLog.bind(console);
  const timeEnd = console.timeEnd.bind(console);
  const dir = console.dir.bind(console);

  expect(() => count("test-label")).not.toThrow();
  expect(() => time("timer-label")).not.toThrow();
  expect(() => timeLog("timer-label")).not.toThrow();
  expect(() => timeEnd("timer-label")).not.toThrow();
  const circular: { self?: unknown } = {};
  circular.self = circular;
  expect(() => dir(circular)).not.toThrow();
});

it("coerces non-string labels for count and time methods", () => {
  expect(() => console.count(1)).not.toThrow();
  expect(() => console.time(1)).not.toThrow();
  expect(() => console.timeLog(1)).not.toThrow();
  expect(() => console.timeEnd(1)).not.toThrow();
  expect(() => console.countReset(1)).not.toThrow();
});

it("should log complex object", () => {
  let date = new Date();

  let func = () => {};
  let ClassType = class Instance {};
  let instance = new ClassType();

  const obj = {
    a: 1,
    b: "foo",
    c: {
      d: date,
      e: [2.2, true, [], {}],
      f: {
        g: 1,
        h: 2,
        i: 3,
        j: {
          k: {
            l: "foo",
          },
          m: new Array(1000).fill(0),
        },
      },
      n: [1, 2, 3],
    },
    o: {},
    p: new (class {})(),
    q: new (class Foo {})(),
    r: () => {},
    s: function () {},
    t: function foo() {},
    u: func,
    v: instance,
    x: ClassType,
    y: null,
    z: undefined,
    1: Symbol.for("foo"),
    2: new Promise(() => {}),
    3: {},
    [3.14]: 1,
    4: [1, 2, 3],
    5: (() => {
      const rejected = Promise.reject(1);
      rejected.catch(() => {});
      return rejected;
    })(),
    6: Promise.resolve(1),
    abc: 123,
  };

  // Add a circular reference
  obj.o = obj;

  const stringObj = util.format(obj);

  expect(stringObj).toEqual(
    `
{
  '1': Symbol(foo),
  '2': Promise { <pending> },
  '3': {},
  '4': [ 1, 2, 3 ],
  '5': Promise {
    <rejected> 1
  },
  '6': Promise {
    1
  },
  a: 1,
  b: \'foo\',
  c: {
    d: ${date.toISOString()},
    e: [ 2.2, true, [], {} ],
    f: { g: 1, h: 2, i: 3, j: { k: [Object], m: [Array] } },
    n: [ 1, 2, 3 ]
  },
  o: [Circular],
  p: {},
  q: Foo {},
  r: [function: r],
  s: [function: s],
  t: [function: foo],
  u: [function: func],
  v: Instance {},
  x: [class: Instance],
  y: null,
  z: undefined,
  '3.14': 1,
  abc: 123
}
`.trim()
  );
});

it("should log Proxy object", () => {
  const target = { a: 1, b: "foo" };
  const proxy = new Proxy(target, {
    set(t, p, v) {
      t[p] = v;
      return true;
    },
  });
  expect(util.format(proxy)).toEqual(`{\n  a: 1,\n  b: 'foo'\n}`);
});

it("should log Headers", () => {
  const headers = new Headers();
  headers.append("foo", "bar");
  expect(util.format(headers)).toEqual(`Headers {
  foo: 'bar'
}`);
});

it("should handle broken utf8 surrogate pairs", () => {
  const s = "🌍🌎🌏";
  expect(util.format(s)).toEqual(s);
  expect(util.format(s.slice(1))).toEqual("�🌎🌏");

  // Test single emoji
  expect(util.format("🌍")).toEqual("🌍");

  // Test broken surrogate at end
  expect(util.format("abc🌍".slice(0, 4))).toEqual("abc�");

  // Test multiple broken surrogates
  const broken = "🌍".slice(0, 1) + "🌎".slice(0, 1) + "🌏";
  expect(util.format(broken)).toEqual("��🌏");

  // Test mixing regular chars and emojis
  expect(util.format("a🌍b🌎c")).toEqual("a🌍b🌎c");
  expect(util.format("a🌍b🌎c".slice(2))).toEqual("�b🌎c");
});

// `console.group` / `groupCollapsed` / `groupEnd` / `table` are exercised here;
// the rest of this file covers formatting and the pre-existing methods.
//
// Output assertions are structural rather than byte-for-byte where a table is
// involved: what callers can reasonably depend on is which columns exist, in
// what order, and which rows they carry. The exact padding is presentation.

it("exposes group, groupCollapsed, groupEnd and table", () => {
  for (const name of ["group", "groupCollapsed", "groupEnd", "table"]) {
    expect(`${name}:${typeof (console as any)[name]}`).toBe(
      `${name}:function`
    );
  }
});

it("group methods do not throw and groupEnd is safe to over-call", () => {
  const c = new Console({ stdout: process.stdout, stderr: process.stderr });
  c.group("g1");
  c.log("inside");
  c.group("g2");
  c.groupCollapsed("g3");
  c.groupEnd();
  c.groupEnd();
  c.groupEnd();
  // More groupEnd than group must stay a no-op, as in Node.
  c.groupEnd();
  c.groupEnd();
  c.log("after extras");
  c.group({ nested: true });
  c.table([{ a: 1 }]);
  c.groupEnd();
});

it("group accepts several arguments and none at all", () => {
  const c = new Console({ stdout: process.stdout, stderr: process.stderr });
  c.group("x", "y", 42);
  c.log("in");
  c.groupEnd();
  c.group();
  c.log("in empty");
  c.groupEnd();
});

it("table accepts the shapes Node tabulates", () => {
  const c = new Console({ stdout: process.stdout, stderr: process.stderr });
  // array of objects -> one column per key
  c.table([{ a: 1, b: 2 }, { a: 3, b: 4 }]);
  // object of objects -> keys become the index column
  c.table({ x: { a: 1 }, y: { a: 2 } });
  // primitives -> a single Values column
  c.table([10, 20, 30]);
  // column filter keeps the requested columns, in that order
  c.table([{ a: 1, b: 2, c: 3 }], ["a", "c"]);
  // nested values stay on one line
  c.table([{ a: { deep: 1 } }]);
  // empty and non-tabular inputs must not throw
  c.table([]);
  c.table(null);
  c.table(42);
  c.table("str");
  c.table([{ a: 1 }], []);
});

it("count keeps working alongside groups", () => {
  const c = new Console({ stdout: process.stdout, stderr: process.stderr });
  c.count("c");
  c.group("gc");
  c.count("c");
  c.groupEnd();
  c.count("c");
  c.countReset("c");
  c.count("c");
});

function makeMemorySink(extra: Record<string, unknown> = {}) {
  let output = "";
  const sink: any = {
    write(chunk: unknown) {
      output += String(chunk);
      return true;
    },
    getOutput() {
      return output;
    },
    clearOutput() {
      output = "";
    },
    ...extra,
  };
  return sink;
}

describe("Symbol formatting and WHATWG console namespace", () => {
  it("formats anonymous and described symbols without crashing", () => {
    expect(util.format(Symbol())).toBe("Symbol()");
    expect(util.format(Symbol("label"))).toBe("Symbol(label)");
    expect(util.format("prefix", Symbol())).toBe("prefix Symbol()");
    expect(util.inspect(Symbol())).toBe("Symbol()");
    expect(util.inspect(Symbol("abc"))).toBe("Symbol(abc)");

    const out = makeMemorySink();
    const c = new Console({ stdout: out, stderr: out, colorMode: false });
    c.log(Symbol(), Symbol("tag"));
    expect(out.getOutput()).toBe("Symbol() Symbol(tag)\n");
  });

  it("matches WHATWG Console WebIDL namespace & Node globalThis.console invariants", () => {
    const desc = Object.getOwnPropertyDescriptor(globalThis, "console")!;
    expect(desc.value).toBe(console);
    expect(desc.writable).toBe(true);
    expect(desc.enumerable).toBe(false);
    expect(desc.configurable).toBe(true);
    expect("get" in desc).toBe(false);
    expect("set" in desc).toBe(false);

    expect(typeof console).toBe("object");
    expect((globalThis as any).Console).toBeUndefined();
    expect(require("node:console")).toBe(console);
    expect(require("console")).toBe(console);
    expect(Object.getOwnPropertyDescriptor(console, "length")).toBeUndefined();
    expect(Object.getOwnPropertyDescriptor(console, "name")).toBeUndefined();
    expect((console as any)[Symbol.toStringTag]).toBe("console");
    expect(Object.prototype.toString.call(console)).toBe("[object console]");
    expect(console instanceof Console).toBe(true);

    const nsProto = Object.getPrototypeOf(console);
    expect(Reflect.ownKeys(nsProto)).toEqual([]);
    expect(Object.getPrototypeOf(nsProto)).toBe(Object.prototype);

    const methods = [
      "assert",
      "clear",
      "count",
      "countReset",
      "debug",
      "dir",
      "dirxml",
      "error",
      "group",
      "groupCollapsed",
      "groupEnd",
      "info",
      "log",
      "table",
      "time",
      "timeEnd",
      "timeLog",
      "trace",
      "warn",
      "profile",
      "profileEnd",
      "timeStamp",
    ];
    for (const name of methods) {
      const fn = (console as any)[name];
      expect(typeof fn).toBe("function");
      expect(fn.name).toBe(name);
      expect(fn.length).toBe(0);
      expect(fn.prototype).toBeUndefined();
      expect(() => new fn()).toThrow(TypeError);
    }
  });
});

describe("Console constructor, streams, and options", () => {
  it("routes stdout/stderr for both options-object and positional constructors", () => {
    const out1 = makeMemorySink();
    const err1 = makeMemorySink();
    const c1 = new Console({ stdout: out1, stderr: err1, colorMode: false });
    c1.log("out");
    c1.info("info");
    c1.debug("debug");
    c1.dirxml("xml", { x: 1 });
    c1.warn("warn");
    c1.error("err");
    expect(out1.getOutput()).toBe("out\ninfo\ndebug\nxml {\n  x: 1\n}\n");
    expect(err1.getOutput()).toBe("warn\nerr\n");

    const out2 = makeMemorySink();
    const err2 = makeMemorySink();
    const c2 = new Console(out2, err2);
    c2.log("p-out");
    c2.error("p-err");
    expect(out2.getOutput()).toBe("p-out\n");
    expect(err2.getOutput()).toBe("p-err\n");

    // stderr defaults to stdout when omitted
    const single = makeMemorySink();
    const c3 = new Console(single);
    c3.log("a");
    c3.error("b");
    expect(single.getOutput()).toBe("a\nb\n");
  });

  it("validates Console constructor arguments like Node", () => {
    const out = makeMemorySink();
    const badStream = (fn: () => unknown) => {
      try {
        fn();
        expect.unreachable();
      } catch (e: any) {
        expect(e).toBeInstanceOf(TypeError);
        expect(e.code).toBe("ERR_CONSOLE_WRITABLE_STREAM");
      }
    };
    badStream(() => new (Console as any)());
    badStream(() => new (Console as any)({}));
    badStream(() => new (Console as any)({ stdout: null }));
    badStream(() => new (Console as any)({ stdout: out, stderr: {} }));

    try {
      new Console({ stdout: out, colorMode: "invalid" as any });
      expect.unreachable();
    } catch (e: any) {
      expect(e).toBeInstanceOf(TypeError);
      expect(e.code).toBe("ERR_INVALID_ARG_VALUE");
    }

    for (const badIndent of [-1, 1001, 1.5]) {
      try {
        new Console({ stdout: out, groupIndentation: badIndent });
        expect.unreachable();
      } catch (e: any) {
        expect(e).toBeInstanceOf(RangeError);
        expect(e.code).toBe("ERR_OUT_OF_RANGE");
      }
    }
  });

  it("honors colorMode, groupIndentation, ignoreErrors, and inspectOptions", () => {
    const outTrue = makeMemorySink();
    const cTrue = new Console({ stdout: outTrue, colorMode: true });
    cTrue.log(42);
    expect(outTrue.getOutput()).toBe("\u001b[33m42\u001b[0m\n");

    const outAuto = makeMemorySink({ isTTY: true });
    const cAuto = new Console({ stdout: outAuto, colorMode: "auto" });
    cAuto.log(42);
    expect(outAuto.getOutput()).toBe("\u001b[33m42\u001b[0m\n");

    const outIndent = makeMemorySink();
    const cIndent = new Console({
      stdout: outIndent,
      colorMode: false,
      groupIndentation: 4,
      inspectOptions: { depth: 1, sorted: true, maxArrayLength: 2 },
    });
    cIndent.group("grp");
    cIndent.log({ z: 1, a: { b: { c: 3 } }, arr: [1, 2, 3, 4] });
    cIndent.groupEnd();
    cIndent.dir({ b: { c: { d: 1 } }, a: 1 });
    cIndent.dir({ b: { c: { d: 1 } }, a: 1 }, { depth: 2 });
    cIndent.dir("quoted");
    cIndent.dir({ z: 1, a: 2 }, { compact: false });
    expect(outIndent.getOutput()).toBe(
      "grp\n" +
        "    {\n" +
        "      a: {\n" +
        "        b: [Object]\n" +
        "      },\n" +
        "      arr: [ 1, 2, ... 2 more items ],\n" +
        "      z: 1\n" +
        "    }\n" +
        "{ a: 1, b: { c: [Object] } }\n" +
        "{ a: 1, b: { c: { d: 1 } } }\n" +
        "'quoted'\n" +
        "{\n  a: 2,\n  z: 1\n}\n"
    );

    const throwingStream = {
      write() {
        throw new Error("write boom");
      },
    };
    const cIgnore = new Console({ stdout: throwingStream, ignoreErrors: true });
    expect(() => cIgnore.log("safe")).not.toThrow();

    const cThrow = new Console({ stdout: throwingStream, ignoreErrors: false });
    expect(() => cThrow.log("fail")).toThrow("write boom");
  });

  it("supports assert truthiness, trace stack output, and devtools no-op methods", () => {
    const out = makeMemorySink();
    const err = makeMemorySink();
    const c = new Console({ stdout: out, stderr: err, colorMode: false });

    c.assert();
    c.assert(0 as any, "zero failed");
    c.assert("" as any, "fmt %d", 42);
    c.assert(1 as any, "not printed");
    c.trace("my trace %s", "arg");

    const lines = err.getOutput().split("\n");
    expect(lines[0]).toBe("Assertion failed");
    expect(lines[1]).toBe("Assertion failed: zero failed");
    expect(lines[2]).toBe("Assertion failed: fmt 42");
    expect(lines[3]).toBe("Trace: my trace arg");
    expect(err.getOutput()).toContain("at ");
    expect(out.getOutput()).toBe("");

    expect(console.profile("p")).toBeUndefined();
    expect(console.profileEnd("p")).toBeUndefined();
    expect(console.timeStamp("ts")).toBeUndefined();
  });

  it("wires full inspect options into util.inspect and util.formatWithOptions", () => {
    const nested = { a: { b: { c: { d: 1 } } } };
    expect(util.inspect(nested, { depth: 0 })).toBe("{ a: [Object] }");
    expect(util.inspect(nested, { depth: 1 })).toBe("{ a: { b: [Object] } }");
    expect(util.inspect(nested, { depth: 2 })).toBe("{ a: { b: { c: [Object] } } }");
    expect(util.inspect(nested, { depth: null })).toBe("{ a: { b: { c: { d: 1 } } } }");
    expect(util.inspect(nested, { depth: null, compact: false })).toBe(
      "{\n  a: {\n    b: {\n      c: {\n        d: 1\n      }\n    }\n  }\n}"
    );
    expect(util.inspect("hello")).toBe("'hello'");
    expect(util.inspect("abcdef", { maxStringLength: 3 })).toBe("'abc'... 3 more characters");
    expect(util.inspect([1, 2, 3, 4], { maxArrayLength: 2 })).toBe("[ 1, 2, ... 2 more items ]");
    expect(util.inspect([1, 2, 3], { maxArrayLength: 0 })).toBe("[ ... 3 more items ]");
    expect(util.inspect({ c: 1, a: 2, b: 3 }, { sorted: true })).toBe("{ a: 2, b: 3, c: 1 }");
    expect(
      util.inspect(
        { c: 1, a: 2, b: 3 },
        { sorted: (a, b) => (a < b ? 1 : a > b ? -1 : 0) }
      )
    ).toBe("{ c: 1, b: 3, a: 2 }");

    const hiddenObj: Record<string, unknown> = { a: 1 };
    Object.defineProperty(hiddenObj, "secret", { value: 2, enumerable: false });
    expect(util.inspect(hiddenObj, { showHidden: false })).toBe("{ a: 1 }");
    expect(util.inspect(hiddenObj, { showHidden: true })).toBe("{ a: 1, [secret]: 2 }");

    const customObj = {
      a: 1,
      [util.inspect.custom]() {
        return "CUSTOM";
      },
    };
    expect(util.inspect(customObj, { customInspect: false })).not.toBe("CUSTOM");
    expect(util.formatWithOptions({ depth: 0, sorted: true }, { z: { inner: 1 }, a: 2 })).toBe(
      "{ a: 2, z: [Object] }"
    );
  });
});
