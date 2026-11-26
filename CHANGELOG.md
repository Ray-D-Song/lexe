# Changelog

All notable changes to Lexe are documented in this file.

## [Unreleased]

- Implemented ES2024 RegExp `/v` (`unicodeSets`) in QuickJS (`libregexp`/`libunicode`): nested `[...]` character classes, `--` subtraction, `&&` intersection, `\q{...}` string disjunctions, `/iv` case folding, and Unicode 17.0.0 properties of strings (`\p{RGI_Emoji}`, `Basic_Emoji`, `Emoji_Keycap_Sequence`, `RGI_Emoji_Modifier_Sequence`, `RGI_Emoji_Flag_Sequence`, `RGI_Emoji_Tag_Sequence`, `RGI_Emoji_ZWJ_Sequence`) with full backtracking and lookbehind support.
- Supported structured cloning (`structuredClone()`, `workerData`, `postMessage`) of `WebAssembly.Module` instances via native clone codecs (`Arc`-shared `wasmi::Module`) and ES module `data:` URL `Worker` scripts in `node:worker_threads`.
- Implemented ES2021 `WeakRef` `KeepDuringJob` clearing at microtask checkpoint boundaries, native `node:http2` streaming `ClientHttp2Session` (`h2::client`) with TLS/TCP `createConnection` handoff (compatible with `undici`), `TLSSocket` EOF-on-FIN / `resetAndDestroy` / `pauseReading` parity, `net` deferred `close` / string port parsing, `node:fs` integer fd support in `writeFile*`/`appendFile*`/`readFile*`, `node:path` named `sep`/`delimiter` ESM exports, `MessagePort.once()`, unref'd `AbortSignal.timeout()`, and `node:tty` `ReadStream.setEncoding()` / auto-resume on `"data"` listener registration / `WriteStream.prototype.hasColors()`.

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
