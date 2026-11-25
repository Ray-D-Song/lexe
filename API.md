# API documentation

> [!NOTE]
> Lexe's primary goal is Node.js compatibility, alongside standard Web Platform APIs ([WinterTC](https://min-common-api.proposal.wintertc.org/)). See the [compatibility matrix](docs/compatibility.md) for current module coverage and remaining gaps.

# Node.js API

## assert

`node:assert` and `node:assert/strict` export the full Node.js assertion surface (`AssertionError`, `CallTracker`, `strict`, `ok`, `fail`, `equal`, `notEqual`, `strictEqual`, `notStrictEqual`, `deepEqual`, `notDeepEqual`, `deepStrictEqual`, `notDeepStrictEqual`, `partialDeepStrictEqual`, `throws`, `doesNotThrow`, `rejects`, `doesNotReject`, `match`, `doesNotMatch`, and `ifError`), with named ESM and CJS support on both `node:assert` and `node:assert/strict`.

[partialDeepStrictEqual](https://nodejs.org/api/assert.html#assertpartialdeepstrictequalactual-expected-message)

[strictEqual](https://nodejs.org/api/assert.html#assertstrictequalactual-expected-message)

[deepStrictEqual](https://nodejs.org/api/assert.html#assertdeepstrictequalactual-expected-message)

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

> AsyncLocalStorage propagates across Promises, `await`, microtasks, and timers by default (no `LEXE_ASYNC_HOOKS` required). Public `createHook()` user callbacks still require `LEXE_ASYNC_HOOKS=1`.

## buffer

> [!NOTE]
> `toString`, `from`, `write`, `alloc`, `byteLength` and `isEncoding` support the Node encodings utf8, utf16le (ucs2), latin1 (binary), ascii, base64, base64url and hex, matched case-insensitively without trimming.

> [!NOTE]
> Instance methods (`fill`, `indexOf`/`lastIndexOf`/`includes`, `compare`/`equals`, `copy`, `slice`/`subarray`, `swap16/32/64`, `toJSON`, the `read*`/`write*` families and the `*Slice`/`*Write` helpers) follow Node's argument coercion and error codes. `Buffer.prototype.inspect` is not provided yet. `Buffer.poolSize` is only a number; Lexe does not pool allocations.

### Static methods

[alloc](https://nodejs.org/api/buffer.html#static-method-bufferallocsize-fill-encoding)

[allocUnsafe](https://nodejs.org/api/buffer.html#static-method-bufferallocunsafesize)

[allocUnsafeSlow](https://nodejs.org/api/buffer.html#static-method-bufferallocunsafeslowsize)

[byteLength](https://nodejs.org/api/buffer.html#static-method-bufferbytelengthstring-encoding)

[compare](https://nodejs.org/api/buffer.html#static-method-buffercomparebuf1-buf2)

[copyBytesFrom](https://nodejs.org/api/buffer.html#static-method-buffercopybytesfromview-offset-length)

[concat](https://nodejs.org/api/buffer.html#static-method-bufferconcatlist-totallength)

[from](https://nodejs.org/api/buffer.html#static-method-bufferfromarray)

[isBuffer](https://nodejs.org/api/buffer.html#static-method-bufferisbufferobj)

[isEncoding](https://nodejs.org/api/buffer.html#static-method-bufferisencodingencoding)

[of](https://nodejs.org/api/buffer.html#static-method-bufferofitems)

[poolSize](https://nodejs.org/api/buffer.html#bufferpoolsize)

### Prototype methods

[compare](https://nodejs.org/api/buffer.html#bufcomparetarget-targetstart-targetend-sourcestart-sourceend)

[copy](https://nodejs.org/api/buffer.html#bufcopytarget-targetstart-sourcestart-sourceend)

[equals](https://nodejs.org/api/buffer.html#bufequalsotherbuffer)

[fill](https://nodejs.org/api/buffer.html#buffillvalue-offset-end-encoding)

[includes](https://nodejs.org/api/buffer.html#bufincludesvalue-byteoffset-encoding)

[indexOf](https://nodejs.org/api/buffer.html#bufindexofvalue-byteoffset-encoding)

[lastIndexOf](https://nodejs.org/api/buffer.html#buflastindexofvalue-byteoffset-encoding)

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

[readBigUInt64BE](https://nodejs.org/api/buffer.html#bufreadbiguint64beoffset)

[readBigUInt64LE](https://nodejs.org/api/buffer.html#bufreadbiguint64leoffset)

[readIntBE](https://nodejs.org/api/buffer.html#bufreadintbeoffset-bytelength)

[readIntLE](https://nodejs.org/api/buffer.html#bufreadintleoffset-bytelength)

[readUIntBE](https://nodejs.org/api/buffer.html#bufreaduintbeoffset-bytelength)

[readUIntLE](https://nodejs.org/api/buffer.html#bufreaduintleoffset-bytelength)

[subarray](https://nodejs.org/api/buffer.html#bufsubarraystart-end)

[swap16](https://nodejs.org/api/buffer.html#bufswap16)

[swap32](https://nodejs.org/api/buffer.html#bufswap32)

[swap64](https://nodejs.org/api/buffer.html#bufswap64)

[toJSON](https://nodejs.org/api/buffer.html#buftojson)

[toLocaleString](https://nodejs.org/api/buffer.html#buftolocalestringencoding-start-end)

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

[writeBigUInt64BE](https://nodejs.org/api/buffer.html#bufwritebiguint64bevalue-offset)

[writeBigUInt64LE](https://nodejs.org/api/buffer.html#bufwritebiguint64levalue-offset)

[writeIntBE](https://nodejs.org/api/buffer.html#bufwriteintbevalue-offset-bytelength)

[writeIntLE](https://nodejs.org/api/buffer.html#bufwriteintlevalue-offset-bytelength)

[writeUIntBE](https://nodejs.org/api/buffer.html#bufwriteuintbevalue-offset-bytelength)

[writeUIntLE](https://nodejs.org/api/buffer.html#bufwriteuintlevalue-offset-bytelength)

### Constants

[constants.MAX_LENGTH](https://nodejs.org/api/buffer.html#bufferconstantsmax_length)

[constants.MAX_STRING_LENGTH](https://nodejs.org/api/buffer.html#bufferconstantsmax_string_length)

`constants.MAX_LENGTH` / `buffer.kMaxLength` is `2147483647` (the largest `ArrayBuffer` QuickJS allows), not Node's `2**53 - 1`.

### Module functions

[transcode](https://nodejs.org/api/buffer.html#buffertranscodesource-fromenc-toenc)

[isUtf8](https://nodejs.org/api/buffer.html#bufferisutf8input)

[isAscii](https://nodejs.org/api/buffer.html#bufferisasciiinput)

[resolveObjectURL](https://nodejs.org/api/buffer.html#bufferresolveobjecturlid)

[SlowBuffer](https://nodejs.org/api/buffer.html#class-slowbuffer)

[Blob](https://nodejs.org/api/buffer.html#class-blob) and [File](https://nodejs.org/api/buffer.html#class-file) are the same objects as the globals. `kMaxLength`, `kStringMaxLength` and `INSPECT_MAX_BYTES` are exported; `INSPECT_MAX_BYTES` is validated but `util.inspect()` does not print a `Buffer` the way Node does, so it has no visible effect yet.

Everything else inherited from [Uint8Array](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Uint8Array)

## child_process

[ChildProcess](https://nodejs.org/api/child_process.html#class-childprocess)

[spawn](https://nodejs.org/api/child_process.html#child_processspawncommand-args-options) (supports `cwd`, `env`, `argv0`, `stdio` including `"pipe"`, `"ignore"`, `"inherit"`, `"overlapped"`, `"ipc"`, numeric fds, stream `.fd`, and extra Unix pipes `stdio[3+]`, `shell`, `detached`, `uid`, `gid`, `timeout`, `killSignal`, `signal` (`AbortSignal`), `serialization`, `windowsVerbatimArguments`, `windowsHide`; exposes `pid`, `spawnfile`, `spawnargs`, `killed`, `exitCode`, `signalCode`, `connected`, `stdio`, `kill()`, `send()`, `disconnect()`, `ref()`, `unref()`, `"spawn"` / `"exit"` / `"close"` / `"disconnect"` events)

[spawnSync](https://nodejs.org/api/child_process.html#child_processspawnsynccommand-args-options) (supports `input`, `encoding`, `timeout`, `killSignal`, `maxBuffer`, `cwd`, `env`, `argv0`, `stdio`, `shell`, `uid`, `gid`, `windowsVerbatimArguments`, `windowsHide`)

[exec](https://nodejs.org/api/child_process.html#child_processexeccommand-options-callback) / [execFile](https://nodejs.org/api/child_process.html#child_processexecfilefile-args-options-callback) (support `shell`, `cwd`, `env`, `uid`, `gid`, `argv0`, `timeout`, `killSignal`, `maxBuffer`, `encoding`, `signal`, `windowsHide`)

[execSync](https://nodejs.org/api/child_process.html#child_processexecsynccommand-options) / [execFileSync](https://nodejs.org/api/child_process.html#child_processexecfilesyncfile-args-options)

[fork](https://nodejs.org/api/child_process.html#child_processforkmodulepath-args-options) (Unix IPC supporting `"advanced"` `clone_wire` and `"json"` serialization)

## console

[Console](https://nodejs.org/api/console.html#class-console)

Global `console` conforms to the WHATWG Console namespace and exposes bound methods matching Node.js. `new Console(stdout[, stderr[, ignoreErrors]])` and `new Console({ stdout, stderr, ignoreErrors, colorMode, inspectOptions, groupIndentation })` are supported.

[assert](https://nodejs.org/api/console.html#consoleassertvalue-message)

[clear](https://nodejs.org/api/console.html#consoleclear)

[count](https://nodejs.org/api/console.html#consolecountlabel)

[countReset](https://nodejs.org/api/console.html#consolecountresetlabel)

[debug](https://nodejs.org/api/console.html#consoledebugdata-args)

[dir](https://nodejs.org/api/console.html#consoledirobj-options)

[dirxml](https://nodejs.org/api/console.html#consoledirxmldata)

[error](https://nodejs.org/api/console.html#consoleerrordata-args)

[group](https://nodejs.org/api/console.html#consolegrouplabel)

[groupCollapsed](https://nodejs.org/api/console.html#consolegroupcollapsed)

[groupEnd](https://nodejs.org/api/console.html#consolegroupend)

[info](https://nodejs.org/api/console.html#consoleinfodata-args)

[log](https://nodejs.org/api/console.html#consolelogdata-args)

[table](https://nodejs.org/api/console.html#consoletabletabulardata-properties)

[time](https://nodejs.org/api/console.html#consoletimelabel)

[timeEnd](https://nodejs.org/api/console.html#consoletimeendlabel)

[timeLog](https://nodejs.org/api/console.html#consoletimeloglabel-args)

[trace](https://nodejs.org/api/console.html#consoletracedata-args)

[warn](https://nodejs.org/api/console.html#consolewarndata-args)

Inspector / devtools methods on `console`: `profile()`, `profileEnd()`, `timeStamp()`.

> [!NOTE]
> `count`, `countReset`, `time`, `timeLog`, and `timeEnd` coerce labels with JavaScript `ToString` (numbers and objects are accepted). `dir` and `inspectOptions` support `colors`, `depth`, `showHidden`, `maxArrayLength`, `maxStringLength`, `compact`, `sorted`, and `customInspect`.

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

### lexe specific hash classes

Lightweight and fast hash classes for lexe.

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

[addMembership](https://nodejs.org/api/dgram.html#socketaddmembershipmulticastaddress-multicastinterface)

[addSourceSpecificMembership](https://nodejs.org/api/dgram.html#socketaddsourcespecificmembershipsourceaddress-groupaddress-multicastinterface)

[address](https://nodejs.org/api/dgram.html#socketaddress)

[bind](https://nodejs.org/api/dgram.html#socketbindport-address-callback)

[close](https://nodejs.org/api/dgram.html#socketclosecallback)

[connect](https://nodejs.org/api/dgram.html#socketconnectport-address-callback)

[disconnect](https://nodejs.org/api/dgram.html#socketdisconnect)

[dropMembership](https://nodejs.org/api/dgram.html#socketdropmembershipmulticastaddress-multicastinterface)

[dropSourceSpecificMembership](https://nodejs.org/api/dgram.html#socketdropsourcespecificmembershipsourceaddress-groupaddress-multicastinterface)

[getRecvBufferSize](https://nodejs.org/api/dgram.html#socketgetrecvbuffersize)

[getSendBufferSize](https://nodejs.org/api/dgram.html#socketgetsendbuffersize)

[getSendQueueCount](https://nodejs.org/api/dgram.html#socketgetsendqueuecount)

[getSendQueueSize](https://nodejs.org/api/dgram.html#socketgetsendqueuesize)

[ref](https://nodejs.org/api/dgram.html#socketref)

[remoteAddress](https://nodejs.org/api/dgram.html#socketremoteaddress)

[send](https://nodejs.org/api/dgram.html#socketsendmsg-offset-length-port-address-callback)

[setBroadcast](https://nodejs.org/api/dgram.html#socketsetbroadcastflag)

[setMulticastInterface](https://nodejs.org/api/dgram.html#socketsetmulticastinterfacemulticastinterface)

[setMulticastLoopback](https://nodejs.org/api/dgram.html#socketsetmulticastloopbackflag)

[setMulticastTTL](https://nodejs.org/api/dgram.html#socketsetmulticastttlttl)

[setRecvBufferSize](https://nodejs.org/api/dgram.html#socketsetrecvbuffersizesize)

[setSendBufferSize](https://nodejs.org/api/dgram.html#socketsetsendbuffersizesize)

[setTTL](https://nodejs.org/api/dgram.html#socketsetttlttl)

[unref](https://nodejs.org/api/dgram.html#socketunref)

[\[Symbol.asyncDispose\]](https://nodejs.org/api/dgram.html#socketsymbolasyncdispose)

## dns

Supports the full Node.js `node:dns` synchronous configuration, lookup, and DNS record resolution APIs: `Resolver` (`resolve`, `resolve4`, `resolve6`, `resolveAny`, `resolveCaa`, `resolveCname`, `resolveMx`, `resolveNaptr`, `resolveNs`, `resolvePtr`, `resolveSoa`, `resolveSrv`, `resolveTlsa`, `resolveTxt`, `reverse`, `cancel`, `getServers`, `setServers`, `setLocalAddress`), top-level default resolver methods, `lookup` (including `ADDRCONFIG`, `V4MAPPED`, `ALL` hints and `order`/`verbatim`), `lookupService`, `getServers`/`setServers`, `getDefaultResultOrder`/`setDefaultResultOrder`, `promises`, and all Node DNS error constants (`NODATA`, `NOTFOUND`, `SERVFAIL`, `TIMEOUT`, `CANCELLED`, etc.).

## dns/promises

Supports all Node.js `node:dns/promises` async DNS APIs (`Resolver`, `lookup`, `lookupService`, `resolve`, `resolve4`, `resolve6`, `resolveAny`, `resolveCaa`, `resolveCname`, `resolveMx`, `resolveNaptr`, `resolveNs`, `resolvePtr`, `resolveSoa`, `resolveSrv`, `resolveTlsa`, `resolveTxt`, `reverse`, `getServers`, `setServers`, `getDefaultResultOrder`, `setDefaultResultOrder`, and DNS error constants), sharing default state with `dns.promises`.

## diagnostics_channel

[channel](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channelchannelname)

[hasSubscribers](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channelhassubscribersname)

[subscribe](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channelsubscribename-onmessage)

[unsubscribe](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channelunsubscribename-onmessage)

[tracingChannel](https://nodejs.org/api/diagnostics_channel.html#diagnostics_channeltracingchannelnameorchannels)

### Class: Channel

[subscribe](https://nodejs.org/api/diagnostics_channel.html#channelsubscribeonmessage)

[unsubscribe](https://nodejs.org/api/diagnostics_channel.html#channelunsubscribeonmessage)

[bindStore](https://nodejs.org/api/diagnostics_channel.html#channelbindstorestore-transform)

[unbindStore](https://nodejs.org/api/diagnostics_channel.html#channelunbindstorestore)

[runStores](https://nodejs.org/api/diagnostics_channel.html#channelrunstorescontext-fn-thisarg-args)

[publish](https://nodejs.org/api/diagnostics_channel.html#channelpublishmessage)

[hasSubscribers](https://nodejs.org/api/diagnostics_channel.html#channelhassubscribers)

### Class: TracingChannel

[hasSubscribers](https://nodejs.org/api/diagnostics_channel.html#tracingchannelhassubscribers)

[subscribe](https://nodejs.org/api/diagnostics_channel.html#tracingchannelsubscribesubscribers)

[unsubscribe](https://nodejs.org/api/diagnostics_channel.html#tracingchannelunsubscribesubscribers)

[traceSync](https://nodejs.org/api/diagnostics_channel.html#tracingchanneltracesyncfn-context-thisarg-args)

[tracePromise](https://nodejs.org/api/diagnostics_channel.html#tracingchanneltracepromisefn-context-thisarg-args)

[traceCallback](https://nodejs.org/api/diagnostics_channel.html#tracingchanneltracecallbackfn-position-context-thisarg-args)

> [!NOTE]
> Built-in diagnostics channels are emitted for `udp.socket`, `net.client.socket`, `net.server.socket`, `http.client.request.created`, `http.client.request.start`, `http.client.request.error`, `http.client.response.finish`, `http.server.response.created`, `http.server.request.start`, and `http.server.response.finish`. Subscriber errors during `publish` and `bindStore` transform errors are reported asynchronously via `process.nextTick`.

## events

[EventEmitter](https://nodejs.org/api/events.html#class-eventemitter)

[EventEmitterAsyncResource](https://nodejs.org/api/events.html#class-eventemitterasyncresource)

[once](https://nodejs.org/api/events.html#eventsonceemitter-name-options)

[on](https://nodejs.org/api/events.html#eventsonemitter-eventname-options)

[getEventListeners](https://nodejs.org/api/events.html#eventsgeteventlistenersemitter-name)

[getMaxListeners](https://nodejs.org/api/events.html#eventsgetmaxlistenersemitter)

[setMaxListeners](https://nodejs.org/api/events.html#eventssetmaxlistenersn-...eventtargets)

[listenerCount](https://nodejs.org/api/events.html#eventemitterlistenercountemitter-eventname)

[addAbortListener](https://nodejs.org/api/events.html#eventsaddabortlistenersignal-listener)

`errorMonitor`, `captureRejectionSymbol`, `captureRejections`, `EventEmitterAsyncResource`, and `defaultMaxListeners` are exported and also attached to the `EventEmitter` constructor, matching Node where the default export is that constructor. `captureRejections` (via `new EventEmitter({ captureRejections: true })` or `EventEmitter.captureRejections`), `errorMonitor`, and `EventEmitterAsyncResource` async-context tracking are supported.

## fs

Supports the full Node.js `node:fs` synchronous, callback, stream, directory, and watcher APIs: `access`/`accessSync`, `appendFile`/`appendFileSync`, `chmod`/`chmodSync`, `chown`/`chownSync`, `close`/`closeSync`, `constants`, `copyFile`/`copyFileSync`, `cp`/`cpSync` (including `filter`, `errorOnExist`, `preserveTimestamps`, `dereference`, `verbatimSymlinks`), `createReadStream`/`createWriteStream` (including `fd`, `FileHandle`, `signal`, custom `fs`, `autoClose`, `emitClose`), `exists`/`existsSync`, `fchmod`/`fchmodSync`, `fchown`/`fchownSync`, `fdatasync`/`fdatasyncSync`, `fstat`/`fstatSync`, `fsync`/`fsyncSync`, `ftruncate`/`ftruncateSync`, `futimes`/`futimesSync`, `glob`/`globSync`, `lchmod`/`lchmodSync`, `lchown`/`lchownSync`, `link`/`linkSync`, `lstat`/`lstatSync`, `lutimes`/`lutimesSync`, `mkdir`/`mkdirSync`, `mkdtemp`/`mkdtempSync`, `open`/`openAsBlob`/`openSync`, `opendir`/`opendirSync`, `read`/`readSync`, `readFile`/`readFileSync`, `readdir`/`readdirSync`, `readlink`/`readlinkSync`, `readv`/`readvSync`, `realpath`/`realpathSync` (and `.native`), `rename`/`renameSync`, `rm`/`rmSync`, `rmdir`/`rmdirSync`, `stat`/`statSync` (including `{ bigint: true }` `BigIntStats` and `{ throwIfNoEntry: false }`), `statfs`/`statfsSync`, `symlink`/`symlinkSync`, `truncate`/`truncateSync`, `unlink`/`unlinkSync`, `utimes`/`utimesSync`, `watch`, `watchFile`/`unwatchFile`, `write`/`writeSync`, `writeFile`/`writeFileSync`, `writev`/`writevSync`, plus classes `Stats`, `BigIntStats`, `Dirent`, `Dir`, `FileHandle`, `FSWatcher`, `ReadStream`/`FileReadStream`, `WriteStream`/`FileWriteStream`.

> [!NOTE]
> On Unix, `access` / `accessSync` / `fs.promises.access` use the OS `access(2)` syscall.
> On Windows, permission checks are metadata-based approximations matching libuv behavior.

## fs/promises

Supports all Node.js `node:fs/promises` APIs: `access`, `appendFile`, `chmod`, `chown`, `constants`, `copyFile`, `cp`, `glob`, `lchmod`, `lchown`, `link`, `lstat`, `lutimes`, `mkdir`, `mkdtemp`, `open`, `opendir`, `readFile`, `readdir`, `readlink`, `realpath`, `rename`, `rm`, `rmdir`, `stat`, `statfs`, `symlink`, `truncate`, `unlink`, `utimes`, `watch` (`AsyncIterable`), `writeFile`, and `FileHandle` (`appendFile`, `chmod`, `chown`, `close`, `createReadStream`, `createWriteStream`, `datasync`, `fd`, `read`, `readFile`, `readv`, `stat`, `sync`, `truncate`, `utimes`, `write`, `writeFile`, `writev`, including `{ bigint: true }` and `position: -1`).

## http

Supports Node.js-compatible `node:http` APIs:

- Server & Lifecycle: [Server](https://nodejs.org/api/http.html#class-httpserver), [createServer](https://nodejs.org/api/http.html#httpcreateserveroptions-requestlistener) (TCP, Unix domain sockets, and Windows named pipes), [IncomingMessage](https://nodejs.org/api/http.html#class-httpincomingmessage), [OutgoingMessage](https://nodejs.org/api/http.html#class-httpoutgoingmessage), [ServerResponse](https://nodejs.org/api/http.html#class-httpserverresponse) (`writeHead`, `writeEarlyHints`, `writeProcessing`, `writeContinue`, `strictContentLength`, `setHeader`/`appendHeader`/`getHeader`/`getHeaderNames`/`getRawHeaderNames`/`getHeaders`/`hasHeader`/`removeHeader`/`setHeaders`, `addTrailers`, `flushHeaders`, `req`, `socket`, `connection`, `setTimeout`), `checkContinue`, `checkExpectation`, `clientError`, `upgrade`, `connect`, `drop`, `dropRequest`, `maxRequestsPerSocket`, `maxConnections`, `maxHeadersCount`, `headersTimeout`, `requestTimeout`, `keepAliveTimeout`, `timeout`, `setTimeout`, `closeAllConnections`, `closeIdleConnections`, `ref`/`unref`, and `insecureHTTPParser`.
- Client & WebSocket: [request](https://nodejs.org/api/http.html#httprequesturl-options-callback), [get](https://nodejs.org/api/http.html#httpgeturl-options-callback), [ClientRequest](https://nodejs.org/api/http.html#class-httpclientrequest) (header manipulation, `flushHeaders` streaming, `setTimeout`/`clearTimeout`, `setNoDelay`, `setSocketKeepAlive`, `socket`/`connection`/`reusedSocket`/`path`/`method`/`host`/`protocol`/`maxHeadersCount`, `URL` objects, `auth`, `agent`, `signal`, and Unix `socketPath`), [Agent](https://nodejs.org/api/http.html#class-httpagent), `globalAgent`, and Node 22+ client [WebSocket](https://nodejs.org/api/http.html#class-websocket).
- Module Constants & Helpers: all 35 `METHODS`, `STATUS_CODES`, `maxHeaderSize`, `setMaxIdleHTTPParsers`, `validateHeaderName`, and `validateHeaderValue`.

## https

Supports [Agent](https://nodejs.org/api/https.html#class-httpsagent), `globalAgent`, [Server](https://nodejs.org/api/https.html#class-httpsserver), [createServer](https://nodejs.org/api/https.html#httpscreateserveroptions-requestlistener), [request](https://nodejs.org/api/https.html#httpsrequesturl-options-callback), and [get](https://nodejs.org/api/https.html#httpsgeturl-options-callback) (built atop `node:tls` and `node:http`).

## http2

Node-compatible HTTP/2 surface via native module `lexe_http2`:

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

[connect](https://nodejs.org/api/net.html#netconnect)

[createConnection](https://nodejs.org/api/net.html#netcreateconnection)

[createServer](https://nodejs.org/api/net.html#netcreateserveroptions-connectionlistener)

- `net.Socket` is a `Duplex` stream supporting TCP, Unix domain sockets, and Windows named pipes (`\\.\pipe\...`), with `setTimeout()`, `destroySoon()`, `resetAndDestroy()`, `setNoDelay()`, `setKeepAlive()`, `ref()`/`unref()`, Happy-Eyeballs (`autoSelectFamily`, `autoSelectFamilyAttemptTimeout`, `autoSelectFamilyAttemptedAddresses`), `BlockList`, and `SocketAddress`.
- `net.Server` supports `{ allowHalfOpen, pauseOnConnect, noDelay, keepAlive, keepAliveInitialDelay, highWaterMark }`, `listen({ port, host, path, backlog })`, `listening`, `maxConnections` with `"drop"` events, `getConnections()`, and `ref()` / `unref()`.

## os

[arch](https://nodejs.org/api/os.html#osarch)

[availableParallelism](https://nodejs.org/api/os.html#osavailableparallelism)

[constants](https://nodejs.org/api/os.html#osconstants)

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

[matchesGlob](https://nodejs.org/api/path.html#pathmatchesglobpath-pattern)

[normalize](https://nodejs.org/api/path.html#pathnormalizepath)

[parse](https://nodejs.org/api/path.html#pathparsepath)

[posix](https://nodejs.org/api/path.html#pathposix)

[relative](https://nodejs.org/api/path.html#pathrelativefrom-to)

[resolve](https://nodejs.org/api/path.html#pathresolvepaths)

[sep](https://nodejs.org/api/path.html#pathsep)

[toNamespacedPath](https://nodejs.org/api/path.html#pathtonamespacedpathpath)

[win32](https://nodejs.org/api/path.html#pathwin32)

`_makeLong` (alias of `toNamespacedPath`)

## perf_hooks

_performance, Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList, and PerformanceResourceTiming are available globally_

- `performance.now()`, `performance.timeOrigin`, `performance.toJSON()`, `performance.mark()`, `performance.measure()`, `performance.getEntries()`, `performance.getEntriesByType()`, `performance.getEntriesByName()`, `performance.clearMarks()`, `performance.clearMeasures()`, `performance.clearResourceTimings()`, `performance.setResourceTimingBufferSize()`, `performance.markResourceTiming()`, `performance.onresourcetimingbufferfull`, `performance.nodeTiming`, `performance.eventLoopUtilization()`, and `performance.timerify()`
- `createHistogram()` (log-linear `HdrHistogram` with `lowest`/`highest`/`figures`, `BigInt` accessors, `recordDelta()`, `add()`, `percentiles`, `toJSON()`)
- `monitorEventLoopDelay()` (`ELDHistogram` measuring nanosecond event-loop delay without keeping the loop alive)
- `PerformanceObserver` supporting `"dns"`, `"function"`, `"gc"`, `"http"`, `"http2"`, `"mark"`, `"measure"`, `"net"`, and `"resource"` entries
- `constants` (`NODE_PERFORMANCE_GC_*`)

## process

_process is available globally_

Process is an EventEmitter. Explicit `process.exit(code)` emits `"exit"` synchronously before terminating. When the event loop drains naturally, `"beforeExit"` then `"exit"` are emitted with the current `process.exitCode`. OS signals (`SIGINT`, `SIGTERM`, `SIGHUP` on Unix; Ctrl+C on Windows) are bridged to `process.emit("signal", name)`; default termination matches Node when no `"signal"` listener is registered.

`process.version` / `process.versions.node` advertise Node `24.3.0` for ecosystem gates; `process.versions.lexe` and CLI `--version` remain the real Lexe version.

[abort](https://nodejs.org/api/process.html#processabort)

[allowedNodeEnvironmentFlags](https://nodejs.org/api/process.html#processallowednodeenvironmentflags)

[arch](https://nodejs.org/api/process.html#processarch)

[argv](https://nodejs.org/api/process.html#processargv)

[argv0](https://nodejs.org/api/process.html#processargv0)

[availableMemory](https://nodejs.org/api/process.html#processavailablememory)

[cwd](https://nodejs.org/api/process.html#processcwd)

[chdir](https://nodejs.org/api/process.html#processchdirdirectory)

[config](https://nodejs.org/api/process.html#processconfig)

[constrainedMemory](https://nodejs.org/api/process.html#processconstrainedmemory)

[cpuUsage](https://nodejs.org/api/process.html#processcpuusagepreviousvalue)

[env](https://nodejs.org/api/process.html#processenv)

[execArgv](https://nodejs.org/api/process.html#processexecargv)

[execPath](https://nodejs.org/api/process.html#processexecpath)

[exit](https://nodejs.org/api/process.html#processexitcode)

[exitCode](https://nodejs.org/api/process.html#processexitcode-1)

[features](https://nodejs.org/api/process.html#processfeatures)

[finalization](https://nodejs.org/api/process.html#processfinalizationregisterref-shutdown)

[getActiveResourcesInfo](https://nodejs.org/api/process.html#processgetactiveresourcesinfo)

[getegid](https://nodejs.org/api/process.html#processgetegid)

[geteuid](https://nodejs.org/api/process.html#processgeteuid)

[getgid](https://nodejs.org/api/process.html#processgetgid)

[getuid](https://nodejs.org/api/process.html#processgetuid)

[hasUncaughtExceptionCaptureCallback](https://nodejs.org/api/process.html#processhasuncaughtexceptioncapturecallback)

[hrtime](https://nodejs.org/api/process.html#processhrtime)

[loadEnvFile](https://nodejs.org/api/process.html#processloadenvfilepath)

[memoryUsage](https://nodejs.org/api/process.html#processmemoryusage) / `memoryUsage.rss()`

[permission](https://nodejs.org/api/process.html#processpermission) (enabled when running with `--permission`; pairs with `--allow-fs-read`, `--allow-fs-write`, `--allow-child-process`, `--allow-worker`, `--allow-addons`, `--allow-wasi`)

[pid](https://nodejs.org/api/process.html#processpid)

[ppid](https://nodejs.org/api/process.html#processppid)

[id](https://nodejs.org/api/process.html#processpid) (deprecated Lexe-legacy PID property; initialized to the same value as `process.pid`, remains writable for backward compatibility and is not a live alias of `pid`)

[kill](https://nodejs.org/api/process.html#processkillpid-signal)

[on](https://nodejs.org/api/events.html#emitteroneventname-listener) / EventEmitter methods (`once`, `off`, `emit`, …)

[platform](https://nodejs.org/api/process.html#processplatform)

[ref](https://nodejs.org/api/process.html#processrefmayberefable) / [unref](https://nodejs.org/api/process.html#processunrefmayberefable)

[release](https://nodejs.org/api/process.html#processrelease)

[report](https://nodejs.org/api/process.html#processreport) (`getReport()`, `writeReport()`, and standard report configuration fields)

[resourceUsage](https://nodejs.org/api/process.html#processresourceusage)

[setUncaughtExceptionCaptureCallback](https://nodejs.org/api/process.html#processsetuncaughtexceptioncapturecallbackfn)

[sourceMapsEnabled](https://nodejs.org/api/process.html#processsourcemapssupport) / [setSourceMapsEnabled](https://nodejs.org/api/process.html#processsetsourcemapsenabledval)

[setegid](https://nodejs.org/api/process.html#processsetegidgid)

[seteuid](https://nodejs.org/api/process.html#processseteuiduid)

[setgid](https://nodejs.org/api/process.html#processsetgidgid)

[setuid](https://nodejs.org/api/process.html#processsetuiduid)

[title](https://nodejs.org/api/process.html#processtitle)

[umask](https://nodejs.org/api/process.html#processumaskmask)

[uptime](https://nodejs.org/api/process.html#processuptime)

[version](https://nodejs.org/api/process.html#processversion)

[versions](https://nodejs.org/api/process.html#processversions)

## constants

Legacy Node.js `constants` module. Exports a frozen flat object (and matching ESM
named exports) combining:

- `fs.constants`: file access modes (`F_OK`, `R_OK`, `W_OK`, `X_OK`), open flags (`O_*`),
  file type and permission masks (`S_*`), `COPYFILE_*`, `UV_FS_*`, and `UV_DIRENT_*`
- Flattened `os.constants`: `errno`, `signals`, `dlopen`, and `priority`
- OpenSSL / crypto constants: `OPENSSL_VERSION_NUMBER`, `SSL_OP_*`, `ENGINE_METHOD_*`,
  `DH_*`, `RSA_*`, `TLS1*_VERSION`, `POINT_CONVERSION_*`, and `defaultCoreCipherList`

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

[unescapeBuffer](https://nodejs.org/api/querystring.html#querystringunescapebufferstr-decodespaces)

> [!NOTE]
> `encode` / `decode` follow Node.js and alias `stringify` / `parse`, not `escape` / `unescape`. Empty `sep` or `eq` arguments fall back to `&` and `=` respectively. `escape` throws `URIError` for lone surrogate code units.

## sqlite

Experimental `node:sqlite` implementation targeting Node.js 24 on SQLite 3.50.1. It is available only through the `node:` scheme, enabled by default, and can be disabled with `--no-experimental-sqlite`.

Exports:

- `DatabaseSync`
- `StatementSync`
- `backup()`
- changeset conflict and authorizer `constants` (`SQLITE_OK`, `SQLITE_DENY`, `SQLITE_IGNORE`, and SQLite authorizer action codes)

`DatabaseSync` supports in-memory and file databases, `{ open, readOnly, enableForeignKeyConstraints, enableDoubleQuotedStringLiterals, defensive, timeout, allowExtension, limits }` options, `open()`, `close()`, `exec()`, `prepare()`, `location()`, scalar `function()`, `aggregate()`, `createSession()`, `applyChangeset()`, `loadExtension()`, `enableLoadExtension()`, `enableDefensive()`, `setAuthorizer()`, `limits`, and `createTagStore()` (`SQLTagStore` with `run`, `get`, `all`, `iterate`, `clear`, `size`, `capacity`, `db`). `StatementSync` supports `run()`, `get()`, `all()`, `iterate()`, `columns()`, named-parameter controls, BigInt reads, and array rows. Session objects expose `changeset()`, `patchset()`, and `close()`, while internal `Session` and `SQLTagStore` constructors are intentionally not module exports.

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

[promises](https://nodejs.org/api/timers.html#timers-promises-api)

[queueMicrotask](https://nodejs.org/api/globals.html#queuemicrotaskcallback)

> `setTimeout` / `setInterval` return `Timeout` handles supporting `ref()`, `unref()`, `hasRef()`, `refresh()`, `close()`, `[Symbol.dispose]()`, and `[Symbol.toPrimitive]()`; `setImmediate` returns `Immediate` handles supporting `ref()`, `unref()`, `hasRef()`, and `[Symbol.dispose]()`. Both `setTimeout` and `setImmediate` support `util.promisify`. Unref'd timers do not keep the process alive when they are the only remaining handle.

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

PFX, custom cipher/signature configuration, DH parameters, CRLs, PSK, OCSP, session/ticket resumption, async `SNICallback`, `ALPNCallback`, soft mTLS, and related advanced APIs are not supported. `TLSSocket.write()` currently returns `true` without backpressure or `drain` signaling. See [`modules/lexe_tls/README.md`](modules/lexe_tls/README.md).

## inspector

Startup probe only — Lexe does **not** implement the Node Inspector debugging protocol.

[url](https://nodejs.org/api/inspector.html#inspectorurl) — always returns `undefined`

> `Session`, `open()`, `close()`, and `waitForDebugger()` are intentionally not exported.

## tty

[isatty](https://nodejs.org/api/tty.html#ttyisattyfd)

[ReadStream](https://nodejs.org/api/tty.html#class-ttyreadstream)

[WriteStream](https://nodejs.org/api/tty.html#class-ttywritestream)

> `ReadStream` / `WriteStream` (including `process.stdin` / `stdout` / `stderr` and arbitrary file descriptors) support `ref()` / `unref()`, non-stdio reads/writes, and automatic `SIGWINCH`-driven `"resize"` events on `WriteStream`. Stdin reader tasks stop blocking exit when unref'd; readline follows `process.stdin`.

## url

### Class

[URL](https://nodejs.org/api/url.html#class-url)

[URLPattern](https://nodejs.org/api/url.html#class-urlpattern)

[URLSearchParams](https://nodejs.org/api/url.html#class-urlsearchparams)

### Prototype methods

[domainToASCII](https://nodejs.org/api/url.html#urldomaintoasciidomain)

[domainToUnicode](https://nodejs.org/api/url.html#urldomaintounicodedomain)

[fileURLToPath](https://nodejs.org/api/url.html#urlfileurltopathurl-options)

[format](https://nodejs.org/api/url.html#urlformaturl-options)

[pathToFileURL](https://nodejs.org/api/url.html#urlpathtofileurlpath-options)

[urlToHttpOptions](https://nodejs.org/api/url.html#urlurltohttpoptionsurl)

### Legacy API

[Url](https://nodejs.org/api/url.html#legacy-urlobject), [parse](https://nodejs.org/api/url.html#urlparseurlstring-parsequerystring-slashesdenotehost), [resolve](https://nodejs.org/api/url.html#urlresolvefrom-to), `resolveObject` and `format` with a string or a legacy object are ported from `lib/url.js` and match Node 24 output. The first call to `parse` prints the `DEP0169` warning, as Node does, unless it is called from inside `node_modules`.

[fileURLToPathBuffer](https://nodejs.org/api/url.html#urlfileurltopathbufferurl-options)

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
> Lexe provides a minimal `vm.runInNewContext` implementation backed by an isolated QuickJS context in the same runtime. This is **not** a security boundary.

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

[unzip](https://nodejs.org/api/zlib.html#zlibunzipbuffer-options-callback)

[unzipSync](https://nodejs.org/api/zlib.html#zlibunzipsyncbuffer-options)

[crc32](https://nodejs.org/api/zlib.html#zlibcrc32data-value)

[brotliCompress](https://nodejs.org/api/zlib.html#zlibbrotlicompressbuffer-options-callback)

[brotliCompressSync](https://nodejs.org/api/zlib.html#zlibbrotlicompresssyncbuffer-options)

[brotliDecompress](https://nodejs.org/api/zlib.html#zlibbrotlidecompressbuffer-options-callback)

[brotliDecompressSync](https://nodejs.org/api/zlib.html#zlibbrotlidecompresssyncbuffer-options)

[zstdCompress](https://nodejs.org/api/zlib.html#zlibzstdcompressbuffer-options-callback)

[zstdCompressSync](https://nodejs.org/api/zlib.html#zlibzstdcompresssyncbuffer-options)

[zstdDecompress](https://nodejs.org/api/zlib.html#zlibzstddecompressbuffer-options-callback)

[zstdDecompressSync](https://nodejs.org/api/zlib.html#zlibzstddecompresssyncbuffer-options)

### Streaming

[createGzip](https://nodejs.org/api/zlib.html#zlibcreategzipoptions) / [createGunzip](https://nodejs.org/api/zlib.html#zlibcreategunzipoptions) / [createDeflate](https://nodejs.org/api/zlib.html#zlibcreatedeflateoptions) / [createInflate](https://nodejs.org/api/zlib.html#zlibcreateinflateoptions) / [createDeflateRaw](https://nodejs.org/api/zlib.html#zlibcreatedeflaterawoptions) / [createInflateRaw](https://nodejs.org/api/zlib.html#zlibcreateinflaterawoptions) / [createUnzip](https://nodejs.org/api/zlib.html#zlibcreateunzipoptions) / [createBrotliCompress](https://nodejs.org/api/zlib.html#zlibcreatebrotlicompressoptions) / [createBrotliDecompress](https://nodejs.org/api/zlib.html#zlibcreatebrotlidecompressoptions) / [createZstdCompress](https://nodejs.org/api/zlib.html#zlibcreatezstdcompressoptions) / [createZstdDecompress](https://nodejs.org/api/zlib.html#zlibcreatezstddecompressoptions)

The matching `Gzip`, `Gunzip`, `Deflate`, `Inflate`, `DeflateRaw`, `InflateRaw`, `Unzip`, `BrotliCompress`, `BrotliDecompress`, `ZstdCompress`, `ZstdDecompress` classes are exported as well, plus `constants`. Zlib streams and convenience methods honor `level`, `windowBits`, `memLevel`, `strategy`, and `dictionary`, and streams expose `flush(kind, cb)`, `params(level, strategy, cb)`, `reset()`, `close(cb)`, and `bytesWritten`.

Each factory returns a `stream.Transform` that emits a **single** framed stream across all chunks written to it (compressor state is kept between chunks), so it is interchangeable with Node's for middleware such as `compression`.

# lexe API

## lexe:hex

```typescript
export function encode(
  value: string | Array | ArrayBuffer | Uint8Array
): string;
export function decode(value: string): Uint8Array;
```

## Timezone-aware Intl

Lexe provides timezone support through the global `Intl.DateTimeFormat` and `Date.prototype.toLocaleString` APIs. There is no public `lexe:timezone` module.

### Intl.DateTimeFormat

lexe provides a minimal `Intl.DateTimeFormat` implementation focused on timezone support. This enables libraries like dayjs to work with timezone conversions transparently.

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

## lexe:qjs

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

## lexe:xml

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


## lexe:util

```typescript
export function dimensions(): [number, number];
export function load(path: string): any;
export function print(value: any): void;
```


# Web Platform API

## COMPRESSION

[CompressionStream](https://developer.mozilla.org/en-US/docs/Web/API/CompressionStream) / [DecompressionStream](https://developer.mozilla.org/en-US/docs/Web/API/DecompressionStream) (`gzip`, `deflate`, `deflate-raw`)

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

[URL.createObjectURL](https://developer.mozilla.org/en-US/docs/Web/API/URL/createObjectURL_static) and [URL.revokeObjectURL](https://developer.mozilla.org/en-US/docs/Web/API/URL/revokeObjectURL_static) accept a `Blob` or `File` and return `blob:nodedata:<uuid>` URLs, as in Node; `buffer.resolveObjectURL()` resolves them.

## WEBASSEMBLY

[WebAssembly](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface)

Lexe installs a fresh, realm-scoped `WebAssembly` global on every QuickJS context, backed by [`wasmi`](https://github.com/wasmi-labs/wasmi) `1.1.0`.

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
> `externref` lifetime: to preserve JS identity for every Wasm reference safely, Lexe retains the `Persistent<Value>` associated with an `externref` until its WebAssembly realm is torn down. `wasmi` 1.1.0 does not expose the complete set of roots required to reclaim individual entries safely (including Wasm-private tables, globals, and active frames). Consequently, a long-lived realm that continuously stores distinct JS values as `externref` can grow its retained memory until realm teardown; applications with that pattern should use bounded/reused references or periodically recreate the realm.

Not implemented (out of scope for this batch): `WebAssembly.Function`, `Tag`, `Exception`, JSPI (`Suspending`/`promising`), the garbage-collection, exception-handling, and function-references proposals, shared memory/threads, `Memory64`, WASI, and the `.wasm` ESM loader. `WebAssembly.validate`/`compile` reject modules using any of these as a `CompileError` (or `false`, for `validate`) rather than passing them through to `wasmi` or risking a panic. Lexe does not provide a WebAssembly-level security sandbox or execution-time/memory quota beyond what `wasmi`'s interpreter itself enforces.

> [!IMPORTANT]
> This implementation requires `wasmi 1.1.0` and Rust `1.86` or newer (the `lexe_webassembly` crate's own `rust-version`; it does not raise the minimum for any other workspace crate).

## WEBCRYPTO

[Crypto](https://developer.mozilla.org/en-US/docs/Web/API/Crypto)

[CryptoKey](https://developer.mozilla.org/en-US/docs/Web/API/CryptoKey)

[SubtleCrypto](https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto)

## WEBIDL

[DOMException](https://developer.mozilla.org/en-US/docs/Web/API/DOMException)

## XHR

[FormData](https://developer.mozilla.org/en-US/docs/Web/API/FormData)
