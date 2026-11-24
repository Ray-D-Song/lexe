# raster_runtime

raster_runtime is a lightweight JavaScript runtime built on QuickJS.

It takes Node.js compatibility as its primary goal, aiming to become a lightweight direct drop-in replacement for Node.js. Its compatibility matrix includes Wasm, N-API, and V8-API.

It's built in Rust, utilizing QuickJS as the JavaScript engine, ensuring efficient memory usage and swift startup.

## Install

```shell
# Linux
curl -fsSL https://raw.githubusercontent.com/Ray-D-Song/raster/main/install-linux.sh | sh

# macOS
curl -fsSL https://raw.githubusercontent.com/Ray-D-Song/raster/main/install-macos.sh | sh
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/Ray-D-Song/raster/main/install-windows.ps1 | iex
```

## Quick start

```shell
printf 'console.log("Hello from Raster")\n' > main.mjs
raster ./main.mjs
raster --version
```

### Test runner

Test runner uses a lightweight Jest-like API and supports Jest/Chai assertions. For examples on how to implement tests for raster_runtime see the `/tests` folder of this repository.

To run tests, execute the `raster_runtime test` command. raster_runtime scans the current directory and sub-directories for files that ends with `*.test.js` or `*.test.mjs`. You can also provide a specific test directory to scan by using the `raster_runtime test -d <directory>` option.

The test runner also has support for filters. Using filters is as simple as adding additional command line arguments, i.e: `raster_runtime test crypto` will only run tests that match the filename containing `crypto`.

## Compatibility matrix

> [!NOTE]
> Below is a high level overview of partially supported APIs and modules. For more details consult the [API](API.md) documentation

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

See [`compat/README.md`](compat/README.md) for exact fixture scope, commands, exclusions, and diagnostics.

_⚠️ = partially supported in RASTER_RUNTIME_<br />
_⏱ = planned partial support_<br />
_\* = Not native_<br />
_`node:http2` is implemented natively (`createServer`, `createSecureServer`, `connect`, settings helpers, and constants). Browser builds may still use the embedded load-time stub when the native module is unavailable._<br />
_`node:sqlite` targets the Node 24.3 API on SQLite 3.50.1 and is enabled by default as an experimental feature; use `--no-experimental-sqlite` to disable it._<br />
_`node:worker_threads` uses **OS-thread worker isolates** (QuickJS per thread), structured-clone `postMessage`, in-process `MessageChannel`, environment sync, and **SharedArrayBuffer** hooks for shared memory within the process._
_`node:module` provides a CommonJS loader facade (`Module`, `require.resolve`, `require.cache`, `require.extensions`, writable `_resolveFilename` / `_nodeModulePaths`, and `Module._compile` for require-hook compatibility), plus `SourceMap`, `findSourceMap`, `getSourceMapsSupport` / `setSourceMapsSupport`, `findPackageJSON`, `syncBuiltinESMExports`, and `constants`. `enableCompileCache` validates its argument and reports `constants.compileCacheStatus.FAILED` (there is no on-disk compile cache); `flushCompileCache` is a no-op and `getCompileCacheDir` returns `undefined`. `findSourceMap` does not rewrite `Error.prepareStackTrace`; while support is enabled it reads a file's source map on demand. `module.register` and `stripTypeScriptTypes` are not implemented. Extension hooks apply to the CommonJS loader only; they do not change static ESM `import` semantics. **N-API** native addon (`.node`) loading is supported when Raster is built with `--features napi` on a dynamically linked target. V8 C++ ABI addons targeting Node 24 (e.g. `better-sqlite3`) are supported with `--features v8-compat` through a limited QuickJS-backed V8 shim._<br />

## Using node_modules (dependencies) with raster_runtime

Since raster_runtime is meant for performance critical application it's not recommended to deploy `node_modules` without bundling, minification and tree-shaking.

raster_runtime can work with any bundler of your choice. Below are some configurations for popular bundlers:

> [!WARNING]
> raster_runtime implements native modules that are largely compatible with the following external packages.
> By implementing the following conversions in the bundler's alias function, your application may be faster, but we recommend that you test thoroughly as they are not fully compatible.

| Node.js         | raster_runtime     |
| --------------- | ------------------ |
| fast-xml-parser | raster_runtime:xml |

### ESBuild

```shell
esbuild index.js --platform=browser --target=es2023 --format=esm --bundle --minify
```

### Rollup

```javascript
import resolve from "@rollup/plugin-node-resolve";
import commonjs from "@rollup/plugin-commonjs";
import terser from "@rollup/plugin-terser";

export default {
  input: "index.js",
  output: {
    file: "dist/bundle.js",
    format: "esm",
    sourcemap: true,
    target: "es2023",
  },
  plugins: [resolve(), commonjs(), terser()],
};
```

## Running TypeScript with raster_runtime

Same principle as dependencies applies when using TypeScript. TypeScript must be bundled and transpiled into ES2023 JavaScript.

> [!NOTE]
> raster_runtime will not support running TypeScript without transpilation. This is by design for performance reasons. Transpiling requires CPU and memory that adds latency and cost during execution. This can be avoided if done ahead of time during deployment.

## Rationale

What justifies the introduction of another JavaScript runtime in light of existing options such as [Node.js](https://nodejs.org/en), [Bun](https://bun.sh) & [Deno](https://deno.com/)?

Node.js, Bun, and Deno represent highly proficient JavaScript runtimes. However, they are designed with general-purpose applications in mind and depend on a ([Just-In-Time compiler (JIT)](https://en.wikipedia.org/wiki/Just-in-time_compilation) for dynamic code compilation and optimization during execution. While JIT compilation offers substantial long-term performance advantages, it carries a computational and memory overhead.

In contrast, raster_runtime distinguishes itself by not incorporating a JIT compiler, a strategic decision that yields two significant advantages:

A) JIT compilation is a notably sophisticated technological component, introducing increased system complexity and contributing substantially to the runtime's overall size.

B) Without the JIT overhead, raster_runtime conserves both CPU and memory resources that can be more efficiently allocated to code execution tasks, thereby reducing application startup times.

## Limitations

There are many cases where raster_runtime shows notable performance drawbacks compared with JIT-powered runtimes, such as large data processing, Monte Carlo simulations or performing tasks with hundreds of thousands or millions of iterations. raster_runtime is most effective when applied to smaller programs such as data transformation, real time processing, authorization, and validation. It is designed to complement existing components rather than serve as a comprehensive replacement for everything. Notably, given its supported APIs are based on Node.js specification, transitioning back to alternative solutions requires minimal code adjustments.

## Crypto and TLS Backend Options

raster_runtime supports multiple cryptographic backends for both the crypto module and TLS connections. These can be configured via Cargo features.

### Crypto Provider Features

| Feature                 | Description                                                       |
| ----------------------- | ----------------------------------------------------------------- |
| `crypto-rust` (default) | Pure Rust crypto using RustCrypto crates                          |
| `crypto-ring`           | Ring-only crypto (limited algorithm support)                      |
| `crypto-ring-rust`      | Ring for hashing/HMAC, RustCrypto for everything else             |
| `crypto-graviola`       | Graviola-only crypto (limited algorithm support)                  |
| `crypto-graviola-rust`  | Graviola for hashing/HMAC/AES-GCM, RustCrypto for everything else |
| `crypto-openssl`        | OpenSSL-based crypto                                              |

### TLS Backend Features

| Feature              | Description                 |
| -------------------- | --------------------------- |
| `tls-ring` (default) | rustls with ring crypto     |
| `tls-graviola`       | rustls with graviola crypto |
| `tls-openssl`        | OpenSSL for TLS             |

`node:tls` is implemented as a **partial** Node core subset (client/server, `TLSSocket`, `SecureContext`, STARTTLS handoff for `net.Socket`, TLS 1.2/1.3, ALPN, SNI, mTLS). Not supported: session resume/tickets, PFX, custom ciphers/sigalgs, OCSP, renegotiation, PSK, and other advanced options (these throw `ERR_TLS_OPTION_NOT_SUPPORTED`). HTTP/HTTPS/Fetch continue to use the internal `BuildClientConfigOptions` / `TLS_CONFIG` Rust API unchanged.

### Building with Different Backends

```bash
# Default (crypto-rust + tls-ring)
cargo build --release

# Using OpenSSL for both crypto and TLS
cargo build --release --no-default-features --features "macro,crypto-openssl,tls-openssl"

# Using Graviola for both crypto and TLS
cargo build --release --no-default-features --features "macro,crypto-graviola-rust,tls-graviola"
```

## Environment Variables

### `RASTER_RUNTIME_ASYNC_HOOKS=value`

Controls **public** `async_hooks.createHook()` user callbacks. When unset, user hook callbacks do not run.

`AsyncLocalStorage` context propagation (Promises, `await`, microtasks, timers) works **without** this variable. Internal async tracking is enabled automatically while ALS instances are active.

Set to `1` to enable user-facing `createHook()` lifecycle callbacks (init/before/after/destroy/promiseResolve).

### `RASTER_RUNTIME_EXTRA_CA_CERTS=file`

Load extra certificate authorities from a PEM encoded file

### `RASTER_RUNTIME_GC_THRESHOLD_MB=value`

Set a memory threshold in MB for garbage collection. Default threshold is 20MB

### `RASTER_RUNTIME_HTTP_VERSION=value`

Extends the HTTP request version. By default, only HTTP/1.1 is enabled. Specifying '2' will enable HTTP/1.1 and HTTP/2.

### `RASTER_RUNTIME_LOG=[target][=][level][,...]`

Filter the log output by target module, level, or both (using `=`). Log levels are case-insensitive and will also enable any higher priority logs.

Log levels in descending priority order:

- `Error`
- `Warn | Warning`
- `Info`
- `Debug`
- `Trace`

Example filters:

- `warn` will enable all warning and error logs
- `raster_runtime_core::vm=trace` will enable all logs in the `raster_runtime_core::vm` module
- `warn,raster_runtime_core::vm=trace` will enable all logs in the `raster_runtime_core::vm` module and all warning and error logs in other modules

### `RASTER_RUNTIME_NET_ALLOW="host[ ...]"`

Space-delimited list of hosts or socket paths which should be allowed for network connections. Network connections will be denied for any host or socket path missing from this list. Set an empty list to deny all connections

### `RASTER_RUNTIME_NET_DENY="host[ ...]"`

Space-delimited list of hosts or socket paths which should be denied for network connections

### `RASTER_RUNTIME_NET_POOL_IDLE_TIMEOUT=value`

Set a timeout in seconds for idle sockets being kept-alive. Default timeout is 15 seconds

### `RASTER_RUNTIME_PLATFORM=value`

Used to explicitly specify a preferred platform for the Node.js package resolver. The default is `browser`. If `node` is specified, "node" takes precedence in the search path. If a value other than `browser` or `node` is specified, it will behave as if "browser" was specified.

### `RASTER_RUNTIME_REGISTER_HOOKS=file`

If you want to enable a hooking mechanism that is mostly compatible with Node.js's `module.registerHooks()`, specify the js file name in this environment variable.

We provide a concrete example in `example/register-hooks`. Hook files can also be specified using the `--import` option.

### `RASTER_RUNTIME_TLS_VERSION=value`

Set the TLS version to be used for network connections. By default only TLS 1.2 is enabled. TLS 1.3 can also be enabled by setting this variable to `1.3`

## License

This library is licensed under the Apache-2.0 License. See the [LICENSE](LICENSE) file.
