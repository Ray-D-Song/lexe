# Compatibility matrix

> [!NOTE]
> Below is a high level overview of partially supported APIs and modules. For more details consult the [API](../API.md) documentation

| [Node.js API](https://nodejs.org/api/index.html) | Node.js | lexe | Key differences from Node.js |
| --- | --- | --- | --- |
| node:assert | ✔︎ | ✔︎⚠️ | Missing `partialDeepStrictEqual` and `snapshot`; `node:assert/strict` lacks ESM named exports ([details](#nodeassert)) |
| node:async_hooks | ✔︎ | ✔︎⚠️ | Public `createHook()` disabled by default (`LEXE_ASYNC_HOOKS=1`); missing `executionAsyncResource`, `asyncWrapProviders`, and `AsyncLocalStorage` Node 24 options ([details](#nodeasync_hooks)) |
| node:buffer | ✔︎ | ✔︎⚠️ | 31-bit `MAX_LENGTH` (`2147483647`); no slab pooling; no `Buffer.prototype.inspect` or `INSPECT_MAX_BYTES` truncation; no `Uint64` paths ([details](#nodebuffer)) |
| node:child_process | ✔︎ | ✔︎⚠️ | Missing `spawnSync`; limited `stdio` (`pipe`/`ignore`/`inherit`) and `spawn` options; `unref()` detaches wait handle ([details](#nodechild_process)) |
| node:cluster | ✔︎ | ✘ | Not implemented |
| node:console | ✔︎ | ✔︎⚠️ | Missing `dirxml`, `profile`/`profileEnd`, `timeStamp`; `Console` options and most `dir` inspect options are ignored ([details](#nodeconsole)) |
| node:constants | ✔︎ | ✔︎⚠️ | Exports `F_OK`/`R_OK`/`W_OK`/`X_OK` only; missing `errno`, `SIG*`, `O_*`, `S_*`, and crypto constants ([details](#nodeconstants)) |
| node:crypto | ✔︎ | ✔︎⚠️ | Missing classic Cipher/Decipher, top-level Sign/Verify, `KeyObject`, X509, DH/ECDH, and KDF sync/async exports; depends on WebCrypto `subtle` ([details](#nodecrypto)) |
| node:dgram | ✔︎ | ✔︎⚠️ | Unconnected UDP only; missing `connect`/`disconnect`, multicast/TTL/broadcast/buffer controls, and `Symbol.asyncDispose` ([details](#nodedgram)) |
| node:diagnostics_channel | ✔︎ | ✔︎⚠️ | Missing `TracingChannel` and `AsyncLocalStorage` store binding (`bindStore`/`runStores`); no built-in runtime channels ([details](#nodediagnostics_channel)) |
| node:dns | ✔︎ | ✔︎⚠️ | Exports `lookup` and `promises.lookup` only; missing `Resolver`, `resolve*`, `reverse`, `lookupService`, and server/order settings ([details](#nodedns-and-nodednspromises)) |
| node:dns/promises | ✔︎ | ✔︎⚠️ | Exports `lookup` only; missing `Resolver` and `resolve*`/`reverse`/`lookupService` ([details](#nodedns-and-nodednspromises)) |
| node:events | ✔︎ | ✔︎⚠️ | Missing `EventEmitterAsyncResource` and automatic `captureRejections` handling ([details](#nodeevents)) |
| node:fs | ✔︎ | ✔︎⚠️ | Missing callback `unlink`/`chmod`/`read`/`write`, `copyFile*`, `cpSync`, `globSync`, `opendirSync`, `statfsSync`, `watchFile`, `openAsBlob`, and `BigIntStats` ([details](#nodefs-and-nodefspromises)) |
| node:fs/promises | ✔︎ | ✔︎⚠️ | Missing `unlink`, `copyFile`, `watch`, `lchmod`, `openAsBlob`, and `BigIntStats` ([details](#nodefs-and-nodefspromises)) |
| node:http | ✔︎ | ✔︎⚠️ | Minimal `ClientRequest` (no header/timeout/socket setters); `Server` rejects `insecureHTTPParser` and lacks some HTTP/1.1 lifecycle APIs ([details](#nodehttp-and-nodehttps)) |
| node:http2 | ✔︎ | ✔︎ | Native HTTP/2 ([notes](#nodehttp2)) |
| node:https | ✔︎ | ✔︎⚠️ | Exports `Agent`, `createServer`, `get`, `request` only (no `Server`/`globalAgent`); inherits `node:http` and `node:tls` limits; requires TLS feature build ([details](#nodehttp-and-nodehttps)) |
| node:inspector | ✔︎ | ✔︎⚠️ | Stub exporting `url()` (`undefined`) only; no Inspector protocol or `Session` ([details](#nodeinspector)) |
| node:inspector/promises | ✔︎ | ✘ | Not implemented |
| node:module | ✔︎ | ✔︎⚠️ | CJS facade and `SourceMap` without on-disk compile cache; missing `module.register`, `stripTypeScriptTypes`, and `Module._load` ([details](#nodemodule)) |
| node:net | ✔︎ | ✔︎⚠️ | `Socket.setTimeout` does not fire idle timeouts; `autoSelectFamily` flag is not wired to Happy-Eyeballs; several `Server` options and `maxConnections` ignored ([details](#nodenet)) |
| node:os | ✔︎ | ✔︎⚠️ | Missing `os.constants`; `setPriority` unsupported on Windows; `networkInterfaces()` reports IPv6 `scopeid: 0` ([details](#nodeos)) |
| node:path | ✔︎ | ✔︎ | — |
| node:perf_hooks | ✔︎ | ✔︎⚠️ | Static `nodeTiming` stub; no automatic GC/resource entries; timer-based ms `monitorEventLoopDelay`; fallback `eventLoopUtilization` ([details](#nodeperf_hooks)) |
| node:process | ✔︎ | ✔︎⚠️ | Missing `execPath`, `execArgv`, `ppid`, `uptime`, `memoryUsage`, `cpuUsage`, `resourceUsage`, `umask`, `loadEnvFile`, `report`, and `permission` ([details](#nodeprocess)) |
| node:querystring | ✔︎ | ✔︎ | — |
| node:readline | ✔︎ | ✔︎ | — |
| node:readline/promises | ✔︎ | ✔︎ | — |
| node:repl | ✔︎ | ✘ | Not implemented |
| node:sqlite | ✔︎ | ✔︎⚠️ | Targets Node 24.3 on SQLite 3.50.1; missing `createTagStore`, `setAuthorizer`, `enableDefensive`, `limits`, and async UDFs ([details](#nodesqlite)) |
| node:stream | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:stream/promises | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:stream/web | ✔︎ | ✔︎⚠️ | `TransformStream` rejects `readableType`/`writableType`; BYOB/transfer and piping WPT edge cases ([details](#nodestreamweb)) |
| node:string_decoder | ✔︎ | ✔︎ | — |
| node:test | ✔︎ | ✔︎\* | JS polyfill (not native) |
| node:timers | ✔︎ | ✔︎⚠️ | `Timeout` lacks `refresh`/`Symbol.dispose`; `Immediate` lacks `hasRef`/`Symbol.dispose`; no `enroll`/`unenroll` ([details](#nodetimers-and-nodetimerspromises)) |
| node:timers/promises | ✔︎ | ✔︎⚠️ | Same `Timeout`/`Immediate` handle limitations as `node:timers` ([details](#nodetimers-and-nodetimerspromises)) |
| node:tls | ✔︎ | ✔︎⚠️ | Rejects `pfx`/`ciphers`/`crl`/`psk`/`ocsp`/sessions/async SNI/soft mTLS; TLS < 1.2 unsupported; no `renegotiate()` or `TLSSocket.write()` backpressure ([details](#nodetls)) |
| node:tty | ✔︎ | ✔︎⚠️ | Non-stdio fd writes and Windows non-stdio TTY reads unsupported; no automatic `SIGWINCH` `resize` event ([details](#nodetty)) |
| node:url | ✔︎ | ✔︎⚠️ | Missing `URLPattern`; minor `url`-crate parser quirks ([details](#nodeurl)) |
| node:util | ✔︎ | ✔︎⚠️ | Missing `parseArgs` and `transferableAbort*`; `inspect` ignores options; `formatWithOptions` only honors `colors`; limited `TextDecoder` encodings ([details](#nodeutil-and-nodeutiltypes)) |
| node:util/types | ✔︎ | ✔︎⚠️ | Missing `isWebAssemblyCompiledModule`; `isExternal`, `isKeyObject`, and `isCryptoKey` always return `false` ([details](#nodeutil-and-nodeutiltypes)) |
| node:v8 | ✔︎ | ✔︎⚠️ | QuickJS memory stats (`"quickjs"` heap space); `setFlagsFromString` no-op; missing Serializer/Deserializer, heap snapshots, and `GCProfiler` ([details](#nodev8)) |
| node:vm | ✔︎ | ✔︎⚠️ | Exports `runInNewContext` only (no `Script`/`createContext`/modules); only `{ filename }` option; shared QuickJS runtime ([details](#nodevm)) |
| node:wasi | ✔︎ | ✘ | Not implemented |
| node:worker_threads | ✔︎ | ✔︎⚠️ | `moveMessagePortToContext` throws; missing `BroadcastChannel`, `SHARE_ENV`, `resourceLimits`, `getHeapSnapshot`, `postMessageToThread`, and piped worker stdio ([details](#nodeworker_threads)) |
| node:zlib | ✔︎ | ✔︎⚠️ | Missing `unzip*` and `crc32`; `flush` ignores flush kind; missing `params`/`close`/`bytesWritten` and `dictionary`/`windowBits`/`memLevel` ([details](#nodezlib)) |

| [lexe API](https://github.com/Ray-D-Song/lexe/blob/main/API.md) | Node.js | lexe |
| --- | --- | --- |
| lexe:hex | ✘ | ✔︎ |
| lexe:qjs | ✘ | ✔︎ |
| lexe:util | ✘ | ✔︎ |
| lexe:xml | ✘ | ✔︎ |

| [Web Platform API](https://min-common-api.proposal.wintertc.org/) | lexe | Key differences from standard / Node.js |
| --- | --- | --- |
| COMPRESSION | ✔︎⚠️ | Supports `gzip`/`deflate`/`deflate-raw` via `node:zlib`; inherits `node:stream/web` and `node:zlib` limits ([details](#compression)) |
| CONSOLE | ✔︎⚠️ | Missing `dirxml`, `profile`/`profileEnd`, `timeStamp`, plus `console-log-symbol` and `idlharness` WPT edge cases ([details](#console)) |
| DOM | ✔︎⚠️ | WinterTC `Event`/`EventTarget`/`AbortController`/`AbortSignal` subset only; no `ErrorEvent`, `CloseEvent`, or browser DOM ([details](#dom)) |
| ECMASCRIPT | ✔︎⚠️ | QuickJS-ng engine (31-bit `ArrayBuffer` limit, no `ShadowRealm`); V8 `Error.captureStackTrace`/`CallSite` fields are stub polyfills ([details](#ecmascript)) |
| ENCODING | ✔︎⚠️ | Supports `utf-8`, `utf-16le`, `utf-16be`, `windows-1252` (plus `hex`/`base64`) only; missing WHATWG legacy encodings ([details](#encoding)) |
| FETCH | ✔︎⚠️ | No `blob:`/`file:` URL fetch; omits browser policy controls (`credentials`, `referrer`, `integrity`, `caches`, CORS); WPT edge cases ([details](#fetch)) |
| FILEAPI | ✔︎⚠️ | `Blob`, `File`, and `URL.createObjectURL` only; missing `FileReader`/`FileReaderSync`/`FileList`; no `fetch("blob:...")` ([details](#fileapi)) |
| HR-TIME | ✔︎ | — |
| HTML | ✔︎⚠️ | `navigator` only exposes `userAgent`; `structuredClone` omits `DOMException`/`Blob`/`File`/`MessagePort`/`CryptoKey`; no `BroadcastChannel` ([details](#html)) |
| STREAMS | ✔︎⚠️ | Same limitations as `node:stream/web` ([details](#streams)) |
| URL | ✔︎ | — |
| URLPATTERN | ✘⏱ | Planned |
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
| Intl | ✔︎⚠️ | Minimal `Intl.DateTimeFormat`, `Intl.supportedValuesOf`, and `Date.prototype.toLocaleString` only; missing other `Intl.*` constructors and full ICU ([details](#intl)) |
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

#### `node:assert`

- Missing `assert.partialDeepStrictEqual()` and `assert.snapshot()`.
- `node:assert/strict` only exports `default`, so ESM named imports such as `import { strictEqual } from "node:assert/strict"` are unavailable.

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

#### `node:child_process`

- Missing `spawnSync()` (only `spawn`, `fork`, `exec`, `execFile`, `execSync`, and `execFileSync` are exported).
- `stdio` only accepts `"pipe"`, `"ignore"`, and `"inherit"` (no `"overlapped"`, raw numeric file descriptors, or arbitrary stream objects; fds beyond `0`/`1`/`2` are ignored outside `fork` IPC).
- `spawn` supports a limited option set (`cwd`, `env`, `stdio`, `shell`, `detached`, `uid`, `gid`, `windowsVerbatimArguments`); options such as `argv0`, `signal`, `timeout`, `killSignal`, `serialization`, and `windowsHide` are not implemented.
- Calling `ChildProcess.unref()` detaches the wait handle immediately, so the parent process may exit without receiving `exit`/`close` events.

#### `node:console`

- Missing `console.dirxml()`, `console.profile()`, `console.profileEnd()`, and `console.timeStamp()`.
- `new Console(options)` does not honor `colorMode`, `inspectOptions`, or `groupIndentation`.
- `console.dir(obj, options)` only honors `options.colors`; other `util.inspect` options (`depth`, `showHidden`, `maxArrayLength`, etc.) are ignored.

#### `node:constants`

- Exports only `F_OK`, `R_OK`, `W_OK`, and `X_OK`.
- Missing OS `errno` constants, `SIG*` signal constants, file open/mode constants (`O_*`, `S_*`), and crypto/OpenSSL constants.

#### `node:crypto`

- Missing classic Node.js crypto APIs: `createCipheriv`/`createDecipheriv`, `createSign`/`createVerify`, top-level `sign`/`verify`, `generateKeyPair`/`generateKeyPairSync`, `generateKey`/`generateKeySync`, `scrypt`/`scryptSync`, `pbkdf2`/`pbkdf2Sync`, `hkdf`/`hkdfSync`, `timingSafeEqual`, `KeyObject`, `createPrivateKey`/`createPublicKey`/`createSecretKey`, `privateDecrypt`/`privateEncrypt`/`publicDecrypt`, `X509Certificate`, `DiffieHellman`/`createDiffieHellman`, `ECDH`/`createECDH`, `getCiphers`, `getHashes`, and `crypto.hash()`. (Symmetric/asymmetric cipher, signature, and KDF operations are primarily provided through WebCrypto `subtle`.)
- `publicEncrypt` / RSA-OAEP rejects `MD5`, and algorithm availability depends on the compiled TLS/crypto backend (for example, `graviola` lacks most operations).

#### `node:dgram`

- Only supports unconnected UDP sockets (`createSocket`, `bind`, `send`, `close`, `address`, `ref`/`unref`).
- Missing connected UDP (`connect()`, `disconnect()`, `remoteAddress()`), broadcast/multicast/TTL controls (`setBroadcast`, `setTTL`, `setMulticastTTL`, `setMulticastLoopback`, `setMulticastInterface`, `addMembership`, `dropMembership`, `addSourceSpecificMembership`, `dropSourceSpecificMembership`), socket buffer controls (`getRecvBufferSize`/`setRecvBufferSize`, `getSendBufferSize`/`setSendBufferSize`), and `Symbol.asyncDispose`.

#### `node:diagnostics_channel`

- Missing `TracingChannel` and `tracingChannel()`.
- `Channel` does not support `AsyncLocalStorage` store binding (`bindStore()`, `unbindStore()`, `runStores()`, `hasStore`).
- Built-in runtime subsystems (such as HTTP, net, and DNS) do not emit Node's built-in diagnostic channels.
- The internal channel map holds strong references to channels, and subscriber exceptions during `publish()` are rethrown via `queueMicrotask` rather than `process.nextTick`.

#### `node:dns` and `node:dns/promises`

- Only `dns.lookup()` and `dns.promises.lookup()` (exposed by `node:dns/promises`) are implemented.
- Missing `Resolver`, `lookupService()`, `reverse()`, all `resolve*()` record methods (`resolve`, `resolve4`, `resolve6`, `resolveAny`, `resolveCaa`, `resolveCname`, `resolveMx`, `resolveNaptr`, `resolveNs`, `resolvePtr`, `resolveSoa`, `resolveSrv`, `resolveTxt`), `getServers()`/`setServers()`, `getDefaultResultOrder()`/`setDefaultResultOrder()`, and lookup flag constants (`ADDRCONFIG`, `V4MAPPED`, `ALL`).

#### `node:events`

- Missing `EventEmitterAsyncResource`.
- Automatic `captureRejections` handling (`{ captureRejections: true }` / `EventEmitter.captureRejections`) is not implemented; `captureRejectionSymbol` is exported without async rejection routing.

#### `node:fs` and `node:fs/promises`

- Missing in `node:fs`: callback `unlink` (only `unlinkSync` exists), callback `chmod` (only `chmodSync` and `fs/promises.chmod` exist), `copyFile`/`copyFileSync`, `cpSync`, `globSync`, `opendirSync`, `statfsSync`, callback `read`/`write` (only `readSync`/`writeSync` and `readv`/`writev` exist), `fchown`/`fchownSync`, `lchmod`/`lchmodSync`, sync fd variants (`fchmodSync`, `fdatasyncSync`, `fsyncSync`, `ftruncateSync`, `futimesSync`, `readvSync`, `writevSync`), `watchFile`/`unwatchFile`, and `openAsBlob`.
- Missing in `node:fs/promises`: `unlink`, `copyFile`, `watch`, `lchmod`, and `openAsBlob`.
- `{ bigint: true }` (`BigIntStats`) on `stat`/`lstat`/`fstat` is not supported and throws `TypeError`.
- `fs.cp` does not support the `{ filter }` option.
- `createReadStream`/`createWriteStream` do not support `fd`, `signal` (`AbortSignal`), custom `fs` providers, or `autoClose: false`.
- `FileHandle` does not support `position: -1`, and `FileHandle.utimes()` is unsupported.
- On Windows, `access`/`accessSync` approximate permissions from file metadata rather than ACLs, and `existsSync()` returns `false` on any metadata error (including permission errors).

#### `node:http` and `node:https`

- `ClientRequest` only exposes `write()`, `end()`, `destroy()`, `abort()`, `aborted`, `destroyed`, and `EventEmitter` methods; missing `setHeader()`, `getHeader()`, `getHeaderNames()`, `getHeaders()`, `hasHeader()`, `removeHeader()`, `flushHeaders()`, `setTimeout()`, `setNoDelay()`, `setSocketKeepAlive()`, `socket`, `reusedSocket`, `path`, `method`, `host`, `protocol`, and `maxHeadersCount`.
- `http.Server` rejects `insecureHTTPParser: true`, lacks `maxRequestsPerSocket`, `checkExpectation`, and `dropRequest` events, and does not support Unix domain sockets on Windows.
- `ServerResponse` lacks `writeEarlyHints()` and `strictContentLength`.
- `node:https` exports only `Agent`, `createServer`, `get`, and `request` (missing `Server` and `globalAgent`); `https.createServer()` throws unless built with a TLS feature (`tls-ring`, `tls-aws-lc`, `tls-graviola`, or `tls-openssl`), and inherits `node:http` and `node:tls` limitations.

#### `node:inspector`

- Stub module exporting only `url()`, which always returns `undefined` (to satisfy startup probes).
- No Inspector debugging protocol; `Session`, `open()`, `close()`, `waitForDebugger()`, and `console` are not implemented.

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

#### `node:net`

- `Socket.prototype.setTimeout(msecs, cb)` sets `this.timeout` and registers a `once("timeout")` listener, but does not schedule an idle timer or emit `"timeout"`.
- `Socket` lacks `resetAndDestroy()` and `destroySoon()`.
- `setDefaultAutoSelectFamily()` and `setDefaultAutoSelectFamilyAttemptTimeout()` persist the process-wide configuration flags, but Happy-Eyeballs multi-family dialing (`autoSelectFamily`) is not implemented.
- `net.Server` constructor honors `allowHalfOpen` only (ignoring `pauseOnConnect`, `noDelay`, `keepAlive`, `keepAliveInitialDelay`, `highWaterMark`); `server.listen()` ignores `backlog`, and `Server.maxConnections` is not implemented.
- Unix domain sockets are not supported on Windows.

#### `node:os`

- `os.constants` (`signals`, `errno`, `dlopen`, `priority`, `libuv`) is not exported.
- `os.setPriority()` is not implemented on Windows.
- `os.networkInterfaces()` always reports `scopeid: 0` for IPv6 addresses.

#### `node:perf_hooks`

- `performance.nodeTiming` is a static stub with placeholder values (`nodeStart: 0`, `v8Start: 0`, `bootstrapComplete: 0`, `environment: 0`, `loopStart: 0`, `loopExit: -1`, `idleTime: 0`).
- `PerformanceObserver.supportedEntryTypes` includes `"gc"` and `"resource"`, but Lexe does not automatically emit GC or HTTP/fetch resource timing entries (resource entries are only recorded by manual `performance.markResourceTiming()` calls); `performance.setResourceTimingBufferSize()` is a no-op.
- `monitorEventLoopDelay()` uses a JS `setInterval` measured in milliseconds instead of Node's native nanosecond libuv histogram.
- `createHistogram()` ignores `lowest`/`highest`/`figures` bucketing parameters and stores exact samples in an array.
- `performance.eventLoopUtilization()` falls back to `{ active: elapsed, idle: 0 }` because native active/idle event-loop counters are not wired in.

#### `node:process`

- Missing common Node.js properties and methods: `execPath`, `execArgv`, `ppid`, `uptime()`, `memoryUsage()` / `memoryUsage.rss()`, `cpuUsage()`, `resourceUsage()`, `constrainedMemory()`, `availableMemory()`, `umask()`, `title`, `config`, `features`, `allowedNodeEnvironmentFlags`, `report`, `permission`, `loadEnvFile()`, `getActiveResourcesInfo()`, `setUncaughtExceptionCaptureCallback()`, `abort()`, `finalization`, `ref()`/`unref()`, and `setSourceMapsEnabled()`.
- `process.version` (`v24.3.0`) and `process.versions.node` (`24.3.0`) advertise Node 24.3 compatibility while `process.release.name` is `"lexe"` and engine keys such as `v8`, `uv`, and `openssl` are absent from `process.versions`.

#### `node:sqlite`

- Targets the Node 24.3 API on SQLite 3.50.1 (enabled by default as an experimental feature; use `--no-experimental-sqlite` to disable it).
- Missing newer Node 24 SQLite APIs: `DatabaseSync.prototype.createTagStore()` / `SQLTagStore`, `setAuthorizer()`, `enableDefensive()` / `{ defensive }` option, and `limits`.
- Asynchronous user-defined SQL functions are not supported.

#### `node:stream/web`

- `TransformStream` rejects `readableType` and `writableType` options.
- WPT edge cases around non-transferable `ArrayBuffer` / BYOB transfer, owning-type byte streams, and some `pipeTo` / `TransformStream` cases are not yet compliant.

#### `node:timers` and `node:timers/promises`

- `Timeout` supports `ref()`, `unref()`, and `hasRef()`, but lacks `refresh()` and `Symbol.dispose`.
- `Immediate` lacks `hasRef()` and `Symbol.dispose`.
- Legacy `enroll()` and `unenroll()` helpers are not implemented.

#### `node:tls`

- Rejects (`ERR_TLS_OPTION_NOT_SUPPORTED`) `pfx`, `ciphers`, `sigalgs`, `dhparam`, `crl`, `psk`, `ocsp`, `session`, `ticket`, async `SNICallback`, `ALPNCallback`, soft mTLS (`requestCert: true` with `rejectUnauthorized: false`), and encrypted private keys on rustls backends (OpenSSL only).
- TLS versions below 1.2 are not supported.
- `TLSSocket.renegotiate()` fails with `"renegotiation not supported"`.
- `TLSSocket.write()` always returns `true` without backpressure or `"drain"` signaling.

#### `node:tty`

- Writing to arbitrary non-stdio file descriptors and reading from non-stdio TTY file descriptors on Windows are not supported.
- Automatic `SIGWINCH`-driven `"resize"` events on `WriteStream` are not hooked into process signal handling (only explicit `refreshSize()` emits `"resize"`).

#### `node:url`

- `URLPattern` is not implemented (tracked separately under Web Platform APIs).
- `URL` has minor `url`-crate parser quirks (for example, negative port handling).

#### `node:util` and `node:util/types`

- `node:util` missing exports: `parseArgs()`, `transferableAbortController()`, `transferableAbortSignal()`, and `inspect.styles` / `inspect.defaultOptions`.
- `util.inspect(value)` ignores its `options` argument, and `util.formatWithOptions(options, ...args)` honors only `options.colors` (ignoring `depth`, `showHidden`, `maxArrayLength`, `compact`, `sorted`, etc.).
- `util.styleText()` does not support hex/RGB truecolor formats.
- `util.TextDecoder` supports only `utf-8`, `utf-16le`, `utf-16be`, and `windows-1252` (plus `hex`/`base64`), lacking full WHATWG legacy encodings.
- `node:util/types` omits `isWebAssemblyCompiledModule()`, and `isExternal()`, `isKeyObject()`, and `isCryptoKey()` always return `false` (even for WebCrypto `CryptoKey` objects) because their host hooks are not registered.

#### `node:v8`

- Backed by QuickJS `JS_ComputeMemoryUsage` rather than V8: `getHeapSpaceStatistics()` always returns a single `"quickjs"` space, and `heap_size_limit` falls back to `Number.MAX_SAFE_INTEGER` when `malloc_limit === 0`.
- `setFlagsFromString()` is a no-op.
- Missing serialization (`serialize`, `deserialize`, `Serializer`, `Deserializer`, `DefaultSerializer`, `DefaultDeserializer`), heap snapshots (`writeHeapSnapshot`, `getHeapSnapshot`), `GCProfiler`, `queryObjects`, `startupSnapshot`, `cachedDataVersionTag`, and coverage APIs (`takeCoverage`, `stopCoverage`).

#### `node:vm`

- Only `vm.runInNewContext()` is exported; missing `Script`, `createContext()`, `isContext()`, `runInContext()`, `runInThisContext()`, `compileFunction()`, `measureMemory()`, `SourceTextModule`, and `SyntheticModule`.
- `runInNewContext()` supports `{ filename }` only (rejecting `timeout`, `breakOnSigint`, `contextCodeGeneration`, `microtaskMode`, `lineOffset`/`columnOffset`, `cachedData`, etc.).
- Sandbox synchronization copies own enumerable string-keyed properties only (no symbols, non-enumerable properties, or property descriptors), and scripts run inside the same QuickJS runtime (not a security boundary).

#### `node:worker_threads`

- Uses **OS-thread worker isolates** (QuickJS per thread) with structured-clone `postMessage`, in-process `MessageChannel`, environment data sync, and **SharedArrayBuffer** hooks for shared memory within the process.
- `moveMessagePortToContext()` throws (not implemented).
- Missing `BroadcastChannel`, `SHARE_ENV`, `resourceLimits`, `Worker.prototype.getHeapSnapshot()`, `postMessageToThread()`, and piped worker stdio (`stdin: true`, `stdout: true`, `stderr: true`).

#### `node:zlib`

- Missing `unzip` / `unzipSync` / `createUnzip` / `Unzip` (auto-detecting gzip/deflate) and `zlib.crc32()`.
- Streaming `flush(kind, cb)` ignores the flush kind and completes immediately.
- Missing stream methods and options: `params()`, `close()`, `bytesWritten`, `dictionary`, `windowBits`, and `memLevel`.

### Web Platform APIs

#### `COMPRESSION`

- Supports `CompressionStream` and `DecompressionStream` for `gzip`, `deflate`, and `deflate-raw` via `node:zlib`, inheriting `node:stream/web` and `node:zlib` streaming limitations.

#### `CONSOLE`

- Missing WHATWG Console methods `dirxml()`, `profile()`, `profileEnd()`, and `timeStamp()`, plus `console-log-symbol` and `idlharness` WPT edge cases.

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

#### `STREAMS`

- Same limitations as [`node:stream/web`](#nodestreamweb) (`TransformStream` rejects `readableType`/`writableType`, plus BYOB/transfer and piping WPT edge cases).

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

- Implements only minimal `Intl.DateTimeFormat` (`format`, `formatToParts`, `resolvedOptions`), `Intl.supportedValuesOf()`, and `Date.prototype.toLocaleString` with `timeZone`.
- Missing `Intl.Collator`, `Intl.NumberFormat`, `Intl.PluralRules`, `Intl.RelativeTimeFormat`, `Intl.ListFormat`, `Intl.DisplayNames`, `Intl.Segmenter`, `Intl.Locale`, `Intl.DurationFormat`, and full ICU locale/calendar/numbering-system support.

#### `Temporal`

- Implements `Duration`, `Instant`, `PlainDate`, `PlainDateTime`, `PlainTime`, `ZonedDateTime`, and `Now` (backed by `jiff`).
- Missing `PlainYearMonth`, `PlainMonthDay`, non-ISO calendar systems (ISO-8601 only), and full `Intl`-backed `toLocaleString()` formatting.

## Other notes

### `node:http2`

- Implemented natively: `createServer`, `createSecureServer`, `connect`, settings helpers, and constants.
- Browser builds may still use the embedded load-time stub when the native module is unavailable.

### Native addons

- **N-API** native addon (`.node`) loading is supported when Lexe is built with `--features napi` on a dynamically linked target.
- **V8 C++ ABI** addons targeting Node 24 (e.g. `better-sqlite3`) are supported with `--features v8-compat` through a limited QuickJS-backed V8 shim.
