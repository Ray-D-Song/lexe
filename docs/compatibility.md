# Compatibility matrix

> [!NOTE]
> Below is a high level overview of partially supported APIs and modules. For more details consult the [API](../API.md) documentation

| [Node.js API](https://nodejs.org/api/index.html) | Node.js | lexe | Key differences from Node.js |
| --- | --- | --- | --- |
| node:assert | ✔︎ | ✔︎ | — |
| node:async_hooks | ✔︎ | ✔︎⚠️ | Public `createHook()` disabled by default (`LEXE_ASYNC_HOOKS=1`) ([details](#nodeasync_hooks)) |
| node:buffer | ✔︎ | ✔︎⚠️ | 31-bit `MAX_LENGTH` (`2147483647`) due to QuickJS-ng `ArrayBuffer` limit ([details](#nodebuffer)) |
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
| node:util | ✔︎ | ✔︎ | — |
| node:util/types | ✔︎ | ✔︎ | — |
| node:v8 | ✔︎ | ✔︎⚠️ | QuickJS memory stats (`"quickjs"` heap space); `setFlagsFromString` no-op; structured-clone-backed serialization; missing V8 heap snapshots and `GCProfiler` ([details](#nodev8)) |
| node:vm | ✔︎ | ✔︎ | Full API (`Script`, `createContext`/`isContext`, `runIn*Context`, `compileFunction`, `SourceTextModule`, `SyntheticModule`, `measureMemory`, `cachedData`, `timeout`, `codeGeneration`, `microtaskMode`, `importModuleDynamically`) |
| node:wasi | ✔︎ | ✘ | Not implemented |
| node:worker_threads | ✔︎ | ✔︎⚠️ | Missing V8 `Worker.prototype.getHeapSnapshot()` ([details](#nodeworker_threads)) |
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
| ECMASCRIPT | ✔︎⚠️ | QuickJS-ng engine (31-bit `ArrayBuffer` limit, no `ShadowRealm`); V8 `Error.captureStackTrace`/`CallSite` fields are stub polyfills ([details](#ecmascript)) |
| ENCODING | ✔︎ | — |
| FETCH | ✔︎⚠️ | No `file:` URL fetch; omits browser policy controls (`credentials`, `referrer`, `integrity`, `caches`, CORS); WPT edge cases ([details](#fetch)) |
| FILEAPI | ✔︎ | — |
| HR-TIME | ✔︎ | — |
| HTML | ✔︎ | — |
| STREAMS | ✔︎ | — |
| URL | ✔︎ | — |
| URLPATTERN | ✔︎ | — |
| WASM-JS-API-2 | ✔︎⚠️ | `wasmi` interpreter; missing GC/exceptions/threads/Memory64/JSPI/`v128` boundary; copied `Memory.buffer`; retained `externref`s ([details](#wasm-js-api-2)) |
| WASM-WEB-API-2 | ✔︎⚠️ | `compileStreaming`/`instantiateStreaming` buffer the full `Response` before compiling (no incremental streaming compilation) ([details](#wasm-web-api-2)) |
| WEBCRYPTO | ✔︎⚠️ | Missing `X448`, `Ed448`, `Argon2`, `AES-OCB`, `ChaCha20-Poly1305` and some `deriveBits`/`deriveKey` WPT edge cases ([details](#webcrypto)) |
| WEBIDL | ✔︎⚠️ | Provides `DOMException` and core coercions; lacks full WebIDL prototype/branding/dictionary/overload compliance ([details](#webidl)) |
| XHR | ✔︎⚠️ | Implements `FormData` and `ProgressEvent`; `XMLHttpRequest` is not implemented (Node.js also omits XHR) ([details](#xhr)) |

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

- ⚠️ = partially supported in Lexe (open architectural/tradeoff items tracked in [Pending Decisions](#pending-decisions))
- ⏱ = planned partial support
- \* = Not native

## Partial support details

### Node.js APIs

#### `node:async_hooks`

- User `createHook()` callbacks are disabled by default and only run when `LEXE_ASYNC_HOOKS=1` is set; by default only internal `AsyncLocalStorage` propagation is active.

#### `node:buffer`

- `constants.MAX_LENGTH` / `kMaxLength` is capped at `2147483647` (QuickJS 31-bit `ArrayBuffer` limit) rather than Node's platform limit.

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

#### `node:v8`

- Backed by QuickJS `JS_ComputeMemoryUsage` rather than V8: `getHeapSpaceStatistics()` always returns a single `"quickjs"` space, and `heap_size_limit` falls back to `Number.MAX_SAFE_INTEGER` when `malloc_limit === 0`.
- `setFlagsFromString()` is a no-op; `cachedDataVersionTag()`, `serialize()`, `deserialize()`, `Serializer`, `Deserializer`, `DefaultSerializer`, and `DefaultDeserializer` are implemented (backed by structured clone wire format rather than V8 binary snapshots).
- Missing V8-specific heap snapshots (`writeHeapSnapshot`, `getHeapSnapshot`), `GCProfiler`, `queryObjects`, `startupSnapshot`, and coverage APIs (`takeCoverage`, `stopCoverage`).

#### `node:worker_threads`

- Uses **OS-thread worker isolates** (QuickJS per thread) with structured-clone `postMessage`, in-process `MessageChannel`, environment data sync, `SHARE_ENV`, `postMessageToThread()`, `moveMessagePortToContext()`, piped worker stdio (`stdin`, `stdout`, `stderr`), QuickJS-backed `resourceLimits` (`stackSizeMb` via QuickJS max stack size, `maxOldGenerationSizeMb` via `JS_SetMemoryLimit`), and **SharedArrayBuffer** hooks for shared memory within the process.
- Missing `Worker.prototype.getHeapSnapshot()` (V8 engine-specific).

### Web Platform APIs

#### `ECMASCRIPT`

- Powered by QuickJS-ng rather than V8: `ArrayBuffer` maximum length is `2147483647` (31-bit limit).
- Full ES2024 RegExp `/v` (`unicodeSets`) is supported, including `--`/`&&`/nested character classes, `\q{...}`, and Unicode 17.0.0 emoji properties of strings (`Basic_Emoji`, `Emoji_Keycap_Sequence`, `RGI_Emoji_*`, `RGI_Emoji`).
- `ShadowRealm` is not implemented.
- V8-specific `Error.captureStackTrace` and `CallSite` methods (`isEval`, `isNative`, `isConstructor`, `isToplevel`, `getEvalOrigin`, `getMethodName`, `getTypeName`, `getThis`) are stub polyfills.

#### `FETCH`

- `file:` scheme throws `TypeError("Unsupported scheme")` (`blob:` URLs from `URL.createObjectURL()` support `GET`/`HEAD`).
- Browser-centric options and policies (`credentials`, `referrer`, `referrerPolicy`, `integrity`, `priority`, HTTP `cache` / `caches` API, CORS preflight, forbidden-header enforcement) are omitted or non-enforcing.
- Some WPT edge cases (`response-clone` under stress, HTTP/2 request-upload streaming) do not yet pass.

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

- Implements `FormData` and `ProgressEvent`; `XMLHttpRequest` is not implemented (Node.js also omits `XMLHttpRequest`, while WinterTC groups `FormData` under XHR).

### Other features

#### `Intl`

- Implements partial `Intl.DateTimeFormat`, `Intl.Collator`, `Intl.NumberFormat`, `Intl.ListFormat`, and `Intl.Segmenter`, plus `Intl.supportedValuesOf()` and `Date.prototype.toLocaleString` with `timeZone`.
- Missing `Intl.PluralRules`, `Intl.RelativeTimeFormat`, `Intl.DisplayNames`, `Intl.Locale`, `Intl.DurationFormat`, and full ICU locale/calendar/numbering-system support.

#### `Temporal`

- Implements `Duration`, `Instant`, `PlainDate`, `PlainDateTime`, `PlainTime`, `ZonedDateTime`, and `Now` (backed by `jiff`).
- Missing `PlainYearMonth`, `PlainMonthDay`, non-ISO calendar systems (ISO-8601 only), and full `Intl`-backed `toLocaleString()` formatting.

## Pending Decisions

Remaining `⚠️` items require explicit product/architecture choices (binary size, dependency weight, engine boundaries, runtime overhead, or security semantics) before implementation or intentional Matrix graduation. Decided items are removed from this list once implemented or accepted as documented behavior.

- [ ] **1. `Intl` + `Temporal` localization & remaining types**
  - **Gap**: `Intl` lacks `PluralRules`, `RelativeTimeFormat`, `DisplayNames`, `Locale`, `DurationFormat`, and full ICU locale/calendar/numbering-system data; `Temporal` lacks `PlainYearMonth`, `PlainMonthDay`, non-ISO calendars, and `Intl`-backed `toLocaleString()`.
  - **Options**:
    - **A (Pure-logic parity + minimal CLDR)**: Implement `Intl.Locale`, `Temporal.PlainYearMonth`, `Temporal.PlainMonthDay`, and core constructors/rule subsets (`PluralRules`, `RelativeTimeFormat`, `DisplayNames`, `DurationFormat`) without embedding heavy full ICU4X data blobs.
    - **B (Full ICU4X static data)**: Bundle compiled ICU4X locale/calendar tables for full multi-locale/non-ISO calendar parity (significant binary size increase).
    - **C (Accept partial / feature-gate heavy ICU)**: Fix `PlainYearMonth`/`PlainMonthDay`/`Intl.Locale` and optionally gate full ICU data behind a Cargo feature or external data path.

- [ ] **2. `WASM-JS-API-2` & `WASM-WEB-API-2` engine capabilities**
  - **Gap**: Backed by `wasmi` interpreter (no JIT, no GC/exceptions/threads/Memory64/JSPI/`v128` boundary, copied `Memory.buffer`, retained `externref`s, full-response buffering in `compileStreaming`/`instantiateStreaming`).
  - **Options**:
    - **A (Enhance on `wasmi`)**: Fix what is feasible inside `wasmi` (e.g., incremental/streamed Wasm framing or zero-copy/dirty-sync views where possible, `v128` boundary handling, `externref` cleanup), accepting spec limits (`GC`, `exceptions`, `Memory64`, `threads`).
    - **B (Add/switch to `wasmtime` backend)**: Support full JIT, shared linear memory, SIMD/GC/exceptions/threads (at large binary size, build time, and static-musl complexity cost), optionally via a Cargo feature.
    - **C (Keep `wasmi` as documented boundary)**: Maintain `wasmi` lightweight interpreter limits without heavy runtime replacement.

- [ ] **3. `node:crypto`, `WEBCRYPTO`, `node:tls`, and `node:https` algorithms & security policies**
  - **Gap**:
    - Crypto/WebCrypto: Missing `createSign`/`createVerify`, `generateKeyPair`/`generateKeyPairSync`, `generateKey`/`generateKeySync`, `scrypt`/`scryptSync`, `pbkdf2`/`pbkdf2Sync`, `privateDecrypt`/`privateEncrypt`/`publicDecrypt`, `DiffieHellman`, `crypto.hash()`, `X448`/`Ed448`/`Argon2`/`AES-OCB`/`ChaCha20-Poly1305`, some `deriveBits`/`deriveKey` edge cases, and `graviola` backend parity.
    - TLS/HTTPS: Rejects `pfx`, `ciphers`, `sigalgs`, `dhparam`, `crl`, `psk`, `ocsp`, `session`, `ticket`, async `SNICallback`, `ALPNCallback`, soft mTLS, encrypted private keys on rustls, TLS < 1.2, `renegotiate()`, and `TLSSocket.write()` `"drain"` backpressure; `https` depends on TLS feature builds.
  - **Options**:
    - **A (Pure additive crypto + non-security TLS fixes)**: Implement missing crypto APIs (`crypto.hash()`, `createSign`/`createVerify`, `generateKeyPair*`, `scrypt*`, `pbkdf2*`, `DiffieHellman`, PKCS#12 `pfx` parsing, `ChaCha20-Poly1305`, `Argon2`, `X448`/`Ed448`, RSA decrypts) + `TLSSocket.write()` backpressure/`drain`, async `SNICallback`/`ALPNCallback`, `session`/`ticket`/soft mTLS, while intentionally keeping TLS < 1.2 and weak cipher strings disabled for security.
    - **B (Backend expansion / OpenSSL unification)**: Allow `tls-openssl` or broader multi-backend support for legacy options (`TLS < 1.2`, arbitrary `ciphers`, `renegotiate()`, `ocsp`/`crl`).
    - **C (Keep minimal crypto/TLS subset)**: Target only specific algorithms needed by common Node/WinterTC libraries and accept remaining gaps.

- [ ] **4. `node:module` TypeScript stripping, ESM Worker Loader, and compile cache**
  - **Gap**: Missing `stripTypeScriptTypes`, `module.register()` (async loader hooks thread), `Module._load`, and on-disk bytecode compile cache (`enableCompileCache` reports `FAILED`).
  - **Options**:
    - **A (Implement lightweight/full features)**: Add `stripTypeScriptTypes` (decide lightweight lexer/stripper vs `oxc` full TS parser), `Module._load`, on-disk QuickJS bytecode compile cache (`enableCompileCache`), and `module.register` loader hooks.
    - **B (Selective subset)**: Add `stripTypeScriptTypes` + `Module._load` + disk compile cache while keeping `module.register` omitted (or async single-isolate hook adapter without dedicated loader worker overhead).
    - **C (Document current CJS/sync hooks boundary)**: Keep `registerHooks` (sync) without worker-based `module.register` or heavy parser deps.

- [ ] **5. `node:async_hooks` (`createHook()` default activation)**
  - **Gap**: `AsyncLocalStorage` propagates unconditionally with low overhead; user `createHook()` callbacks only run when `LEXE_ASYNC_HOOKS=1` is set.
  - **Options**:
    - **A (Enable on-demand when `createHook().enable()` is called)**: Keep zero overhead when no hook is created/enabled, and activate Promise/async tracking dynamically as soon as any user `AsyncHook.enable()` is called (eliminating the `LEXE_ASYNC_HOOKS=1` gate while preserving default fast-path throughput).
    - **B (Always keep `LEXE_ASYNC_HOOKS=1` opt-in)**: Preserve the explicit environment gate to guarantee zero accidental user-hook activation overhead in production services.

- [ ] **6. `FETCH` policy boundaries & remaining WPT edge cases**
  - **Gap**: `fetch("file://...")` throws `Unsupported scheme`; omits `credentials`, `referrer`, `referrerPolicy`, `integrity`, `priority`, `cache`/`caches`, CORS preflight, and forbidden-header enforcement; minor WPT edge cases (`response-clone` under stress, HTTP/2 request-upload streaming).
  - **Options**:
    - **A (Fix runtime gaps + server-appropriate features)**: Fix `response-clone` & HTTP/2 upload streaming and add SRI `integrity` verification / optional in-memory `caches`, while keeping CORS/browser-origin policies intentionally disabled (`non-browser runtime`) and deciding explicitly whether `fetch("file://...")` should be allowed or stay rejected for SSRF/security isolation.
    - **B (Fix WPT runtime edge cases only)**: Fix `response-clone` + HTTP/2 upload streaming and accept server-runtime omissions (`file:`, `credentials`, CORS, `caches`, `integrity`).

- [ ] **7. Engine boundaries & runtime positioning (`QuickJS`, `node:inspector`, `node:sea`, `WEBIDL`, `XHR`)**
  - **Gap**:
    - `ECMASCRIPT` / `node:buffer` / `node:v8` / `node:worker_threads`: QuickJS 31-bit `MAX_LENGTH` (`2147483647`), no `ShadowRealm`, V8 `CallSite` stub methods, no V8 heap snapshots (`getHeapSnapshot`/`writeHeapSnapshot`), `GCProfiler`, `queryObjects`, `startupSnapshot`, `takeCoverage`/`stopCoverage`.
    - `node:inspector`: No CDP/WebSocket inspector server (`url()` is `undefined`, `Session` returns `ERR_INSPECTOR_NOT_AVAILABLE`).
    - `node:sea`: Lexe compiles into its own SEA container format rather than Node's ELF/Mach-O/PE postject blob format.
    - `WEBIDL` & `XHR`: Partial WebIDL (`idlharness` skipped); omits `XMLHttpRequest` (matching Node.js).
  - **Options**:
    - **A (Accept as documented engine/runtime positioning)**: Fix lightweight conformance gaps (e.g. QuickJS inspector CDP protocol if desired, `WEBIDL` constructor/branding helpers, `Error.captureStackTrace`/`CallSite` real frames), while explicitly documenting QuickJS 31-bit limits, Lexe SEA container format, no `ShadowRealm`/V8 heap snapshots, and no `XMLHttpRequest` as intentional design boundaries (or moving non-issue items out of `⚠️`).
    - **B (Targeted deep work)**: Select specific engine/runtime capabilities to invest in (e.g., implementing a real CDP WebSocket inspector server, `ShadowRealm`, 64-bit `ArrayBuffer` fork in QuickJS-ng, or V8-independent `WebIDL` hardening).

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
