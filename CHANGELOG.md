# Changelog

All notable changes to Lexe are documented in this file.

## [Unreleased]

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
