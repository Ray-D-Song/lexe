import vm, { runInNewContext } from "node:vm";
import Module from "node:module";
import process from "node:process";

it("registers vm as a builtin module", () => {
  expect(Module.isBuiltin("vm")).toBe(true);
  expect(require("vm").runInNewContext).toBeTypeOf("function");
  expect(require("node:vm").runInNewContext).toBeTypeOf("function");
  expect(vm.runInNewContext).toBeTypeOf("function");
});

it("returns the last expression and syncs sandbox globals", () => {
  const sandbox: { count: number; name?: string } = { count: 0 };
  expect(runInNewContext("count += 1; name = 'lexe'; count", sandbox)).toBe(1);
  expect(sandbox.count).toBe(1);
  expect(sandbox.name).toBe("lexe");
});

it("reads nested sandbox values in the child context", () => {
  const inner = { bar: 1 };
  const sandbox = { inner };
  expect(runInNewContext("inner.bar", sandbox)).toBe(1);
});

it("preserves sandbox object identity for nested objects", () => {
  const inner = { bar: 1 };
  const sandbox = { inner };
  const result = runInNewContext("inner.bar = 2; inner", sandbox) as typeof inner;
  expect(inner.bar).toBe(2);
  expect(sandbox.inner).toBe(inner);
  expect(result).toBe(inner);
});

it("returns functions without JSON serialization", () => {
  const fn = () => 42;
  const sandbox = { fn };
  expect(runInNewContext("fn", sandbox)).toBe(fn);
});

it("returns circular objects by reference", () => {
  const result = runInNewContext("const obj = {}; obj.self = obj; obj", {}) as {
    self: unknown;
  };
  expect(result.self).toBe(result);
});

it("prefers script exceptions over sandbox sync failures", () => {
  const sandbox = Object.freeze({});
  expect(() =>
    runInNewContext(`x = 1; throw new Error("script boom")`, sandbox)
  ).toThrow("script boom");
});

it("invokes sandbox setter-backed properties only when written", () => {
  let setterCalls = 0;
  const sandbox: Record<string, unknown> = {};
  Object.defineProperty(sandbox, "tracked", {
    configurable: true,
    enumerable: true,
    get() {
      return 1;
    },
    set() {
      setterCalls += 1;
    },
  });
  expect(runInNewContext("tracked", sandbox)).toBe(1);
  expect(setterCalls).toBe(0);
  runInNewContext("tracked = 2", sandbox);
  expect(setterCalls).toBe(1);
});

it("propagates sandbox ownKeys exceptions when enumerated", () => {
  const sandbox = new Proxy(
    {},
    {
      ownKeys() {
        throw new Error("ownKeys failure");
      },
    }
  );
  expect(runInNewContext("1", sandbox)).toBe(1);
  expect(() => runInNewContext("Object.keys(globalThis)", sandbox)).toThrow(
    "ownKeys failure"
  );
});

it("writes manifest-like globals back to the sandbox", () => {
  const sandbox: Record<string, unknown> = {};
  runInNewContext(
    "globalThis.__RSC_MANIFEST = { routes: ['/'] }; undefined",
    sandbox
  );
  expect(sandbox.__RSC_MANIFEST).toEqual({ routes: ["/"] });
});

it("reads process.env from a sandbox object", () => {
  const sandbox = { process };
  expect(runInNewContext("process.env.NODE_ENV", sandbox)).toBe(process.env.NODE_ENV);
});

it("isolates child globals from the parent realm", () => {
  (globalThis as { childOnly?: string }).childOnly = "parent";
  const sandbox = {};
  runInNewContext("childOnly = 'child'", sandbox);
  expect((globalThis as { childOnly?: string }).childOnly).toBe("parent");
  expect((sandbox as { childOnly?: string }).childOnly).toBe("child");
});

it("propagates child exceptions to the caller", () => {
  expect(() => runInNewContext("throw new Error('boom')", {})).toThrow("boom");
});

it("returns child-created closures that capture child globals", () => {
  const fn = runInNewContext("globalThis.x = 42; () => x", {}) as () => number;
  expect(fn()).toBe(42);
});

it("treats explicit undefined contextObject like an omitted argument", () => {
  expect(runInNewContext("1", undefined)).toBe(1);
  expect(runInNewContext("1", {}, undefined)).toBe(1);
});

it("rejects source code containing NUL bytes with a syntax error", () => {
  let caught: unknown;
  try {
    runInNewContext("1\u0000", {});
  } catch (error) {
    caught = error;
  }
  expect(caught).toMatchObject({ name: "SyntaxError" });
});

it("allows NUL bytes in comments and string literals", () => {
  expect(runInNewContext("/*\u0000*/ 1", {})).toBe(1);
  expect(runInNewContext('"a\u0000b"', {})).toBe("a\u0000b");
});

it("rejects callable context objects", () => {
  expect(() => runInNewContext("1", function () {})).toThrow(
    'The "contextObject" argument must be an object'
  );
  expect(() =>
    runInNewContext("1", new Proxy(function () {}, {}))
  ).toThrow('The "contextObject" argument must be an object');
});

it("rejects filenames containing NUL bytes", () => {
  expect(() => runInNewContext("1", {}, { filename: "left\u0000right" })).toThrow(
    'The "options.filename" property must not contain null bytes'
  );
  expect(() => runInNewContext("1", {}, "left\u0000right")).toThrow(
    'The "options.filename" property must not contain null bytes'
  );
});

it("accepts filename as a string option", () => {
  expect(() =>
    runInNewContext("throw new Error('stack')", {}, "custom.js")
  ).toThrow();
});

it("treats filename: undefined like an omitted option", () => {
  expect(runInNewContext("41 + 1", {}, { filename: undefined })).toBe(42);
});

it("accepts arrays as context objects", () => {
  const sandbox = ["value"];
  expect(runInNewContext("this[0]", sandbox)).toBe("value");
});

it("ignores sloppy writes on frozen sandboxes and throws in strict mode", () => {
  const sandbox = Object.freeze({ a: 1 });
  expect(runInNewContext("x = 1; ({ payload: true })", sandbox)).toEqual({
    payload: true,
  });
  expect((sandbox as Record<string, unknown>).x).toBeUndefined();
  let strictUndeclErr: unknown;
  try {
    runInNewContext("'use strict'; x = 1", sandbox);
  } catch (err) {
    strictUndeclErr = err;
  }
  expect((strictUndeclErr as { name?: string }).name).toBe("ReferenceError");
  let strictReadonlyErr: unknown;
  try {
    runInNewContext("'use strict'; a = 2", sandbox);
  } catch (err) {
    strictReadonlyErr = err;
  }
  expect((strictReadonlyErr as { name?: string }).name).toBe("TypeError");
  expect(
    runInNewContext("Object.getOwnPropertyDescriptor(globalThis, 'a')", sandbox)
  ).toEqual({
    value: 1,
    writable: false,
    enumerable: true,
    configurable: false,
  });
});

it("transfers sandbox getter exceptions to the caller on access", () => {
  const sandbox: Record<string, unknown> = {};
  Object.defineProperty(sandbox, "boom", {
    enumerable: true,
    get() {
      throw new Error("getter failure");
    },
  });
  expect(runInNewContext("1", sandbox)).toBe(1);
  expect(() => runInNewContext("boom", sandbox)).toThrow("getter failure");
});

it("does not panic when global TypeError is poisoned", () => {
  const previous = globalThis.TypeError;
  globalThis.TypeError = () => {
    throw new Error("poisoned");
  };
  try {
    expect(() => runInNewContext("1", null as unknown as object)).toThrow();
  } finally {
    globalThis.TypeError = previous;
  }
});
