import defaultImport from "node:assert";
import legacyImport from "assert";

import assert from "node:assert";

it("node:assert should be the same as assert", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const { ok } = defaultImport;

describe(`ok`, () => {
  it("Should be returned 'undefined' (So it's not an error)", () => {
    expect(ok(true)).toBeUndefined(); //bool
    expect(ok(1)).toBeUndefined(); // numeric
    expect(ok("non-empty string")).toBeUndefined(); // string
    expect(ok([])).toBeUndefined(); // array
    expect(ok({})).toBeUndefined(); // object
    expect(ok(() => {})).toBeUndefined(); // function
    expect(ok(123n)).toBeUndefined(); // bigint
    expect(ok(Symbol())).toBeUndefined(); // symbol
    expect(ok(new Error())).toBeUndefined(); // error
    class AssertTestClass {}
    expect(ok(AssertTestClass)).toBeUndefined(); // constructor
  });

  it("Should be returned exception", () => {
    // Node 24 behaviour: the thrown value is an AssertionError whose `name` is
    // "AssertionError" and whose `code` is ERR_ASSERTION. The message body is
    // "The expression evaluated to a falsy value: ..." with no "AssertionError: "
    // prefix (Raster's old private format added one). Assert the semantics and a
    // stable message fragment rather than the exact wording, so future phrasing
    // tweaks do not fail this test.
    for (const falsy of [false, 0, "", null] as any[]) {
      let caught: any;
      try {
        ok(falsy);
      } catch (e) {
        caught = e;
      }
      expect(caught).toBeInstanceOf(Error);
      expect(caught.name).toBe("AssertionError");
      expect(caught.code).toBe("ERR_ASSERTION");
      expect(caught.message).toContain("evaluated to a falsy value");
      expect(caught.message.startsWith("AssertionError: ")).toBe(false);
    }
  });

  it("should be returned as original error message", () => {
    const errMsg = "Error: Value must be true";
    expect(() => ok(false, errMsg)).toThrow(errMsg);
  });

  it("should be returned as original error", () => {
    const errMsg = "Error: This is error";
    expect(() => ok(false, Error(errMsg))).toThrow(errMsg);
  });

  it("Should be handled correctly even within functions", () => {
    const errMsg = "Error: Value should be truthy";
    function checkValue(value) {
      ok(value, errMsg);
    }
    expect(checkValue(true)).toBeUndefined();
    expect(() => checkValue(false)).toThrow(errMsg);
  });
});

it("Should be returned 'undefined' (So it's not an error)", () => {
  expect(assert(true)).toBeUndefined();
  expect(assert(1)).toBeUndefined();
  expect(assert("non-empty string")).toBeUndefined();
  expect(assert([])).toBeUndefined();
  expect(assert({})).toBeUndefined();
});
