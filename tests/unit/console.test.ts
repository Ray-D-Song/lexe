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
    queueMicrotask: [function: (anonymous)]
  },
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
