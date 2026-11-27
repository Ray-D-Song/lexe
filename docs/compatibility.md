# Compatibility matrix

> [!NOTE]
> Below is a high level overview of partially supported APIs and modules. For more details consult the [API](../API.md) documentation

| [Node.js API](https://nodejs.org/api/index.html) | Node.js | lexe | Key differences from Node.js |
| --- | --- | --- | --- |
| node:assert | ✔︎ | ✔︎ | — |
| node:async_hooks | ✔︎ | ✔︎ | `AsyncLocalStorage`, `AsyncResource`, and on-demand `createHook()` lifecycle tracking ([notes](#nodeasync_hooks)) |
| node:buffer | ✔︎ | ✔︎ | 31-bit `MAX_LENGTH` (`2147483647`) due to QuickJS-ng `ArrayBuffer` limit ([notes](#nodebuffer)) |
| node:child_process | ✔︎ | ✔︎⚠️ | Missing internal `_forkChild` |
| node:cluster | ✔︎ | ✘ | Not implemented |
| node:console | ✔︎ | ✔︎ | — |
| node:constants | ✔︎ | ✔︎ | — |
| node:crypto | ✔︎ | ✔︎⚠️ | WebCrypto + classic APIs (`hash`, `Hash`/`Hmac`, `createSign`/`createVerify`, `pbkdf2*`, `scrypt*`, `generateKey*`/`generateKeyPair*`, `DiffieHellman*`, `checkPrime*`/`generatePrime*`, RSA encrypt/decrypt, `chacha20-poly1305`, `getCipherInfo`, Ed448/X448); omits OpenSSL engine/FIPS/SPKAC `Certificate` ([details](#nodecrypto)) |
| node:dgram | ✔︎ | ✔︎⚠️ | Missing internal `_createSocketHandle` |
| node:diagnostics_channel | ✔︎ | ✔︎ | — |
| node:dns | ✔︎ | ✔︎ | — |
| node:dns/promises | ✔︎ | ✔︎ | — |
| node:events | ✔︎ | ✔︎ | — |
| node:fs | ✔︎ | ✔︎ | `lchmod`/`lchmodSync` return `ENOTSUP` on Linux |
| node:fs/promises | ✔︎ | ✔︎ | — |
| node:http | ✔︎ | ✔︎ | — |
| node:http2 | ✔︎ | ✔︎⚠️ | Native HTTP/2 surface; `performServerHandshake`, server push, and streaming server request bodies are unimplemented ([notes](#nodehttp2)) |
| node:https | ✔︎ | — | Requires TLS feature build; follows `node:tls` security policy ([notes](#nodehttps)) |
| node:inspector | ✔︎ | ✔︎⚠️ | `url()` returns `undefined`; `Session` shim reports unavailable protocol operations ([details](#nodeinspector)) |
| node:inspector/promises | ✔︎ | ✘ | Not implemented |
| node:module | ✔︎ | ✔︎ | TypeScript stripping, worker `register`, on-disk bytecode compile cache, `Module._load`, and CJS/ESM loader ([notes](#nodemodule)) |
| node:net | ✔︎ | ✔︎⚠️ | Missing internal `_createServerHandle` |
| node:os | ✔︎ | ✔︎ | — |
| node:path | ✔︎ | ✔︎ | — |
| node:perf_hooks | ✔︎ | ✔︎ | — |
| node:process | ✔︎ | ✔︎⚠️ | Runtime identity; missing `execve`, deprecated `domain`, and internal C++/V8 hooks (`binding`, `_linkedBinding`, `moduleLoadList`, `_debugProcess`, etc.) ([notes](#nodeprocess)) |
| node:querystring | ✔︎ | ✔︎ | — |
| node:readline | ✔︎ | ✔︎ | — |
| node:readline/promises | ✔︎ | ✔︎ | — |
| node:repl | ✔︎ | ✘ | Not implemented |
| node:sea | ✔︎ | ✔︎ | Asset APIs supported; Lexe uses its own compiled SEA container format ([notes](#nodesea)) |
| node:sqlite | ✔︎ | ✔︎ | — |
| node:stream | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:stream/promises | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:stream/web | ✔︎ | ✔︎ | — |
| node:string_decoder | ✔︎ | ✔︎ | — |
| node:test | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:timers | ✔︎ | ✔︎ | — |
| node:timers/promises | ✔︎ | ✔︎ | — |
| node:tls | ✔︎ | ✔︎ | Supports `pfx`/encrypted PEMs, async `SNICallback`, `ALPNCallback`, soft mTLS (`requestCert: true, rejectUnauthorized: false`), TLS session resumption, `rootCertificates`/`getCACertificates()`, and write backpressure; intentionally disables TLS < 1.2, weak `ciphers`/`sigalgs`/`dhparam`/`crl`/`psk`/`ocsp`, and `renegotiate()` ([notes](#nodetls)) |
| node:tty | ✔︎ | ✔︎ | — |
| node:url | ✔︎ | ✔︎ | — |
| node:util | ✔︎ | ✔︎ | — |
| node:util/types | ✔︎ | ✔︎ | — |
| node:v8 | ✔︎ | ✔︎⚠️ | QuickJS memory stats (`"quickjs"` heap space); `setFlagsFromString` no-op; structured-clone-backed serialization; missing V8 heap snapshots and `GCProfiler` ([details](#nodev8)) |
| node:vm | ✔︎ | ✔︎ | Full API (`Script`, `createContext`/`isContext`, `runIn*Context`, `compileFunction`, `SourceTextModule`, `SyntheticModule`, `measureMemory`, `cachedData`, `timeout`, `codeGeneration`, `microtaskMode`, `importModuleDynamically`) |
| node:wasi | ✔︎ | ✔︎⚠️ | WASI Preview 1 (`WASI({ version: 'preview1', args, env, preopens, returnOnExit, stdin, stdout, stderr })`, `wasiImport`, `getImportObject()`, `start()`, `initialize()`, JS overrides); backed by `wasmz` (no WASI Preview 2) ([details](#nodewasi)) |
| node:worker_threads | ✔︎ | ✔︎ | OS-thread worker isolates, `SHARE_ENV`, piped stdio, `resourceLimits`, and `SharedArrayBuffer`; omits V8-only `Worker.prototype.getHeapSnapshot()` ([notes](#nodeworker_threads)) |
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
| DOM | ✔︎ | — |
| ECMASCRIPT | ✔︎ | QuickJS-ng engine (31-bit `ArrayBuffer` limit, no `ShadowRealm`, stub `Error.captureStackTrace`/`CallSite` fields) ([notes](#ecmascript)) |
| ENCODING | ✔︎ | — |
| FETCH | ✔︎ | Supports HTTP/1.1 & HTTP/2 streaming, `file:` / `blob:` / `data:` URLs, SRI `integrity`, and in-memory `caches` (`CacheStorage`/`Cache`); intentionally omits browser-origin policies (CORS preflight, `credentials`, `referrer`, `forbidden-header`) ([notes](#fetch)) |
| FILEAPI | ✔︎ | — |
| HR-TIME | ✔︎ | — |
| HTML | ✔︎ | — |
| STREAMS | ✔︎ | — |
| URL | ✔︎ | — |
| URLPATTERN | ✔︎ | — |
| WASM-JS-API-2 | ✔︎⚠️ | `wasmz` engine (GC, exception handling, typed function references, shared memory, Memory64, multi-value, bulk memory, reference types, SIMD); missing JS constructors `WebAssembly.Function`/`Tag`/`Exception`, JSPI (`Suspending`/`promising`), and `v128` JS boundary; synchronously mirrored `Memory.buffer`; retained `externref`s ([details](#wasm-js-api-2)) |
| WASM-WEB-API-2 | ✔︎⚠️ | `compileStreaming`/`instantiateStreaming` buffer the full `Response` before compiling (no incremental streaming compilation) ([details](#wasm-web-api-2)) |
| WEBCRYPTO | ✔︎ | Supports standard & modern WebCrypto (`Ed448`, `X448`, `Argon2`, `ChaCha20-Poly1305`); omits `AES-OCB` and backend parity on `graviola` ([notes](#webcrypto)) |
| WEBIDL | ✔︎ | Provides `DOMException` and core coercions; skips full WebIDL `idlharness` prototype/branding/overload conformance ([notes](#webidl)) |
| XHR | ✔︎ | Implements `FormData` and `ProgressEvent`; omits `XMLHttpRequest` matching Node.js ([notes](#xhr)) |

| Other features | lexe | Key differences from standard / Node.js |
| --- | --- | --- |
| async/await | ✔︎ | — |
| esm | ✔︎ | — |
| cjs | ✔︎ | — |
| Intl | ✔︎⚠️ | Implements `DateTimeFormat`, `Collator`, `NumberFormat`, `ListFormat`, `Segmenter`, `Locale`, `PluralRules`, `RelativeTimeFormat`, `DisplayNames`, `DurationFormat`, and `supportedValuesOf`; compact built-in CLDR subset without full ICU tables ([details](#intl)) |
| Temporal | ✔︎⚠️ | Implements `Duration`, `Instant`, `PlainDate`, `PlainDateTime`, `PlainTime`, `PlainYearMonth`, `PlainMonthDay`, `ZonedDateTime`, and `Now`; ISO-8601 calendar only, without non-ISO calendars or full ICU `toLocaleString()` ([details](#temporal)) |

### Database driver compatibility

| Package | Tested versions | Local acceptance | CI policy |
| --- | --- | --- | --- |
| `mysql2` | mysql2 3.23.2 / MySQL 8.4 | Node and Lexe pass the Promise, callback, transaction, error, and pool fixture | Blocking |
| `pg` (node-postgres) | pg 8.22.0 / PostgreSQL 16.14 | Node and Lexe pass all 25 SCRAM, query, pool, transaction, TLS, cancellation, recovery, notification, shutdown, and `.pgpass` cases | Node baseline and infrastructure are blocking; Lexe failures currently emit warnings |

See [`compat/README.md`](../compat/README.md) for exact fixture scope, commands, exclusions, and diagnostics.

**Legend**

- ⚠️ = partially supported in Lexe (internal-only exports, accepted engine/security constraints, or targeted feature subsets; see details/notes below)
- ⏱ = planned partial support
- \* = Not native

## Partial support details

### Node.js APIs

#### `node:crypto`

- Supports WebCrypto and classic Node crypto APIs: streaming `Hash`/`Hmac`, one-shot `crypto.hash()`, `createSign`/`createVerify`, `Sign`/`Verify`, `pbkdf2`/`pbkdf2Sync`, `scrypt`/`scryptSync`, `generateKey`/`generateKeySync`, `generateKeyPair`/`generateKeyPairSync`, `DiffieHellman`/`DiffieHellmanGroup`/`createDiffieHellman*`/`getDiffieHellman`, `checkPrime*`/`generatePrime*`, `privateEncrypt`/`publicDecrypt`/`publicEncrypt`/`privateDecrypt`, `chacha20-poly1305`, `getCipherInfo`, `Ed448`/`X448`, encrypted PKCS#8 PEM import/export, and `prng`/`pseudoRandomBytes`/`rng`/`fips`/`getFips`/`DH_*`/`OPENSSL_VERSION_NUMBER`.
- Intentionally rejects `RSA_PKCS1_PADDING` on `privateDecrypt` (CVE-2023-46809 mitigation matching Node 24) and `MD5` in RSA-OAEP; omits OpenSSL engine/SPKAC/FIPS mutability APIs (`Certificate`, `setFips`, `setEngine`, `secureHeapUsed`, `ENGINE_*`/`SSL_OP_*`/`RSA_X931_PADDING` constants); algorithm availability depends on the compiled backend (default RustCrypto vs `graviola`).

#### `node:inspector`

- `url()` always returns `undefined`. A `Session` shim supports startup probes but reports `ERR_INSPECTOR_NOT_AVAILABLE` for protocol requests.
- No CDP/WebSocket debugging protocol server; `open()`, `close()`, `waitForDebugger()`, `Network`, and `console` are omitted by design.

#### `node:v8`

- Backed by QuickJS `JS_ComputeMemoryUsage` rather than V8: `getHeapSpaceStatistics()` always returns a single `"quickjs"` space, and `heap_size_limit` falls back to `Number.MAX_SAFE_INTEGER` when `malloc_limit === 0`.
- `setFlagsFromString()` is a no-op; `cachedDataVersionTag()`, `serialize()`, `deserialize()`, `Serializer`, `Deserializer`, `DefaultSerializer`, and `DefaultDeserializer` are implemented (backed by structured clone wire format rather than V8 binary snapshots).
- Missing V8-specific heap snapshots (`writeHeapSnapshot`, `getHeapSnapshot`), `GCProfiler`, `queryObjects`, `startupSnapshot`, and coverage APIs (`takeCoverage`, `stopCoverage`).

#### `node:wasi`

- Implements WASI Preview 1 (`version: "preview1"`) backed by `wasmz`: `new WASI({ version, args, env, preopens, returnOnExit, stdin, stdout, stderr })`, `wasi.wasiImport` (46 Preview 1 functions, overridable with custom JS functions), `wasi.getImportObject()`, `wasi.start(instance)` (returns exit code when `returnOnExit: true`), and `wasi.initialize(instance)`.
- WASI Preview 2 (`preview2` / Component Model) is not supported.

### Web Platform APIs

#### `WASM-JS-API-2`

- Backed by `wasmz` (`wasmz` bytecode VM) rather than V8.
- Supports GC types (`struct`/`array`/`i31`), exception handling, typed function references, shared memory (`new WebAssembly.Memory({ initial, maximum, shared: true })`), threads/atomics, `Memory64`, multi-value, bulk memory, reference types (`externref`/`funcref`), and internal `v128` SIMD.
- Missing JS-level constructors/helpers (`WebAssembly.Function`, `Tag`, `Exception`, JSPI `Suspending`/`promising`) and crossing `v128` SIMD values at the JS/Wasm boundary (`TypeError`).
- `Memory.buffer` uses a synchronously copied mirror `ArrayBuffer` instead of zero-copy shared memory, and `externref` entries are retained until realm teardown.

#### `WASM-WEB-API-2`

- `compileStreaming()` and `instantiateStreaming()` validate `Response.ok` and `Content-Type: application/wasm`, then buffer the entire response via `response.arrayBuffer()` before compiling (no incremental streaming compilation).

### Other features

#### `Intl`

- Implements `Intl.DateTimeFormat`, `Intl.Collator`, `Intl.NumberFormat`, `Intl.ListFormat`, `Intl.Segmenter`, `Intl.Locale`, `Intl.PluralRules`, `Intl.RelativeTimeFormat`, `Intl.DisplayNames`, `Intl.DurationFormat`, and `Intl.supportedValuesOf()` (`timeZone`, `calendar`, `collation`, `currency`, `numberingSystem`, `unit`), plus `Date.prototype.toLocaleString` with `timeZone`.
- Uses a compact pure-Rust / built-in CLDR rule and label subset (no full ICU4X data tables; unknown/unlisted codes or locales fall back to `en-US` / code fallback).

#### `Temporal`

- Implements `Duration`, `Instant`, `PlainDate`, `PlainDateTime`, `PlainTime`, `PlainYearMonth`, `PlainMonthDay`, `ZonedDateTime`, and `Now` (backed by `jiff`), including `PlainDate.prototype.toPlainYearMonth()` / `toPlainMonthDay()`.
- Supports the `iso8601` calendar only (no non-ISO calendar systems or full ICU-backed `toLocaleString()`).

## Pending Decisions

All previously tracked architectural/product decisions have been resolved: QuickJS engine limits (`ArrayBuffer` 31-bit limit, no `ShadowRealm`/V8 heap snapshots), no CDP inspector server, Lexe's native SEA container format, no `XMLHttpRequest`, and compact WebIDL coercions are accepted as documented runtime positioning. Remaining internal-only or platform/engine API-surface exclusions are enforced in [`compat/api-surface/allowlist.json`](../compat/api-surface/allowlist.json).

## Other notes

### `node:buffer`

- `constants.MAX_LENGTH` / `kMaxLength` is capped at `2147483647` (QuickJS 31-bit `ArrayBuffer` limit) rather than Node's platform limit.

### `node:sea`

- Asset APIs (`isSea`, `getAsset`, `getAssetAsBlob`, `getRawAsset`) are supported; Lexe compiles into its own self-contained SEA executable container format rather than Node's postject ELF/Mach-O/PE resource layout.

### `node:worker_threads`

- Uses **OS-thread worker isolates** (QuickJS per thread) with structured-clone `postMessage`, in-process `MessageChannel`, environment data sync, `SHARE_ENV`, `postMessageToThread()`, `moveMessagePortToContext()`, piped worker stdio (`stdin`, `stdout`, `stderr`), QuickJS-backed `resourceLimits` (`stackSizeMb` via QuickJS max stack size, `maxOldGenerationSizeMb` via `JS_SetMemoryLimit`), and **SharedArrayBuffer** hooks for shared memory within the process.
- Omits `Worker.prototype.getHeapSnapshot()` (V8 engine-specific).

### `ECMASCRIPT`

- Powered by QuickJS-ng rather than V8: `ArrayBuffer` maximum length is `2147483647` (31-bit limit).
- Full ES2024 RegExp `/v` (`unicodeSets`) is supported, including `--`/`&&`/nested character classes, `\q{...}`, and Unicode 17.0.0 emoji properties of strings (`Basic_Emoji`, `Emoji_Keycap_Sequence`, `RGI_Emoji_*`, `RGI_Emoji`).
- `ShadowRealm` is not implemented (intentional engine boundary).
- V8-specific `Error.captureStackTrace` and `CallSite` methods (`isEval`, `isNative`, `isConstructor`, `isToplevel`, `getEvalOrigin`, `getMethodName`, `getTypeName`, `getThis`) are lightweight stub polyfills.

### `WEBIDL`

- Exposes `DOMException` and core JS/Rust type coercions; omits full WebIDL prototype, branding, dictionary, and overload machinery (`idlharness` WPT suites are excluded).

### `XHR`

- Implements `FormData` and `ProgressEvent`; omits `XMLHttpRequest` intentionally (matching Node.js; server/CLI runtimes use `fetch()` or `node:http`/`node:https`).

### `node:async_hooks`

- Implements `AsyncLocalStorage`, `AsyncResource`, `createHook()`, `executionAsyncId()`, `executionAsyncResource()`, `triggerAsyncId()`, and `asyncWrapProviders`.
- Async lifecycle tracking stays on a zero-overhead fast path when no `AsyncLocalStorage` instance or enabled `createHook()` hook is active, and activates dynamically on demand as soon as any `createHook().enable()` (or `AsyncLocalStorage`) is enabled without requiring `LEXE_ASYNC_HOOKS=1`.

### `node:https`

- `https.Server` wraps `tls.Server` and delegates HTTP lifecycle handling to `node:http`, following the `node:tls` security policy.
- Requires building with a TLS feature (`tls-ring`, `tls-aws-lc`, `tls-graviola`, or `tls-openssl`); non-TLS builds stub out `createServer`/`get`/`request` and leave `Agent`/`Server`/`globalAgent` as `undefined`.

### `node:tls`

- Supports `pfx`/passphrase, encrypted PKCS#8 PEMs, async `SNICallback`, `ALPNCallback`, soft mTLS (`requestCert: true, rejectUnauthorized: false`), TLS session resumption (`session` / `ticket` / `"session"` event / `isSessionReused()`), `tls.rootCertificates`, `tls.getCACertificates()`, and write backpressure / `"drain"` signaling.
- Intentionally enforces TLS 1.2+ (`TLS < 1.2` unsupported), rejects weak/custom `ciphers`/`sigalgs`/`dhparam`/`crl`/`psk`/`ocsp`, and disables `TLSSocket.renegotiate()` for security.

### `WEBCRYPTO`

- Supports `AES-CBC`/`CTR`/`GCM`/`KW`, `HMAC`, `RSASSA-PKCS1-v1_5`, `RSA-PSS`, `RSA-OAEP`, `ECDSA`, `ECDH`, `Ed25519`, `Ed448`, `X25519`, `X448`, `HKDF`, `PBKDF2`, `Argon2d`/`Argon2i`/`Argon2id`, `ChaCha20-Poly1305`, and `SHA-*`.
- Omits `AES-OCB`, and availability varies on non-default crypto backends (e.g. `graviola`).

### `FETCH`

- Supports HTTP/1.1 & HTTP/2 (`h2` over TLS ALPN) streaming `Request`/`Response` bodies (`duplex: "half"`, abort & mid-stream error reset propagation), `Response.clone()`/`Request.clone()`, `data:`, `blob:`, and local `file:` URLs (`GET`/`HEAD` reading with HTTP `200`/`403`/`404`/`500` status mapping).
- Supports Subresource Integrity (`integrity` with `sha256`/`sha384`/`sha512` strongest-tier verification) and per-context in-memory `CacheStorage` (`caches`) / `Cache` (`open`, `has`, `keys`, `delete`, `match`, `matchAll`, `add`, `addAll`, `put`, `Vary` matching).
- Intentionally omits browser-origin policies (`credentials`, `referrer`, `referrerPolicy`, `priority`, CORS preflight, and forbidden-header enforcement) for a server/CLI runtime.

### `node:http2`

- Implemented natively: `createServer`, `createSecureServer`, `connect`, `Http2ServerRequest`, `Http2ServerResponse`, `getDefaultSettings`, 6-byte `getPackedSettings`/`getUnpackedSettings`, `sensitiveHeaders`, and full `constants` (253 entries).
- `performServerHandshake` throws `ERR_HTTP2_ERROR`, server push (`createPushResponse`) throws `ERR_HTTP2_PUSH_DISABLED`, and server request bodies are not streamed into `Http2ServerRequest` yet.
- Browser builds may still use the embedded load-time stub when the native module is unavailable.

### `node:module`

- Provides CommonJS and ESM module loading (`Module`, `createRequire`, `require.resolve`, `require.cache`, `require.extensions`, writable `_resolveFilename`/`_nodeModulePaths`, `Module._load`, `Module._compile`, and internal compatibility helpers `_resolveLookupPaths`, `_findPath`, `_stat`, `_initPaths`, `_pathCache`, `_preloadModules`, `_readPackage`, `globalPaths`, `runMain`, `wrap`, `wrapper`).
- Supports `stripTypeScriptTypes(code[, options])` (`'strip'` and `'transform'` modes, inline/URL source maps, `ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX` on non-erasable TypeScript syntax) and executing `.ts` / `.mts` / `.cts` files directly.
- Supports on-disk QuickJS bytecode compile cache (`enableCompileCache`, `flushCompileCache`, `getCompileCacheDir`, `NODE_COMPILE_CACHE`, `NODE_DISABLE_COMPILE_CACHE`) for both CommonJS and ESM modules.
- Supports synchronous `registerHooks()` and OS worker-thread `module.register(specifier[, parentURL][, options])` (`initialize`, `transferList` / `MessagePort`, async `resolve` and `load` hook chaining).
- `findSourceMap` reads file source maps on demand when source map support is enabled without rewriting `Error.prepareStackTrace`; CJS `require.extensions` hooks apply to CommonJS loads only.
- Native `.node` addon loading requires building with `--features napi` or `--features v8-compat` (see [Native addons](#native-addons)).

### `node:process`

- `process.version` (`v24.3.0`) and `process.versions.node` (`24.3.0`) advertise Node 24.3 compatibility while `process.release.name` is `"lexe"` and V8/libuv/OpenSSL-specific keys (`v8`, `uv`, `openssl`) are replaced by Lexe runtime keys (`lexe`, `node`, `sqlite`).

### Native addons

- **N-API** native addon (`.node`) loading is supported when Lexe is built with `--features napi` on a dynamically linked target.
- Linux release binaries come in two flavours. The `-gnu` builds are dynamically linked against glibc (baseline 2.28) and support native addons. The `-musl` builds are fully static and do not load `.node` files, because `dlopen` is unavailable in a static musl binary. npm selects the right one automatically. Packages that ship native addons such as `node-pty` need the `-gnu` build.
- **V8 C++ ABI** addons targeting Node 24 (e.g. `better-sqlite3`) are supported with `--features v8-compat` through a limited QuickJS-backed V8 shim.
