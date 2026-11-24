(function () {
  "use strict";

  const objectToString = Object.prototype.toString;
  const hasOwn = (o, k) => Object.prototype.hasOwnProperty.call(o, k);
  const isObjectLike = (v) => typeof v === "object" && v !== null;
  const tagOf = (v) => objectToString.call(v).slice(8, -1);
  const isTypedArray = (v) => ArrayBuffer.isView(v) && !(v instanceof DataView);

  const sameValueZero = (a, b) => a === b || (a !== a && b !== b);
  // eslint-disable-next-line eqeqeq
  const loosePrimitiveEqual = (a, b) => sameValueZero(a, b) || a == b;

  function ownEnumerableKeys(obj, includeSymbols) {
    const keys = Object.keys(obj);
    if (!includeSymbols) return keys;
    for (const sym of Object.getOwnPropertySymbols(obj)) {
      if (Object.getOwnPropertyDescriptor(obj, sym).enumerable) keys.push(sym);
    }
    return keys;
  }

  const bytesOf = (view) => new Uint8Array(view.buffer, view.byteOffset, view.byteLength);

  function typedArraysEqual(a, b, strict) {
    if (a.byteLength !== b.byteLength) return false;
    if (tagOf(a) !== tagOf(b)) return false;
    // Strict mode also requires the same prototype, which is what separates
    // `Buffer` from a plain `Uint8Array` holding the same bytes.
    if (strict && Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
    const x = bytesOf(a);
    const y = bytesOf(b);
    for (let i = 0; i < x.length; i++) if (x[i] !== y[i]) return false;
    return true;
  }

  function mapsEqual(a, b, strict, seen) {
    if (a.size !== b.size) return false;
    const aKeys = Array.from(a.keys());
    const bKeys = Array.from(b.keys());
    const used = new Array(bKeys.length).fill(false);
    for (const ak of aKeys) {
      let found = false;
      for (let i = 0; i < bKeys.length; i++) {
        if (used[i]) continue;
        if (
          deepEqualInner(ak, bKeys[i], strict, seen) &&
          deepEqualInner(a.get(ak), b.get(bKeys[i]), strict, seen)
        ) {
          used[i] = true;
          found = true;
          break;
        }
      }
      if (!found) return false;
    }
    return true;
  }

  function setsEqual(a, b, strict, seen) {
    if (a.size !== b.size) return false;
    const aItems = Array.from(a);
    const bItems = Array.from(b);
    const used = new Array(bItems.length).fill(false);
    for (const av of aItems) {
      let found = false;
      for (let i = 0; i < bItems.length; i++) {
        if (used[i]) continue;
        if (deepEqualInner(av, bItems[i], strict, seen)) {
          used[i] = true;
          found = true;
          break;
        }
      }
      if (!found) return false;
    }
    return true;
  }

  function keysEqual(a, b, keysA, keysB, strict, seen) {
    if (keysA.length !== keysB.length) return false;
    const remaining = keysB.slice();
    for (const ka of keysA) {
      const idx = remaining.indexOf(ka);
      if (idx === -1) return false;
      const kb = remaining.splice(idx, 1)[0];
      if (!deepEqualInner(a[ka], b[kb], strict, seen)) return false;
    }
    return true;
  }

  function deepEqualInner(a, b, strict, seen) {
    if (strict) {
      if (Object.is(a, b)) return true;
    } else if (loosePrimitiveEqual(a, b)) {
      return true;
    } else if (!isObjectLike(a) || !isObjectLike(b)) {
      return false;
    }

    if (!isObjectLike(a) || !isObjectLike(b)) return false;

    for (const pair of seen) {
      if (pair[0] === a && pair[1] === b) return true;
    }
    seen.push([a, b]);

    if (tagOf(a) !== tagOf(b)) return false;

    switch (tagOf(a)) {
      case "Date":
        return a.getTime() === b.getTime();
      case "RegExp":
        return a.source === b.source && a.flags === b.flags;
      case "Number":
      case "String":
      case "Boolean":
      case "BigInt":
      case "Symbol":
        return deepEqualInner(a.valueOf(), b.valueOf(), strict, seen);
      case "Map":
        return mapsEqual(a, b, strict, seen);
      case "Set":
        return setsEqual(a, b, strict, seen);
      case "ArrayBuffer":
        return typedArraysEqual(new Uint8Array(a), new Uint8Array(b), strict);
      case "Error":
        if (a.message !== b.message || a.name !== b.name) return false;
        break;
      case "Promise":
      case "WeakMap":
      case "WeakSet":
        return false;
      default:
        break;
    }

    if (isTypedArray(a)) return typedArraysEqual(a, b, strict);

    // Prototype identity is a strict-mode rule only, which is why
    // `Object.create(null)` equals `{}` in loose mode and not in strict.
    if (strict && Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;

    if (Array.isArray(a)) {
      if (a.length !== b.length) return false;
      for (let i = 0; i < a.length; i++) {
        // A hole is not the same as an explicit `undefined`.
        if (hasOwn(a, i) !== hasOwn(b, i)) return false;
        if (!hasOwn(a, i)) continue;
        if (!deepEqualInner(a[i], b[i], strict, seen)) return false;
      }
      const extraA = ownEnumerableKeys(a, strict).filter(
        (k) => typeof k === "symbol" || !/^\d+$/.test(String(k))
      );
      const extraB = ownEnumerableKeys(b, strict).filter(
        (k) => typeof k === "symbol" || !/^\d+$/.test(String(k))
      );
      return keysEqual(a, b, extraA, extraB, strict, seen);
    }

    const keysA = ownEnumerableKeys(a, strict);
    const keysB = ownEnumerableKeys(b, strict);
    return keysEqual(a, b, keysA, keysB, strict, seen);
  }

  function deepEqual(a, b) {
    return deepEqualInner(a, b, false, []);
  }

  function deepStrictEqual(a, b) {
    return deepEqualInner(a, b, true, []);
  }

  // --- AssertionError -----------------------------------------------------

  class AssertionError extends Error {
    constructor(options) {
      const opts = options || {};
      const message =
        opts.message !== undefined
          ? String(opts.message)
          : "Failed";
      super(message);
      this.name = "AssertionError";
      this.code = "ERR_ASSERTION";
      if ("actual" in opts) this.actual = opts.actual;
      if ("expected" in opts) this.expected = opts.expected;
      this.operator = opts.operator || "fail";
      if (opts.generatedMessage === undefined) this.generatedMessage = false;
      else this.generatedMessage = opts.generatedMessage;
    }
  }
  Object.defineProperty(AssertionError.prototype, "name", {
    value: "AssertionError",
    writable: true,
    configurable: true,
  });

  function fail(message, actual, expected, operator) {
    if (message instanceof Error) throw message;
    throw new AssertionError({
      message: message === undefined ? "Failed" : message,
      actual,
      expected,
      operator: operator || "fail",
    });
  }

  function ok(value, message) {
    if (!value) {
      throw new AssertionError({
        message:
          message === undefined
            ? "The expression evaluated to a falsy value: " + String(value)
            : message,
        actual: value,
        expected: true,
        operator: "==",
        generatedMessage: message === undefined,
      });
    }
  }

  // --- equality wrappers --------------------------------------------------

  function equal(actual, expected, message) {
    // eslint-disable-next-line eqeqeq
    if (!loosePrimitiveEqual(actual, expected)) {
      throw new AssertionError({
        message: message === undefined ? "Expected values to be loosely equal" : message,
        actual,
        expected,
        operator: "==",
      });
    }
  }

  function notEqual(actual, expected, message) {
    // eslint-disable-next-line eqeqeq
    if (loosePrimitiveEqual(actual, expected)) {
      throw new AssertionError({
        message: message === undefined ? "Expected values to not be loosely equal" : message,
        actual,
        expected,
        operator: "!=",
      });
    }
  }

  function strictEqual(actual, expected, message) {
    if (!Object.is(actual, expected)) {
      throw new AssertionError({
        message: message === undefined ? "Expected values to be strictly equal" : message,
        actual,
        expected,
        operator: "strictEqual",
      });
    }
  }

  function notStrictEqual(actual, expected, message) {
    if (Object.is(actual, expected)) {
      throw new AssertionError({
        message: message === undefined ? "Expected values to not be strictly equal" : message,
        actual,
        expected,
        operator: "notStrictEqual",
      });
    }
  }

  function deepEqualAssert(actual, expected, message) {
    if (!deepEqual(actual, expected)) {
      throw new AssertionError({
        message: message === undefined ? "Expected values to be loosely deep-equal" : message,
        actual,
        expected,
        operator: "deepEqual",
      });
    }
  }

  function deepStrictEqualAssert(actual, expected, message) {
    if (!deepStrictEqual(actual, expected)) {
      throw new AssertionError({
        message: message === undefined ? "Expected values to be strictly deep-equal" : message,
        actual,
        expected,
        operator: "deepStrictEqual",
      });
    }
  }

  function notDeepEqualAssert(actual, expected, message) {
    if (deepEqual(actual, expected)) {
      throw new AssertionError({
        message: message === undefined ? "Expected values not to be loosely deep-equal" : message,
        actual,
        expected,
        operator: "notDeepEqual",
      });
    }
  }

  function notDeepStrictEqualAssert(actual, expected, message) {
    if (deepStrictEqual(actual, expected)) {
      throw new AssertionError({
        message: message === undefined ? "Expected values not to be strictly deep-equal" : message,
        actual,
        expected,
        operator: "notDeepStrictEqual",
      });
    }
  }

  // --- throws / rejects ---------------------------------------------------
  //
  // Five matcher shapes, each with different semantics (measured):
  //   undefined        → only "something was thrown"
  //   RegExp           → message must match
  //   function         → thrown error must be instanceof it
  //   error instance   → must be strictly the same object
  //   plain object     → SUBSET match on the error's own properties
  //   string message   → ERR_AMBIGUOUS_ARGUMENT in Node (error/message is ambiguous)

  function describeReceived(err) {
    return err && err.message ? err.message : String(err);
  }

  function matchError(thrown, expected) {
    if (expected === undefined || expected === null) return null; // any error ok
    if (expected instanceof RegExp) {
      if (!expected.test(String(thrown && thrown.message))) {
        return "The input did not match the regular expression " + expected;
      }
      return null;
    }
    if (typeof expected === "function") {
      if (!(thrown instanceof expected)) {
        return 'The error is expected to be an instance of "' + (expected.name || "") + '"';
      }
      return null;
    }
    if (expected instanceof Error) {
      if (thrown !== expected) return "The error is expected to be the same object";
      return null;
    }
    if (typeof expected === "string") {
      // Node rejects this form outright: it cannot tell error from message.
      const err = new TypeError(
        'The "error/message" argument is ambiguous. The error message "' +
          expected +
          '" is identical to the message.'
      );
      err.code = "ERR_AMBIGUOUS_ARGUMENT";
      throw err;
    }
    if (isObjectLike(expected)) {
      // Subset match: every own enumerable key of `expected` must deep-strict
      // equal the corresponding property of the thrown error. Extra properties
      // on the error are fine.
      for (const key of Object.keys(expected)) {
        if (!deepStrictEqual(thrown[key], expected[key])) {
          return (
            "Expected values to be strictly deep-equal: property " + key
          );
        }
      }
      return null;
    }
    return null;
  }

  function throws(fn, expected, message) {
    if (typeof fn !== "function") {
      throw new TypeError('The "fn" argument must be of type function');
    }
    let thrown;
    let didThrow = false;
    try {
      fn();
    } catch (e) {
      thrown = e;
      didThrow = true;
    }
    if (!didThrow) {
      throw new AssertionError({
        message: message === undefined ? "Missing expected exception." : message,
        operator: "throws",
      });
    }
    const problem = matchError(thrown, expected);
    if (problem) {
      throw new AssertionError({
        message: message === undefined ? problem : message,
        actual: thrown,
        expected,
        operator: "throws",
      });
    }
    return thrown;
  }

  function doesNotThrow(fn, expected, message) {
    if (typeof fn !== "function") {
      throw new TypeError('The "fn" argument must be of type function');
    }
    try {
      fn();
    } catch (e) {
      // A matcher narrows which errors are tolerated, same as `throws`.
      if (expected === undefined || matchError(e, expected) === null) {
        throw new AssertionError({
          message: message === undefined ? "Got unwanted exception: " + describeReceived(e) : message,
          actual: e,
          expected,
          operator: "doesNotThrow",
        });
      }
      throw e;
    }
  }

  function rejects(promiseOrFn, expected, message) {
    let promise;
    if (typeof promiseOrFn === "function") {
      try {
        promise = promiseOrFn();
      } catch (e) {
        return Promise.reject(e);
      }
    } else {
      promise = promiseOrFn;
    }
    if (!promise || typeof promise.then !== "function") {
      return Promise.reject(
        new TypeError('The "promiseFn" argument must be of type function or Promise')
      );
    }
    return Promise.resolve(promise).then(
      () => {
        throw new AssertionError({
          message: message === undefined ? "Missing expected rejection." : message,
          operator: "rejects",
        });
      },
      (err) => {
        const problem = matchError(err, expected);
        if (problem) {
          throw new AssertionError({
            message: message === undefined ? problem : message,
            actual: err,
            expected,
            operator: "rejects",
          });
        }
        return undefined;
      }
    );
  }

  function doesNotReject(promiseOrFn, expected, message) {
    let promise;
    if (typeof promiseOrFn === "function") {
      try {
        promise = promiseOrFn();
      } catch (e) {
        throw e;
      }
    } else {
      promise = promiseOrFn;
    }
    return Promise.resolve(promise).then(
      () => undefined,
      (err) => {
        if (expected === undefined || matchError(err, expected) === null) {
          throw new AssertionError({
            message:
              message === undefined ? "Got unwanted rejection: " + describeReceived(err) : message,
            actual: err,
            expected,
            operator: "doesNotReject",
          });
        }
        throw err;
      }
    );
  }

  // --- match / ifError / CallTracker --------------------------------------

  function match(value, regexp, message) {
    if (!(regexp instanceof RegExp)) {
      throw new TypeError('The "regexp" argument must be an instance of RegExp');
    }
    if (typeof value !== "string") {
      throw new TypeError('The "value" argument must be of type string');
    }
    if (!regexp.test(value)) {
      throw new AssertionError({
        message: message === undefined ? "The input did not match the regular expression " + regexp : message,
        actual: value,
        expected: regexp,
        operator: "match",
      });
    }
  }

  function doesNotMatch(value, regexp, message) {
    if (!(regexp instanceof RegExp)) {
      throw new TypeError('The "regexp" argument must be an instance of RegExp');
    }
    if (typeof value !== "string") {
      throw new TypeError('The "value" argument must be of type string');
    }
    if (regexp.test(value)) {
      throw new AssertionError({
        message: message === undefined ? "The input was expected to not match " + regexp : message,
        actual: value,
        expected: regexp,
        operator: "doesNotMatch",
      });
    }
  }

  function ifError(value) {
    if (value !== null && value !== undefined) {
      throw new AssertionError({
        message: "ifError got unwanted exception: " + describeReceived(value),
        actual: value,
        expected: null,
        operator: "ifError",
      });
    }
  }

  class CallTracker {
    #calls = new Map();

    calls(fn, exact) {
      const count = exact === undefined ? 1 : exact;
      const existing = this.#calls.get(fn);
      if (existing) {
        existing.count += count;
      } else {
        this.#calls.set(fn, { count, fn });
      }
      return fn;
    }

    report() {
      for (const [, entry] of this.#calls) {
        if (entry.count > 0) {
          throw new AssertionError({
            message: "Expected the function to be called " + entry.count + " more time(s)",
            operator: "CallTracker.report",
          });
        }
      }
    }

    verify() {
      this.report();
    }

    reset() {
      this.#calls.clear();
    }
  }

  // --- assemble -----------------------------------------------------------

  const assert = ok;
  assert.ok = ok;
  assert.fail = fail;
  assert.equal = equal;
  assert.notEqual = notEqual;
  assert.strictEqual = strictEqual;
  assert.notStrictEqual = notStrictEqual;
  assert.deepEqual = deepEqualAssert;
  assert.notDeepEqual = notDeepEqualAssert;
  assert.deepStrictEqual = deepStrictEqualAssert;
  assert.notDeepStrictEqual = notDeepStrictEqualAssert;
  assert.throws = throws;
  assert.doesNotThrow = doesNotThrow;
  assert.rejects = rejects;
  assert.doesNotReject = doesNotReject;
  assert.match = match;
  assert.doesNotMatch = doesNotMatch;
  assert.ifError = ifError;
  assert.AssertionError = AssertionError;
  assert.CallTracker = CallTracker;

  // `assert.strict` is the same surface with the strict variants as the
  // defaults, matching `node:assert/strict`.
  const strict = function strict(value, message) {
    ok(value, message);
  };
  for (const key of Object.keys(assert)) strict[key] = assert[key];
  strict.ok = ok;
  strict.equal = strictEqual;
  strict.notEqual = notStrictEqual;
  strict.deepEqual = deepStrictEqualAssert;
  strict.notDeepEqual = notDeepStrictEqualAssert;
  strict.strictEqual = strictEqual;
  strict.notStrictEqual = notStrictEqual;
  strict.deepStrictEqual = deepStrictEqualAssert;
  strict.notDeepStrictEqual = notDeepStrictEqualAssert;
  assert.strict = strict;

  return assert;
})()
