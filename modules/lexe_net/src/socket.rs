// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    io,
    net::{IpAddr, Shutdown, SocketAddr},
    str::FromStr,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_events::{EmitError, Emitter, EventEmitter, EventKey, EventList};
use lexe_stream::{
    readable::{ReadableStream, ReadableStreamInner},
    writable::{WritableStream, WritableStreamInner},
    SteamEvents,
};
use lexe_utils::{
    event_loop::EventLoopRef,
    object::ObjectExt,
    perf::{self, PerfEntryKind},
    result::ResultExt,
    time,
};
use rquickjs::{
    class::{Trace, Tracer},
    function::Constructor,
    prelude::{Opt, Rest, This},
    Array, Class, Ctx, Error, Exception, Function, IntoJs, JsLifetime, Object, Result, Value,
};
use socket2::{SockRef, TcpKeepalive};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions as NamedPipeClientOptions;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{lookup_host, tcp::OwnedReadHalf, tcp::OwnedWriteHalf, TcpStream},
    sync::{mpsc::UnboundedSender, oneshot::Receiver},
    task::JoinSet,
};
use tracing::trace;

use super::{
    ensure_access, get_address_parts, get_default_auto_select_family,
    get_default_auto_select_family_attempt_timeout, get_hostname, rw_join, ReadyState, LOCALHOST,
};

impl<'js> SteamEvents<'js> for Socket<'js> {
    fn emit_close(this: Class<'js, Self>, ctx: &Ctx<'js>, had_error: bool) -> Result<()> {
        // A closed socket owns no further I/O: it must stop keeping the process
        // alive, exactly like Node releasing the handle at `close`.
        this.borrow().loop_ref.unreference();
        // Emit first (listeners may inspect the handle), then drop the owner
        // reference so the cycle can be collected.
        let result = Self::emit_str(
            this.clone(),
            ctx,
            "close",
            vec![had_error.into_js(ctx)?],
            false,
        );
        this.borrow_mut().js_owner = None;
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    Attached,
    HandoffPending,
    Detached,
    Closed,
}

#[derive(Debug, Clone, Default)]
struct KeepAliveOptions {
    enabled: bool,
    initial_delay: Option<Duration>,
}

#[derive(Debug, Clone, Default)]
struct TcpSocketOptions {
    no_delay: Option<bool>,
    keep_alive: Option<KeepAliveOptions>,
}

enum RawShutdown {
    Tcp(Arc<std::net::TcpStream>),
    #[cfg(unix)]
    Unix(Arc<std::os::unix::net::UnixStream>),
}

#[rquickjs::class]
#[allow(dead_code)]
pub struct Socket<'js> {
    emitter: EventEmitter<'js>,
    readable_stream_inner: ReadableStreamInner<'js>,
    writable_stream_inner: WritableStreamInner<'js>,
    connecting: bool,
    destroyed: bool,
    pending: bool,
    local_address: Option<String>,
    local_family: Option<String>,
    local_port: Option<u16>,
    remote_address: Option<String>,
    remote_family: Option<String>,
    remote_port: Option<u16>,
    ready_state: ReadyState,
    allow_half_open: bool,
    raw_writer: Option<UnboundedSender<Vec<u8>>>,
    raw_reader: Option<Arc<Mutex<Vec<u8>>>>,
    raw_shutdown: Option<RawShutdown>,
    transport_state: TransportState,
    tcp_read_half: Option<OwnedReadHalf>,
    tcp_write_half: Option<OwnedWriteHalf>,
    handoff_prefix: Option<Vec<u8>>,
    tcp_options: TcpSocketOptions,
    auto_select_family_attempted_addresses: Vec<String>,
    /// Control handle for TCP socket options / force-close. Not shared with
    /// the raw HTTP shutdown handle (`raw_shutdown`).
    pub(crate) tcp_control: Option<Arc<std::net::TcpStream>>,
    loop_ref: Arc<EventLoopRef>,
    /// The JS `net.Socket` (a Duplex) owning this handle, when one exists.
    ///
    /// Released at `destroy()`/`close` so the `this <-> _handle` cycle becomes
    /// collectable before runtime teardown (see `gc_obj_list`).
    js_owner: Option<Object<'js>>,
}

unsafe impl<'js> JsLifetime<'js> for Socket<'js> {
    type Changed<'to> = Socket<'to>;
}

impl<'js> Trace<'js> for Socket<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
        // The owning Duplex must stay alive while the handle holds it: an
        // untraced `Persistent` lets the collector free the object, and every
        // later restore hands out a dangling reference (which surfaces as
        // `TypeError: RustClass object expected` on the next class probe).
        self.js_owner.trace(tracer);
    }
}

impl<'js> Emitter<'js> for Socket<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }

    /// A Socket is a Duplex: the listener table lives on the JS object.
    fn emits_through_js() -> bool {
        true
    }

    fn js_owner(&self, _ctx: &Ctx<'js>) -> Option<Object<'js>> {
        self.js_owner.clone()
    }

    fn on_event_changed(&mut self, _event: EventKey<'js>, _added: bool) -> Result<()> {
        // Readable/writable state is owned by the JS Duplex; mirroring listener
        // changes here would make `ReadableStreamInner.state` a second, unsynced
        // authority over "should we keep reading".
        Ok(())
    }
}

impl<'js> ReadableStream<'js> for Socket<'js> {
    fn inner_mut(&mut self) -> &mut ReadableStreamInner<'js> {
        &mut self.readable_stream_inner
    }

    fn inner(&self) -> &ReadableStreamInner<'js> {
        &self.readable_stream_inner
    }
}

impl<'js> WritableStream<'js> for Socket<'js> {
    fn inner_mut(&mut self) -> &mut WritableStreamInner<'js> {
        &mut self.writable_stream_inner
    }

    fn inner(&self) -> &WritableStreamInner<'js> {
        &self.writable_stream_inner
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Socket<'js> {
    /// Create the native backend for the JS `net.Socket` class.
    pub fn handle_factory(ctx: Ctx<'js>, opts: Opt<Object<'js>>) -> Result<Class<'js, Self>> {
        let mut allow_half_open = false;
        if let Some(opts) = opts.0 {
            if let Some(v) = opts.get_optional("allowHalfOpen")? {
                allow_half_open = v;
            }
        }
        Self::new(ctx, allow_half_open)
    }

    /// Bind the owning JS Duplex so native events reach its EventEmitter.
    pub fn set_owner(this: This<Class<'js, Self>>, owner: Object<'js>) -> Class<'js, Self> {
        this.borrow_mut().js_owner = Some(owner);
        this.0
    }

    /// The JS Duplex owning this handle. Read by the stream read loop so it can
    /// `push` into the JS object rather than the bare handle.
    #[qjs(get, enumerable)]
    pub fn js_owner_ref(&self) -> Option<Object<'js>> {
        self.js_owner.clone()
    }

    /// Whether the native writer has not started yet, so a write should wait.
    ///
    /// Only `Closed` means "no transport attached yet": the Duplex parks the
    /// write in its own `_writableState` until `connect`/`process_stream`
    /// attaches one. `Attached` writes immediately through, and
    /// `HandoffPending`/`Detached` are *permanently* unwritable (the transport
    /// belongs to a TLS/HTTP layer now), so those must reach the native write
    /// path and produce its error instead of being parked forever.
    #[qjs(get, enumerable)]
    pub fn transport_attached(&self) -> bool {
        self.raw_writer.is_some() || self.transport_state != TransportState::Closed
    }

    /// `_read` told us the consumer drained: let the native read loop continue.
    pub fn resume_reading(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().readable_stream_inner.resume_reading();
        this.0
    }

    /// `pause()` told us to back off: park the native read loop.
    pub fn pause_reading(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().readable_stream_inner.pause_reading();
        this.0
    }

    pub fn reset_and_destroy(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        if let Some(control) = this.borrow().tcp_control.as_ref() {
            let _ = SockRef::from(control.as_ref()).set_linger(Some(Duration::ZERO));
        }
        Self::destroy_inner(this, Opt(None), false)
    }

    #[qjs(constructor)]
    pub fn ctor(ctx: Ctx<'js>, opts: Opt<Object<'js>>) -> Result<Class<'js, Self>> {
        let mut allow_half_open = false;
        if let Some(opts) = opts.0 {
            if let Some(opt_allow_half_open) = opts.get_optional("allowHalfOpen")? {
                allow_half_open = opt_allow_half_open;
            }
        }

        Self::new(ctx, allow_half_open)
    }

    #[qjs(get, enumerable)]
    pub fn connecting(&self) -> bool {
        self.connecting
    }

    #[qjs(get, enumerable)]
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// Node-compatible: `!destroyed && !readableEnded`.
    #[qjs(get, enumerable)]
    pub fn readable(&self) -> bool {
        !self.destroyed && !self.readable_stream_inner.is_ended()
    }

    /// Node-compatible: `!destroyed && !writableFinished`.
    #[qjs(get, enumerable)]
    pub fn writable(&self) -> bool {
        !self.destroyed && !self.writable_stream_inner.is_finished()
    }

    #[qjs(get, enumerable)]
    pub fn destroyed(&self) -> bool {
        self.destroyed
    }

    #[qjs(get, enumerable)]
    pub fn readable_ended(&self) -> bool {
        self.readable_stream_inner.is_ended()
    }

    /// Node duplex semantics: finished write side that was not force-destroyed.
    #[qjs(get, enumerable)]
    pub fn writable_ended(&self) -> bool {
        self.writable_stream_inner.is_finished() && !self.writable_stream_inner.is_destroyed()
    }

    #[qjs(get, enumerable)]
    pub fn remote_address(&self) -> Option<String> {
        self.remote_address.clone()
    }

    pub fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        cb: Opt<Function<'js>>,
    ) -> Result<()> {
        if Self::blocks_transport_writes(&this.borrow()) {
            return Err(Exception::throw_message(
                &ctx,
                "Socket transport is not available for writes",
            ));
        }
        if let Some(writer) = this.borrow().raw_writer.clone() {
            let bytes = lexe_utils::bytes::ObjectBytes::from(&ctx, &value)?.into_bytes(&ctx)?;
            writer
                .send(bytes)
                .map_err(|_| Exception::throw_message(&ctx, "Socket is closed"))?;
            if let Some(callback) = cb.0 {
                callback.call::<_, ()>(())?;
            }
            return Ok(());
        }
        WritableStream::write_flushed(this, ctx.clone(), value, cb)?;
        Ok(())
    }

    pub fn end(this: This<Class<'js, Self>>, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        if Self::blocks_transport_writes(&this.borrow()) {
            return Err(Exception::throw_message(
                &ctx,
                "Socket transport is not available for writes",
            ));
        }
        let mut args = args.0.into_iter();
        let first = args.next();
        let (value, callback) = match first {
            Some(value) if value.is_function() => (None, value.into_function()),
            Some(value) => (
                Some(value),
                args.next().and_then(|value| value.into_function()),
            ),
            None => (None, None),
        };
        if let Some(cb) = callback {
            Self::add_event_listener_str(this.clone(), &ctx, "end", cb, true, true)?;
        }

        if this.borrow().raw_writer.is_some() {
            if let Some(value) = value {
                let bytes = lexe_utils::bytes::ObjectBytes::from(&ctx, &value)?.into_bytes(&ctx)?;
                if let Some(writer) = &this.borrow().raw_writer {
                    writer
                        .send(bytes)
                        .map_err(|_| Exception::throw_message(&ctx, "Socket is closed"))?;
                }
            }
            this.borrow_mut().raw_writer.take();
            return Ok(());
        }
        if let Some(value) = value {
            WritableStream::write_flushed(This(this.0.clone()), ctx.clone(), value, Opt(None))?;
        }
        //ReadableStream::destroy(This(this.clone()), ctx.clone())?;
        WritableStream::end(this);

        Ok(())
    }

    pub fn destroy(this: This<Class<'js, Self>>, error: Opt<Value<'js>>) -> Class<'js, Self> {
        Self::destroy_inner(this, error, true)
    }

    fn destroy_inner(
        this: This<Class<'js, Self>>,
        error: Opt<Value<'js>>,
        shutdown: bool,
    ) -> Class<'js, Self> {
        {
            let mut borrow = this.borrow_mut();
            // When no I/O loop is active to call `emit_close`, release the JS owner now
            // so `this <-> _handle` doesn't pin GC objects across teardown.
            if matches!(borrow.transport_state, TransportState::Closed) && !borrow.connecting {
                borrow.js_owner = None;
            }
            if let Some(control) = borrow.tcp_control.take() {
                if shutdown {
                    let _ = control.shutdown(Shutdown::Both);
                }
            }
            if shutdown {
                if let Some(stream) = &borrow.raw_shutdown {
                    match stream {
                        RawShutdown::Tcp(stream) => {
                            let _ = stream.shutdown(Shutdown::Both);
                        },
                        #[cfg(unix)]
                        RawShutdown::Unix(stream) => {
                            let _ = stream.shutdown(Shutdown::Both);
                        },
                    }
                }
            }
        }
        this.borrow_mut().destroyed = true;
        // `destroy()` can happen before the connect path ever runs, so release
        // the loop reference here too (Node does it at close/destroy).
        this.borrow().loop_ref.unreference();
        ReadableStream::destroy(This(this.clone()), Opt(None));
        WritableStream::destroy(This(this.clone()), error);
        this.0
    }

    /// Node-compatible `socket.ref()` — keep the event loop alive while the socket is open.
    #[qjs(rename = "ref")]
    pub fn js_ref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.reference();
        this.0
    }

    /// Node-compatible `socket.unref()` — allow the process to exit if the socket is
    /// the only remaining handle.
    #[qjs(rename = "unref")]
    pub fn js_unref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.unreference();
        this.0
    }

    /// Node-compatible `socket.setNoDelay([noDelay])`. Defaults to `true`.
    pub fn set_no_delay(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        no_delay: Opt<bool>,
    ) -> Result<Class<'js, Self>> {
        let enable = no_delay.0.unwrap_or(true);
        let control = {
            let mut borrow = this.borrow_mut();

            // Skip redundant syscalls when the same value is already stored.
            if borrow.tcp_options.no_delay == Some(enable) {
                None
            } else {
                borrow.tcp_options.no_delay = Some(enable);

                // Detached (post-STARTTLS) sockets only update local state.
                if borrow.transport_state == TransportState::Detached {
                    None
                } else {
                    borrow.tcp_control.clone()
                }
            }
        };

        if let Some(control) = control {
            control
                .set_nodelay(enable)
                .map_err(|err| Exception::throw_message(&ctx, &err.to_string()))?;
        }

        Ok(this.0)
    }

    /// Node-compatible `socket.setKeepAlive([enable[, initialDelay]])`.
    /// Only the traditional two-argument form is supported.
    pub fn set_keep_alive(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        enable: Opt<Value<'js>>,
        initial_delay: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let enabled = match enable.0 {
            Some(value) => value_to_bool(&value),
            None => false,
        };
        let delay_secs = normalize_keep_alive_delay(&ctx, initial_delay.0)?;
        let delay = if delay_secs == 0 {
            None
        } else {
            Some(Duration::from_secs(delay_secs))
        };

        let apply = {
            let mut borrow = this.borrow_mut();
            let prev_delay = borrow
                .tcp_options
                .keep_alive
                .as_ref()
                .and_then(|ka| ka.initial_delay);
            // Keep prior delay when disabling so re-enable can reapply it.
            let saved_delay = delay.or(prev_delay);

            let options = KeepAliveOptions {
                enabled,
                initial_delay: if enabled {
                    delay.or(prev_delay)
                } else {
                    saved_delay
                },
            };

            let unchanged = borrow.tcp_options.keep_alive.as_ref().map(|ka| {
                ka.enabled == options.enabled && ka.initial_delay == options.initial_delay
            }) == Some(true);

            if unchanged {
                None
            } else {
                borrow.tcp_options.keep_alive = Some(options.clone());
                if borrow.transport_state == TransportState::Detached {
                    None
                } else {
                    borrow.tcp_control.clone().map(|c| (c, options))
                }
            }
        };

        if let Some((control, options)) = apply {
            apply_keep_alive(&control, &options)
                .map_err(|err| Exception::throw_message(&ctx, &err.to_string()))?;
        }

        Ok(this.0)
    }

    pub fn read(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        size: Opt<usize>,
    ) -> Result<Value<'js>> {
        if let Some(reader) = &this.borrow().raw_reader {
            let mut bytes = reader.lock().unwrap();
            if !bytes.is_empty() {
                let count = size.0.unwrap_or(bytes.len()).min(bytes.len());
                return Buffer(bytes.drain(..count).collect()).into_js(&ctx);
            }
        }
        ReadableStream::read(this, ctx, size)
    }

    #[qjs(get, enumerable)]
    pub fn local_address(&self) -> Option<String> {
        self.local_address.clone()
    }

    #[qjs(get, enumerable)]
    pub fn remote_family(&self) -> Option<String> {
        self.remote_family.clone()
    }

    #[qjs(get, enumerable)]
    pub fn local_family(&self) -> Option<String> {
        self.local_family.clone()
    }

    #[qjs(get, enumerable)]
    pub fn remote_port(&self) -> Option<u16> {
        self.remote_port
    }

    #[qjs(get, enumerable)]
    pub fn local_port(&self) -> Option<u16> {
        self.local_port
    }

    #[qjs(get, enumerable)]
    pub fn auto_select_family_attempted_addresses(&self) -> Vec<String> {
        self.auto_select_family_attempted_addresses.clone()
    }

    #[qjs(get, enumerable)]
    pub fn ready_state(&self) -> String {
        self.ready_state.to_string()
    }

    pub fn connect(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let args = args.0;

        let borrow = this.borrow();
        let allow_half_open = borrow.allow_half_open;
        if borrow.destroyed {
            return Err(Exception::throw_message(&ctx, "Socket destroyed"));
        }
        drop(borrow);

        let mut port = None;
        let mut host = String::from(LOCALHOST);
        let mut path = None;
        let mut listener = None;
        let mut last = None;
        let mut family: u8 = 0;
        let mut auto_select_family = get_default_auto_select_family();
        let mut auto_select_family_attempt_timeout =
            get_default_auto_select_family_attempt_timeout();
        let mut connect_no_delay: Option<bool> = None;
        let mut connect_keep_alive: Option<bool> = None;
        let mut connect_keep_alive_initial_delay: Option<Value<'js>> = None;

        let mut args = args.into_iter();

        if let Some(first) = args.next() {
            if let Some(opts) = first.as_object() {
                port = opts.get_optional("port")?;
                path = opts.get_optional("path")?;
                if let Some(host_arg) = opts.get_optional("host")? {
                    host = host_arg
                }
                if let Some(fam) = opts.get_optional::<_, i32>("family")? {
                    if !matches!(fam, 0 | 4 | 6) {
                        return Err(super::blocklist::throw_coded(
                            &ctx,
                            super::blocklist::CodedError::Type,
                            "ERR_INVALID_ARG_VALUE",
                            "The property 'options.family' must be one of: 0, 4, 6",
                        ));
                    }
                    family = fam as u8;
                }
                if let Some(asf) = opts.get_optional::<_, bool>("autoSelectFamily")? {
                    auto_select_family = asf;
                }
                if let Some(t) = opts.get_optional::<_, f64>("autoSelectFamilyAttemptTimeout")? {
                    let t = t.trunc();
                    if !(1.0..=2147483647.0).contains(&t) {
                        return Err(super::blocklist::throw_coded(
                            &ctx,
                            super::blocklist::CodedError::Range,
                            "ERR_OUT_OF_RANGE",
                            "The value of \"options.autoSelectFamilyAttemptTimeout\" is out of range",
                        ));
                    }
                    auto_select_family_attempt_timeout = t as u32;
                }
                connect_no_delay = opts.get_optional("noDelay")?;
                connect_keep_alive = opts.get_optional("keepAlive")?;
                connect_keep_alive_initial_delay = opts.get_optional("keepAliveInitialDelay")?;
            } else if let Some(path_arg) = first.as_string() {
                path = Some(path_arg.to_string()?);
            } else if let Some(port_arg) = first.as_int() {
                port = Some(port_arg as u16);
                if let Some(next) = args.next() {
                    if let Some(host_arg) = next.as_string() {
                        host = host_arg.to_string()?;
                    } else {
                        last = Some(next)
                    }
                }
            }
        }

        if let Some(last) = last.or_else(|| args.next()) {
            if let Some(cb) = last.as_function() {
                listener = Some(cb.to_owned());
            }
        }

        if path.is_none() && port.is_none() {
            return Err(Exception::throw_type(&ctx, "port or path are required"));
        }

        if let Some(path) = path.clone() {
            ensure_access(&ctx, &path)?;
        }
        if let Some(port) = port {
            let hostname = get_hostname(&host, port);
            ensure_access(&ctx, &hostname)?;
        }

        let this = this.0;

        if let Some(nd) = connect_no_delay {
            let _ = Self::set_no_delay(This(this.clone()), ctx.clone(), Opt(Some(nd)))?;
        }
        if connect_keep_alive.is_some() || connect_keep_alive_initial_delay.is_some() {
            let enable = Value::new_bool(ctx.clone(), connect_keep_alive.unwrap_or(false));
            let _ = Self::set_keep_alive(
                This(this.clone()),
                ctx.clone(),
                Opt(Some(enable)),
                Opt(connect_keep_alive_initial_delay),
            )?;
        }

        {
            let mut borrow = this.borrow_mut();
            borrow.connecting = true;
            borrow.auto_select_family_attempted_addresses.clear();
        }

        let this2 = this.clone();

        if let Some(listener) = listener {
            Socket::add_event_listener_str(this.clone(), &ctx, "connect", listener, false, true)?;
        }

        let perf_start_ms = perf::has_observers(perf::rt_key_for_ctx(&ctx), PerfEntryKind::Net)
            .then(time::monotonic_elapsed_ms);

        ctx.clone().spawn_exit(async move {
            let ctx2 = ctx.clone();
            let ctx3 = ctx.clone();
            let this3 = this2.clone();
            if this3.borrow().destroyed {
                Socket::emit_close(this3.clone(), &ctx3, false)?;
                return Ok(());
            }
            let detail_path = perf_start_ms.as_ref().and_then(|_| path.clone());
            let detail_host = perf_start_ms.as_ref().map(|_| host.clone());
            let detail_port = port;
            let connect = async move {
                let (readable_done, writable_done) = if let Some(path) = path {
                    #[cfg(unix)]
                    {
                        let stream = UnixStream::connect(path).await.or_throw(&ctx3)?;
                        Self::process_unix_stream(&this2, &ctx3, stream, allow_half_open)
                    }
                    #[cfg(windows)]
                    {
                        let client = loop {
                            match NamedPipeClientOptions::new().open(&path) {
                                Ok(client) => break client,
                                Err(err) if err.raw_os_error() == Some(231) => {
                                    tokio::time::sleep(Duration::from_millis(5)).await;
                                },
                                Err(err) => return Err(err).or_throw(&ctx3),
                            }
                        };
                        Self::mark_connected(&this2);
                        Self::process_io(&this2, &ctx3, client, allow_half_open)
                    }
                    #[cfg(all(not(unix), not(windows)))]
                    {
                        _ = path;
                        return Err(Exception::throw_type(
                            &ctx3,
                            "Unix domain sockets are not supported on this platform",
                        ));
                    }
                } else if let Some(port) = port {
                    let stream = Self::connect_tcp(
                        &this2,
                        &ctx3,
                        host,
                        port,
                        family,
                        auto_select_family,
                        auto_select_family_attempt_timeout,
                    )
                    .await?;
                    Self::process_tcp_stream(&this2, &ctx3, stream, allow_half_open)
                } else {
                    unreachable!()
                }?;

                // Start the join/cleanup task *before* emitting `connect`.
                // If a connect listener throws, tcp_control must still be released.
                let this4 = this2.clone();
                let ctx4 = ctx3.clone();
                ctx3.spawn_exit_simple(async move {
                    let _ = async {
                        let join_result =
                            rw_join(&ctx4, &this4, readable_done, writable_done).await;
                        // Drop the option-control clone after drivers finish, even on error.
                        // This must not affect a normally closed primary stream.
                        this4.borrow_mut().tcp_control = None;
                        let had_error = join_result?;

                        if !matches!(
                            this4.borrow().transport_state,
                            TransportState::HandoffPending | TransportState::Detached
                        ) {
                            Socket::emit_close(this4, &ctx4, had_error)?;
                        }

                        Ok::<_, Error>(())
                    }
                    .await;
                    Ok(())
                });

                if let Some(start_ms) = perf_start_ms {
                    let dur = (time::monotonic_elapsed_ms() - start_ms).max(0.0);
                    perf::emit_node_entry(
                        &ctx3,
                        PerfEntryKind::Net,
                        "connect",
                        start_ms,
                        dur,
                        |ctx| {
                            let detail = Object::new(ctx.clone())?;
                            if let Some(p) = &detail_path {
                                detail.set("path", p.as_str())?;
                            } else {
                                if let Some(h) = &detail_host {
                                    detail.set("host", h.as_str())?;
                                }
                                if let Some(port_num) = detail_port {
                                    detail.set("port", port_num)?;
                                }
                            }
                            Ok(detail.into_value())
                        },
                    );
                }

                Socket::emit_str(this2.clone(), &ctx3, "connect", vec![], false)?;

                Ok::<_, Error>(())
            }
            .await;

            if connect.is_err() {
                this3.borrow_mut().connecting = false;
            }
            connect.emit_error("connect", &ctx2, this3)?;
            Ok(())
        })?;

        Ok(this)
    }
}

impl<'js> Socket<'js> {
    pub fn new(ctx: Ctx<'js>, allow_half_open: bool) -> Result<Class<'js, Self>> {
        let emitter = EventEmitter::new();

        let readable_stream_inner = ReadableStreamInner::new(emitter.clone(), false);
        let writable_stream_inner = WritableStreamInner::new(emitter.clone(), false);

        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);

        let instance = Class::instance(
            ctx,
            Self {
                emitter,
                connecting: false,
                destroyed: false,
                pending: true,
                ready_state: ReadyState::Closed,
                local_address: None,
                local_family: None,
                local_port: None,
                remote_address: None,
                remote_family: None,
                remote_port: None,
                readable_stream_inner,
                writable_stream_inner,
                allow_half_open,
                raw_writer: None,
                raw_reader: None,
                raw_shutdown: None,
                transport_state: TransportState::Closed,
                tcp_read_half: None,
                tcp_write_half: None,
                handoff_prefix: None,
                tcp_options: TcpSocketOptions::default(),
                auto_select_family_attempted_addresses: Vec::new(),
                tcp_control: None,
                loop_ref,
                js_owner: None,
            },
        )?;
        Ok(instance)
    }

    pub fn is_detached(&self) -> bool {
        self.transport_state == TransportState::Detached
    }

    async fn connect_tcp(
        this: &Class<'js, Self>,
        ctx: &Ctx<'js>,
        host: String,
        port: u16,
        family: u8,
        auto_select_family: bool,
        attempt_timeout_ms: u32,
    ) -> Result<TcpStream> {
        if let Ok(ip) = IpAddr::from_str(&host) {
            if (family == 4 && ip.is_ipv6()) || (family == 6 && ip.is_ipv4()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Address {host} does not match family {family}"),
                ))
                .or_throw(ctx);
            }
            let addr = SocketAddr::new(ip, port);
            if auto_select_family {
                this.borrow_mut()
                    .auto_select_family_attempted_addresses
                    .push(addr.to_string());
            }
            return TcpStream::connect(addr).await.or_throw(ctx);
        }

        let resolved: Vec<SocketAddr> = lookup_host((host.as_str(), port))
            .await
            .or_throw(ctx)?
            .filter(|a| match family {
                4 => a.is_ipv4(),
                6 => a.is_ipv6(),
                _ => true,
            })
            .collect();

        if resolved.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("No addresses resolved for {host}:{port}"),
            ))
            .or_throw(ctx);
        }

        let (ordered, use_happy_eyeballs) = if auto_select_family && family == 0 {
            let mut v4 = Vec::new();
            let mut v6 = Vec::new();
            for addr in &resolved {
                if addr.is_ipv6() {
                    v6.push(*addr);
                } else {
                    v4.push(*addr);
                }
            }
            if !v4.is_empty() && !v6.is_empty() {
                let first_is_v6 = resolved[0].is_ipv6();
                let mut interleaved = Vec::with_capacity(resolved.len());
                let (mut i4, mut i6) = (0usize, 0usize);
                let mut turn_v6 = first_is_v6;
                while i4 < v4.len() || i6 < v6.len() {
                    if turn_v6 {
                        if i6 < v6.len() {
                            interleaved.push(v6[i6]);
                            i6 += 1;
                        } else if i4 < v4.len() {
                            interleaved.push(v4[i4]);
                            i4 += 1;
                        }
                    } else if i4 < v4.len() {
                        interleaved.push(v4[i4]);
                        i4 += 1;
                    } else if i6 < v6.len() {
                        interleaved.push(v6[i6]);
                        i6 += 1;
                    }
                    turn_v6 = !turn_v6;
                }
                (interleaved, true)
            } else {
                (resolved, false)
            }
        } else {
            (resolved, false)
        };

        if !use_happy_eyeballs {
            let mut errors: Vec<(SocketAddr, io::Error)> = Vec::new();
            for addr in ordered {
                if auto_select_family {
                    this.borrow_mut()
                        .auto_select_family_attempted_addresses
                        .push(addr.to_string());
                }
                match TcpStream::connect(addr).await {
                    Ok(stream) => return Ok(stream),
                    Err(err) => errors.push((addr, err)),
                }
            }
            return Err(build_connect_error(ctx, errors));
        }

        let timeout_dur = Duration::from_millis(attempt_timeout_ms.max(10) as u64);
        let mut join_set: JoinSet<(SocketAddr, io::Result<TcpStream>)> = JoinSet::new();
        let mut errors: Vec<(SocketAddr, io::Error)> = Vec::new();
        let mut idx = 0usize;

        while idx < ordered.len() || !join_set.is_empty() {
            if idx < ordered.len() {
                let addr = ordered[idx];
                idx += 1;
                this.borrow_mut()
                    .auto_select_family_attempted_addresses
                    .push(addr.to_string());
                join_set.spawn(async move { (addr, TcpStream::connect(addr).await) });
            }

            if idx < ordered.len() {
                let timer = tokio::time::sleep(timeout_dur);
                tokio::pin!(timer);
                loop {
                    tokio::select! {
                        _ = &mut timer => break,
                        res = join_set.join_next() => {
                            match res {
                                Some(Ok((_addr, Ok(stream)))) => {
                                    join_set.abort_all();
                                    return Ok(stream);
                                },
                                Some(Ok((addr, Err(err)))) => {
                                    errors.push((addr, err));
                                    if join_set.is_empty() {
                                        break;
                                    }
                                },
                                Some(Err(_)) | None => break,
                            }
                        }
                    }
                }
            } else if let Some(res) = join_set.join_next().await {
                match res {
                    Ok((_addr, Ok(stream))) => {
                        join_set.abort_all();
                        return Ok(stream);
                    },
                    Ok((addr, Err(err))) => errors.push((addr, err)),
                    Err(_) => {},
                }
            }
        }

        Err(build_connect_error(ctx, errors))
    }

    fn blocks_transport_writes(socket: &Socket<'_>) -> bool {
        matches!(
            socket.transport_state,
            TransportState::HandoffPending | TransportState::Detached
        )
    }

    pub async fn begin_tls_handoff(
        ctx: &Ctx<'js>,
        this: Class<'js, Socket<'js>>,
    ) -> Result<(OwnedReadHalf, OwnedWriteHalf, Vec<u8>)> {
        {
            let borrow = this.borrow();
            if borrow.destroyed {
                return Err(Exception::throw_message(ctx, "Socket destroyed"));
            }
            if borrow.raw_writer.is_some() || borrow.raw_reader.is_some() {
                return Err(Exception::throw_message(
                    ctx,
                    "STARTTLS handoff is not supported for raw transport sockets",
                ));
            }
            if borrow.transport_state != TransportState::Attached {
                return Err(Exception::throw_message(
                    ctx,
                    "Socket transport is not attached",
                ));
            }
            if borrow.ready_state != ReadyState::Open {
                return Err(Exception::throw_message(ctx, "Socket is not connected"));
            }
        }

        this.borrow_mut().transport_state = TransportState::HandoffPending;

        let flush_rx = WritableStream::request_flush_barrier(this.clone(), ctx)?;
        match tokio::time::timeout(Duration::from_secs(10), flush_rx).await {
            Ok(Ok(())) => {},
            Ok(Err(_)) => {
                this.borrow_mut().transport_state = TransportState::Attached;
                return Err(Exception::throw_message(ctx, "Flush barrier failed"));
            },
            Err(_) => {
                this.borrow_mut().transport_state = TransportState::Attached;
                return Err(Exception::throw_message(ctx, "Flush barrier timeout"));
            },
        }

        this.borrow_mut().transport_state = TransportState::Detached;

        let fail_handoff = |this: &Class<'js, Socket<'js>>| {
            let mut borrow = this.borrow_mut();
            borrow.transport_state = TransportState::Closed;
            borrow.destroyed = true;
        };

        let read_rx = match ReadableStream::request_handoff::<OwnedReadHalf>(this.clone(), ctx) {
            Ok(rx) => rx,
            Err(err) => {
                fail_handoff(&this);
                return Err(err);
            },
        };
        let write_rx = match WritableStream::request_handoff::<OwnedWriteHalf>(this.clone(), ctx) {
            Ok(rx) => rx,
            Err(err) => {
                fail_handoff(&this);
                return Err(err);
            },
        };

        let (read_half, read_prefix) =
            match tokio::time::timeout(Duration::from_secs(10), read_rx).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) | Err(_) => {
                    fail_handoff(&this);
                    return Err(Exception::throw_message(ctx, "Readable handoff failed"));
                },
            };
        let write_half = match tokio::time::timeout(Duration::from_secs(10), write_rx).await {
            Ok(Ok(half)) => half,
            Ok(Err(_)) | Err(_) => {
                fail_handoff(&this);
                return Err(Exception::throw_message(ctx, "Writable handoff failed"));
            },
        };

        let prefix = read_prefix;

        {
            let mut borrow = this.borrow_mut();
            borrow.handoff_prefix = Some(prefix.clone());
            // Full handoff succeeded: drop the TCP control handle so detached
            // sockets only update local option state.
            borrow.tcp_control = None;
        }

        Ok((read_half, write_half, prefix))
    }

    pub fn process_tcp_stream(
        this: &Class<'js, Self>,
        ctx: &Ctx<'js>,
        stream: TcpStream,
        allow_half_open: bool,
    ) -> Result<(Receiver<bool>, Receiver<bool>)> {
        Self::set_addresses(this, ctx, &stream)?;

        let std_stream = stream.into_std().or_throw(ctx)?;
        let control_std = std_stream.try_clone().or_throw(ctx)?;
        let control = Arc::new(control_std);

        let options = this.borrow().tcp_options.clone();
        if let Err(err) = apply_tcp_options(&control, &options) {
            return Err(Exception::throw_message(ctx, &err.to_string()));
        }

        this.borrow_mut().tcp_control = Some(control);

        let stream = match TcpStream::from_std(std_stream) {
            Ok(stream) => stream,
            Err(err) => {
                this.borrow_mut().tcp_control = None;
                return Err(Exception::throw_message(ctx, &err.to_string()));
            },
        };
        let (reader, writer) = stream.into_split();
        match Self::process_stream(this, ctx, reader, writer, allow_half_open) {
            Ok(done) => Ok(done),
            Err(err) => {
                // Clear control handle if the stream driver failed to start.
                this.borrow_mut().tcp_control = None;
                Err(err)
            },
        }
    }

    /// Attach a raw byte writer for upgraded HTTP connections. The HTTP
    /// connection owns the transport, so this bypasses the normal socket
    /// stream driver for the write half while preserving the public API.
    pub fn attach_raw_writer(this: &Class<'js, Self>, writer: UnboundedSender<Vec<u8>>) {
        this.borrow_mut().raw_writer = Some(writer);
    }

    pub fn attach_raw_reader(this: &Class<'js, Self>, reader: Arc<Mutex<Vec<u8>>>) {
        this.borrow_mut().raw_reader = Some(reader);
    }

    pub fn attach_raw_tcp_shutdown(this: &Class<'js, Self>, stream: Arc<std::net::TcpStream>) {
        this.borrow_mut().raw_shutdown = Some(RawShutdown::Tcp(stream));
    }

    #[cfg(unix)]
    pub fn attach_raw_unix_shutdown(
        this: &Class<'js, Self>,
        stream: Arc<std::os::unix::net::UnixStream>,
    ) {
        this.borrow_mut().raw_shutdown = Some(RawShutdown::Unix(stream));
    }

    /// Marks a socket supplied by a server listener as an already-open
    /// connection. Unlike an outbound `connect()`, there is no connect event
    /// or pending phase for an accepted socket.
    pub fn mark_connected(this: &Class<'js, Self>) {
        let mut socket = this.borrow_mut();
        socket.connecting = false;
        socket.pending = false;
        socket.ready_state = ReadyState::Open;
    }

    #[cfg(unix)]
    pub fn process_unix_stream(
        this: &Class<'js, Self>,
        ctx: &Ctx<'js>,
        stream: UnixStream,
        allow_half_open: bool,
    ) -> Result<(Receiver<bool>, Receiver<bool>)> {
        let (reader, writer) = stream.into_split();
        Self::process_stream(this, ctx, reader, writer, allow_half_open)
    }

    pub fn process_io<T: AsyncRead + AsyncWrite + Send + 'static + 'js + Unpin>(
        this: &Class<'js, Self>,
        ctx: &Ctx<'js>,
        stream: T,
        allow_half_open: bool,
    ) -> Result<(Receiver<bool>, Receiver<bool>)> {
        let (reader, writer) = tokio::io::split(stream);
        Self::process_stream(this, ctx, reader, writer, allow_half_open)
    }

    fn process_stream<
        R: AsyncRead + Send + 'static + 'js + Unpin,
        W: AsyncWrite + Send + 'static + 'js + Unpin,
    >(
        this: &Class<'js, Self>,
        ctx: &Ctx<'js>,
        reader: R,
        writer: W,
        allow_half_open: bool,
    ) -> Result<(Receiver<bool>, Receiver<bool>)> {
        let loop_ref = this.borrow().loop_ref.clone();
        // A JS `Duplex` owning this handle (`net.Socket`) gets the push-driven
        // loop: it owns flowing/paused and backpressure, so the classic
        // `ReadableStreamInner.state` machine must not run beside it. Everything
        // else keeps the untouched classic path.
        let owner = {
            let borrow = this.borrow();
            borrow.js_owner_ref()
        };
        let end_this = this.clone();
        let on_end = move || {
            if !allow_half_open && !end_this.borrow().is_detached() {
                WritableStream::end(This(end_this.clone()));
            }
        };
        let readable_done = match owner.clone() {
            Some(owner) => ReadableStream::process_callback_loop_duplex(
                this.clone(),
                ctx,
                reader,
                on_end,
                owner,
                loop_ref.clone(),
            )?,
            None => ReadableStream::process_callback_loop(
                this.clone(),
                ctx,
                reader,
                on_end,
                loop_ref.clone(),
            )?,
        };
        let writable_done =
            WritableStream::process_loop(this.clone(), ctx, writer, Some(loop_ref))?;

        trace!("Connected to stream");
        let mut borrow = this.borrow_mut();
        borrow.connecting = false;
        borrow.pending = false;
        borrow.ready_state = ReadyState::Open;
        borrow.transport_state = TransportState::Attached;
        drop(borrow);

        // Release a write the Duplex held back while no transport was attached.
        if let Some(owner) = owner {
            if let Ok(hook) = owner.get::<_, Value>("_onWritesReady") {
                if let Some(hook) = hook.as_function() {
                    hook.call::<_, ()>((This(owner.clone()),))?;
                }
            }
        }

        Ok((readable_done, writable_done))
    }

    pub fn set_addresses<'a>(
        this: &'a Class<'js, Self>,
        ctx: &Ctx<'js>,
        stream: &TcpStream,
    ) -> Result<()> {
        let mut borrow = this.borrow_mut();

        let (remote_address, remote_port, remote_family) =
            get_address_parts(ctx, stream.peer_addr())?;
        borrow.remote_address = Some(remote_address);
        borrow.remote_port = Some(remote_port);
        borrow.remote_family = Some(remote_family);

        let (local_address, local_port, local_family) =
            get_address_parts(ctx, stream.local_addr())?;
        borrow.local_address = Some(local_address);
        borrow.local_port = Some(local_port);
        borrow.local_family = Some(local_family);

        drop(borrow);
        Ok(())
    }
}

fn build_connect_error(ctx: &Ctx<'_>, errors: Vec<(SocketAddr, io::Error)>) -> Error {
    if errors.len() <= 1 {
        let err = errors
            .into_iter()
            .next()
            .map(|(_, e)| e)
            .unwrap_or_else(|| io::Error::other("Connection failed"));
        return Err::<(), _>(err).or_throw(ctx).unwrap_err();
    }

    if let Ok(ctor) = ctx.globals().get::<_, Constructor>("AggregateError") {
        if let Ok(arr) = Array::new(ctx.clone()) {
            for (i, (addr, err)) in errors.iter().enumerate() {
                let msg = format!("connect {} {addr}", err);
                let _ = arr.set(i, Exception::from_message(ctx.clone(), &msg).ok());
            }
            if let Ok(agg) = ctor.construct::<_, Value>((arr, "")) {
                return ctx.throw(agg);
            }
        }
    }

    let (_, first) = errors.into_iter().next().unwrap();
    Err::<(), _>(first).or_throw(ctx).unwrap_err()
}

fn value_to_bool(value: &Value<'_>) -> bool {
    if let Some(b) = value.as_bool() {
        return b;
    }
    if let Some(n) = value.as_number() {
        return n != 0.0 && !n.is_nan();
    }
    if value.is_null() || value.is_undefined() {
        return false;
    }
    if let Some(s) = value.as_string() {
        return s.to_string().map(|s| !s.is_empty()).unwrap_or(false);
    }
    true
}

fn normalize_keep_alive_delay(ctx: &Ctx<'_>, value: Option<Value<'_>>) -> Result<u64> {
    let Some(value) = value else {
        return Ok(0);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(0);
    }
    let Some(n) = value.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            "The \"initialDelay\" argument must be of type number.",
        ));
    };
    if !n.is_finite() || n < 0.0 {
        return Err(Exception::throw_range(
            ctx,
            "The value of \"initialDelay\" is out of range.",
        ));
    }
    // Truncate toward zero; values under 1000ms become 0 seconds.
    Ok((n as u64) / 1000)
}

fn apply_keep_alive(stream: &std::net::TcpStream, options: &KeepAliveOptions) -> io::Result<()> {
    let sock = SockRef::from(stream);
    sock.set_keepalive(options.enabled)?;
    if options.enabled {
        if let Some(delay) = options.initial_delay {
            if !delay.is_zero() {
                let ka = TcpKeepalive::new().with_time(delay);
                sock.set_tcp_keepalive(&ka)?;
            }
            // delay == 0: enable keepalive only; keep the OS default idle timeout.
        }
    }
    Ok(())
}

fn apply_tcp_options(stream: &std::net::TcpStream, options: &TcpSocketOptions) -> io::Result<()> {
    if let Some(no_delay) = options.no_delay {
        stream.set_nodelay(no_delay)?;
    }
    if let Some(ref keep_alive) = options.keep_alive {
        apply_keep_alive(stream, keep_alive)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lexe_buffer as buffer;
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};
    use rquickjs::{
        function::IntoArgs, module::Evaluated, prelude::Opt, prelude::This, Class, Ctx, FromJs,
        Module, Value,
    };
    use socket2::SockRef;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use crate::NetModule;

    async fn server() -> u16 {
        // Use port 0 to let the OS assign an available port
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Spawn the accept loop so we can return the port immediately
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.set_nodelay(true).unwrap();

            // Read
            let mut buf = vec![0; 1024];
            let n = stream.read(&mut buf).await.unwrap();

            // Write
            stream.write_all(&buf[..n]).await.unwrap();
            stream.flush().await.unwrap();
        });

        port
    }

    async fn call_test_delay<'js, T, A>(
        ctx: &Ctx<'js>,
        module: &Module<'js, Evaluated>,
        args: A,
    ) -> T
    where
        T: FromJs<'js>,
        A: IntoArgs<'js>,
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
        call_test::<T, _>(ctx, module, args).await
    }

    #[tokio::test]
    async fn test_server_echo() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<NetModule>(ctx.clone(), "net")
                    .await
                    .unwrap();

                // Start server and get OS-assigned port
                let port = server().await;

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { connect } from 'net';

                        export async function test(port) {
                            const socket = connect({ port });
                            const txData = "Hello World";
                            return new Promise((resolve, reject) => {
                                socket.on('connect', () => {
                                    socket.write(txData, (err) => {
                                        if (err) {
                                            reject(err);
                                        }
                                    });
                                });
                                socket.on('data', (rxData) => {
                                    resolve(rxData.toString() === txData);
                                });
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let ok: bool = call_test_delay(&ctx, &module, (port,)).await;
                assert!(ok)
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_set_no_delay_default_and_pre_connect() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<NetModule>(ctx.clone(), "net")
                    .await
                    .unwrap();

                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let (hold_tx, hold_rx) = tokio::sync::oneshot::channel::<()>();
                tokio::spawn(async move {
                    let (_stream, _) = listener.accept().await.unwrap();
                    let _ = hold_rx.await;
                });

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { Socket } from 'net';
                        export async function test(port) {
                            const socket = new Socket();
                            const ret = socket.setNoDelay();
                            if (ret !== socket) return 'not-this';
                            return new Promise((resolve, reject) => {
                                socket.on('connect', () => {
                                    resolve(socket);
                                });
                                socket.on('error', reject);
                                socket.setNoDelay(true);
                                socket.connect(port, '127.0.0.1');
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let socket: Class<crate::Socket> = call_test_delay(&ctx, &module, (port,)).await;
                {
                    let borrow = socket.borrow();
                    assert_eq!(borrow.tcp_options.no_delay, Some(true));
                    let control = borrow.tcp_control.as_ref().expect("tcp_control");
                    assert!(control.nodelay().unwrap());
                }
                socket.borrow_mut().tcp_control = None;
                let _ = hold_tx.send(());
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_set_no_delay_toggle_after_connect() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<NetModule>(ctx.clone(), "net")
                    .await
                    .unwrap();

                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let (hold_tx, hold_rx) = tokio::sync::oneshot::channel::<()>();
                tokio::spawn(async move {
                    let (_stream, _) = listener.accept().await.unwrap();
                    let _ = hold_rx.await;
                });

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { connect } from 'net';
                        export async function test(port) {
                            const socket = connect({ port, host: '127.0.0.1' });
                            return new Promise((resolve, reject) => {
                                socket.on('connect', () => {
                                    socket.setNoDelay(false);
                                    resolve(socket);
                                });
                                socket.on('error', reject);
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let socket: Class<crate::Socket> = call_test_delay(&ctx, &module, (port,)).await;
                {
                    let borrow = socket.borrow();
                    assert_eq!(borrow.tcp_options.no_delay, Some(false));
                    assert!(!borrow.tcp_control.as_ref().unwrap().nodelay().unwrap());
                }
                // Toggle back to true
                {
                    let this = This(socket.clone());
                    crate::Socket::set_no_delay(this, ctx.clone(), Opt(Some(true))).unwrap();
                    assert!(socket
                        .borrow()
                        .tcp_control
                        .as_ref()
                        .unwrap()
                        .nodelay()
                        .unwrap());
                }
                socket.borrow_mut().tcp_control = None;
                let _ = hold_tx.send(());
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_set_keep_alive_pre_and_post_connect() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<NetModule>(ctx.clone(), "net")
                    .await
                    .unwrap();

                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let (hold_tx, hold_rx) = tokio::sync::oneshot::channel::<()>();
                tokio::spawn(async move {
                    let (_stream, _) = listener.accept().await.unwrap();
                    let _ = hold_rx.await;
                });

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { Socket } from 'net';
                        export async function test(port) {
                            const socket = new Socket();
                            socket.setKeepAlive(true, 0);
                            return new Promise((resolve, reject) => {
                                socket.on('connect', () => {
                                    resolve(socket);
                                });
                                socket.on('error', reject);
                                socket.connect(port, '127.0.0.1');
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let socket: Class<crate::Socket> = call_test_delay(&ctx, &module, (port,)).await;
                {
                    let borrow = socket.borrow();
                    let ka = borrow.tcp_options.keep_alive.as_ref().unwrap();
                    assert!(ka.enabled);
                    assert!(ka.initial_delay.is_none());
                    let control = borrow.tcp_control.as_ref().unwrap();
                    assert!(SockRef::from(control.as_ref()).keepalive().unwrap());
                }

                // Immediately on connect path: set again with positive delay.
                {
                    let this = This(socket.clone());
                    crate::Socket::set_keep_alive(
                        this,
                        ctx.clone(),
                        Opt(Some(Value::new_bool(ctx.clone(), true))),
                        Opt(Some(Value::new_int(ctx.clone(), 3000))),
                    )
                    .unwrap();
                    let borrow = socket.borrow();
                    let ka = borrow.tcp_options.keep_alive.as_ref().unwrap();
                    assert_eq!(ka.initial_delay, Some(Duration::from_secs(3)));
                    assert!(SockRef::from(borrow.tcp_control.as_ref().unwrap().as_ref())
                        .keepalive()
                        .unwrap());
                }

                socket.borrow_mut().tcp_control = None;
                let _ = hold_tx.send(());
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_set_keep_alive_invalid_delay() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<NetModule>(ctx.clone(), "net")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { Socket } from 'net';
                        export async function test() {
                            const socket = new Socket();
                            const cases = [];
                            for (const d of [-1, NaN, Infinity, -Infinity]) {
                                try {
                                    socket.setKeepAlive(true, d);
                                    cases.push('ok');
                                } catch (e) {
                                    cases.push(e.name || 'Error');
                                }
                            }
                            return cases;
                        }
                    "#,
                )
                .await
                .unwrap();

                let cases: Vec<String> = call_test(&ctx, &module, ()).await;
                assert_eq!(cases.len(), 4);
                for c in cases {
                    assert_eq!(c, "RangeError");
                }
            })
        })
        .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_set_options_on_unix_socket_noop() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<NetModule>(ctx.clone(), "net")
                    .await
                    .unwrap();

                let dir = std::env::temp_dir();
                let path = dir.join(format!("lexe-net-ka-{}.sock", std::process::id()));
                let _ = std::fs::remove_file(&path);
                let listener = tokio::net::UnixListener::bind(&path).unwrap();
                let path_str = path.to_string_lossy().to_string();
                let (hold_tx, hold_rx) = tokio::sync::oneshot::channel::<()>();
                tokio::spawn(async move {
                    let (_stream, _) = listener.accept().await.unwrap();
                    let _ = hold_rx.await;
                });

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { connect } from 'net';
                        export async function test(path) {
                            const socket = connect(path);
                            return new Promise((resolve, reject) => {
                                socket.on('connect', () => {
                                    const a = socket.setNoDelay(true);
                                    const b = socket.setKeepAlive(true, 0);
                                    resolve(a === socket && b === socket);
                                });
                                socket.on('error', reject);
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let ok: bool = call_test_delay(&ctx, &module, (path_str,)).await;
                assert!(ok);
                let _ = hold_tx.send(());
                let _ = std::fs::remove_file(&path);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_connect_listener_throw_still_clears_tcp_control() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<NetModule>(ctx.clone(), "net")
                    .await
                    .unwrap();

                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let port = listener.local_addr().unwrap().port();
                tokio::spawn(async move {
                    let (_stream, _) = listener.accept().await.unwrap();
                    // Hold until the client destroys / remote EOF completes join.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                });

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { Socket } from 'net';
                        export async function test(port) {
                            const socket = new Socket();
                            return new Promise((resolve) => {
                                socket.on('connect', () => {
                                    // Join cleanup is scheduled before emit invokes this listener.
                                    throw new Error('connect listener boom');
                                });
                                socket.on('error', () => {
                                    // Tear down so rw_join completes and clears tcp_control.
                                    socket.destroy();
                                });
                                socket.on('close', () => resolve(socket));
                                socket.connect(port, '127.0.0.1');
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let socket: Class<crate::Socket> = call_test_delay(&ctx, &module, (port,)).await;

                assert!(
                    socket.borrow().tcp_control.is_none(),
                    "tcp_control must be cleared even if connect listener throws"
                );
            })
        })
        .await;
    }
}
