# Environment Variables

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
