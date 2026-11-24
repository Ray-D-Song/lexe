# API documentation

> [!NOTE]
> The long term goal for raster_runtime is to become [WinterTC compliant](https://min-common-api.proposal.wintertc.org/). Not every API from Node.js will be supported.

# Node.js API

## assert

[ok](https://nodejs.org/api/assert.html#assertokvalue-message)

## async_hooks

### Static methods

[createHook](https://nodejs.org/api/async_hooks.html#async_hookscreatehookcallbacks)

[currentId](https://nodejs.org/api/async_hooks.html#async_hooksexecutionasyncid)

[executionAsyncId](https://nodejs.org/api/async_hooks.html#async_hooksexecutionasyncid)

[triggerAsyncId](https://nodejs.org/api/async_hooks.html#async_hookstriggerasyncid)

[AsyncResource.bind](https://nodejs.org/api/async_context.html#static-method-asyncresourcebindfn-type-thisarg)

### Class: AsyncHook

[enable](https://nodejs.org/api/async_hooks.html#asynchookenable)

[disable](https://nodejs.org/api/async_hooks.html#asynchookdisable)

#### Hook callbacks

[init](https://nodejs.org/api/async_hooks.html#initasyncid-type-triggerasyncid-resource)

[before](https://nodejs.org/api/async_hooks.html#beforeasyncid)

[after](https://nodejs.org/api/async_hooks.html#afterasyncid)

[destroy](https://nodejs.org/api/async_hooks.html#destroyasyncid)

[promiseResolve](https://nodejs.org/api/async_hooks.html#promiseresolveasyncid)

### Class: AsyncResource

[constructor](https://nodejs.org/api/async_context.html#new-asyncresourcetype-options)

[runInAsyncScope](https://nodejs.org/api/async_context.html#asyncresourceruninasyncscopefn-thisarg-args)

[emitDestroy](https://nodejs.org/api/async_context.html#asyncresourceemitdestroy)

[asyncId](https://nodejs.org/api/async_context.html#asyncresourceasyncid)

[triggerAsyncId](https://nodejs.org/api/async_context.html#asyncresourcetriggerasyncid)

[bind](https://nodejs.org/api/async_context.html#asyncresourcebindfn-thisarg)

### Class: AsyncLocalStorage

[constructor](https://nodejs.org/api/async_context.html#new-asynclocalstoraget)

[run](https://nodejs.org/api/async_context.html#asynclocalstoragerunstore-callback-args)

[exit](https://nodejs.org/api/async_context.html#asynclocalstorageexitcallback-args)

[enterWith](https://nodejs.org/api/async_context.html#asynclocalstorageenterwithstore)

[getStore](https://nodejs.org/api/async_context.html#asynclocalstoragegetstore)

[disable](https://nodejs.org/api/async_context.html#asynclocalstoragedisable)

[bind](https://nodejs.org/api/async_context.html#asynclocalstoragebindfn)

[AsyncLocalStorage.bind](https://nodejs.org/api/async_context.html#static-method-asynclocalstoragebindfn)

[AsyncLocalStorage.snapshot](https://nodejs.org/api/async_context.html#static-method-asynclocalstoragesnapshot)

> AsyncLocalStorage propagates across Promises, `await`, microtasks, and timers by default (no `RASTER_RUNTIME_ASYNC_HOOKS` required). Public `createHook()` user callbacks still require `RASTER_RUNTIME_ASYNC_HOOKS=1`.

## buffer

> [!NOTE]
> `toString`, `from`, `write`, `alloc`, `byteLength` and `isEncoding` support the Node encodings utf8, utf16le (ucs2), latin1 (binary), ascii, base64, base64url and hex, matched case-insensitively without trimming.

### Static methods

[alloc](https://nodejs.org/api/buffer.html#static-method-bufferallocsize-fill-encoding)

[allocUnsafe](https://nodejs.org/api/buffer.html#static-method-bufferallocunsafesize)

[allocUnsafeSlow](https://nodejs.org/api/buffer.html#static-method-bufferallocunsafeslowsize)

[byteLength](https://nodejs.org/api/buffer.html#static-method-bufferbytelengthstring-encoding)

[concat](https://nodejs.org/api/buffer.html#static-method-bufferconcatlist-totallength)

[from](https://nodejs.org/api/buffer.html#static-method-bufferfromarray)

[isBuffer](https://nodejs.org/api/buffer.html#static-method-bufferisbufferobj)

[isEncoding](https://nodejs.org/api/buffer.html#static-method-bufferisencodingencoding)

### Prototype methods

[copy](https://nodejs.org/api/buffer.html#bufcopytarget-targetstart-sourcestart-sourceend)

[readBigInt64BE](https://nodejs.org/api/buffer.html#bufreadbigint64beoffset)

[readBigInt64LE](https://nodejs.org/api/buffer.html#bufreadbigint64leoffset)

[readDoubleBE](https://nodejs.org/api/buffer.html#bufreaddoublebeoffset)

[readDoubleLE](https://nodejs.org/api/buffer.html#bufreaddoubleleoffset)

[readFloatBE](https://nodejs.org/api/buffer.html#bufreadfloatbeoffset)

[readFloatLE](https://nodejs.org/api/buffer.html#bufreadfloatleoffset)

[readInt8](https://nodejs.org/api/buffer.html#bufreadint8offset)

[readInt16BE](https://nodejs.org/api/buffer.html#bufreadint16beoffset)

[readInt16LE](https://nodejs.org/api/buffer.html#bufreadint16leoffset)

[readInt32BE](https://nodejs.org/api/buffer.html#bufreadint32beoffset)

[readInt32LE](https://nodejs.org/api/buffer.html#bufreadint32leoffset)

[readUInt8](https://nodejs.org/api/buffer.html#bufreaduint8offset)

[readUInt16BE](https://nodejs.org/api/buffer.html#bufreaduint16beoffset)

[readUInt16LE](https://nodejs.org/api/buffer.html#bufreaduint16leoffset)

[readUInt32BE](https://nodejs.org/api/buffer.html#bufreaduint32beoffset)

[readUInt32LE](https://nodejs.org/api/buffer.html#bufreaduint32leoffset)

[subarray](https://nodejs.org/api/buffer.html#bufsubarraystart-end)

[toString](https://nodejs.org/api/buffer.html#buftostringencoding-start-end)

[write](https://nodejs.org/api/buffer.html#bufwritestring-offset-length-encoding)

[writeBigInt64BE](https://nodejs.org/api/buffer.html#bufwritebigint64bevalue-offset)

[writeBigInt64LE](https://nodejs.org/api/buffer.html#bufwritebigint64levalue-offset)

[writeDoubleBE](https://nodejs.org/api/buffer.html#bufwritedoublebevalue-offset)

[writeDoubleLE](https://nodejs.org/api/buffer.html#bufwritedoublelevalue-offset)

[writeFloatBE](https://nodejs.org/api/buffer.html#bufwritefloatbevalue-offset)

[writeFloatLE](https://nodejs.org/api/buffer.html#bufwritefloatlevalue-offset)

[writeInt8](https://nodejs.org/api/buffer.html#bufwriteint8value-offset)

[writeInt16BE](https://nodejs.org/api/buffer.html#bufwriteint16bevalue-offset)

[writeInt16LE](https://nodejs.org/api/buffer.html#bufwriteint16levalue-offset)

[writeInt32BE](https://nodejs.org/api/buffer.html#bufwriteint32bevalue-offset)

[writeInt32LE](https://nodejs.org/api/buffer.html#bufwriteint32levalue-offset)

[writeUInt8](https://nodejs.org/api/buffer.html#bufwriteuint8value-offset)

[writeUInt16BE](https://nodejs.org/api/buffer.html#bufwriteuint16bevalue-offset)

[writeUInt16LE](https://nodejs.org/api/buffer.html#bufwriteuint16levalue-offset)

[writeUInt32BE](https://nodejs.org/api/buffer.html#bufwriteuint32bevalue-offset)

[writeUInt32LE](https://nodejs.org/api/buffer.html#bufwriteuint32levalue-offset)

### Constants

[constants.MAX_LENGTH](https://nodejs.org/api/buffer.html#bufferconstantsmax_length)

[constants.MAX_STRING_LENGTH](https://nodejs.org/api/buffer.html#bufferconstantsmax_string_length)

Everything else inherited from [Uint8Array](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Uint8Array)

## child_process

> [!WARNING]
> `spawn` uses native streams that is not 100% compatible with the Node.js Streams API.

[spawn](https://nodejs.org/api/child_process.html#child_processspawncommand-args-options)

> `ChildProcess.ref()` / `unref()` control whether the parent waits on the child for event-loop lifetime. After `unref()`, the child keeps running but the parent may exit without receiving `exit`/`close` from the detached wait handle.

## console

[Console](https://nodejs.org/api/console.html#class-console)

Global `console` is a `Console` instance with methods bound to itself (matching Node.js).

[assert](https://nodejs.org/api/console.html#consoleassertvalue-message)

[clear](https://nodejs.org/api/console.html#consoleclear)

[count](https://nodejs.org/api/console.html#consolecountlabel)

[countReset](https://nodejs.org/api/console.html#consolecountresetlabel)

[debug](https://nodejs.org/api/console.html#consoledebugdata-args)

[dir](https://nodejs.org/api/console.html#consoledirobj-options)

[error](https://nodejs.org/api/console.html#consoleerrordata-args)

[info](https://nodejs.org/api/console.html#consoleinfodata-args)

[log](https://nodejs.org/api/console.html#consolelogdata-args)

[time](https://nodejs.org/api/console.html#consoletimelabel)

[timeEnd](https://nodejs.org/api/console.html#consoletimeendlabel)

[timeLog](https://nodejs.org/api/console.html#consoletimeloglabel-args)

[trace](https://nodejs.org/api/console.html#consoletracedata-args)

[warn](https://nodejs.org/api/console.html#consolewarndata-args)

> [!NOTE]
> `count`, `countReset`, `time`, `timeLog`, and `timeEnd` coerce labels with JavaScript `ToString` (numbers and objects are accepted). `dir` supports a `colors` option; other Node `dir` options are not implemented.

## crypto

[createHash](https://nodejs.org/api/crypto.html#cryptocreatehashalgorithm-options)

[createHmac](https://nodejs.org/api/crypto.html#cryptocreatehmacalgorithm-key-options)

[getRandomValues](https://nodejs.org/api/crypto.html#cryptogetrandomvaluestypedarray)

[randomBytes](https://nodejs.org/api/crypto.html#cryptorandombytessize-callback)

[randomFill](https://nodejs.org/api/crypto.html#cryptorandomfillbuffer-offset-size-callback)

[randomFillSync](https://nodejs.org/api/crypto.html#cryptorandomfillsyncbuffer-offset-size)

[randomInt](https://nodejs.org/api/crypto.html#cryptorandomintmin-max-callback)

[randomUUID](https://nodejs.org/api/crypto.html#cryptorandomuuidoptions)

[webcrypto](https://nodejs.org/api/crypto.html#cryptowebcrypto)

### raster_runtime specific hash classes

Lightweight and fast hash classes for raster_runtime.

- `Md5`
- `Sha1`
- `Sha256`
- `Sha384`
- `Sha512`
- `Crc32`
- `Crc32c`

## crypto.subtle

[subtle.decrypt](https://nodejs.org/api/webcrypto.html#subtledecryptalgorithm-key-data)

[subtle.deriveBits](https://nodejs.org/api/webcrypto.html#subtlederivebitsalgorithm-basekey-length)

[subtle.digest](https://nodejs.org/api/webcrypto.html#subtledigestalgorithm-data)

[subtle.encrypt](https://nodejs.org/api/webcrypto.html#subtleencryptalgorithm-key-data)

[subtle.exportKey](https://nodejs.org/api/webcrypto.html#subtleexportkeyformat-key)

[subtle.generateKey](https://nodejs.org/api/webcrypto.html#subtlegeneratekeyalgorithm-extractable-keyusages)

[subtle.importKey](https://nodejs.org/api/webcrypto.html#subtleimportkeyformat-keydata-algorithm-extractable-keyusages)

[subtle.sign](https://nodejs.org/api/webcrypto.html#subtlesignalgorithm-key-data)

[subtle.verify](https://nodejs.org/api/webcrypto.html#subtleverifyalgorithm-key-signature-datah)

## dgram

[createSocket](https://nodejs.org/api/dgram.html#dgramcreatesocketoptions-callback)

### Class: dgram.Socket

[address](https://nodejs.org/api/dgram.html#socketaddress)

[bind](https://nodejs.org/api/dgram.html#socketbindport-address-callback)

[close](https://nodejs.org/api/dgram.html#socketclosecallback)

[ref](https://nodejs.org/api/dgram.html#socketref)

[send](https://nodejs.org/api/dgram.html#socketsendmsg-offset-length-port-address-callback)

[unref](https://nodejs.org/api/dgram.html#socketunref)

## dns

[lookup](https://nodejs.org/api/dns.html#dnslookuphostname-options-callback)

[promises.lookup](https://nodejs.org/api/dns.html#dnspromiseslookuphostname-options) (also via `dns/promises` / `node:dns/promises`)

## diagnostics_channel

[channel](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channelchannelname)

[hasSubscribers](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channelhassubscribersname)

[subscribe](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channelsubscribename-onmessage)

[unsubscribe](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channelunsubscribename-onmessage)

### Class: Channel

[subscribe](https://nodejs.org/api/diagnostics_channel.html#channelsubscribeonmessage)

[unsubscribe](https://nodejs.org/api/diagnostics_channel.html#channelunsubscribeonmessage)

[publish](https://nodejs.org/api/diagnostics_channel.html#channelpublishmessage)

[hasSubscribers](https://nodejs.org/api/diagnostics_channel.html#channelhassubscribers)

> [!NOTE]
> `TracingChannel` is not implemented. Subscriber errors during `publish` are reported asynchronously via `queueMicrotask`. `publish` snapshots the subscriber list so callbacks that unsubscribe themselves do not skip later subscribers.

## events

[EventEmitter](https://nodejs.org/api/events.html#class-eventemitter)

[once](https://nodejs.org/api/events.html#eventsonceemitter-name-options)

[on](https://nodejs.org/api/events.html#eventsonemitter-eventname-options)

[getEventListeners](https://nodejs.org/api/events.html#eventsgeteventlistenersemitter-name)

[getMaxListeners](https://nodejs.org/api/events.html#eventsgetmaxlistenersemitter)

[setMaxListeners](https://nodejs.org/api/events.html#eventssetmaxlistenersn-...eventtargets)

[addAbortListener](https://nodejs.org/api/events.html#eventsaddabortlistenersignal-listener)

`errorMonitor`, `captureRejectionSymbol` and `defaultMaxListeners` are exported and also attached to the `EventEmitter` constructor, matching Node where the default export is that constructor.

> `EventEmitterAsyncResource` and automatic `captureRejections` behaviour are not implemented.

## fs

[access](https://nodejs.org/api/fs.html#fsaccesspath-mode-callback)

[accessSync](https://nodejs.org/api/fs.html#fsaccesssyncpath-mode)

[constants](https://nodejs.org/api/fs.html#file-access-constants)

[existsSync](https://nodejs.org/api/fs.html#fsexistssyncpath)

[lstat](https://nodejs.org/api/fs.html#fslstatpath-options-callback)

[lstatSync](https://nodejs.org/api/fs.html#fslstatsyncpath-options)

[mkdirSync](https://nodejs.org/api/fs.html#fsmkdirsyncpath-options)

[mkdtempSync](https://nodejs.org/api/fs.html#fsmkdtempsyncprefix-options)

[readdirSync](https://nodejs.org/api/fs.html#fsreaddirsyncpath-options)

[readFileSync](https://nodejs.org/api/fs.html#fsreadfilesyncpath-options)

[rmdirSync](https://nodejs.org/api/fs.html#fsrmdirsyncpath-options)

[rmSync](https://nodejs.org/api/fs.html#fsrmsyncpath-options)

[stat](https://nodejs.org/api/fs.html#fsstatpath-options-callback)

[statSync](https://nodejs.org/api/fs.html#fsstatsyncpath-options)

[writeFileSync](https://nodejs.org/api/fs.html#fswritefilesyncfile-data-options)

[chmodSync](https://nodejs.org/api/fs.html#fschmodsyncpath-mode)

[renameSync](https://nodejs.org/api/fs.html#fsrenamesyncoldpath-newpath)

[symlinkSync](https://nodejs.org/api/fs.html#fssymlinksynctarget-path-type)

[realpathSync](https://nodejs.org/api/fs.html#fsrealpathsyncpath-options)

[realpath](https://nodejs.org/api/fs.html#fsrealpathpath-options-callback)

> [!NOTE]
> Callback `stat`/`lstat` reject `{ bigint: true }` with a clear error (BigIntStats is not supported).
> `existsSync` returns `false` for any `metadata` failure, including permission errors.
> On Unix, `access` / `accessSync` / `fs.promises.access` use the OS `access(2)` syscall.
> On Windows, checks are metadata-based approximations (existence + readonly for `W_OK`).

## fs/promises

[access](https://nodejs.org/api/fs.html#fspromisesaccesspath-mode)

[constants](https://nodejs.org/api/fs.html#file-access-constants)

[lstat](https://nodejs.org/api/fs.html#fspromiseslstatpath-options)

[mkdir](https://nodejs.org/api/fs.html#fsmkdirpath-options-callback)

[mkdtemp](https://nodejs.org/api/fs.html#fsmkdtempprefix-options-callback)

[open](https://nodejs.org/api/fs.html#fspromisesopenpath-flags-mode)

[readdir](https://nodejs.org/api/fs.html#fspromisesreaddirpath-options)

[readFile](https://nodejs.org/api/fs.html#filehandlereadfileoptions)

[rm](https://nodejs.org/api/fs.html#fsrmpath-options-callback)

[rmdir](https://nodejs.org/api/fs.html#fsrmdirpath-options-callback)

[stat](https://nodejs.org/api/fs.html#fsstatpath-options-callback)

[writeFile](https://nodejs.org/api/fs.html#fspromiseswritefilefile-data-options)

[chmod](https://nodejs.org/api/fs.html#fspromiseschmodpath-mode)

[rename](https://nodejs.org/api/fs.html#fspromisesrenameoldpath-newpath)

[symlink](https://nodejs.org/api/fs.html#fspromisessymlinktarget-path-type)

[realpath](https://nodejs.org/api/fs.html#fspromisesrealpathpath-options)

## http

> `http.Server` supports Node-compatible `ref()` / `unref()` on the listen handle (same event-loop semantics as `net.Server`). `https.createServer` wraps `tls.createServer` and hands decrypted sockets to an internal `http.Server` (Node-style).

## https

[Agent](https://nodejs.org/api/https.html#class-httpsagent)

## http2

Node-compatible HTTP/2 surface via native module `raster_runtime_http2`:

- `createServer()` / `createSecureServer()` — h2c or TLS+ALPN `h2`, `(req, res)` callback with `res.end()` buffering.
- `connect()` — h2c or `https` client session with `request()` and `connect` event.
- `constants`, `getDefaultSettings()`, `getPackedSettings()`, `getUnpackedSettings()`, and `sensitiveHeaders` are exported from the native module.

## module

Supported public APIs:

[builtinModules](https://nodejs.org/api/module.html#modulebuiltinmodules)

[createRequire](https://nodejs.org/api/module.html#modulecreaterequirefilename)

> [!NOTE]
> `require` is available from ESM modules natively. `createRequire` returns a scoped `require` with `resolve` and a shared `cache`.

[isBuiltin](https://nodejs.org/api/module.html#moduleisbuiltinmodulename)

[registerHooks](https://nodejs.org/api/module.html#moduleregisterhooksoptions)

CommonJS loader facade (also on the default `Module` export):

- `Module` constructor with `prototype.require()`
- `require.resolve(request, { paths? })` and `require.cache`
- `Module._resolveFilename`, `Module._nodeModulePaths`, `Module._cache` — exposed for CommonJS ecosystem compatibility (e.g. Next.js require hooks); writable but not a full Node internals implementation

> [!NOTE]
> Does not implement `Module._load` or complete `require.main` semantics. **N-API** native addon (`.node`) loading is available when built with `--features napi` (see `make compat-napi`). **V8 C++ ABI** addons (Node 24 / `NODE_MODULE_VERSION` 137, e.g. better-sqlite3) load when built with `--features v8-compat` (see `make compat-v8`, `make compat-better-sqlite3`); this is a QuickJS-backed V8 shim, not a full V8 engine. N-API: `napi_wrap` / `napi_add_finalizer` finalizers run on GC (deferred to safe points); nested handle scopes use a flat arena so outer `napi_value` handles remain valid; thread-safe functions and async work use a per-env driver on the JS thread (`napi_ref`/`napi_unref` TSFN affect event-loop lifetime). See `compat/README.md`. `require.extensions` and `Module._compile` are supported for CommonJS loading hooks (for example Next.js config transpilation); they do not affect static ESM `import` resolution.

## net

> [!WARNING]
> These APIs uses native streams that is not 100% compatible with the Node.js Streams API. Server APIs like `createSever` provides limited functionality useful for testing purposes. Some server options are not supported:
> `highWaterMark`, `pauseOnConnect`, `keepAlive`, `noDelay`, `keepAliveInitialDelay`

[connect](https://nodejs.org/api/net.html#netconnect)

[createConnection](https://nodejs.org/api/net.html#netcreateconnection)

[createServer](https://nodejs.org/api/net.html#netcreateserveroptions-connectionlistener)

> `net.Server` (and sockets) expose Node-compatible `ref()` / `unref()`. An unref'd listening server stops blocking `process` exit while still accepting until the listen task is torn down.

## os

[arch](https://nodejs.org/api/os.html#osarch)

[availableParallelism](https://nodejs.org/api/os.html#osavailableparallelism)

[cpus](https://nodejs.org/api/os.html#oscpus)

[devNull](https://nodejs.org/api/os.html#osdevnull)

[endianness](https://nodejs.org/api/os.html#osendianness)

[EOL](https://nodejs.org/api/os.html#oseol)

[freemem](https://nodejs.org/api/os.html#osfreemem)

[getPriority](https://nodejs.org/api/os.html#osgetprioritypid)

[homedir](https://nodejs.org/api/os.html#oshomedir)

[hostname](https://nodejs.org/api/os.html#oshostname)

[loadavg](https://nodejs.org/api/os.html#osloadavg)

[machine](https://nodejs.org/api/os.html#osmachine)

[networkInterfaces](https://nodejs.org/api/os.html#osnetworkinterfaces)

[platform](https://nodejs.org/api/os.html#osplatform)

[release](https://nodejs.org/api/os.html#osrelease)

[setPriority](https://nodejs.org/api/os.html#ossetprioritypid-priority)

[tmpdir](https://nodejs.org/api/os.html#osplatform)

[totalmem](https://nodejs.org/api/os.html#ostotalmem)

[type](https://nodejs.org/api/os.html#ostype)

[uptime](https://nodejs.org/api/os.html#osuptime)

[userInfo](https://nodejs.org/api/os.html#osuserinfooptions)

[version](https://nodejs.org/api/os.html#osversion)

## path

[basename](https://nodejs.org/api/path.html#pathbasenamepath-suffix)

[delimiter](https://nodejs.org/api/path.html#pathdelimiter)

[dirname](https://nodejs.org/api/path.html#pathdirnamepath)

[extname](https://nodejs.org/api/path.html#pathextnamepath)

[format](https://nodejs.org/api/path.html#pathformatpathobject)

[isAbsolute](https://nodejs.org/api/path.html#pathisabsolutepath)

[join](https://nodejs.org/api/path.html#pathjoinpaths)

[normalize](https://nodejs.org/api/path.html#pathnormalizepath)

[parse](https://nodejs.org/api/path.html#pathparsepath)

[relative](https://nodejs.org/api/path.html#pathrelativefrom-to)

[resolve](https://nodejs.org/api/path.html#pathresolvepaths)

## perf_hooks

_performance is available globally_

[performance.now](https://nodejs.org/api/perf_hooks.html#performancenow)

## process

_process is available globally_

Process is an EventEmitter. Explicit `process.exit(code)` emits `"exit"` synchronously before terminating. When the event loop drains naturally, `"beforeExit"` then `"exit"` are emitted with the current `process.exitCode`. OS signals (`SIGINT`, `SIGTERM`, `SIGHUP` on Unix; Ctrl+C on Windows) are bridged to `process.emit("signal", name)`; default termination matches Node when no `"signal"` listener is registered.

`process.version` / `process.versions.node` advertise Node `24.3.0` for ecosystem gates; `process.versions.raster_runtime` and CLI `--version` remain the real Raster version.

[arch](https://nodejs.org/api/process.html#processarch)

[argv](https://nodejs.org/api/process.html#processargv)

[argv0](https://nodejs.org/api/process.html#processargv0)

[cwd](https://nodejs.org/api/process.html#processcwd)

[chdir](https://nodejs.org/api/process.html#processchdirdirectory)

[env](https://nodejs.org/api/process.html#processenv)

[exit](https://nodejs.org/api/process.html#processexitcode)

[exitCode](https://nodejs.org/api/process.html#processexitcode-1)

[getegid](https://nodejs.org/api/process.html#processgetegid)

[geteuid](https://nodejs.org/api/process.html#processgeteuid)

[getgid](https://nodejs.org/api/process.html#processgetgid)

[getuid](https://nodejs.org/api/process.html#processgetuid)

[hrtime](https://nodejs.org/api/process.html#processhrtime)

[pid](https://nodejs.org/api/process.html#processpid)

[id](https://nodejs.org/api/process.html#processpid) (deprecated Raster-legacy PID property; initialized to the same value as `process.pid`, remains writable for backward compatibility and is not a live alias of `pid`)

[kill](https://nodejs.org/api/process.html#processkillpid-signal)

[on](https://nodejs.org/api/events.html#emitteroneventname-listener) / EventEmitter methods (`once`, `off`, `emit`, …)

[platform](https://nodejs.org/api/process.html#processplatform)

[release](https://nodejs.org/api/process.html#processrelease)

[setegid](https://nodejs.org/api/process.html#processsetegidgid)

[seteuid](https://nodejs.org/api/process.html#processseteuiduid)

[setgid](https://nodejs.org/api/process.html#processsetgidgid)

[setuid](https://nodejs.org/api/process.html#processsetuiduid)

[version](https://nodejs.org/api/process.html#processversion)

[versions](https://nodejs.org/api/process.html#processversions)

## constants

Legacy Node.js `constants` module. Flat object with fs access-mode bits only:

- `F_OK` (0)
- `R_OK` (4)
- `W_OK` (2)
- `X_OK` (1)

Values match `fs.constants`. The export object is frozen. Open flags (`O_*`),
errno, crypto, and signal constants are not exposed.

## v8

Node-compatible subset of the `v8` module. Statistics are derived from **QuickJS**
`JS_ComputeMemoryUsage`, not Google V8. Heap space name is always `"quickjs"`.
When QuickJS has no malloc limit (`malloc_limit === 0`), `heap_size_limit` is
`Number.MAX_SAFE_INTEGER`. `setFlagsFromString` is a compatibility no-op (does not
configure QuickJS). Serialization, snapshots, and profilers are not implemented.

[getHeapStatistics](https://nodejs.org/api/v8.html#v8getheapstatistics)

[getHeapSpaceStatistics](https://nodejs.org/api/v8.html#v8getheapspacestatistics)

[getHeapCodeStatistics](https://nodejs.org/api/v8.html#v8getheapcodestatistics)

[setFlagsFromString](https://nodejs.org/api/v8.html#v8setflagsfromstringflags) (no-op)

## querystring

[decode](https://nodejs.org/api/querystring.html#querystringdecodestr-sep-eq-options) (alias of `parse`)

[encode](https://nodejs.org/api/querystring.html#querystringencodestr-sep-eq-options) (alias of `stringify`)

[escape](https://nodejs.org/api/querystring.html#querystringescapestr)

[parse](https://nodejs.org/api/querystring.html#querystringparsestr-sep-eq-options)

[stringify](https://nodejs.org/api/querystring.html#querystringstringifyobj-sep-eq-options)

[unescape](https://nodejs.org/api/querystring.html#querystringunescapestr)

> [!NOTE]
> `encode` / `decode` follow Node.js and alias `stringify` / `parse`, not `escape` / `unescape`. Empty `sep` or `eq` arguments fall back to `&` and `=` respectively. `escape` throws `URIError` for lone surrogate code units.

## sqlite

Experimental `node:sqlite` implementation targeting the Node.js 24.3 API and backed by SQLite 3.50.1. It is available only through the `node:` scheme, enabled by default, and can be disabled with `--no-experimental-sqlite`.

Exports:

- `DatabaseSync`
- `StatementSync`
- `backup()`
- changeset conflict `constants`

`DatabaseSync` supports in-memory and file databases, `open()`, `close()`, `exec()`, `prepare()`, `location()`, scalar `function()`, `aggregate()`, `createSession()`, `applyChangeset()`, `loadExtension()`, and `enableLoadExtension()`. `StatementSync` supports `run()`, `get()`, `all()`, `iterate()`, `columns()`, named-parameter controls, BigInt reads, and array rows. Session objects expose `changeset()`, `patchset()`, and `close()`, but `Session` is intentionally not a module export.

The Node 24.3 differential fixture covers module shape, statements, functions, aggregates, sessions/changesets, extension loading, backup, errors, and repeated lifecycle/backup stability. See [`compat/node-sqlite/README.md`](compat/node-sqlite/README.md).

## stream

[Duplex](https://nodejs.org/api/stream.html#class-streamduplex)

[PassThrough](https://nodejs.org/api/stream.html#class-streampassthrough)

[Readable](https://nodejs.org/api/stream.html#class-streamreadable)

[Stream](https://nodejs.org/api/stream.html#class-stream)

[Transform](https://nodejs.org/api/stream.html#class-streamtransform)

[Writable](https://nodejs.org/api/stream.html#class-streamwritable)

[finished](https://nodejs.org/api/stream.html#streamfinishedstream-options-callback)

[pipeline](https://nodejs.org/api/stream.html#streampipelinestreams-callback)

### State predicates

[isReadable](https://nodejs.org/api/stream.html#streamisreadablestream) / [isWritable](https://nodejs.org/api/stream.html#streamiswritablestream) / [isDestroyed](https://nodejs.org/api/stream.html#streamisdestroyedstream) / [isDisturbed](https://nodejs.org/api/stream.html#streamisdisturbedstream) / [isErrored](https://nodejs.org/api/stream.html#streamiserroredstream) / [isNodeStream](https://nodejs.org/api/stream.html#streamisnodestreamstream) / [isReadableStream](https://nodejs.org/api/stream.html#streamisreadablestreamstream) / [isWritableStream](https://nodejs.org/api/stream.html#streamiswritablestreamstream)

These return `null` (not `false`) when the argument is not a stream, matching Node.

### Composition helpers

[compose](https://nodejs.org/api/stream.html#streamcomposestreams) / [duplexPair](https://nodejs.org/api/stream.html#streamduplexpairoptions) / [addAbortSignal](https://nodejs.org/api/stream.html#streamaddabortsignalsignal-stream) / [destroy](https://nodejs.org/api/stream.html#streamdestroystream-error) / [getDefaultHighWaterMark](https://nodejs.org/api/stream.html#streamgetdefaulthighwatermarkobjectmode) / [setDefaultHighWaterMark](https://nodejs.org/api/stream.html#streamsetdefaulthighwatermarkobjectmode-value)

All helpers are also attached to the default export (`stream.isReadable(s)`).

> `compose` collapses Node's pipeline/web-stream branches into duplex `.pipe()` wiring, so it does not give the returned stream `compose` iteration semantics over async generators. `duplexPair` acks writes on `drain` rather than modelling true half-open backpressure.

## stream/web

Also available as `node:stream/web`. `TextEncoderStream` and `TextDecoderStream` are installed globally when the stream/web module is loaded.

[ByteLengthQueuingStrategy](https://nodejs.org/api/webstreams.html#class-bytelengthqueuingstrategy)

[CountQueuingStrategy](https://nodejs.org/api/webstreams.html#class-countqueuingstrategy)

[ReadableByteStreamController](https://nodejs.org/api/webstreams.html#class-readablebytestreamcontroller)

[ReadableStream](https://nodejs.org/api/webstreams.html#class-readablestream)

[ReadableStreamBYOBReader](https://nodejs.org/api/webstreams.html#class-readablestreambyobreader)

[ReadableStreamBYOBRequest](https://nodejs.org/api/webstreams.html#class-readablestreambyobrequest)

[ReadableStreamDefaultController](https://nodejs.org/api/webstreams.html#class-readablestreamdefaultcontroller)

[ReadableStreamDefaultReader](https://nodejs.org/api/webstreams.html#class-readablestreamdefaultreader)

[TextDecoderStream](https://nodejs.org/api/webstreams.html#class-textdecoderstream)

[TextEncoderStream](https://nodejs.org/api/webstreams.html#class-textencoderstream)

[TransformStream](https://nodejs.org/api/webstreams.html#class-transformstream)

[WritableStream](https://nodejs.org/api/webstreams.html#class-writablestream)

[WritableStreamDefaultController](https://nodejs.org/api/webstreams.html#class-writablestreamdefaultcontroller)

[WritableStreamDefaultWriter](https://nodejs.org/api/webstreams.html#class-writablestreamdefaultwriter)

## stream/promises

[finished](https://nodejs.org/api/stream.html#streamfinishedstream-options-callback)

[pipeline](https://nodejs.org/api/stream.html#streampipelinestreams-callback)

## string_decoder

[StringDecoder](https://nodejs.org/api/string_decoder.html#class-stringdecoder)

## test

> Embedded JS (`bundle/js/test.js`). Exports `test`, `describe` / `suite`, `it`, hook helpers, `test.skip` / `only` / `todo`, and `run()` (summary object). Top-level `test()` runs immediately and returns a `Promise`. Node’s `TestsStream` / `node --test` file discovery are not implemented yet.

## timers

_Also available globally_

[clearImmediate](https://nodejs.org/api/timers.html#clearimmediateimmediate)

[clearInterval](https://nodejs.org/api/timers.html#clearintervaltimeout)

[clearTimeout](https://nodejs.org/api/timers.html#cleartimeouttimeout)

[setImmediate](https://nodejs.org/api/timers.html#setimmediatecallback-args)

[setInterval](https://nodejs.org/api/timers.html#setintervalcallback-delay-args)

[setTimeout](https://nodejs.org/api/timers.html#settimeoutcallback-delay-args)

> Timer handles are `Timeout` objects with Node-compatible `ref()` / `unref()`. Unref'd timers do not keep the process alive when they are the only remaining handle.

## timers/promises

[setTimeout](https://nodejs.org/api/timers.html#timerspromisessettimeoutdelay-value-options)

[setInterval](https://nodejs.org/api/timers.html#timerspromisessetintervaldelay-value-options) — returns an async iterator that yields `value` once per tick until the `signal` aborts or the consumer stops iterating.

[setImmediate](https://nodejs.org/api/timers.html#timerspromisessetimmediatevalue-options)

[scheduler](https://nodejs.org/api/timers.html#timerspromisesscheduler) — `wait(delay, options)` and `yield()`.

> `{ ref: false }` schedules an unref'd timer (same as calling `.unref()` on the underlying handle).

## tls

Partial `node:tls` support:

- Client: `connect()`, `TLSSocket`, `createSecureContext()`, `checkServerIdentity()`
- Server: `createServer()` and `Server` (`listen`, `close`, `addContext`, `setSecureContext`)
- STARTTLS upgrade through `connect({ socket, ... })`
- TLS 1.2 and TLS 1.3 with the configured `tls-ring`, `tls-aws-lc`, `tls-graviola`, or `tls-openssl` backend

PFX, custom cipher/signature configuration, DH parameters, CRLs, PSK, OCSP, session/ticket resumption, async `SNICallback`, `ALPNCallback`, soft mTLS, and related advanced APIs are not supported. `TLSSocket.write()` currently returns `true` without backpressure or `drain` signaling. See [`modules/raster_runtime_tls/README.md`](modules/raster_runtime_tls/README.md).

## inspector

Startup probe only — Raster does **not** implement the Node Inspector debugging protocol.

[url](https://nodejs.org/api/inspector.html#inspectorurl) — always returns `undefined`

> `Session`, `open()`, `close()`, and `waitForDebugger()` are intentionally not exported.

## tty

[isatty](https://nodejs.org/api/tty.html#ttyisattyfd)

> `ReadStream` / `WriteStream` (including `process.stdin` / `stdout` / `stderr`) support `ref()` / `unref()`. Stdin reader tasks stop blocking exit when unref'd; readline follows `process.stdin`.

## url

### Class

[URL](https://nodejs.org/api/url.html#class-url)

[URLSearchParams](https://nodejs.org/api/url.html#class-urlsearchparams)

### Prototype methods

[domainToASCII](https://nodejs.org/api/url.html#urldomaintoasciidomain)

[domainToUnicode](https://nodejs.org/api/url.html#urldomaintounicodedomain)

[fileURLToPath](https://nodejs.org/api/url.html#urlfileurltopathurl-options)

[format](https://nodejs.org/api/url.html#urlformaturl-options)

[pathToFileURL](https://nodejs.org/api/url.html#urlpathtofileurlpath-options)

[urlToHttpOptions](https://nodejs.org/api/url.html#urlurltohttpoptionsurl)

## util

> [!IMPORTANT]
> Supported encodings: hex, base64, utf-8, utf-16le, windows-1252 and their aliases.

[debug](https://nodejs.org/api/util.html#utildebuglogsection) (alias of `debuglog`)

[debuglog](https://nodejs.org/api/util.html#utildebuglogsection)

[format](https://nodejs.org/api/util.html#utilformatformat-args)

[formatWithOptions](https://nodejs.org/api/util.html#utilformatwithoptionsinspectoptions-format-args)

[inspect](https://nodejs.org/api/util.html#utilinspectobject-options)

[inherits](https://nodejs.org/api/util.html#utilinheritsconstructor-superconstructor)

[promisify](https://nodejs.org/api/util.html#utilpromisifyoriginal)

[toUSVString](https://nodejs.org/api/util.html#utiltousvstringstring)

[types.isAnyArrayBuffer](https://nodejs.org/api/util.html#utiltypesisanyarraybuffervalue)

[types.isArrayBuffer](https://nodejs.org/api/util.html#utiltypesisarraybuffervalue)

[types.isDataView](https://nodejs.org/api/util.html#utiltypesisdataviewvalue)

[types.isPromise](https://nodejs.org/api/util.html#utiltypesispromisevalue)

[types.isProxy](https://nodejs.org/api/util.html#utiltypesisproxyvalue)

[types.isSharedArrayBuffer](https://nodejs.org/api/util.html#utiltypesissharedarraybuffervalue)

[types.isTypedArray](https://nodejs.org/api/util.html#utiltypesistypedarrayvalue)

[types.isUint8Array](https://nodejs.org/api/util.html#utiltypesisuint8arrayvalue)

[TextDecoder](https://nodejs.org/api/util.html#class-utiltextdecoder)

[TextEncoder](https://nodejs.org/api/util.html#class-utiltextencoder)

> [!NOTE]
> `util.promisify.custom` and multi-value callback result mapping are not implemented.
> Callbacks follow the Node error-first convention and only the first success value is resolved.
>
> `format` does not append a trailing newline. `inspect.custom` uses the `nodejs.util.inspect.custom` symbol.
>
> `formatWithOptions` currently honors only the `colors` inspect option; other Node inspect options are ignored.
>
> `debuglog` reads `process.env.NODE_DEBUG` (comma/whitespace separated, `*` wildcard suffix supported). `types.isSharedArrayBuffer` always returns `false`.
>
> `TextDecoder.decode` supports the `stream` option for incremental decoding across calls. BOM stripping works across streaming chunks when `ignoreBOM` is `false`.

## vm

> [!IMPORTANT]
> Raster provides a minimal `vm.runInNewContext` implementation backed by an isolated QuickJS context in the same runtime. This is **not** a security boundary.

[runInNewContext](https://nodejs.org/api/vm.html#vmruninnewcontextcode-contextobject-options)

> [!NOTE]
> Only `filename` is supported in the options object. Other Node options such as `timeout`, `breakOnSigint`, `contextCodeGeneration`, `microtaskMode`, `offset`, and `cachedData` throw `Error("vm.runInNewContext option '<name>' is not supported")`.
>
> Sandbox synchronization copies own enumerable string-keyed properties only. Non-enumerable properties, symbol properties, and full property descriptor forwarding are not supported.

## worker_threads

Workers run on **OS threads** with a **separate QuickJS `AsyncRuntime` isolate** per worker (same process). Cross-thread `postMessage` uses **structured-clone** encoding (`clone_wire` + in-memory channels), not child-process IPC.

- `isMainThread`, `parentPort`, `workerData`, `threadId`, nested `Worker`
- `Worker` options/events: `workerData`, `env` / `argv` / `name`, `online` / `message` / `error` / `exit`
- `transferList` for detached `ArrayBuffer` (structured clone); **`SharedArrayBuffer`** via process-wide QuickJS SAB hooks (shared backing store; Atomics when views share memory)
- In-process `MessageChannel` / `MessagePort` (structured clone + microtasks)
- `getEnvironmentData` / `setEnvironmentData` with worker sync

`moveMessagePortToContext()` still throws. `MessagePort.ref()` / `unref()` tie into the event-loop ref model.

## zlib

### Convenience methods

[deflate](https://nodejs.org/api/zlib.html#zlibdeflatebuffer-options-callback)

[deflateSync](https://nodejs.org/api/zlib.html#zlibdeflatesyncbuffer-options)

[deflateRaw](https://nodejs.org/api/zlib.html#zlibdeflaterawbuffer-options-callback)

[deflateRawSync](https://nodejs.org/api/zlib.html#zlibdeflaterawsyncbuffer-options)

[gzip](https://nodejs.org/api/zlib.html#zlibgzipbuffer-options-callback)

[gzipSync](https://nodejs.org/api/zlib.html#zlibgzipsyncbuffer-options)

[inflate](https://nodejs.org/api/zlib.html#zlibinflatebuffer-options-callback)

[inflateSync](https://nodejs.org/api/zlib.html#zlibinflatesyncbuffer-options)

[inflateRaw](https://nodejs.org/api/zlib.html#zlibinflaterawbuffer-options-callback)

[inflateRawSync](https://nodejs.org/api/zlib.html#zlibinflaterawsyncbuffer-options)

[gunzip](https://nodejs.org/api/zlib.html#zlibgunzipbuffer-options-callback)

[gunzipSync](https://nodejs.org/api/zlib.html#zlibgunzipsyncbuffer-options)

[brotliCompress](https://nodejs.org/api/zlib.html#zlibbrotlicompressbuffer-options-callback)

[brotliCompressSync](https://nodejs.org/api/zlib.html#zlibbrotlicompresssyncbuffer-options)

[brotliDecompress](https://nodejs.org/api/zlib.html#zlibbrotlidecompressbuffer-options-callback)

[brotliDecompressSync](https://nodejs.org/api/zlib.html#zlibbrotlidecompresssyncbuffer-options)

[zstdCompress](https://nodejs.org/api/zlib.html#zlibzstdcompressbuffer-options-callback)

[zstdCompressSync](https://nodejs.org/api/zlib.html#zlibzstdcompresssyncbuffer-options)

[zstdDecompress](https://nodejs.org/api/zlib.html#zlibzstddecompressbuffer-options-callback)

[zstdDecompressSync](https://nodejs.org/api/zlib.html#zlibzstddecompresssyncbuffer-options)

### Streaming

[createGzip](https://nodejs.org/api/zlib.html#zlibcreategzipoptions) / [createGunzip](https://nodejs.org/api/zlib.html#zlibcreategunzipoptions) / [createDeflate](https://nodejs.org/api/zlib.html#zlibcreatedeflateoptions) / [createInflate](https://nodejs.org/api/zlib.html#zlibcreateinflateoptions) / [createDeflateRaw](https://nodejs.org/api/zlib.html#zlibcreatedeflaterawoptions) / [createInflateRaw](https://nodejs.org/api/zlib.html#zlibcreateinflaterawoptions) / [createBrotliCompress](https://nodejs.org/api/zlib.html#zlibcreatebrotlicompressoptions) / [createBrotliDecompress](https://nodejs.org/api/zlib.html#zlibcreatebrotlidecompressoptions) / [createZstdCompress](https://nodejs.org/api/zlib.html#zlibcreatezstdcompressoptions) / [createZstdDecompress](https://nodejs.org/api/zlib.html#zlibcreatezstddecompressoptions)

The matching `Gzip`, `Gunzip`, `Deflate`, `Inflate`, `DeflateRaw`, `InflateRaw`, `BrotliCompress`, `BrotliDecompress`, `ZstdCompress`, `ZstdDecompress` classes are exported as well, plus `constants`.

Each factory returns a `stream.Transform` that emits a **single** framed stream across all chunks written to it (compressor state is kept between chunks), so it is interchangeable with Node's for middleware such as `compression`.

> `flush(kind, cb)` currently completes immediately without honouring the flush kind, and `params()` / `close()` are not implemented.

# raster_runtime API

## raster_runtime:hex

```typescript
export function encode(
  value: string | Array | ArrayBuffer | Uint8Array
): string;
export function decode(value: string): Uint8Array;
```

## Timezone-aware Intl

Raster provides timezone support through the global `Intl.DateTimeFormat` and `Date.prototype.toLocaleString` APIs. There is no public `raster_runtime:timezone` module.

### Intl.DateTimeFormat

raster_runtime provides a minimal `Intl.DateTimeFormat` implementation focused on timezone support. This enables libraries like dayjs to work with timezone conversions transparently.

```javascript
// Create a formatter for a specific timezone
const formatter = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/Denver",
  hour12: false,
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

// Format a date
const date = new Date("2022-03-02T15:45:34Z");
console.log(formatter.format(date)); // "03/02/2022, 08:45:34"

// Get formatted parts
const parts = formatter.formatToParts(date);
// [{ type: "month", value: "03" }, { type: "literal", value: "/" }, ...]

// Get resolved options
const options = formatter.resolvedOptions();
console.log(options.timeZone); // "America/Denver"
```

### Date.prototype.toLocaleString with timezone

`Date.prototype.toLocaleString` is enhanced to support the `timeZone` option:

```javascript
const date = new Date("2022-03-02T15:45:34Z");

// Convert to Denver time
console.log(date.toLocaleString("en-US", { timeZone: "America/Denver" }));
// "03/02/2022, 8:45:34 AM"

// Convert to Tokyo time
console.log(date.toLocaleString("en-US", { timeZone: "Asia/Tokyo" }));
// "03/03/2022, 12:45:34 AM"
```

### Using with dayjs

The timezone module enables dayjs timezone support without polyfills:

```javascript
const dayjs = require("dayjs");
const utc = require("dayjs/plugin/utc");
const timezone = require("dayjs/plugin/timezone");

dayjs.extend(utc);
dayjs.extend(timezone);

// Convert between timezones
const date = dayjs("2022-03-02T15:45:34Z");
console.log(date.tz("America/Denver").format()); // "2022-03-02T08:45:34-07:00"
console.log(date.tz("Asia/Tokyo").format()); // "2022-03-03T00:45:34+09:00"

// Get start of day in a specific timezone
const denver = date.tz("America/Denver");
console.log(denver.startOf("day").format()); // "2022-03-02T00:00:00-07:00"
```

## raster_runtime:qjs

```typescript
interface MemoryInfo {
  malloc_size: number;
  malloc_limit: number;
  memory_used_size: number;
  malloc_count: number;
  memory_used_count: number;
  atom_count: number;
  atom_size: number;
  str_count: number;
  str_size: number;
  obj_count: number;
  obj_size: number;
  prop_count: number;
  prop_size: number;
  shape_count: number;
  shape_size: number;
  js_func_count: number;
  js_func_size: number;
  js_func_code_size: number;
  js_func_pc2line_count: number;
  js_func_pc2line_size: number;
  c_func_count: number;
  array_count: number;
  fast_array_count: number;
  fast_array_elements: number;
  binary_object_count: number;
  binary_object_size: number;
}
export function ComputeMemoryUsage(): MemoryInfo;
```

## raster_runtime:xml

A lightweight and fast XML parser and builder

```typescript
export class XmlText {
  constructor(text: string);
  toString(): string;
}

export class XmlNode {
  constructor(name: string);
  withName(name: string): this;
  addAttribute(name: string, value: string): this;
  addChildNode(node: XmlNode | XmlText): this;
  removeAttribute(name: string): this;
  toString(): string;
}

type XmlParserOptions = {
    ignoreAttributes?: boolean;
    attributeNamePrefix?: string;
    textNodeName?: string;
    attributeValueProcessor?: (attrName: string, attrValue: string, jpath: string) => unknown;
    tagValueProcessor?: (attrName: string, attrValue: string, jpath: string, hasAttributes: boolean) => unknown;
}
export class XMLParser(options?: XmlParserOptions){
    parse(xml:string):object
}
```


## raster_runtime:util

```typescript
export function dimensions(): [number, number];
export function load(path: string): any;
export function print(value: any): void;
```


# Web Platform API

## CONSOLE

[Console](https://developer.mozilla.org/en-US/docs/Web/API/console)

## DOM

[AbortController](https://developer.mozilla.org/en-US/docs/Web/API/AbortController)

[AbortSignal](https://developer.mozilla.org/en-US/docs/Web/API/AbortSignal)

[CustomEvent](https://developer.mozilla.org/en-US/docs/Web/API/CustomEvent)

[Event](https://developer.mozilla.org/en-US/docs/Web/API/Event)

[EventTarget](https://developer.mozilla.org/en-US/docs/Web/API/EventTarget)

## ECMASCRIPT

[globalThis](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/globalThis)

## ENCODING

[TextDecoder](https://developer.mozilla.org/en-US/docs/Web/API/TextDecoder)

[TextEncoder](https://developer.mozilla.org/en-US/docs/Web/API/TextEncoder)

[TextDecoderStream](https://developer.mozilla.org/en-US/docs/Web/API/TextDecoderStream)

[TextEncoderStream](https://developer.mozilla.org/en-US/docs/Web/API/TextEncoderStream)

> [!NOTE]
> `TextEncoderStream` and `TextDecoderStream` are provided by the `stream/web` builtin and are also available on `globalThis` when that module is loaded.

## FETCH

[Headers](https://developer.mozilla.org/en-US/docs/Web/API/Headers)

[Request](https://developer.mozilla.org/en-US/docs/Web/API/Request)

[Response](https://developer.mozilla.org/en-US/docs/Web/API/Response)

[fetch](https://developer.mozilla.org/en-US/docs/Web/API/Headers)

> [!IMPORTANT]
> There are some differences with the [WHATWG standard](https://fetch.spec.whatwg.org). Mainly browser specific behavior is removed:
>
> - `keepalive` is always true
> - `request.body` can only be `string`, `Array`, `ArrayBuffer` or `Uint8Array`
> - `response.body` returns `null`. Use `response.text()`, `response.json()` etc
> - `mode`, `credentials`, `referrerPolicy`, `priority`, `cache` is not available/applicable

## FILEAPI

[Blob](https://developer.mozilla.org/en-US/docs/Web/API/Blob)

[File](https://developer.mozilla.org/en-US/docs/Web/API/File)

## HR-TIME

[performance.now](https://developer.mozilla.org/en-US/docs/Web/API/Performance/now)

[performance.timeOrigin](https://developer.mozilla.org/en-US/docs/Web/API/Performance/timeOrigin)

## HTML

[atob](https://developer.mozilla.org/en-US/docs/Web/API/atob)

[btoa](https://developer.mozilla.org/en-US/docs/Web/API/btoa)

[clearInterval](https://developer.mozilla.org/en-US/docs/Web/API/Window/clearInterval)

[clearTimeout](https://developer.mozilla.org/en-US/docs/Web/API/Window/clearTimeout)

[navigator](https://developer.mozilla.org/en-US/docs/Web/API/Window/navigator)

[queueMicrotask](https://developer.mozilla.org/en-US/docs/Web/API/Window/queueMicrotask)

[setInterval](https://developer.mozilla.org/en-US/docs/Web/API/Window/setInterval)

[setTimeout](https://developer.mozilla.org/en-US/docs/Web/API/Window/setTimeout)

[structuredClone](https://developer.mozilla.org/en-US/docs/Web/API/Window/structuredClone)

[userAgent](https://developer.mozilla.org/en-US/docs/Web/API/Navigator/userAgent)

## STREAMS

[ByteLengthQueuingStrategy](https://developer.mozilla.org/en-US/docs/Web/API/ByteLengthQueuingStrategy)

[CountQueuingStrategy](https://developer.mozilla.org/en-US/docs/Web/API/CountQueuingStrategy)

[ReadableByteStreamController](https://developer.mozilla.org/en-US/docs/Web/API/ReadableByteStreamController)

[ReadableStream](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStream)

[ReadableStreamBYOBReader](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStreamBYOBReader)

[ReadableStreamBYOBRequest](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStreamBYOBRequest)

[ReadableStreamDefaultController](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStreamDefaultController)

[ReadableStreamDefaultReader](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStreamDefaultReader)

[WritableStream](https://developer.mozilla.org/en-US/docs/Web/API/WritableStream)

[WritableStreamDefaultController](https://developer.mozilla.org/en-US/docs/Web/API/WritableStreamDefaultController)

[WritableStreamDefaultWriter](https://developer.mozilla.org/en-US/docs/Web/API/WritableStreamDefaultWriter)

## URL

[URL](https://developer.mozilla.org/en-US/docs/Web/API/URL)

[URLSearchParams](https://developer.mozilla.org/en-US/docs/Web/API/URLSearchParams)

## WEBASSEMBLY

[WebAssembly](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface)

Raster installs a fresh, realm-scoped `WebAssembly` global on every QuickJS context, backed by [`wasmi`](https://github.com/wasmi-labs/wasmi) `1.1.0`.

Implemented:

- `WebAssembly.compile`, `instantiate` (both overloads), `validate`, `compileStreaming`, `instantiateStreaming`.
- `WebAssembly.Module` (including `Module.imports`/`Module.exports`/`Module.customSections`), `Instance`, `Memory`, `Table`, `Global`.
- `WebAssembly.CompileError`, `LinkError`, `RuntimeError`.
- JS function imports and Wasm function exports, including reentrant calls from a JS import callback back into the same instance.
- The multi-value, bulk-memory, reference-types (`externref`/`funcref`), mutable-globals, and SIMD proposals (SIMD executes inside Wasm only; a `v128` value crossing the JS/Wasm boundary throws `TypeError`).
- A synchronously-mirrored, non-shared `wasm32` linear `Memory`, bidirectionally visible from JS and Wasm.

> [!NOTE]
> Numeric/BigInt mapping: `i32` round-trips through JS `Number` using `ToInt32` semantics (matching Node/browsers' wraparound behavior for out-of-range values); `f32`/`f64` round-trip through `Number`; `i64` only accepts/returns `BigInt` (never `Number`); a multi-result export returns a plain JS `Array`, and a multi-result import callback must return an array with at least that many elements.
>
> Memory model: each `Memory` lazily materializes a QuickJS `ArrayBuffer` "mirror" the first time `.buffer` is read. Wasm's actual linear memory is copied into the mirror immediately before returning from an exported call or before invoking a JS import callback, and copied back out immediately after -- so JS and Wasm always observe each other's writes at every boundary crossing, at the cost of a full-buffer copy per crossing rather than a zero-copy view. A `grow()` (including `grow(0)`, and including a Wasm-internal `memory.grow`) detaches the previous mirror `ArrayBuffer` and creates a new one.
>
> Every `Instance`/`Memory`/`Table`/`Global`/exported-function object is scoped to the QuickJS context (realm) that created it; passing one into a different realm's `WebAssembly` APIs (e.g. across a `vm.runInNewContext` boundary, if that child context also had `WebAssembly` installed) throws `LinkError` rather than silently mixing `wasmi` stores.
>
> `externref` lifetime: to preserve JS identity for every Wasm reference safely, Raster retains the `Persistent<Value>` associated with an `externref` until its WebAssembly realm is torn down. `wasmi` 1.1.0 does not expose the complete set of roots required to reclaim individual entries safely (including Wasm-private tables, globals, and active frames). Consequently, a long-lived realm that continuously stores distinct JS values as `externref` can grow its retained memory until realm teardown; applications with that pattern should use bounded/reused references or periodically recreate the realm.

Not implemented (out of scope for this batch): `WebAssembly.Function`, `Tag`, `Exception`, JSPI (`Suspending`/`promising`), the garbage-collection, exception-handling, and function-references proposals, shared memory/threads, `Memory64`, WASI, and the `.wasm` ESM loader. `WebAssembly.validate`/`compile` reject modules using any of these as a `CompileError` (or `false`, for `validate`) rather than passing them through to `wasmi` or risking a panic. Raster does not provide a WebAssembly-level security sandbox or execution-time/memory quota beyond what `wasmi`'s interpreter itself enforces.

> [!IMPORTANT]
> This implementation requires `wasmi 1.1.0` and Rust `1.86` or newer (the `raster_runtime_webassembly` crate's own `rust-version`; it does not raise the minimum for any other workspace crate).

## WEBCRYPTO

[Crypto](https://developer.mozilla.org/en-US/docs/Web/API/Crypto)

[CryptoKey](https://developer.mozilla.org/en-US/docs/Web/API/CryptoKey)

[SubtleCrypto](https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto)

## WEBIDL

[DOMException](https://developer.mozilla.org/en-US/docs/Web/API/DOMException)

## XHR

[FormData](https://developer.mozilla.org/en-US/docs/Web/API/FormData)
