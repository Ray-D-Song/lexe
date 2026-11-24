# Compatibility matrix

> [!NOTE]
> Below is a high level overview of partially supported APIs and modules. For more details consult the [API](../API.md) documentation

| [Node.js API](https://nodejs.org/api/index.html) | Node.js | lexe |
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

| [lexe API](https://github.com/Ray-D-Song/lexe/blob/main/API.md) | Node.js | lexe |
| ----------------------------------------------------------------------------------- | ------- | -------------- |
| lexe:hex                                                                  | ✘       | ✔︎              |
| lexe:qjs                                                                  | ✘       | ✔︎              |
| lexe:util                                                                 | ✘       | ✔︎              |
| lexe:xml                                                                  | ✘       | ✔︎              |

| [Web Platform API](https://min-common-api.proposal.wintertc.org/) | lexe |
| ----------------------------------------------------------------- | -------------- |
| COMPRESSION                                                       | ✔︎⚠️            |
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

| Other features | lexe |
| -------------- | -------------- |
| async/await    | ✔︎              |
| esm            | ✔︎              |
| cjs            | ✔︎              |
| Intl           | ✔︎⚠️            |
| Temporal       | ✔︎⚠️            |

### Database driver compatibility

| Package              | Tested versions              | Local acceptance                                                                                                                      | CI policy                                                                              |
| -------------------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `mysql2`             | mysql2 3.23.2 / MySQL 8.4    | Node and Lexe pass the Promise, callback, transaction, error, and pool fixture                                                      | Blocking                                                                               |
| `pg` (node-postgres) | pg 8.22.0 / PostgreSQL 16.14 | Node and Lexe pass all 25 SCRAM, query, pool, transaction, TLS, cancellation, recovery, notification, shutdown, and `.pgpass` cases | Node baseline and infrastructure are blocking; Lexe failures currently emit warnings |

See [`compat/README.md`](../compat/README.md) for exact fixture scope, commands, exclusions, and diagnostics.

**Legend**

- ⚠️ = partially supported in Lexe
- ⏱ = planned partial support
- \* = Not native

## Notes

### `node:http2`

- Implemented natively: `createServer`, `createSecureServer`, `connect`, settings helpers, and constants.
- Browser builds may still use the embedded load-time stub when the native module is unavailable.

### `node:sqlite`

- Targets the Node 24.3 API on SQLite 3.50.1.
- Enabled by default as an experimental feature; use `--no-experimental-sqlite` to disable it.

### `node:worker_threads`

- Uses **OS-thread worker isolates** (QuickJS per thread).
- Supports structured-clone `postMessage`, in-process `MessageChannel`, environment sync, and **SharedArrayBuffer** hooks for shared memory within the process.

### `node:module`

- Provides a CommonJS loader facade:
  - `Module`, `require.resolve`, `require.cache`, `require.extensions`
  - writable `_resolveFilename` / `_nodeModulePaths`
  - `Module._compile` for require-hook compatibility
- Also provides `SourceMap`, `findSourceMap`, `getSourceMapsSupport` / `setSourceMapsSupport`, `findPackageJSON`, `syncBuiltinESMExports`, and `constants`.
- Compile cache:
  - `enableCompileCache` validates its argument and reports `constants.compileCacheStatus.FAILED` (there is no on-disk compile cache).
  - `flushCompileCache` is a no-op.
  - `getCompileCacheDir` returns `undefined`.
- Source maps: `findSourceMap` does not rewrite `Error.prepareStackTrace`; while support is enabled it reads a file's source map on demand.
- Not implemented: `module.register` and `stripTypeScriptTypes`.
- Extension hooks apply to the CommonJS loader only; they do not change static ESM import semantics.

### Native addons

- **N-API** native addon (`.node`) loading is supported when Lexe is built with `--features napi` on a dynamically linked target.
- **V8 C++ ABI** addons targeting Node 24 (e.g. `better-sqlite3`) are supported with `--features v8-compat` through a limited QuickJS-backed V8 shim.
