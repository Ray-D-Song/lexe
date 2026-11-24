# Environment Variables

### `LEXE_ASYNC_HOOKS=value`

Controls **public** `async_hooks.createHook()` user callbacks. When unset, user hook callbacks do not run.

`AsyncLocalStorage` context propagation (Promises, `await`, microtasks, timers) works **without** this variable. Internal async tracking is enabled automatically while ALS instances are active.

Set to `1` to enable user-facing `createHook()` lifecycle callbacks (init/before/after/destroy/promiseResolve).

### `LEXE_BIN_DIR=dir`

Directory containing prebuilt target-platform `lexe` runtime binaries used by `lexe compile --executable -p ...` before consulting the local download cache.

### `LEXE_BINARY_PATH=path`

Path to a local `lexe` runtime binary used by the `lexe` npm CLI wrapper instead of downloading a release binary.

### `LEXE_CACHE_DIR=dir`

Override the cache directory where `lexe compile --executable` and the `lexe` npm CLI wrapper store auto-downloaded release binaries (defaults to `$XDG_CACHE_HOME/lexe`, `~/.cache/lexe`, or `%LOCALAPPDATA%\lexe\cache`).

### `LEXE_EXTRA_CA_CERTS=file`

Load extra certificate authorities from a PEM encoded file

### `LEXE_GC_THRESHOLD_MB=value`

Set a memory threshold in MB for garbage collection. Default threshold is 20MB

### `LEXE_HTTP_VERSION=value`

Extends the HTTP request version. By default, only HTTP/1.1 is enabled. Specifying '2' will enable HTTP/1.1 and HTTP/2.

### `LEXE_INSTALL_DIR=dir`

Installation directory used by `install-linux.sh`, `install-macos.sh`, and `install-windows.ps1`.

### `LEXE_LOG=[target][=][level][,...]`

Filter the log output by target module, level, or both (using `=`). Log levels are case-insensitive and will also enable any higher priority logs.

Log levels in descending priority order:

- `Error`
- `Warn | Warning`
- `Info`
- `Debug`
- `Trace`

Example filters:

- `warn` will enable all warning and error logs
- `lexe_core::vm=trace` will enable all logs in the `lexe_core::vm` module
- `warn,lexe_core::vm=trace` will enable all logs in the `lexe_core::vm` module and all warning and error logs in other modules

### `LEXE_NET_ALLOW="host[ ...]"`

Space-delimited list of hosts or socket paths which should be allowed for network connections. Network connections will be denied for any host or socket path missing from this list. Set an empty list to deny all connections

### `LEXE_NET_DENY="host[ ...]"`

Space-delimited list of hosts or socket paths which should be denied for network connections

### `LEXE_NET_POOL_IDLE_TIMEOUT=value`

Set a timeout in seconds for idle sockets being kept-alive. Default timeout is 15 seconds

### `LEXE_PLATFORM=value`

Used to explicitly specify a preferred platform for the Node.js package resolver. The default is `browser`. If `node` is specified, "node" takes precedence in the search path. If a value other than `browser` or `node` is specified, it will behave as if "browser" was specified.

### `LEXE_REGISTER_HOOKS=file`

If you want to enable a hooking mechanism that is mostly compatible with Node.js's `module.registerHooks()`, specify the js file name in this environment variable.

We provide a concrete example in `example/register-hooks`. Hook files can also be specified using the `--import` option.

### `LEXE_RELEASE_BASE_URL=url`

Override the base URL used by `lexe compile --executable`, `lexe upgrade`, and the `lexe` npm CLI wrapper to fetch `SHA256SUMS` and prebuilt runtime assets (defaults to `https://github.com/Ray-D-Song/lexe/releases/download/v<VERSION>`).

### `LEXE_SKIP_DOWNLOAD=value`

Set to `1` or `true` to skip pre-downloading the native `lexe` binary during `npm install` (the npm wrapper will download it on first run if needed).

### `LEXE_TLS_VERSION=value`

Set the TLS version to be used for network connections. By default only TLS 1.2 is enabled. TLS 1.3 can also be enabled by setting this variable to `1.3`
