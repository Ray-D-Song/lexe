// Primordials shim — Node uses primordials for integrity; we map to standard builtins.
export function ArrayFrom(...args) { return Array.from(...args); }
export function ArrayPrototypeFilter(a, ...args) { return a.filter(...args); }
export function ArrayPrototypeJoin(a, ...args) { return a.join(...args); }
export function ArrayPrototypeMap(a, ...args) { return a.map(...args); }
export function ArrayPrototypePop(a) { return a.pop(); }
export function ArrayPrototypePush(a, ...items) { return a.push(...items); }
export function ArrayPrototypeReverse(a) { return a.reverse(); }
export function ArrayPrototypeShift(a) { return a.shift(); }
export function ArrayPrototypeUnshift(a, ...items) { return a.unshift(...items); }
export function ArrayPrototypeIndexOf(a, ...args) { return a.indexOf(...args); }
export function ArrayPrototypeSplice(a, ...args) { return a.splice(...args); }
export function ArrayPrototypeToSorted(a, ...args) { return a.toSorted ? a.toSorted(...args) : [...a].sort(...args); }
export function ArrayPrototypeSome(a, ...args) { return a.some(...args); }
export const Boolean = globalThis.Boolean;
export function DateNow() { return Date.now(); }
export function FunctionPrototypeBind(fn, thisArg, ...args) { return fn.bind(thisArg, ...args); }
export function FunctionPrototypeCall(fn, thisArg, ...args) { return fn.call(thisArg, ...args); }
export function MathCeil(n) { return Math.ceil(n); }
export function MathFloor(n) { return Math.floor(n); }
export function MathMax(...args) { return Math.max(...args); }
export function MathMaxApply(arr) { return Math.max.apply(null, arr); }
export function NumberIsFinite(n) { return Number.isFinite(n); }
export function NumberIsInteger(n) { return Number.isInteger(n); }
export function NumberIsNaN(n) { return Number.isNaN(n); }
export function ObjectDefineProperty(o, k, d) { return Object.defineProperty(o, k, d); }
export function ObjectDefineProperties(o, d) { return Object.defineProperties(o, d); }
export function ObjectEntries(o) { return Object.entries(o); }
export function ObjectSetPrototypeOf(o, p) { return Object.setPrototypeOf(o, p); }
export function ObjectPrototypeHasOwnProperty(o, k) { return Object.prototype.hasOwnProperty.call(o, k); }
export function RegExpPrototypeExec(re, s) { return re.exec(s); }
export function RegExpPrototypeSymbolSplit(re, s, limit) { return s.split(re, limit); }
/** Iterable of Unicode code points (matches Node SafeStringIterator). */
export function SafeStringIterator(s: string) {
  // Support both `new SafeStringIterator(s)` and `SafeStringIterator(s)`.
  if (!(this instanceof (SafeStringIterator as any))) {
    return new (SafeStringIterator as any)(s);
  }
  this._s = String(s);
  this._i = 0;
  return this;
}
// Use globalThis.Symbol to avoid TDZ if bundler reorders our Symbol export.
(SafeStringIterator as any).prototype[globalThis.Symbol.iterator] = function () {
  return this;
};
(SafeStringIterator as any).prototype.next = function () {
  if (this._i >= this._s.length) {
    return { value: undefined, done: true };
  }
  const cp = this._s.codePointAt(this._i)!;
  const ch = String.fromCodePoint(cp);
  this._i += ch.length;
  return { value: ch, done: false };
};
export function SafeMap(entries) { return new Map(entries); }
export function StringFromCharCode(...args) { return String.fromCharCode(...args); }
export function StringPrototypeCharCodeAt(s, i) { return s.charCodeAt(i); }
export function StringPrototypeCodePointAt(s, i) { return s.codePointAt(i); }
export function StringPrototypeEndsWith(s, ...args) { return s.endsWith(...args); }
export function StringPrototypeIncludes(s, ...args) { return s.includes(...args); }
export function StringPrototypeRepeat(s, n) { return s.repeat(n); }
export function StringPrototypeReplaceAll(s, ...args) { return s.replaceAll(...args); }
export function StringPrototypeSlice(s, ...args) { return s.slice(...args); }
export function StringPrototypeSplit(s, ...args) { return s.split(...args); }
export function StringPrototypeStartsWith(s, ...args) { return s.startsWith(...args); }
export function StringPrototypeToLowerCase(s) { return s.toLowerCase(); }
export function StringPrototypeTrim(s) { return s.trim(); }
export function StringPrototypeNormalize(s, form) { return s.normalize(form); }
export const Symbol = globalThis.Symbol;
export const SymbolAsyncIterator = Symbol.asyncIterator;
export const SymbolDispose = Symbol.dispose || Symbol.for('nodejs.dispose');
export const Promise = globalThis.Promise;
export function PromiseReject(r) { return Promise.reject(r); }
