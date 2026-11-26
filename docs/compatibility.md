# Compatibility matrix

> [!NOTE]
> Below is a high level overview of partially supported APIs and modules. For more details consult the [API](../API.md) documentation

| [Node.js API](https://nodejs.org/api/index.html) | Node.js | lexe | Key differences from Node.js |
| --- | --- | --- | --- |
| node:assert | ✔︎ | ✔︎ | — |
| node:async_hooks | ✔︎ | ✔︎⚠️ | Public `createHook()` disabled by default (`LEXE_ASYNC_HOOKS=1`); missing `executionAsyncResource`, `asyncWrapProviders`, and `AsyncLocalStorage` Node 24 options ([details](#nodeasync_hooks)) |
| node:buffer | ✔︎ | ✔︎⚠️ | 31-bit `MAX_LENGTH` (`2147483647`); no slab pooling; no `Buffer.prototype.inspect` or `INSPECT_MAX_BYTES` truncation; no `Uint64` paths ([details](#nodebuffer)) |
| node:child_process | ✔︎ | — |
| node:cluster | ✔︎ | ✘ | Not implemented |
| node:console | ✔︎ | ✔︎ | — |
| node:constants | ✔︎ | ✔︎ | — |
| node:crypto | ✔︎ | ✔︎⚠️ | WebCrypto and partial classic ciphers, signatures, keys, X.509, ECDH, and HKDF; algorithm/backend limitations remain ([details](#nodecrypto)) |
| node:dgram | ✔︎ | ✔︎ | — |
| node:diagnostics_channel | ✔︎ | ✔︎ | — |
| node:dns | ✔︎ | ✔︎ | — |
| node:dns/promises | ✔︎ | ✔︎ | — |
| node:events | ✔︎ | ✔︎ | — |
| node:fs | ✔︎ | ✔︎ | — |
| node:fs/promises | ✔︎ | ✔︎ | — |
| node:http | ✔︎ | ✔︎ | — |
| node:http2 | ✔︎ | ✔︎ | Native HTTP/2 ([notes](#nodehttp2)) |
| node:https | ✔︎ | ✔︎⚠️ | Inherits `node:tls` limitations; requires TLS feature build ([details](#nodehttps)) |
| node:inspector | ✔︎ | ✔︎⚠️ | `url()` returns `undefined`; `Session` shim reports unavailable protocol operations ([details](#nodeinspector)) |
| node:inspector/promises | ✔︎ | ✘ | Not implemented |
| node:module | ✔︎ | ✔︎⚠️ | CJS facade and `SourceMap` without on-disk compile cache; missing `module.register`, `stripTypeScriptTypes`, and `Module._load` ([details](#nodemodule)) |
| node:net | ✔︎ | ✔︎ | — |
| node:os | ✔︎ | ✔︎ | — |
| node:path | ✔︎ | ✔︎ | — |
| node:perf_hooks | ✔︎ | ✔︎ | — |
| node:process | ✔︎ | ✔︎ | Runtime identity ([notes](#nodeprocess)) |
| node:querystring | ✔︎ | ✔︎ | — |
| node:readline | ✔︎ | ✔︎ | — |
| node:readline/promises | ✔︎ | ✔︎ | — |
| node:repl | ✔︎ | ✘ | Not implemented |
| node:sea | ✔︎ | ✔︎⚠️ | Asset APIs supported; Lexe uses its own compiled SEA container format |
| node:sqlite | ✔︎ | ✔︎ | — |
| node:stream | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:stream/promises | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:stream/web | ✔︎ | ✔︎ | — |
| node:string_decoder | ✔︎ | ✔︎ | — |
| node:test | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:timers | ✔︎ | ✔︎ | — |
| node:timers/promises | ✔︎ | ✔︎ | — |
| node:tls | ✔︎ | ✔︎⚠️ | Rejects `pfx`/`ciphers`/`crl`/`psk`/`ocsp`/sessions/async SNI/soft mTLS; TLS < 1.2 unsupported; no `renegotiate()` or `TLSSocket.write()` backpressure ([details](#nodetls)) |
| node:tty | ✔︎ | ✔︎ | — |
| node:url | ✔︎ | ✔︎ | — |
| node:util | ✔︎ | ✔︎⚠️ | Missing `parseArgs` and `transferableAbort*`; limited `TextDecoder` encodings ([details](#nodeutil-and-nodeutiltypes)) |
| node:util/types | ✔︎ | ✔︎ | — |
| node:v8 | ✔︎ | ✔︎⚠️ | QuickJS memory stats (`"quickjs"` heap space); `setFlagsFromString` no-op; missing Serializer/Deserializer, heap snapshots, and `GCProfiler` ([details](#nodev8)) |
| node:vm | ✔︎ | ✔︎ | Full API (`Script`, `createContext`/`isContext`, `runIn*Context`, `compileFunction`, `SourceTextModule`, `SyntheticModule`, `measureMemory`, `cachedData`, `timeout`, `codeGeneration`, `microtaskMode`, `importModuleDynamically`) |
| node:wasi | ✔︎ | ✘ | Not implemented |
| node:worker_threads | ✔︎ | ✔︎⚠️ | `moveMessagePortToContext` throws; missing `BroadcastChannel`, `SHARE_ENV`, `resourceLimits`, `getHeapSnapshot`, `postMessageToThread`, and piped worker stdio ([details](#nodeworker_threads)) |
| node:zlib | ✔︎ | ✔︎ | — |

| [lexe API](https://github.com/Ray-D-Song/lexe/blob/main/API.md) | Node.js | lexe |
| --- | --- | --- |
| lexe:hex | ✘ | ✔︎ |
| lexe:qjs | ✘ | ✔︎ |
| lexe:util | ✘ | ✔︎ |
| lexe:xml | ✘ | ✔︎ |

| [Web Platform API](https://min-common-api.proposal.wintertc.org/) | lexe | Key differences from standard / Node.js |
| --- | --- | --- |
| COMPRESSION | ✔︎ | — |
| CONSOLE | ✔︎ | — |
| DOM | ✔︎⚠️ | WinterTC `Event`/`EventTarget`/`AbortController`/`AbortSignal` subset only; no `ErrorEvent`, `CloseEvent`, or browser DOM ([details](#dom)) |
| ECMASCRIPT | ✔︎⚠️ | QuickJS-ng engine (31-bit `ArrayBuffer` limit, no `ShadowRealm`); V8 `Error.captureStackTrace`/`CallSite` fields are stub polyfills ([details](#ecmascript)) |
| ENCODING | ✔︎⚠️ | Supports `utf-8`, `utf-16le`, `utf-16be`, `windows-1252` (plus `hex`/`base64`) only; missing WHATWG legacy encodings ([details](#encoding)) |
| FETCH | ✔︎⚠️ | No `blob:`/`file:` URL fetch; omits browser policy controls (`credentials`, `referrer`, `integrity`, `caches`, CORS); WPT edge cases ([details](#fetch)) |
| FILEAPI | ✔︎⚠️ | `Blob`, `File`, and `URL.createObjectURL` only; missing `FileReader`/`FileReaderSync`/`FileList`; no `fetch("blob:...")` ([details](#fileapi)) |
| HR-TIME | ✔︎ | — |
| HTML | ✔︎⚠️ | `navigator` only exposes `userAgent`; `structuredClone` omits `DOMException`/`Blob`/`File`/`MessagePort`/`CryptoKey`; no `BroadcastChannel` ([details](#html)) |
| STREAMS | ✔︎ | — |
| URL | ✔︎ | — |
| URLPATTERN | ✔︎ | — |
| WASM-JS-API-2 | ✔︎⚠️ | `wasmi` interpreter; missing GC/exceptions/threads/Memory64/JSPI/`v128` boundary; copied `Memory.buffer`; retained `externref`s ([details](#wasm-js-api-2)) |
| WASM-WEB-API-2 | ✔︎⚠️ | `compileStreaming`/`instantiateStreaming` buffer the full `Response` before compiling (no incremental streaming compilation) ([details](#wasm-web-api-2)) |
| WEBCRYPTO | ✔︎⚠️ | Missing `X448`, `Ed448`, `Argon2`, `AES-OCB`, `ChaCha20-Poly1305` and some `deriveBits`/`deriveKey` WPT edge cases ([details](#webcrypto)) |
| WEBIDL | ✔︎⚠️ | Provides `DOMException` and core coercions; lacks full WebIDL prototype/branding/dictionary/overload compliance ([details](#webidl)) |
| XHR | ✔︎⚠️ | Implements `FormData` only; `XMLHttpRequest` and `ProgressEvent` are not implemented (Node.js also omits XHR) ([details](#xhr)) |

| Other features | lexe | Key differences from standard / Node.js |
| --- | --- | --- |
| async/await | ✔︎ | — |
| esm | ✔︎ | — |
| cjs | ✔︎ | — |
| Intl | ✔︎⚠️ | Partial date/time, number, collation, list, and segmentation support; missing other constructors and full ICU ([details](#intl)) |
| Temporal | ✔︎⚠️ | Missing `PlainYearMonth`, `PlainMonthDay`, non-ISO calendar systems, and full `Intl`-backed `toLocaleString()` ([details](#temporal)) |

### Database driver compatibility

| Package | Tested versions | Local acceptance | CI policy |
| --- | --- | --- | --- |
| `mysql2` | mysql2 3.23.2 / MySQL 8.4 | Node and Lexe pass the Promise, callback, transaction, error, and pool fixture | Blocking |
| `pg` (node-postgres) | pg 8.22.0 / PostgreSQL 16.14 | Node and Lexe pass all 25 SCRAM, query, pool, transaction, TLS, cancellation, recovery, notification, shutdown, and `.pgpass` cases | Node baseline and infrastructure are blocking; Lexe failures currently emit warnings |

See [`compat/README.md`](../compat/README.md) for exact fixture scope, commands, exclusions, and diagnostics.

**Legend**

- ⚠️ = partially supported in Lexe
- ⏱ = planned partial support
- \* = Not native

## Partial support details

### Node.js APIs

#### `node:async_hooks`

- User `createHook()` callbacks are disabled by default and only run when `LEXE_ASYNC_HOOKS=1` is set; by default only internal `AsyncLocalStorage` propagation is active.
- Missing `executionAsyncResource()` and `asyncWrapProviders`.
- `currentId()` is a deprecated stub that always returns `0`.
- `AsyncLocalStorage` does not support newer Node options such as `defaultValue` or `name` in its constructor.

#### `node:buffer`

- `constants.MAX_LENGTH` / `kMaxLength` is capped at `2147483647` (QuickJS 31-bit `ArrayBuffer` limit) rather than Node's platform limit.
- `Buffer.poolSize` is exposed as a numeric property only; Lexe does not use slab-pooled backing allocations.
- `Buffer.prototype.inspect` is not implemented, and `INSPECT_MAX_BYTES` does not limit `util.inspect()` output.
- `Uint64` typed-array conversion paths throw `"Uint64 is not supported"`.

#### `node:crypto`

- The RustCrypto build supports streaming AES ciphers, HKDF, timing-safe comparison, KeyObject import/export, top-level signatures, ECDH, and X.509 certificates. Missing classic APIs include `createSign`/`createVerify`, `generateKeyPair`/`generateKeyPairSync`, `generateKey`/`generateKeySync`, `scrypt`/`scryptSync`, `pbkdf2`/`pbkdf2Sync`, `privateDecrypt`/`privateEncrypt`/`publicDecrypt`, `DiffieHellman`/`createDiffieHellman`, and `crypto.hash()`.
- `publicEncrypt` / RSA-OAEP rejects `MD5`, and algorithm availability depends on the compiled TLS/crypto backend (for example, `graviola` lacks most operations).

#### `node:https`

- `https.Server` wraps `tls.Server` and delegates HTTP lifecycle handling to `node:http`, so it inherits all `node:tls` limitations (missing `pfx`/custom `ciphers`/`crl`/PSK/OCSP/session reuse/async SNI/soft mTLS, TLS < 1.2 unsupported, no `renegotiate()`).
- Requires building with a TLS feature (`tls-ring`, `tls-aws-lc`, `tls-graviola`, or `tls-openssl`); non-TLS builds stub out `createServer`/`get`/`request` and leave `Agent`/`Server`/`globalAgent` as `undefined`.

#### `node:inspector`

- `url()` always returns `undefined`. A `Session` shim supports startup probes but reports `ERR_INSPECTOR_NOT_AVAILABLE` for protocol requests.
- No Inspector debugging protocol; `open()`, `close()`, `waitForDebugger()`, and `console` are not implemented.

#### `node:module`

- Provides a CommonJS loader facade:
  - `Module`, `createRequire`, `require.resolve`, `require.cache`, `require.extensions`
  - writable `_resolveFilename` / `_nodeModulePaths`
  - `Module._compile` for require-hook compatibility
- Also provides `SourceMap`, `findSourceMap`, `getSourceMapsSupport` / `setSourceMapsSupport`, `findPackageJSON`, `syncBuiltinESMExports`, `registerHooks`, `builtinModules`, `isBuiltin`, and `constants`.
- Compile cache:
  - `enableCompileCache` validates its argument and reports `constants.compileCacheStatus.FAILED` (there is no on-disk compile cache).
  - `flushCompileCache` is a no-op.
  - `getCompileCacheDir` returns `undefined`.
- Source maps: `findSourceMap` does not rewrite `Error.prepareStackTrace`; while support is enabled it reads a file's source map on demand.
- Not implemented: `module.register`, `stripTypeScriptTypes`, and `Module._load`.
- Extension hooks apply to the CommonJS loader only; they do not change static ESM import semantics.
- Native `.node` addon loading requires building with `--features napi` or `--features v8-compat` (see [Native addons](#native-addons)).

#### `node:tls`

- Rejects (`ERR_TLS_OPTION_NOT_SUPPORTED`) `pfx`, `ciphers`, `sigalgs`, `dhparam`, `crl`, `psk`, `ocsp`, `session`, `ticket`, async `SNICallback`, `ALPNCallback`, soft mTLS (`requestCert: true` with `rejectUnauthorized: false`), and encrypted private keys on rustls backends (OpenSSL only).
- TLS versions below 1.2 are not supported.
- `TLSSocket.renegotiate()` fails with `"renegotiation not supported"`.
- `TLSSocket.write()` always returns `true` without backpressure or `"drain"` signaling.

#### `node:util` and `node:util/types`

- `node:util` missing exports: `parseArgs()`, `transferableAbortController()`, and `transferableAbortSignal()`.
- `util.inspect()` and `util.formatWithOptions()` support `colors`, `depth`, `showHidden`, `maxArrayLength`, `maxStringLength`, `compact`, `sorted`, and `customInspect` (while `showProxy`, `getters`, `numericSeparator`, and line-wrapping `breakLength` are not implemented).
- `util.styleText()` does not support hex/RGB truecolor formats.
- `util.TextDecoder` supports only `utf-8`, `utf-16le`, `utf-16be`, and `windows-1252` (plus `hex`/`base64`), lacking full WHATWG legacy encodings.
- `node:util/types` matches Node 24 exports, including native checks for `isExternal()`, `isKeyObject()`, and `isCryptoKey()` when those subsystems are enabled.

#### `node:v8`

- Backed by QuickJS `JS_ComputeMemoryUsage` rather than V8: `getHeapSpaceStatistics()` always returns a single `"quickjs"` space, and `heap_size_limit` falls back to `Number.MAX_SAFE_INTEGER` when `malloc_limit === 0`.
- `setFlagsFromString()` is a no-op.
- Missing serialization (`serialize`, `deserialize`, `Serializer`, `Deserializer`, `DefaultSerializer`, `DefaultDeserializer`), heap snapshots (`writeHeapSnapshot`, `getHeapSnapshot`), `GCProfiler`, `queryObjects`, `startupSnapshot`, `cachedDataVersionTag`, and coverage APIs (`takeCoverage`, `stopCoverage`).

#### `node:worker_threads`

- Uses **OS-thread worker isolates** (QuickJS per thread) with structured-clone `postMessage`, in-process `MessageChannel`, environment data sync, and **SharedArrayBuffer** hooks for shared memory within the process.
- `moveMessagePortToContext()` throws (not implemented).
- Missing `BroadcastChannel`, `SHARE_ENV`, `resourceLimits`, `Worker.prototype.getHeapSnapshot()`, `postMessageToThread()`, and piped worker stdio (`stdin: true`, `stdout: true`, `stderr: true`).

### Web Platform APIs

#### `DOM`

- Implements the non-browser WinterTC subset only (`Event`, `CustomEvent`, `EventTarget`, `AbortController`, and `AbortSignal` with `abort()`, `any()`, `timeout()`, `throwIfAborted()`).
- Missing `ErrorEvent`, `CloseEvent`, and browser DOM tree APIs (`Document`, `Node`, `Element`).

#### `ECMASCRIPT`

- Powered by QuickJS-ng rather than V8: `ArrayBuffer` maximum length is `2147483647` (31-bit limit).
- `ShadowRealm` is not implemented.
- V8-specific `Error.captureStackTrace` and `CallSite` methods (`isEval`, `isNative`, `isConstructor`, `isToplevel`, `getEvalOrigin`, `getMethodName`, `getTypeName`, `getThis`) are stub polyfills.

#### `ENCODING`

- `TextDecoder` / `TextDecoderStream` support only `utf-8`, `utf-16le`, `utf-16be`, and `windows-1252` (plus non-standard `hex`/`base64`).
- Missing WHATWG legacy encodings (`iso-2022-jp`, `shift_jis`, `euc-jp`, `gbk`, `gb18030`, `big5`, `euc-kr`, `iso-8859-2`..`16`, `windows-1250`..`1258` except `1252`, and replacement-encoding behavior).

#### `FETCH`

- `blob:` and `file:` schemes throw `TypeError("Unsupported scheme")`.
- Browser-centric options and policies (`credentials`, `referrer`, `referrerPolicy`, `integrity`, `priority`, HTTP `cache` / `caches` API, CORS preflight, forbidden-header enforcement) are omitted or non-enforcing.
- Some WPT edge cases (`response-clone` under stress, HTTP/2 request-upload streaming) do not yet pass.

#### `FILEAPI`

- Supports `Blob`, `File`, and `URL.createObjectURL()` / `URL.revokeObjectURL()`.
- Missing `FileReader`, `FileReaderSync`, and `FileList`.
- `fetch("blob:...")` is unsupported, and minor WPT edge cases remain in `Blob` construction/slicing and multipart `FormData` file uploads.

#### `HTML`

- `navigator` exposes only `userAgent` (missing `hardwareConcurrency`, `language`/`languages`, `platform`).
- `structuredClone()` does not clone `DOMException`, `Blob`, `File`, `MessagePort`, or `CryptoKey`.
- `BroadcastChannel` is not implemented (and there are no browser `Window` or DOM globals).

#### `WASM-JS-API-2`

- Backed by the `wasmi` interpreter rather than V8.
- Missing `WebAssembly.Function`, `Tag`, `Exception`, JSPI (`Suspending`/`promising`), Wasm GC, exception handling, typed function references, shared memory/threads, and `Memory64`.
- `v128` SIMD values cannot cross the JS/Wasm boundary (`TypeError`).
- `Memory.buffer` uses a synchronously copied mirror `ArrayBuffer` instead of zero-copy shared memory, and `externref` entries are retained until realm teardown.

#### `WASM-WEB-API-2`

- `compileStreaming()` and `instantiateStreaming()` validate `Response.ok` and `Content-Type: application/wasm`, then buffer the entire response via `response.arrayBuffer()` before compiling (no incremental streaming compilation).

#### `WEBCRYPTO`

- Supports `AES-CBC`/`CTR`/`GCM`/`KW`, `HMAC`, `RSASSA-PKCS1-v1_5`, `RSA-PSS`, `RSA-OAEP`, `ECDSA`, `ECDH`, `Ed25519`, `X25519`, `HKDF`, `PBKDF2`, and `SHA-*`.
- Missing newer/extended algorithms (`X448`, `Ed448`, `Argon2`, `AES-OCB`, `ChaCha20-Poly1305`) and some `deriveBits`/`deriveKey` WPT edge cases; availability varies on non-default crypto backends (e.g. `graviola`).

#### `WEBIDL`

- Exposes `DOMException` and basic JS/Rust type conversions, but lacks full WebIDL prototype, branding, dictionary, and overload compliance (`idlharness` WPT tests are skipped).

#### `XHR`

- Implements `FormData` only; `XMLHttpRequest` and `ProgressEvent` are not implemented (Node.js also omits `XMLHttpRequest`, while WinterTC groups `FormData` under XHR).

### Other features

#### `Intl`

- Implements partial `Intl.DateTimeFormat`, `Intl.Collator`, `Intl.NumberFormat`, `Intl.ListFormat`, and `Intl.Segmenter`, plus `Intl.supportedValuesOf()` and `Date.prototype.toLocaleString` with `timeZone`.
- Missing `Intl.PluralRules`, `Intl.RelativeTimeFormat`, `Intl.DisplayNames`, `Intl.Locale`, `Intl.DurationFormat`, and full ICU locale/calendar/numbering-system support.

#### `Temporal`

- Implements `Duration`, `Instant`, `PlainDate`, `PlainDateTime`, `PlainTime`, `ZonedDateTime`, and `Now` (backed by `jiff`).
- Missing `PlainYearMonth`, `PlainMonthDay`, non-ISO calendar systems (ISO-8601 only), and full `Intl`-backed `toLocaleString()` formatting.

## Other notes

### `node:http2`

- Implemented natively: `createServer`, `createSecureServer`, `connect`, settings helpers, and constants.
- Browser builds may still use the embedded load-time stub when the native module is unavailable.

### `node:process`

- `process.version` (`v24.3.0`) and `process.versions.node` (`24.3.0`) advertise Node 24.3 compatibility while `process.release.name` is `"lexe"` and V8/libuv/OpenSSL-specific keys (`v8`, `uv`, `openssl`) are replaced by Lexe runtime keys (`lexe`, `node`, `sqlite`).

### Native addons

- **N-API** native addon (`.node`) loading is supported when Lexe is built with `--features napi` on a dynamically linked target.
- Linux release binaries come in two flavours. The `-gnu` builds are dynamically linked against glibc (baseline 2.28) and support native addons. The `-musl` builds are fully static and do not load `.node` files, because `dlopen` is unavailable in a static musl binary. npm selects the right one automatically. Packages that ship native addons such as `node-pty` need the `-gnu` build.
- **V8 C++ ABI** addons targeting Node 24 (e.g. `better-sqlite3`) are supported with `--features v8-compat` through a limited QuickJS-backed V8 shim.
