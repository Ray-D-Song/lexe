import assert from "node:assert";
import assertStrict, {
  equal as strictEqualNamed,
  deepEqual as deepStrictEqualNamed,
  partialDeepStrictEqual as strictPartialNamed,
  CallTracker as StrictCallTracker,
} from "node:assert/strict";

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

describe("assert.partialDeepStrictEqual", () => {
  const partial = (a: any, b: any) => eq(assert.partialDeepStrictEqual, a, b);

  it("matches subset objects, prototypes, and symbol keys", () => {
    expect(partial({ a: 1, b: 2 }, { a: 1 })).toBe(true);
    expect(partial({ a: { x: 1, y: 2 }, b: 3 }, { a: { x: 1 } })).toBe(true);
    expect(partial({ a: 1 }, { a: "1" })).toBe(false);
    expect(partial({}, { a: undefined })).toBe(false);
    expect(partial({ a: undefined }, { a: undefined })).toBe(true);
    expect(partial(Object.create(null), {})).toBe(true);
    expect(partial(new A(), { x: 1 })).toBe(true);
    expect(partial({ x: 1, y: 2 }, new A())).toBe(true);
    const s = Symbol.for("partial.k");
    expect(partial({ [s]: 1, b: 2 }, { [s]: 1 })).toBe(true);
    expect(partial({ b: 2 }, { [s]: undefined })).toBe(false);
  });

  it("matches ordered array subsequences, sparse holes, and extra keys", () => {
    expect(partial([1, 2, 3], [1, 3])).toBe(true);
    expect(partial([1, 2, 3], [3, 1])).toBe(false);
    expect(partial([{ a: 1, b: 2 }, { c: 3 }], [{ a: 1 }])).toBe(true);
    // eslint-disable-next-line no-sparse-arrays
    expect(partial([1, , 3], [, 3])).toBe(true);
    // eslint-disable-next-line no-sparse-arrays
    expect(partial([1, , 3], [undefined])).toBe(false);
    // eslint-disable-next-line no-sparse-arrays
    expect(partial([undefined], [,])).toBe(true);
    const withExtra = Object.assign([1, 2, 3], { meta: { ok: true, n: 2 } });
    const expExtra = Object.assign([1, 3], { meta: { ok: true } });
    expect(partial(withExtra, expExtra)).toBe(true);
  });

  it("matches Sets, Maps, TypedArrays, ArrayBuffers, Dates, RegExps, and Errors", () => {
    expect(
      partial(
        new Set([{ a: 1, b: 2 }, { a: 1, c: 3 }]),
        new Set([{ a: 1 }, { a: 1, c: 3 }])
      )
    ).toBe(true);
    expect(partial(new Set([1, 2]), new Set([1, 1]))).toBe(true);
    expect(
      partial(
        new Map<any, any>([[{ k: 1, x: 2 }, { v: 1, y: 2 }], [{ other: 3 }, 9]]),
        new Map<any, any>([[{ k: 1 }, { v: 1 }]])
      )
    ).toBe(true);
    expect(partial(new Uint8Array([1, 2, 3]), new Uint8Array([1, 3]))).toBe(true);
    expect(partial(Buffer.from([1, 2, 3]), new Uint8Array([1, 3]))).toBe(true);
    expect(partial(new Uint8Array([1, 2]), new Uint16Array([1, 2]))).toBe(false);
    expect(partial(new Uint8Array([1, 2, 3]).buffer, new Uint8Array([1, 3]).buffer)).toBe(
      true
    );
    expect(
      partial(
        Object.assign(new Error("boom"), { code: "E1", extra: 2 }),
        Object.assign(new Error("boom"), { code: "E1" })
      )
    ).toBe(true);
    expect(partial(new TypeError("boom"), new Error("boom"))).toBe(false);
    expect(partial(new Error("boom"), { message: "boom" })).toBe(false);
  });

  it("throws AssertionError with operator partialDeepStrictEqual", () => {
    let err: any;
    try {
      assert.partialDeepStrictEqual({ a: 1 }, { a: 2 });
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(assert.AssertionError);
    expect(err.code).toBe("ERR_ASSERTION");
    expect(err.operator).toBe("partialDeepStrictEqual");
  });
});

describe("node:assert/strict ESM named exports and CallTracker", () => {
  it("exposes named strict exports and strict.strict self-reference", () => {
    expect(assertStrict).toBe(assert.strict);
    expect(assert.strict.strict).toBe(assert.strict);
    expect(strictEqualNamed).toBe(assert.strictEqual);
    expect(deepStrictEqualNamed).toBe(assert.deepStrictEqual);
    expect(strictPartialNamed).toBe(assert.partialDeepStrictEqual);
    expect(StrictCallTracker).toBe(assert.CallTracker);
    expect(() => strictEqualNamed(1, "1" as any)).toThrow();
    expect(() => strictPartialNamed({ a: 1, b: 2 }, { a: 1 })).not.toThrow();
  });

  it("tracks calls, getCalls, report, verify, and reset", () => {
    const tracker = new assert.CallTracker();
    const fn = tracker.calls(function add(a: number, b: number) {
      return a + b;
    }, 2);
    expect(fn(3, 4)).toBe(7);
    expect(tracker.getCalls(fn)).toEqual([{ thisArg: undefined, arguments: [3, 4] }]);
    const firstReport = tracker.report();
    expect(firstReport.length).toBe(1);
    expect(firstReport[0].actual).toBe(1);
    expect(firstReport[0].expected).toBe(2);
    expect(() => tracker.verify()).toThrow(assert.AssertionError);

    expect(fn.call({ ctx: 1 }, 5, 6)).toBe(11);
    expect(tracker.report()).toEqual([]);
    expect(() => tracker.verify()).not.toThrow();

    tracker.reset(fn);
    expect(tracker.getCalls(fn)).toEqual([]);
    expect(tracker.report().length).toBe(1);
  });
});
