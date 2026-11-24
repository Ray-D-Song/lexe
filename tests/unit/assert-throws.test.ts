import assert from "node:assert";

// `throws` / `rejects` matcher semantics, measured on Node 24.21. The five
// matcher shapes behave differently and the object form is a subset match.

const boom = () => {
  const error: any = new Error("boom");
  error.code = "E_BOOM";
  throw error;
};

const rejectsBoom = async () => {
  const error: any = new Error("boom");
  error.code = "E_BOOM";
  throw error;
};

const passes = (fn: () => void) => {
  try {
    fn();
    return true;
  } catch {
    return false;
  }
};

describe("assert.throws matchers", () => {
  it("accepts any throw when no matcher is given", () => {
    expect(passes(() => assert.throws(boom))).toBe(true);
  });

  it("fails when nothing is thrown", () => {
    expect(passes(() => assert.throws(() => {}))).toBe(false);
  });

  it("matches a RegExp against the message", () => {
    expect(passes(() => assert.throws(boom, /boom/))).toBe(true);
    expect(passes(() => assert.throws(boom, /nope/))).toBe(false);
  });

  it("matches a constructor with instanceof", () => {
    expect(passes(() => assert.throws(boom, Error))).toBe(true);
    expect(passes(() => assert.throws(boom, TypeError))).toBe(false);
  });

  it("matches an exact error instance by identity", () => {
    const error = new Error("x");
    expect(
      passes(() =>
        assert.throws(() => {
          throw error;
        }, error)
      )
    ).toBe(true);
    // A different instance with the same message must not satisfy it.
    expect(
      passes(() =>
        assert.throws(() => {
          throw new Error("x");
        }, error)
      )
    ).toBe(false);
  });

  it("treats a plain object as a subset match on properties", () => {
    expect(passes(() => assert.throws(boom, { code: "E_BOOM" }))).toBe(true);
    // Extra properties on the thrown error are irrelevant.
    expect(passes(() => assert.throws(boom, { message: "boom" }))).toBe(true);
    expect(passes(() => assert.throws(boom, { code: "E_BOOM", message: "boom" }))).toBe(true);
    expect(passes(() => assert.throws(boom, { code: "WRONG" }))).toBe(false);
  });

  it("rejects a bare string as ambiguous", () => {
    let code = "";
    try {
      assert.throws(boom, "boom" as any);
    } catch (e: any) {
      code = e.code;
    }
    expect(code).toBe("ERR_AMBIGUOUS_ARGUMENT");
  });
});

describe("assert.rejects matchers", () => {
  it("accepts a rejected promise", async () => {
    await expect(assert.rejects(rejectsBoom())).resolves.toBeUndefined();
  });

  it("accepts a function returning a promise", async () => {
    await expect(assert.rejects(rejectsBoom)).resolves.toBeUndefined();
  });

  it("fails when the promise resolves", async () => {
    let failed = false;
    try {
      await assert.rejects(async () => 1);
    } catch {
      failed = true;
    }
    expect(failed).toBe(true);
  });

  it("supports RegExp, constructor and object matchers", async () => {
    await assert.rejects(rejectsBoom, /boom/);
    await assert.rejects(rejectsBoom, { code: "E_BOOM" });
    let failed = false;
    try {
      await assert.rejects(rejectsBoom, TypeError);
    } catch {
      failed = true;
    }
    expect(failed).toBe(true);
  });
});

describe("assert.doesNotThrow / doesNotReject", () => {
  it("passes when the function returns normally", () => {
    expect(passes(() => assert.doesNotThrow(() => {}))).toBe(true);
  });

  it("fails when the function throws", () => {
    expect(passes(() => assert.doesNotThrow(boom))).toBe(false);
  });

  it("passes when the promise resolves", async () => {
    await expect(assert.doesNotReject(async () => 1)).resolves.toBeUndefined();
  });
});

describe("assert.match / doesNotMatch / fail / ifError", () => {
  it("match and doesNotMatch test a RegExp against a string", () => {
    expect(passes(() => assert.match("hello", /ell/))).toBe(true);
    expect(passes(() => assert.match("hello", /xyz/))).toBe(false);
    expect(passes(() => assert.doesNotMatch("hello", /xyz/))).toBe(true);
    expect(passes(() => assert.doesNotMatch("hello", /ell/))).toBe(false);
  });

  it("fail always throws an AssertionError", () => {
    let caught: any;
    try {
      assert.fail("nope");
    } catch (e) {
      caught = e;
    }
    expect(caught.code).toBe("ERR_ASSERTION");
    expect(caught.message).toBe("nope");
  });

  it("ifError ignores null and undefined only", () => {
    expect(passes(() => assert.ifError(null))).toBe(true);
    expect(passes(() => assert.ifError(undefined))).toBe(true);
    expect(passes(() => assert.ifError(new Error("x")))).toBe(false);
    expect(passes(() => assert.ifError("x" as any))).toBe(false);
  });
});

describe("assert.strict namespace", () => {
  it("uses the strict variant for equal and deepEqual", () => {
    expect(passes(() => assert.strict.equal(1, "1" as any))).toBe(false);
    expect(passes(() => assert.strict.deepEqual({ x: 1 }, new (class {
      x = 1;
    })() as any))).toBe(false);
  });

  it("exposes the same surface as the default export", () => {
    for (const key of ["ok", "throws", "deepStrictEqual", "AssertionError"] as const) {
      expect(typeof (assert.strict as any)[key]).toBe(typeof (assert as any)[key]);
    }
  });
});

describe("assert strict/loose equality helpers", () => {
  it("distinguishes == from Object.is", () => {
    expect(passes(() => assert.equal(1, "1" as any))).toBe(true);
    expect(passes(() => assert.strictEqual(1, "1" as any))).toBe(false);
    expect(passes(() => assert.notEqual(1, 2))).toBe(true);
    expect(passes(() => assert.notStrictEqual(1, "1" as any))).toBe(true);
  });
});
