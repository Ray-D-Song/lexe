import assert from "node:assert";

// Parity coverage for the `deepEqual` cases that are easy to get wrong. Every
// expectation below was taken from Node 24.21; several contradict the docs.

const eq = (fn: (a: any, b: any) => void, a: any, b: any) => {
  try {
    fn(a, b);
    return true;
  } catch {
    return false;
  }
};

const loose = (a: any, b: any) => eq(assert.deepEqual, a, b);
const strict = (a: any, b: any) => eq(assert.deepStrictEqual, a, b);

class A {
  x = 1;
}
class B {
  x = 1;
}

describe("assert.deepEqual prototype rules", () => {
  it("ignores prototypes in loose mode only", () => {
    expect(loose({ x: 1 }, new A())).toBe(true);
    expect(strict({ x: 1 }, new A())).toBe(false);
  });

  it("distinguishes two classes with the same shape in strict mode", () => {
    expect(loose(new A(), new B())).toBe(true);
    expect(strict(new A(), new B())).toBe(false);
  });

  it("treats Object.create(null) as equal to {} in loose mode only", () => {
    expect(loose(Object.create(null), {})).toBe(true);
    expect(strict(Object.create(null), {})).toBe(false);
  });
});

describe("assert.deepEqual key handling", () => {
  it("ignores Symbol keys in loose mode and requires identity in strict", () => {
    // Loose mode never looks at symbols at all.
    expect(loose({ [Symbol()]: 1 }, {})).toBe(true);
    expect(strict({ [Symbol()]: 1 }, {})).toBe(false);

    const shared = Symbol("k");
    expect(loose({ [shared]: 1 }, { [shared]: 1 })).toBe(true);
    expect(strict({ [shared]: 1 }, { [shared]: 1 })).toBe(true);
  });

  it("ignores non-enumerable properties in both modes", () => {
    const withHidden = {};
    Object.defineProperty(withHidden, "hidden", { value: 1, enumerable: false });
    expect(loose(withHidden, {})).toBe(true);
    expect(strict(withHidden, {})).toBe(true);
  });
});

describe("assert.deepEqual structural edge cases", () => {
  it("handles cyclic structures", () => {
    const a: any = {};
    a.self = a;
    const b: any = {};
    b.self = b;
    expect(loose(a, b)).toBe(true);
    expect(strict(a, b)).toBe(true);
    // A cycle on one side only is not equal.
    expect(loose(a, {})).toBe(false);
    expect(strict(a, {})).toBe(false);
  });

  it("distinguishes a sparse array from an explicit undefined", () => {
    // eslint-disable-next-line no-sparse-arrays
    expect(loose([, 1], [undefined, 1])).toBe(false);
    expect(strict([, 1], [undefined, 1])).toBe(false);
  });

  it("does not treat arguments objects as arrays", () => {
    const args = (function (..._: any[]) {
      return arguments;
    })(1, 2);
    expect(loose(args, [1, 2])).toBe(false);
    expect(strict(args, [1, 2])).toBe(false);
  });
});

describe("assert.deepEqual typed arrays", () => {
  it("compares Buffer and Uint8Array by bytes in loose mode only", () => {
    // Both report [object Uint8Array] and hold the same bytes, so loose mode
    // accepts them; strict mode also compares prototypes, so it does not.
    expect(loose(new Uint8Array([1, 2, 3]), Buffer.from([1, 2, 3]))).toBe(true);
    expect(strict(new Uint8Array([1, 2, 3]), Buffer.from([1, 2, 3]))).toBe(false);
    expect(strict(Buffer.from([1, 2, 3]), Buffer.from([1, 2, 3]))).toBe(true);
  });

  it("distinguishes typed array element types", () => {
    expect(loose(new Uint8Array([1, 2, 3]), new Int8Array([1, 2, 3]))).toBe(false);
    expect(strict(new Uint8Array([1, 2, 3]), new Int8Array([1, 2, 3]))).toBe(false);
  });
});

describe("assert.deepEqual collections and primitives", () => {
  it("treats Map and Set as unordered", () => {
    expect(strict(new Map([["a", 1], ["b", 2]]), new Map([["b", 2], ["a", 1]]))).toBe(true);
    expect(strict(new Set([1, 2, 3]), new Set([3, 2, 1]))).toBe(true);
  });

  it("matches NaN but distinguishes -0 from +0 in strict mode", () => {
    expect(loose(NaN, NaN)).toBe(true);
    expect(strict(NaN, NaN)).toBe(true);
    expect(loose(-0, 0)).toBe(true);
    expect(strict(-0, 0)).toBe(false);
  });

  it("compares Date by time and never against a number", () => {
    expect(strict(new Date(0), new Date(0))).toBe(true);
    expect(loose(new Date(0), new Date(0))).toBe(true);
    expect(loose(new Date(0), 0)).toBe(false);
    expect(strict(new Date(0), 0)).toBe(false);
  });
});

describe("assert.AssertionError shape", () => {
  it("is an Error with code ERR_ASSERTION and the comparison fields", () => {
    let caught: any;
    try {
      assert.deepStrictEqual({ a: 1 }, { a: 2 });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(Error);
    expect(caught).toBeInstanceOf(assert.AssertionError);
    expect(caught.name).toBe("AssertionError");
    expect(caught.code).toBe("ERR_ASSERTION");
    expect(caught.operator).toBe("deepStrictEqual");
    expect("actual" in caught).toBe(true);
    expect("expected" in caught).toBe(true);
  });
});
