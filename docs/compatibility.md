# Compatibility matrix

> [!NOTE]
> Below is a high level overview of partially supported APIs and modules. For more details consult the [API](../API.md) documentation

| [Node.js API](https://nodejs.org/api/index.html) | Node.js | raster_runtime |
| ------------------------------------------------ | ------- | -------------- |
| node:assert                                      | ✔︎       | ✔︎️⚠️            |
| node:async_hooks                                 | ✔︎       | ✔︎️⚠️            |
| node:buffer                                      | ✔︎       | ✔︎️⚠️            |
| node:child_process                               | ✔︎       | ✔︎⚠️            |
| node:cluster                                     | ✔︎       | ✘              |
| node:console                                     | ✔︎       | ✔︎⚠️            |
| node:constants                                   | ✔︎       | ✔︎⚠️            |
| node:crypto                                      | ✔︎       | ✔︎⚠️            |
| node:dgram                                       | ✔︎       | ✔︎⚠️            |
| node:diagnostics_channel                         | ✔︎       | ✔︎⚠️            |
| node:dns                                         | ✔︎       | ✔︎⚠️            |
| node:dns/promises                                | ✔︎       | ✔︎⚠️            |
| node:events                                      | ✔︎       | ✔︎⚠️            |
| node:fs                                          | ✔︎       | ✔︎⚠️            |
| node:fs/promises                                 | ✔︎       | ✔︎⚠️            |
| node:http                                        | ✔︎       | ✔︎⚠️            |
| node:http2                                       | ✔︎       | ✔︎            |
| node:https                                       | ✔︎       | ✔︎⚠️            |
| node:inspector                                   | ✔︎       | ✔︎⚠️            |
| node:inspector/promises                          | ✔︎       | ✘              |
| node:module                                      | ✔︎       | ✔︎⚠️            |
| node:net                                         | ✔︎       | ✔︎⚠️            |
| node:os                                          | ✔︎       | ✔︎⚠️            |
| node:path                                        | ✔︎       | ✔︎⚠️            |
| node:perf_hooks                                  | ✔︎       | ✔︎⚠️            |
| node:process                                     | ✔︎       | ✔︎⚠️            |
| node:querystring                                 | ✔︎       | ✔︎⚠️            |
| node:readline                                    | ✔︎       | ✔︎              |
| node:readline/promises                           | ✔︎       | ✔︎              |
| node:repl                                        | ✔︎       | ✘              |
| node:sqlite                                      | ✔︎       | ✔︎⚠️            |
| node:stream                                      | ✔︎       | ✔︎\*            |
| node:stream/promises                             | ✔︎       | ✔︎\*            |
| node:stream/web                                  | ✔︎       | ✔︎⚠️            |
| node:string_decoder                              | ✔︎       | ✔︎              |
| node:test                                        | ✔︎       | ✔︎\*            |
| node:timers                                      | ✔︎       | ✔︎⚠️            |
| node:timers/promises                             | ✔︎       | ✔︎⚠️            |
| node:tls                                         | ✔︎       | ✔︎⚠️            |
| node:tty                                         | ✔︎       | ✔︎⚠️            |
| node:url                                         | ✔︎       | ✔︎⚠️            |
| node:util                                        | ✔︎       | ✔︎⚠️            |
| node:util/types                                  | ✔︎       | ✔︎⚠️            |
| node:v8                                          | ✔︎       | ✔︎⚠️            |
| node:vm                                          | ✔︎       | ✔︎⚠️            |
| node:wasi                                        | ✔︎       | ✘              |
| node:worker_threads                              | ✔︎       | ✔︎⚠️            |
| node:zlib                                        | ✔︎       | ✔︎⚠️            |

| [raster_runtime API](https://github.com/Ray-D-Song/raster/blob/main/API.md) | Node.js | raster_runtime |
| ----------------------------------------------------------------------------------- | ------- | -------------- |
| raster_runtime:hex                                                                  | ✘       | ✔︎              |
| raster_runtime:qjs                                                                  | ✘       | ✔︎              |
| raster_runtime:util                                                                 | ✘       | ✔︎              |
| raster_runtime:xml                                                                  | ✘       | ✔︎              |

| [Web Platform API](https://min-common-api.proposal.wintertc.org/) | raster_runtime |
| ----------------------------------------------------------------- | -------------- |
| COMPRESSION                                                       | ✘⏱             |
| CONSOLE                                                           | ✔︎⚠️            |
| DOM                                                               | ✔︎⚠️            |
| ECMASCRIPT                                                        | ✔︎⚠️            |
| ENCODING                                                          | ✔︎⚠️            |
| FETCH                                                             | ✔︎⚠️            |
| FILEAPI                                                           | ✔︎⚠️            |
| HR-TIME                                                           | ✔︎              |
| HTML                                                              | ✔︎⚠️            |
| STREAMS                                                           | ✔︎⚠️            |
| URL                                                               | ✔︎              |
| URLPATTERN                                                        | ✘⏱             |
| WASM-JS-API-2                                                     | ✔︎⚠️            |
| WASM-WEB-API-2                                                    | ✔︎⚠️            |
| WEBCRYPTO                                                         | ✔︎⚠️            |
| WEBIDL                                                            | ✔︎⚠️            |
| XHR                                                               | ✔︎⚠️            |

| Other features | raster_runtime |
| -------------- | -------------- |
| async/await    | ✔︎              |
| esm            | ✔︎              |
| cjs            | ✔︎              |
| Intl           | ✔︎⚠️            |
| Temporal       | ✔︎⚠️            |

### Database driver compatibility

| Package              | Tested versions              | Local acceptance                                                                                                                      | CI policy                                                                              |
| -------------------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `mysql2`             | mysql2 3.23.2 / MySQL 8.4    | Node and Raster pass the Promise, callback, transaction, error, and pool fixture                                                      | Blocking                                                                               |
| `pg` (node-postgres) | pg 8.22.0 / PostgreSQL 16.14 | Node and Raster pass all 25 SCRAM, query, pool, transaction, TLS, cancellation, recovery, notification, shutdown, and `.pgpass` cases | Node baseline and infrastructure are blocking; Raster failures currently emit warnings |

See [`compat/README.md`](../compat/README.md) for exact fixture scope, commands, exclusions, and diagnostics.

_⚠️ = partially supported in RASTER_RUNTIME_<br />
_⏱ = planned partial support_<br />
_\* = Not native_<br />
_`node:http2` is implemented natively (`createServer`, `createSecureServer`, `connect`, settings helpers, and constants). Browser builds may still use the embedded load-time stub when the native module is unavailable._<br />
_`node:sqlite` targets the Node 24.3 API on SQLite 3.50.1 and is enabled by default as an experimental feature; use `--no-experimental-sqlite` to disable it._<br />
_`node:worker_threads` uses **OS-thread worker isolates** (QuickJS per thread), structured-clone `postMessage`, in-process `MessageChannel`, environment sync, and **SharedArrayBuffer** hooks for shared memory within the process._
_`node:module` provides a CommonJS loader facade (`Module`, `require.resolve`, `require.cache`, `require.extensions`, writable `_resolveFilename` / `_nodeModulePaths`, and `Module._compile` for require-hook compatibility), plus `SourceMap`, `findSourceMap`, `getSourceMapsSupport` / `setSourceMapsSupport`, `findPackageJSON`, `syncBuiltinESMExports`, and `constants`. `enableCompileCache` validates its argument and reports `constants.compileCacheStatus.FAILED` (there is no on-disk compile cache); `flushCompileCache` is a no-op and `getCompileCacheDir` returns `undefined`. `findSourceMap` does not rewrite `Error.prepareStackTrace`; while support is enabled it reads a file's source map on demand. `module.register` and `stripTypeScriptTypes` are not implemented. Extension hooks apply to the CommonJS loader only; they do not change static ESM `import` semantics. **N-API** native addon (`.node`) loading is supported when Raster is built with `--features napi` on a dynamically linked target. V8 C++ ABI addons targeting Node 24 (e.g. `better-sqlite3`) are supported with `--features v8-compat` through a limited QuickJS-backed V8 shim._<br />
