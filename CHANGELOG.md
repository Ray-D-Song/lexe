# Changelog

All notable changes to Lexe are documented in this file.

## [Unreleased]

- Expanded `node:crypto` and WebCrypto (`WEBCRYPTO`) compatibility (`docs/compatibility.md`):
  - Added classic APIs: `Hash`/`Hmac` constructors, one-shot `crypto.hash()`, `Sign`/`Verify`/`createSign`/`createVerify`, `pbkdf2`/`pbkdf2Sync`, `scrypt`/`scryptSync`, `generateKey`/`generateKeySync`, `generateKeyPair`/`generateKeyPairSync`, `DiffieHellman`/`DiffieHellmanGroup`/`createDiffieHellman`/`createDiffieHellmanGroup`/`getDiffieHellman`, `checkPrime`/`checkPrimeSync`/`generatePrime`/`generatePrimeSync`, `privateDecrypt` (`OAEP` / `RSA_NO_PADDING`, intentionally rejecting `RSA_PKCS1_PADDING`), `privateEncrypt`/`publicDecrypt`, streaming `chacha20-poly1305` cipher/decipher, `getCipherInfo()`, `Ed448`/`X448` `KeyObject` + sign/verify/diffieHellman, encrypted PKCS#8 PEM import/export (`passphrase`/`cipher`), and `prng`/`pseudoRandomBytes`/`rng`/`fips`/`getFips`/`DH_*`/`OPENSSL_VERSION_NUMBER` aliases & constants.
  - Added WebCrypto `Ed448`, `X448`, `Argon2d`/`Argon2i`/`Argon2id`, and `ChaCha20-Poly1305`, plus `deriveBits`/`deriveKey` WPT edge-case fixes.
- Expanded non-security `node:tls` and `node:https` compatibility (`docs/compatibility.md`; `node:tls`, `node:https`, and `WEBCRYPTO` graduated to `✔︎` with documented security policies):
  - Added PKCS#12 `pfx`/passphrase and encrypted PKCS#8 PEM parsing, async `SNICallback` & `ALPNCallback`, soft mTLS (`requestCert: true, rejectUnauthorized: false`), TLS session resumption (`session`, `getSession()`, `getTLSTicket()`, `isSessionReused()`, `"session"` event), `tls.rootCertificates` & `tls.getCACertificates()`, and native `TLSSocket.write()` backpressure/`"drain"` propagation.
- Completed `node:module` TypeScript stripping, compile cache, CJS internals, and worker loader hooks (`docs/compatibility.md`; `node:module` graduated to `✔︎`):
  - Added `stripTypeScriptTypes()` (`strip`/`transform` modes, V3 source maps, `ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX`) and direct execution of `.ts`/`.mts`/`.cts` modules across CJS and ESM.
  - Added on-disk QuickJS bytecode compile cache (`enableCompileCache`, `flushCompileCache`, `getCompileCacheDir`, `NODE_COMPILE_CACHE`, `NODE_DISABLE_COMPILE_CACHE`) keyed by SHA-256 across CJS and ESM modules.
  - Added OS worker-thread `module.register(specifier[, parentURL][, options])` with async `initialize`, `transferList` (`MessagePort`), and chained `resolve`/`load` hooks, plus `Module._load` and legacy CJS loader helpers (`_resolveLookupPaths`, `_findPath`, `_stat`, `_initPaths`, `_pathCache`, `_preloadModules`, `_readPackage`, `globalPaths`, `runMain`, `wrap`, `wrapper`).
- Enabled `node:async_hooks` `createHook()` dynamically on demand (`docs/compatibility.md`; `node:async_hooks` graduated to `✔︎`):
  - Removed the `LEXE_ASYNC_HOOKS=1` gate (`HOOKING_MODE`) and activated async/Promise hook tracking on demand whenever `createHook().enable()` is called, returning `this` for `enable()`/`disable()` while preserving the zero-overhead fast path when no `AsyncLocalStorage` consumer or enabled user hook is active.
- Expanded `FETCH` compatibility (`docs/compatibility.md`; `FETCH` graduated to `✔︎` with documented server-runtime policy):
  - Fixed `Response.clone()` / `Request.clone()` when `.body` has been accessed prior to cloning (avoiding dual-reader stream stalls and enforcing locked/disturbed validation), and added explicit `duplex: "half"` validation and `Request.duplex`.
  - Enabled TLS ALPN `h2` negotiation on `FETCH_HTTP_CLIENT` and added full `StreamingBody` error/abort reset propagation (`HyperBodyError`, `Notify`-backed reader cancellation) for HTTP/2 streaming request uploads.
  - Added W3C Subresource Integrity (`integrity` with `sha256`/`sha384`/`sha512` strongest-tier verification across `http:`/`https:`/`data:`/`blob:`/`file:`), in-memory `CacheStorage` (`caches`) & `Cache` APIs, and `GET`/`HEAD` `file://` local file fetching (`200`/`403`/`404`/`500` status mapping).

## [0.1.10] - 2026-11-27

- Replaced `wasmi` with `wasmz` in `lexe_webassembly` and implemented `node:wasi` (`docs/compatibility.md`):
  - Switched `WebAssembly` runtime backend from `wasmi` to `wasmz`, adding Wasm GC, exception handling, typed function references, shared memory (`new WebAssembly.Memory({ initial, maximum, shared: true })`), threads/atomics, and `Memory64` support alongside multi-value, bulk-memory, reference types, and internal SIMD.
  - Added `node:wasi` / `wasi` (`WASI` class with `version: "preview1"`, `args`, `env`, `preopens`, `returnOnExit`, `stdin`/`stdout`/`stderr`, `wasiImport`, `getImportObject()`, `start()`, `initialize()`, and per-function JS override support).
- Improved `Intl` and `Temporal` compatibility without heavy ICU data tables (`docs/compatibility.md`):
  - Added `Intl.Locale` (BCP 47 parsing/canonicalization, `-u-` extension getters, options overrides, `maximize()`/`minimize()`, `getCalendars()`, `getCollations()`, `getHourCycles()`, `getNumberingSystems()`, `getTextInfo()`, `getWeekInfo()`, `getTimeZones()`), `Intl.PluralRules` (cardinal/ordinal CLDR rule subset, `selectRange`, `supportedLocalesOf`), `Intl.RelativeTimeFormat` (`always`/`auto`, `long`/`short`/`narrow`, `formatToParts`), `Intl.DisplayNames` (`language`, `region`, `script`, `currency`, `calendar`, `dateTimeField`), and `Intl.DurationFormat` (`long`/`short`/`narrow`/`digital`, `fractionalDigits`), and expanded `Intl.supportedValuesOf()` keys (`calendar`, `collation`, `currency`, `numberingSystem`, `unit`).
  - Added `Temporal.PlainYearMonth` and `Temporal.PlainMonthDay` (ISO-8601 `iso8601` calendar, `from()`, `compare()`, `with()`, `add()`, `subtract()`, `until()`, `since()`, `toPlainDate()`, `overflow: "constrain" | "reject"`), `PlainDate.prototype.toPlainYearMonth()` / `toPlainMonthDay()`, and fixed `{ year }` parsing in `DateExt::date_with()`.
- Completed `ENCODING` and `node:util` `TextDecoder` / `TextDecoderStream` compatibility (`ENCODING` and `node:util` now `✔︎` in `docs/compatibility.md`):
  - Integrated `encoding_rs` into `libs/lexe_encoding` (`Encoder::Whatwg` + streaming `IncrementalDecoder`), adding full WHATWG Encoding Standard support (`gbk`/`gb2312`, `gb18030`, `big5`, `shift_jis`, `euc-jp`, `iso-2022-jp`, `euc-kr`, `iso-8859-2`..`16`, `windows-1250`..`1258`, `windows-874`, `koi8-r`/`koi8-u`, `ibm866`, `macintosh`, `x-mac-cyrillic`, `x-user-defined`, and `replacement`) with lowercase canonical labels, streaming chunk state, and `fatal` `TypeError` handling.
- Expanded `node:http2` compatibility and fixed shutdown lifecycle (`v8_compat` / ASan):
  - Exported `Http2ServerRequest`, `Http2ServerResponse`, `performServerHandshake`, packed/unpacked settings, and full `constants` on `node:http2`.
  - Fixed `v8_compat::shutdown_runtime_addr` to run between dropping `JSContext` and freeing `JSRuntime` (eliminating a teardown `heap-use-after-free`), and fell back to the system allocator under ASan builds (`CARGO_CFG_SANITIZE=address`).

## [0.1.9] - 2026-11-27

- Improved `node:v8` compatibility (`docs/compatibility.md`):
  - Added `serialize()`, `deserialize()`, `Serializer`, `Deserializer`, `DefaultSerializer`, `DefaultDeserializer` (including `writeHeader`/`readHeader`, `getWireFormatVersion`, `writeUint32`/`readUint32`, `writeUint64`/`readUint64`, `writeDouble`/`readDouble`, `writeRawBytes`/`readRawBytes`/`_readRawBytes`, `transferArrayBuffer`, and custom `_writeHostObject`/`_readHostObject`), and `cachedDataVersionTag()`.
  - Added `BigInt` and `ArrayBufferView` (`Uint8Array`, TypedArrays, `Buffer`, `DataView`) structured-clone wire encoding/decoding in `lexe_utils::clone_wire`.

- Improved `node:async_hooks` compatibility (`docs/compatibility.md`):
  - Added `executionAsyncResource()`, `asyncWrapProviders`, and `AsyncLocalStorage({ defaultValue, name })` constructor options plus `AsyncLocalStorage.prototype.name`.
  - Updated `currentId()` to return the active `executionAsyncId()` instead of hardcoded `0`.

- Improved `node:worker_threads` compatibility (`docs/compatibility.md`):
  - Added shared `EnvStoreHandle` (`lexe_utils::env_store`) backing `process.env` so `SHARE_ENV` synchronizes live mutations across main and worker threads while keeping isolated snapshots and default worker environments scoped per isolate.
  - Added `postMessageToThread()` and `process` `"workerMessage"` cross-thread messaging (`THREAD_HUB`) with `ERR_WORKER_MESSAGING_FAILED` on exited/unknown targets.
  - Added piped worker stdio (`stdin: true`, `stdout: true`, `stderr: true`) with `Writable`/`PassThrough` streams, worker `console` redirection, and `EOF` delivery on exit.
  - Added `moveMessagePortToContext()` for moving `MessagePort` instances into `node:vm` contexts while detaching the original host port.
  - Added `resourceLimits` and `worker.resourceLimits` enforcement (`stackSizeMb` via QuickJS `max_stack_size`, `maxOldGenerationSizeMb` via `JS_SetMemoryLimit`) and ESM detection in `{ eval: true }` workers.

- Improved `node:util` compatibility (`docs/compatibility.md`):
  - Added `util.transferableAbortController()` and `util.transferableAbortSignal()` backed by native structured-clone/transfer codecs for `AbortSignal` with cross-isolate abort propagation (`ABORT_HUB`) and `DataCloneError` enforcement for detached/non-transferable signals.
  - Added `util.inspect()` and `util.formatWithOptions()` support for `showProxy`, `getters` (`true`, `"get"`, `"set"`), `numericSeparator`, and `breakLength` line wrapping.
  - Kept an active `EventLoopRef` alive for non-`eval` worker isolates until `Terminate` so unreferenced background watchers (`AbortSignal`, `MessagePort`) remain responsive for the worker's lifetime.
- Improved `node:buffer` compatibility (`docs/compatibility.md`):
  - Implemented `Buffer` slab pooling (`Buffer.poolSize`, default `64 * 1024`) for `Buffer.allocUnsafe()` and `Buffer.from()` with 8-byte alignment, while keeping `Buffer.allocUnsafeSlow()` and unseeded `Buffer.alloc()` unpooled.
  - Added `Buffer.prototype.inspect()` (aliased to `Buffer.prototype[util.inspect.custom]`) and dynamic `buffer.INSPECT_MAX_BYTES` truncation in both `Buffer.prototype.inspect()` and `util.inspect()`.
  - Enabled native 64-bit unsigned integer read/write support (`NumberKind::BigUInt`) in `lexe_buffer` via `JS_ToBigUint64` and `BigInt::from_u64`.
- Completed non-browser `DOM` compatibility (`DOM` now `✔︎` in `docs/compatibility.md`):
  - Added global `ErrorEvent` and `CloseEvent` inheriting from `Event` with WebIDL init coercion and defaults.
  - Added `Event.prototype` propagation and cancellation semantics (`preventDefault()`, `stopPropagation()`, `stopImmediatePropagation()`, `composedPath()`, `defaultPrevented`, `returnValue`, `cancelBubble`, `eventPhase`, `srcElement`, `timeStamp`, `isTrusted`, and phase constants) and fixed `EventTarget.dispatchEvent()` to honor `stopImmediatePropagation()` without treating `"error"` as Node's `ERR_UNHANDLED_ERROR`.
  - Updated `CustomEvent` to support `bubbles`/`cancelable`/`composed` and `[Symbol.toStringTag]` on `Event` and `EventTarget` classes.
- Completed `HTML` compatibility (`HTML` now `✔︎` in `docs/compatibility.md`):
  - Added `navigator.hardwareConcurrency`, `navigator.language`, `navigator.languages`, and `navigator.platform` alongside `navigator[Symbol.toStringTag]`.
  - Extended `structuredClone()` and transfer serialization (`clone_wire`) to support `DOMException`, `Blob`, `File`, `CryptoKey`, and transferable `MessagePort` (including across `Worker` threads and `workerData` `transferList`), throwing `DOMException("DataCloneError")` for non-transferable `MessagePort` and uncloneable values.
  - Implemented global `MessageEvent` (inheriting from `Event`), global/worker `BroadcastChannel` (inheriting from `EventTarget` with cross-isolate delivery and `InvalidStateError` on closed sends), and global `MessageChannel`/`MessagePort` getters linked to `node:worker_threads`.
  - Switched worker isolate main-channel event loop (`worker_isolate`) to non-blocking async `tokio::select!` with `Vm::idle()` so worker `MessagePort`, `BroadcastChannel`, and background async tasks execute alongside parent IPC.
- Completed full `FILEAPI` parity (`FILEAPI` now `✔︎` in `docs/compatibility.md`):
  - Added `FileReader`, `FileReaderSync`, `FileList`, and `ProgressEvent` (inheriting from `EventTarget` / `Event`), with `File` prototype inheritance from `Blob`.
  - Added `Blob.prototype.stream()` / `File.prototype.stream()` byte streams (with default and BYOB readers), fixed `Blob.prototype.slice()` bounds/defaults, `USVString` lone-surrogate replacement, UTF-8 BOM stripping in `Blob.prototype.text()`, `endings: "native"` line-ending conversion, and `File::from_bytes`.
  - Added `fetch("blob:...")` support (`GET`/`HEAD`) and `FormData` 3rd `filename` parameter, `USVString` coercion, first-entry `get()`, binary-safe multipart serialization/parsing, and `File`/`FormData` bodies in `fetch()`.
  - Fixed `DOMException` initialization so `lexe_exceptions::DOMException` attaches properly and `windows-1252` decoding in `lexe_encoding` maps bytes `0x80`–`0x9F` per WHATWG.

## [0.1.8] - 2026-11-26

- Implemented ES2024 RegExp `/v` (`unicodeSets`) in QuickJS (`libregexp`/`libunicode`): nested `[...]` character classes, `--` subtraction, `&&` intersection, `\q{...}` string disjunctions, `/iv` case folding, and Unicode 17.0.0 properties of strings (`\p{RGI_Emoji}`, `Basic_Emoji`, `Emoji_Keycap_Sequence`, `RGI_Emoji_Modifier_Sequence`, `RGI_Emoji_Flag_Sequence`, `RGI_Emoji_Tag_Sequence`, `RGI_Emoji_ZWJ_Sequence`) with full backtracking and lookbehind support.
- Supported structured cloning (`structuredClone()`, `workerData`, `postMessage`) of `WebAssembly.Module` instances via native clone codecs (`Arc`-shared `wasmi::Module`) and ES module `data:` URL `Worker` scripts in `node:worker_threads`.
- Implemented ES2021 `WeakRef` `KeepDuringJob` clearing at microtask checkpoint boundaries, native `node:http2` streaming `ClientHttp2Session` (`h2::client`) with TLS/TCP `createConnection` handoff (compatible with `undici`), `TLSSocket` EOF-on-FIN / `resetAndDestroy` / `pauseReading` parity, `net` deferred `close` / string port parsing, `node:fs` integer fd support in `writeFile*`/`appendFile*`/`readFile*`, `node:path` named `sep`/`delimiter` ESM exports, `MessagePort.once()`, unref'd `AbortSignal.timeout()`, and `node:tty` `ReadStream.setEncoding()` / auto-resume on `"data"` listener registration / `WriteStream.prototype.hasColors()`.
- Implemented `util.parseArgs`, native `util.types` checks (`isCryptoKey`, `isKeyObject`, `isExternal`, module namespace objects, cross-realm native errors), and HTTP server / SSE / SQLite DBCONFIG / Unix domain socket compatibility fixes.

## [0.1.7] - 2026-11-26

- Reduced release binary size (~41% on stable `darwin-arm64`, ~49% with nightly `build-std`):
  - Switched `[profile.release]` to `opt-level = "z"` while keeping `rquickjs-sys` at `opt-level = 3` for QuickJS VM execution speed.
  - Trimmed unused SQLite extensions (`GEOPOLY`, `RTREE`, `RBU`, `FTS3`, `DBSTAT_VTAB`) from `lexe_sqlite`, retaining `FTS5`, `SESSION`, `PREUPDATE_HOOK`, `COLUMN_METADATA`, and `MATH_FUNCTIONS`.
  - Switched `jiff` to system timezone databases (`tzdb-zoneinfo`, `tzdb-concatenated`) instead of bundling the full IANA tzdb.
  - Switched default TLS root certificates in `lexe_tls`, `lexe_http`, `lexe_fetch`, and `lexe_modules` from `webpki-roots` to `native-roots` (`rustls-native-certs`). Pass `--features webpki-roots` when targeting minimal containers without system CA bundles.
  - Verified nightly size-optimized `build-std` flags: `RUSTFLAGS="-Zunstable-options -Cpanic=immediate-abort" cargo +nightly build --release --target <triple> -Z build-std=std,panic_abort -Z build-std-features=optimize_for_size`.

## [0.1.6] - 2026-11-26

- Fixed Windows MSVC `lexe.exe` builds so native `.node` addons resolve the `napi_*` and `node_api_*` host symbols. The executable now exports the full N-API surface through a generated module definition, so Windows standalone CLI builds no longer panic while loading native addons.
- x86_64 macOS release binaries are now cross-compiled on the `macos-15` arm64 runner instead of the slower `macos-15-intel` runner. The x86_64 Mach-O architecture is verified; the x86_64 binary is not executed in CI.

## [0.1.5] - 2026-11-26

- Split Linux release binaries by C library. `lexe-linux-x64-gnu` and `lexe-linux-arm64-gnu` are dynamically linked against glibc with a glibc 2.28 baseline and export N-API symbols, so native `.node` addons such as `node-pty` load. `lexe-linux-x64-musl` and `lexe-linux-arm64-musl` remain statically linked and do not load native addons. The unadorned `lexe-linux-x64` and `lexe-linux-arm64` assets are no longer published.
- Added `linux-x64-musl` and `linux-arm64-musl` to `lexe compile -p`. `linux-x64` and `linux-arm64` now refer to the glibc builds.
- The npm package now installs a Linux runtime from `lexe-linux-<arch>-<libc>` optional subpackages, selected automatically by npm's `libc` field. `LEXE_LIBC=gnu|musl` overrides detection.
- `lexe upgrade` and the release download fallback select the `-gnu` or `-musl` asset for the running binary.
- Changed the compiled bytecode file extension from `.lrt` to `.lbt`. `lexe compile` now defaults to `<input>.lbt`, and `require` and the embedded loader recognize `.lbt`. The bytecode header magic (`lrt01`) is unchanged, so the format version does not change.

- Added Single Executable Application assets and the `node:sea` API (`isSea`, `getAssetKeys`, `getAsset`, `getRawAsset`, and `getAssetAsBlob`). `lexe compile --executable` now accepts `--sea-config`, `--asset`, and `--assets-dir`, embeds compressed assets, and preserves application arguments and entrypoint metadata.
- Added `node:ffi`, backed by libffi, with dynamic library loading, typed native calls, JavaScript callbacks, pointer access, and copied or shared ArrayBuffer views. Native FFI support is enabled in default builds.
- Expanded native addon compatibility with N-API thread-safe functions, async cleanup hooks, callback and finalizer handling, dynamic symbol lookup, and OpenTUI native-library support.
- Added classic Crypto APIs, including streaming AES ciphers, HKDF, KeyObject import/export, signatures, ECDH key generation and shared secrets, timing-safe comparison, and X.509 certificate parsing and verification.
- Added `Intl.Collator`, `Intl.NumberFormat`, `Intl.ListFormat`, and `Intl.Segmenter`, including UTF-16 segment indices, and improved date/time formatting.
- Improved ESM/CommonJS loading, synchronous module hooks, file URLs, package resolution, and `import.meta` in source and compiled applications; keep metadata properties writable and configurable for tools such as Vite+.
- Added an Inspector `Session` compatibility shim that reports unavailable protocol operations, and `util.isDeepStrictEqual` for structured values.
- Fixed filesystem numeric open flags, mode parsing, descriptor handling, and Node-style errors; added stream consumers and improved readable/writable lifecycle, encoding, backpressure, and callbacks.
- Fixed HTTP response header handling and retained keep-alive socket shutdown, worker evaluation and termination promises, child-process events, TTY output/raw mode, TextDecoder input handling, and zlib compatibility. Increased the QuickJS BigInt limit to 16 Mi-bits.
- Validated a macOS arm64 Release build of OpenCode using embedded native assets, including cold-start model listing and interactive PTY TUI rendering and clean shutdown. This validation does not cover Web UI pages or model inference.

## [0.1.3] - 2026-11-25

- Completed full WHATWG Streams and Compression Streams parity (`node:stream/web`, `STREAMS`, `COMPRESSION` now `✔︎` in `docs/compatibility.md`, reducing `⚠️` items from 27 to 24):
  - `TransformStream` follows the specification (`start`/`transform`/`flush`/`cancel`, backpressure, error propagation, `readableType`/`writableType` validation); `pipeTo`/`pipeThrough`/`values` accept `null` options with WebIDL ordering; BYOB reads validate `min` and reject `SharedArrayBuffer` and non-detachable `WebAssembly.Memory` buffers; queuing strategies coerce `highWaterMark` and expose shared callable `size` functions; all stream classes expose `Symbol.toStringTag`.
  - `CompressionStream`/`DecompressionStream` are `GenericTransformStream`s (no longer `TransformStream` subclasses), decode concatenated gzip members and reject trailing data with `TypeError`.
  - Fixed QuickJS shutdown leaks from untraced promise reactions and shared state in `tee()`, `ReadableStream.from()`, and `pipeTo()`.
- Added `node:sea` to the compatibility table as not implemented.

## [0.1.2] - 2026-11-25

- Closed full Node.js v24.3.0 parity gaps across 20+ built-in modules, reducing `⚠️` items in `docs/compatibility.md` from 53 to 28 and increasing full `✔︎` modules from 20 to 46:
  - **Networking & Web**: `node:http` & `node:https` (`Agent`/`globalAgent`, `https.Server`, lenient HTTP/1.1 parser `insecureHTTPParser`, full timeouts/close/destroy lifecycle, trailers, upgrade/WebSocket parity), `node:dns` & `node:dns/promises` (`Resolver`, full record query types via DNS-over-UDP/TCP wire client, `getDefaultResultOrder`/`setDefaultResultOrder`, `lookupService`), `node:net` (`Happy Eyeballs` / `autoSelectFamily`, socket timeouts, `resetAndDestroy`, `destroySoon`, `Server` options), and `node:dgram` (connected UDP, multicast/SSM, buffer options, `Symbol.asyncDispose`).
  - **Filesystem, Process & Runtime**: `node:fs` & `node:fs/promises` (`fs.watch` & `fs.promises.watch`, `statfs`, `glob`, `openAsBlob`, `FileHandle` / fd parity, `cp`, `lutimes`/`lchown`, recursive `readdir`), `node:process` (`process.report`, `process.finalization`, `--permission`, resource usage, `sourceMapsEnabled`, exit/signal lifecycle), `node:child_process` (`spawnSync`/`execSync`/`execFileSync`, full `stdio`/fd/IPC/`fork` options, `unref`, `AbortSignal`), `node:os` & `node:constants`, `node:tty` (`SIGWINCH` resize events, Windows/Unix fd parity), and `node:timers` & `node:timers/promises`.
  - **Observability, Core & Utilities**: `node:perf_hooks` (pure-Rust `HdrHistogram`, Event Loop Delay `monitorEventLoopDelay`, `eventLoopUtilization`, `PerformanceNodeTiming`, `PerformanceObserver`, `markResourceTiming`, `timerify`, and automatic `resource`/`http`/`http2`/`dns`/`net`/`gc` instrumentation via QuickJS GC hooks), `node:diagnostics_channel` (`TracingChannel`, store binding, built-in channels), `node:console`, `node:url` (`URLPattern`, WHATWG URL setter/error parity), `node:sqlite` (`SQLTagStore`, `setAuthorizer`, `enableDefensive`, limits), `node:zlib` (`unzip`, `crc32`, flush/params/close), `node:events`, `node:assert`/`assert/strict`, `node:path`, and `node:querystring`.
- Fixed Windows CI compilation and clippy issues in `lexe_net`, `lexe_dns`, `lexe_child_process`, and `lexe_fs`.

## [0.1.1] - 2026-11-24

- Replaced the legacy runtime core with the upgraded QuickJS-based Node.js-compatible runtime core (supporting `node:http`, `node:https`, `node:http2`, `node:tls`, `node:sqlite`, `node:worker_threads`, N-API, V8 ABI 137 shim, and WebAssembly).
- Rebranded all crates, built-in modules (`lexe:*`), environment variables (`LEXE_*`), scripts, CI workflows, and documentation to `lexe`.
- Unified single-executable compilation under `lexe compile --executable` using `libsui` across 6 platforms (`linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, `windows-x64`, `windows-arm64`), a prebuilt zstd dictionary (`prebuild/compression.dict`), and built-in GitHub Release auto-download with `SHA256SUMS` verification.

## [0.1.0] - 2026-08-09

Initial release.
