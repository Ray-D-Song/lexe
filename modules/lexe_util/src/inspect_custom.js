// `util.inspect.custom` for the five built-ins that define one in Node 24.21:
// Buffer, URL, URLSearchParams, TextDecoder, AbortSignal.
//
// Only those five have it — Error, Promise, Map, Set, Date, RegExp, the
// TypedArray family and plain objects all report `undefined` in Node, which was
// verified rather than assumed.
//
// Node's contract:
//   - signature is `(depth, options, inspect)` with the receiver (`this`) set to
//     the value being inspected; `fn.length` is 2 for all five (so they are
//     declared with two parameters here — a zero-param function is detectable
//     as `length === 0`),
//   - `depth` and `options` are ignored by every one of them,
//   - the return value is the complete rendering including the class prefix.
//
// Every format below is byte-for-byte from Node 24.21.

(function (symbol) {
  "use strict";

  const quote = (value) =>
    "'" + String(value).replace(/\\/g, "\\\\").replace(/'/g, "\\'") + "'";

  function urlToInspect(depth, options) {
    const url = this;
    void depth;
    const lines = [
      "href: " + quote(url.href),
      "origin: " + quote(url.origin),
      "protocol: " + quote(url.protocol),
      "username: " + quote(url.username),
      "password: " + quote(url.password),
      "host: " + quote(url.host),
      "hostname: " + quote(url.hostname),
      "port: " + quote(url.port),
      "pathname: " + quote(url.pathname),
      "search: " + quote(url.search),
      // Node inspects the nested URLSearchParams with the same options, so a
      // `depth: 0` call renders it as `[Object]`.
      "searchParams: " + inspect(url.searchParams, options),
      "hash: " + quote(url.hash),
    ];
    return "URL {\n  " + lines.join(",\n  ") + "\n}";
  }

  function urlSearchParamsToInspect(depth, options) {
    const params = this;
    void depth;
    void options;
    const parts = [];
    for (const [key, value] of params) {
      parts.push(quote(key) + " => " + quote(value));
    }
    // Node prints `URLSearchParams {}` with no inner space when empty.
    return parts.length
      ? "URLSearchParams { " + parts.join(", ") + " }"
      : "URLSearchParams {}";
  }

  function textDecoderToInspect(depth, options) {
    const decoder = this;
    void depth;
    void options;
    return (
      "TextDecoder { encoding: " +
      quote(decoder.encoding) +
      ", fatal: " +
      decoder.fatal +
      ", ignoreBOM: " +
      decoder.ignoreBOM +
      " }"
    );
  }

  function abortSignalToInspect(depth, options) {
    const signal = this;
    void depth;
    void options;
    return "AbortSignal { aborted: " + signal.aborted + " }";
  }

  // Buffer keeps the dedicated `<Buffer 00 01 ff>` form; exposing it here lets
  // libraries call it directly, as they can in Node.
  function bufferToInspect(depth, options) {
    const buffer = this;
    void depth;
    void options;
    return inspect(buffer);
  }

  let inspect = () => "";

  const install = (proto, fn) => {
    if (!proto) return;
    Object.defineProperty(proto, symbol, {
      value: fn,
      writable: true,
      configurable: true,
      enumerable: false,
    });
  };

  // Returns the installer so the host can hand it a real `inspect`.
  return function installAll(inspectFn) {
    inspect = inspectFn;
    install(Buffer.prototype, bufferToInspect);
    install(URL.prototype, urlToInspect);
    install(URLSearchParams.prototype, urlSearchParamsToInspect);
    install(TextDecoder.prototype, textDecoderToInspect);
    install(AbortSignal.prototype, abortSignalToInspect);
  };
})
