// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        Arc, RwLock,
    },
};

use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_events::{EmitError, Emitter, EventEmitter, EventKey, EventList};
use lexe_utils::{
    bytes::ObjectBytes,
    event_loop::{self, EventLoopRef},
    latch::Latch,
    object::ObjectExt,
    result::ResultExt,
};
use rquickjs::{
    class::{Trace, Tracer},
    prelude::{Opt, Rest, This},
    Class, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime, Object, Result, Value,
};
use tokio::{
    net::UdpSocket,
    sync::{broadcast, mpsc, oneshot},
};
use tracing::trace;

use crate::sockopts::{
    configure_and_bind_socket, disconnect_udp_socket, make_coded_exception, resolve_socket_addr,
    sock_get_recv_buffer_size, sock_get_send_buffer_size, sock_membership, sock_set_broadcast,
    sock_set_multicast_interface, sock_set_multicast_loopback, sock_set_multicast_ttl,
    sock_set_recv_buffer_size, sock_set_send_buffer_size, sock_set_ttl, sock_source_membership,
    throw_coded, SocketBindOptions,
};

const CONNECT_DISCONNECTED: u8 = 0;
const CONNECT_CONNECTING: u8 = 1;
const CONNECT_CONNECTED: u8 = 2;

type SendResult = std::result::Result<usize, String>;
type SendMessage = (
    Vec<u8>,
    Option<(String, u16)>,
    Option<oneshot::Sender<SendResult>>,
);

#[rquickjs::class]
pub struct Socket<'js> {
    emitter: EventEmitter<'js>,
    socket: Option<Arc<UdpSocket>>,
    is_bound: Arc<AtomicBool>,
    is_closed: Arc<AtomicBool>,
    is_ipv6: bool,
    bind_options: SocketBindOptions,
    local_address: Option<String>,
    local_port: Option<u16>,
    local_family: Option<String>,
    connect_state: Arc<AtomicU8>,
    remote_addr: Arc<RwLock<Option<SocketAddr>>>,
    connect_latch: Arc<Latch>,
    receiver_running: Arc<AtomicBool>,
    close_tx: broadcast::Sender<()>,
    send_tx: Option<mpsc::UnboundedSender<SendMessage>>,
    send_queue_bytes: Arc<AtomicUsize>,
    send_queue_count: Arc<AtomicUsize>,
    ready_latch: Arc<Latch>,
    loop_ref: Arc<EventLoopRef>,
}

unsafe impl<'js> JsLifetime<'js> for Socket<'js> {
    type Changed<'to> = Socket<'to>;
}

impl<'js> Trace<'js> for Socket<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

impl<'js> Emitter<'js> for Socket<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }

    fn on_event_changed(&mut self, _event: EventKey<'js>, _added: bool) -> Result<()> {
        Ok(())
    }
}

fn validate_port(ctx: &Ctx<'_>, val: &Value<'_>) -> Result<u16> {
    let port = if let Some(i) = val.as_int() {
        i as f64
    } else if let Some(n) = val.as_number() {
        n
    } else {
        return Err(throw_coded(
            ctx,
            "RangeError",
            "ERR_SOCKET_BAD_PORT",
            "Port should be > 0 and < 65536.",
        ));
    };

    if !port.is_finite() || port.fract() != 0.0 || port <= 0.0 || port >= 65536.0 {
        return Err(throw_coded(
            ctx,
            "RangeError",
            "ERR_SOCKET_BAD_PORT",
            format!("Port should be > 0 and < 65536. Received {port}."),
        ));
    }
    Ok(port as u16)
}

fn parse_bind_port(ctx: &Ctx<'_>, val: &Value<'_>) -> Result<u16> {
    let port = if let Some(i) = val.as_int() {
        i as f64
    } else if let Some(n) = val.as_number() {
        n
    } else {
        return Err(throw_coded(
            ctx,
            "RangeError",
            "ERR_SOCKET_BAD_PORT",
            "Port should be >= 0 and < 65536.",
        ));
    };
    if !port.is_finite() || port.fract() != 0.0 || port < 0.0 || port >= 65536.0 {
        return Err(throw_coded(
            ctx,
            "RangeError",
            "ERR_SOCKET_BAD_PORT",
            format!("Port should be >= 0 and < 65536. Received {port}."),
        ));
    }
    Ok(port as u16)
}

fn extract_single_bytes<'js>(ctx: &Ctx<'js>, val: Value<'js>) -> Result<Vec<u8>> {
    if let Some(str_val) = val.as_string() {
        return Ok(str_val.to_string()?.into_bytes());
    }
    ObjectBytes::from_js(ctx, val)
        .map_err(|_| {
            throw_coded(
                ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "First argument must be a Buffer, TypedArray, DataView, or string.",
            )
        })?
        .try_into()
        .map_err(|e: std::rc::Rc<str>| Exception::throw_type(ctx, &e))
}

fn extract_send_bytes<'js>(
    ctx: &Ctx<'js>,
    msg: Value<'js>,
    slice: Option<(Value<'js>, Value<'js>)>,
) -> Result<Vec<u8>> {
    if let Some((offset_val, length_val)) = slice {
        if msg.is_array() {
            return Err(throw_coded(
                ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "The \"buffer\" argument must not be an array when offset and length are specified.",
            ));
        }
        let bytes = extract_single_bytes(ctx, msg)?;
        let offset = offset_val
            .as_int()
            .map(|i| i as i64)
            .or_else(|| offset_val.as_number().map(|n| n as i64))
            .ok_or_else(|| {
                throw_coded(
                    ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_TYPE",
                    "The \"offset\" argument must be of type number.",
                )
            })?;
        let length = length_val
            .as_int()
            .map(|i| i as i64)
            .or_else(|| length_val.as_number().map(|n| n as i64))
            .ok_or_else(|| {
                throw_coded(
                    ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_TYPE",
                    "The \"length\" argument must be of type number.",
                )
            })?;

        if offset < 0 || (offset as usize) > bytes.len() {
            return Err(throw_coded(
                ctx,
                "RangeError",
                "ERR_OUT_OF_RANGE",
                "The value of \"offset\" is out of range.",
            ));
        }
        let offset = offset as usize;
        if length < 0 || offset.saturating_add(length as usize) > bytes.len() {
            return Err(throw_coded(
                ctx,
                "RangeError",
                "ERR_OUT_OF_RANGE",
                "The value of \"length\" is out of range.",
            ));
        }
        let end = offset + (length as usize);
        return Ok(bytes[offset..end].to_vec());
    }

    if let Some(arr) = msg.as_array() {
        let mut combined = Vec::new();
        for item in arr.iter::<Value<'js>>() {
            let item = item?;
            combined.extend_from_slice(&extract_single_bytes(ctx, item)?);
        }
        return Ok(combined);
    }

    extract_single_bytes(ctx, msg)
}

fn wire_abort_signal<'js>(
    ctx: &Ctx<'js>,
    signal: Value<'js>,
    socket_class: &Class<'js, Socket<'js>>,
) -> Result<()> {
    let check_and_attach: Function<'js> = ctx.eval(
        r#"(function (signal, socket) {
  if (typeof globalThis.AbortSignal === "function" && !(signal instanceof globalThis.AbortSignal)) {
    const err = new TypeError('The "options.signal" property must be an AbortSignal.');
    err.code = "ERR_INVALID_ARG_TYPE";
    throw err;
  }
  if (!signal || typeof signal.aborted !== "boolean") {
    const err = new TypeError('The "options.signal" property must be an AbortSignal.');
    err.code = "ERR_INVALID_ARG_TYPE";
    throw err;
  }
  if (signal.aborted) {
    try { socket.close(); } catch (_) {}
    return;
  }
  if (typeof signal.addEventListener === "function") {
    signal.addEventListener("abort", function () {
      try { socket.close(); } catch (_) {}
    }, { once: true });
  }
})"#,
    )?;
    check_and_attach.call::<_, ()>((signal, socket_class.clone()))
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Socket<'js> {
    #[qjs(constructor)]
    pub fn ctor(
        ctx: Ctx<'js>,
        type_or_options: Value<'js>,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        let mut bind_options = SocketBindOptions::default();
        let mut signal_val: Option<Value<'js>> = None;

        let socket_type = if let Some(obj) = type_or_options.as_object() {
            let ty = obj
                .get_optional::<_, String>("type")?
                .ok_or_else(|| {
                    throw_coded(
                        &ctx,
                        "TypeError",
                        "ERR_INVALID_ARG_TYPE",
                        "The \"options.type\" property must be 'udp4' or 'udp6'.",
                    )
                })?;
            bind_options.reuse_addr = obj.get_optional::<_, bool>("reuseAddr")?.unwrap_or(false);
            bind_options.reuse_port = obj.get_optional::<_, bool>("reusePort")?.unwrap_or(false);
            bind_options.ipv6_only = obj.get_optional::<_, bool>("ipv6Only")?.unwrap_or(false);
            bind_options.recv_buffer_size = obj.get_optional::<_, usize>("recvBufferSize")?;
            bind_options.send_buffer_size = obj.get_optional::<_, usize>("sendBufferSize")?;
            if let Some(sig) = obj.get_optional::<_, Value<'js>>("signal")? {
                if !sig.is_undefined() && !sig.is_null() {
                    signal_val = Some(sig);
                }
            }
            ty
        } else if let Some(type_str) = type_or_options.as_string() {
            type_str.to_string()?
        } else {
            return Err(throw_coded(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "Socket type must be 'udp4' or 'udp6'.",
            ));
        };

        if socket_type != "udp4" && socket_type != "udp6" {
            return Err(throw_coded(
                &ctx,
                "Error",
                "ERR_SOCKET_BAD_TYPE",
                format!("Bad socket type specified. Valid types are: udp4, udp6. Received: {socket_type}"),
            ));
        }

        let is_ipv6 = socket_type == "udp6";
        let emitter = EventEmitter::new();
        let (close_tx, _) = broadcast::channel(1);
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);

        let instance = Self {
            emitter,
            socket: None,
            is_bound: Arc::new(AtomicBool::new(false)),
            is_closed: Arc::new(AtomicBool::new(false)),
            is_ipv6,
            bind_options,
            local_address: None,
            local_port: None,
            local_family: Some(if is_ipv6 {
                "IPv6".to_string()
            } else {
                "IPv4".to_string()
            }),
            connect_state: Arc::new(AtomicU8::new(CONNECT_DISCONNECTED)),
            remote_addr: Arc::new(RwLock::new(None)),
            connect_latch: Arc::new(Latch::default()),
            receiver_running: Arc::new(AtomicBool::new(false)),
            close_tx,
            send_tx: None,
            send_queue_bytes: Arc::new(AtomicUsize::new(0)),
            send_queue_count: Arc::new(AtomicUsize::new(0)),
            ready_latch: Arc::new(Latch::default()),
            loop_ref,
        };

        let cls = Class::instance(ctx.clone(), instance)?;
        if let Some(cb) = callback.0 {
            Self::add_event_listener_str(cls.clone(), &ctx, "message", cb, false, true)?;
        }
        if let Some(sig) = signal_val {
            wire_abort_signal(&ctx, sig, &cls)?;
        }
        let _ = lexe_utils::diagnostics::publish(&ctx, "udp.socket", |ctx| {
            let msg = Object::new(ctx.clone())?;
            msg.set("socket", cls.clone().into_value())?;
            Ok(msg)
        });
        Ok(cls)
    }

    #[qjs(skip)]
    fn start_listening(
        socket_class: Class<'js, Self>,
        ctx: Ctx<'js>,
        bind_host: String,
        bind_port: u16,
    ) -> Result<()> {
        let (is_closed, receiver_running, ready_latch, is_ipv6, bind_options, connect_latch, q_bytes, q_count) = {
            let borrow = socket_class.borrow();
            if borrow.is_bound.load(Ordering::SeqCst) {
                return Err(throw_coded(
                    &ctx,
                    "Error",
                    "ERR_SOCKET_ALREADY_BOUND",
                    "Socket is already bound",
                ));
            }
            if borrow.is_closed.load(Ordering::SeqCst) {
                return Err(throw_coded(
                    &ctx,
                    "Error",
                    "ERR_SOCKET_DGRAM_NOT_RUNNING",
                    "Not running",
                ));
            }
            (
                borrow.is_closed.clone(),
                borrow.receiver_running.clone(),
                borrow.ready_latch.clone(),
                borrow.is_ipv6,
                borrow.bind_options.clone(),
                borrow.connect_latch.clone(),
                borrow.send_queue_bytes.clone(),
                borrow.send_queue_count.clone(),
            )
        };

        ready_latch.increment();

        let socket_class2 = socket_class.clone();
        let socket_class3 = socket_class.clone();
        ctx.clone().spawn_exit(async move {
            let bind_result = async {
                let resolved_bind = match resolve_socket_addr(&bind_host, bind_port, is_ipv6).await {
                    Ok(a) => a,
                    Err(e) => {
                        ready_latch.decrement();
                        return Err(e).or_throw(&ctx);
                    },
                };
                let socket = match configure_and_bind_socket(resolved_bind, is_ipv6, &bind_options) {
                    Ok(s) => s,
                    Err(e) => {
                        ready_latch.decrement();
                        return Err(e).or_throw(&ctx);
                    },
                };
                let local_addr = match socket.local_addr() {
                    Ok(a) => a,
                    Err(e) => {
                        ready_latch.decrement();
                        return Err(e).or_throw(&ctx);
                    },
                };

                if is_closed.load(Ordering::SeqCst) {
                    ready_latch.decrement();
                    return Ok(());
                }

                let socket_arc = Arc::new(socket);
                let (send_tx, mut send_rx) = mpsc::unbounded_channel::<SendMessage>();

                let (recv_close_rx, send_close_rx) = {
                    let mut borrow = socket_class.borrow_mut();
                    borrow.socket = Some(socket_arc.clone());
                    borrow.local_address = Some(local_addr.ip().to_string());
                    borrow.local_port = Some(local_addr.port());
                    borrow.is_bound.store(true, Ordering::SeqCst);
                    borrow.send_tx = Some(send_tx);
                    (borrow.close_tx.subscribe(), borrow.close_tx.subscribe())
                };

                ready_latch.decrement();

                trace!("UDP socket bound to {}", local_addr);

                Self::emit_str(socket_class.clone(), &ctx, "listening", vec![], false)?;

                let recv_started = receiver_running
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok();

                if recv_started {
                    let recv_socket = socket_arc.clone();
                    let recv_class = socket_class.clone();
                    let recv_closed = is_closed.clone();
                    let recv_running = receiver_running.clone();
                    let recv_ctx = ctx.clone();
                    let mut close_rx = recv_close_rx;
                    let loop_ref = socket_class.borrow().loop_ref.clone();

                    ctx.clone().spawn_exit(async move {
                        let recv_result = async {
                            let mut buf = vec![0u8; 65536];

                            loop {
                                if !event_loop::keeps_event_loop_alive(&loop_ref) {
                                    break;
                                }

                                tokio::select! {
                                    _ = event_loop::wait_until_io_unref(loop_ref.clone()) => {
                                        break;
                                    }
                                    _ = close_rx.recv() => {
                                        break;
                                    }
                                    result = recv_socket.recv_from(&mut buf) => {
                                        match result {
                                            Ok((size, peer_addr)) => {
                                                let data = Buffer(buf[..size].to_vec()).into_js(&recv_ctx)?;
                                                let info = Object::new(recv_ctx.clone())?;
                                                info.set("address", peer_addr.ip().to_string())?;
                                                info.set(
                                                    "family",
                                                    if peer_addr.is_ipv4() { "IPv4" } else { "IPv6" },
                                                )?;
                                                info.set("port", peer_addr.port())?;
                                                info.set("size", size)?;

                                                let info_val: Value = info.into();
                                                Self::emit_str(
                                                    recv_class.clone(),
                                                    &recv_ctx,
                                                    "message",
                                                    vec![data, info_val],
                                                    false,
                                                )?;
                                            },
                                            Err(e) => {
                                                if recv_closed.load(Ordering::SeqCst) {
                                                    break;
                                                }
                                                return Err(Exception::throw_message(
                                                    &recv_ctx,
                                                    &format!("UDP receive error: {e}"),
                                                ));
                                            },
                                        }
                                    }
                                }
                            }

                            recv_running.store(false, Ordering::SeqCst);
                            Ok(())
                        }
                        .await;

                        recv_result.emit_error("recv", &recv_ctx, recv_class)?;
                        Ok(())
                    })?;
                }

                let send_socket = socket_arc.clone();
                let mut close_rx = send_close_rx;
                let send_ctx = ctx.clone();
                let send_loop_ref = socket_class.borrow().loop_ref.clone();
                ctx.spawn_exit(async move {
                    let send_result: Result<()> = async {
                        loop {
                            if !event_loop::keeps_event_loop_alive(&send_loop_ref) {
                                break;
                            }

                            tokio::select! {
                                _ = event_loop::wait_until_io_unref(send_loop_ref.clone()) => {
                                    break;
                                }
                                _ = close_rx.recv() => {
                                    break;
                                }
                                msg = send_rx.recv() => {
                                    let Some((bytes, dest_opt, result_tx)) = msg else {
                                        break;
                                    };
                                    q_count.fetch_sub(1, Ordering::SeqCst);
                                    q_bytes.fetch_sub(bytes.len(), Ordering::SeqCst);

                                    let io_res = match dest_opt {
                                        Some((dest_host, dest_port)) => {
                                            match resolve_socket_addr(&dest_host, dest_port, is_ipv6).await {
                                                Ok(target) => send_socket.send_to(&bytes, target).await,
                                                Err(e) => Err(e),
                                            }
                                        },
                                        None => {
                                            connect_latch.wait().await;
                                            send_socket.send(&bytes).await
                                        },
                                    };

                                    if let Some(result_tx) = result_tx {
                                        result_tx
                                            .send(io_res.map_err(|e| e.to_string()))
                                            .map_err(|_| Exception::throw_message(&send_ctx, "Failed to call callback in send, channel closed!"))?;
                                        continue;
                                    }
                                    io_res.or_throw(&send_ctx)?;
                                }
                            }
                        }
                        Ok(())
                    }.await;
                    send_result.emit_error("send", &send_ctx, socket_class3)?;
                    Ok(())
                })?;

                Ok(())
            }
            .await;

            bind_result.emit_error("bind", &ctx, socket_class2)?;
            Ok(())
        })?;

        Ok(())
    }

    #[qjs(skip)]
    fn ensure_listening(this: Class<'js, Self>, ctx: Ctx<'js>) -> Result<Arc<Latch>> {
        let (is_bound, ready_latch, is_ipv6) = {
            let borrow = this.borrow();
            (
                borrow.is_bound.load(Ordering::SeqCst),
                borrow.ready_latch.clone(),
                borrow.is_ipv6,
            )
        };

        if !is_bound {
            let default_host = if is_ipv6 { "::" } else { "0.0.0.0" }.to_string();
            Self::start_listening(this, ctx, default_host, 0)?;
        }

        Ok(ready_latch)
    }

    pub fn bind(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let is_ipv6 = this.borrow().is_ipv6;
        let mut port = 0u16;
        let mut address = if is_ipv6 { "::" } else { "0.0.0.0" }.to_string();
        let mut callback: Option<Function> = None;

        let mut args_iter = args.0.into_iter();

        if let Some(first_arg) = args_iter.next() {
            if let Some(func) = first_arg.as_function() {
                callback = Some(func.clone());
            } else if first_arg.is_undefined() || first_arg.is_null() {
                if let Some(second_arg) = args_iter.next() {
                    if let Some(func) = second_arg.as_function() {
                        callback = Some(func.clone());
                    }
                }
            } else if first_arg.is_number() || first_arg.is_int() {
                port = parse_bind_port(&ctx, &first_arg)?;
                if let Some(second_arg) = args_iter.next() {
                    if let Some(func) = second_arg.as_function() {
                        callback = Some(func.clone());
                    } else {
                        if let Some(addr_str) = second_arg.as_string() {
                            let s = addr_str.to_string()?;
                            if !s.is_empty() {
                                address = s;
                            }
                        }
                        if let Some(third_arg) = args_iter.next() {
                            if let Some(func) = third_arg.as_function() {
                                callback = Some(func.clone());
                            }
                        }
                    }
                }
            } else if let Some(obj) = first_arg.as_object() {
                if let Some(p_val) = obj.get_optional::<_, Value<'js>>("port")? {
                    if !p_val.is_undefined() && !p_val.is_null() {
                        port = parse_bind_port(&ctx, &p_val)?;
                    }
                }
                if let Some(addr) = obj.get_optional::<_, String>("address")? {
                    if !addr.is_empty() {
                        address = addr;
                    }
                }
                if let Some(second_arg) = args_iter.next() {
                    if let Some(func) = second_arg.as_function() {
                        callback = Some(func.clone());
                    }
                }
            }
        }

        if let Some(cb) = callback {
            Self::add_event_listener_str(this.clone(), &ctx, "listening", cb, true, true)?;
        }

        Self::start_listening(this.0.clone(), ctx, address, port)?;

        Ok(this.0)
    }

    pub fn connect(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        port_val: Value<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<()> {
        let port = validate_port(&ctx, &port_val)?;
        let (is_ipv6, connect_state, remote_addr, connect_latch, is_closed) = {
            let borrow = this.borrow();
            if borrow.is_closed.load(Ordering::SeqCst) {
                return Err(throw_coded(
                    &ctx,
                    "Error",
                    "ERR_SOCKET_DGRAM_NOT_RUNNING",
                    "Not running",
                ));
            }
            if borrow.connect_state.load(Ordering::SeqCst) != CONNECT_DISCONNECTED {
                return Err(throw_coded(
                    &ctx,
                    "Error",
                    "ERR_SOCKET_DGRAM_IS_CONNECTED",
                    "Already connected",
                ));
            }
            (
                borrow.is_ipv6,
                borrow.connect_state.clone(),
                borrow.remote_addr.clone(),
                borrow.connect_latch.clone(),
                borrow.is_closed.clone(),
            )
        };

        let mut address = if is_ipv6 { "::1" } else { "127.0.0.1" }.to_string();
        let mut callback: Option<Function<'js>> = None;
        let mut iter = args.0.into_iter();
        if let Some(arg1) = iter.next() {
            if let Some(f) = arg1.as_function() {
                callback = Some(f.clone());
            } else {
                if let Some(s) = arg1.as_string() {
                    let addr_s = s.to_string()?;
                    if !addr_s.is_empty() {
                        address = addr_s;
                    }
                }
                if let Some(arg2) = iter.next() {
                    if let Some(f) = arg2.as_function() {
                        callback = Some(f.clone());
                    }
                }
            }
        }

        connect_state.store(CONNECT_CONNECTING, Ordering::SeqCst);
        connect_latch.increment();

        let ready_latch = match Self::ensure_listening(this.0.clone(), ctx.clone()) {
            Ok(l) => l,
            Err(e) => {
                connect_state.store(CONNECT_DISCONNECTED, Ordering::SeqCst);
                connect_latch.decrement();
                return Err(e);
            },
        };

        let socket_class = this.0.clone();
        ctx.clone().spawn_exit(async move {
            ready_latch.wait().await;
            let res: std::result::Result<SocketAddr, String> = async {
                let sock_opt = socket_class.borrow().socket.clone();
                let Some(sock) = sock_opt else {
                    return Err("Not running".to_string());
                };
                let peer = resolve_socket_addr(&address, port, is_ipv6)
                    .await
                    .map_err(|e| e.to_string())?;
                sock.connect(peer).await.map_err(|e| e.to_string())?;
                Ok(peer)
            }
            .await;

            match res {
                Ok(peer) => {
                    if !is_closed.load(Ordering::SeqCst) {
                        *remote_addr.write().unwrap() = Some(peer);
                        connect_state.store(CONNECT_CONNECTED, Ordering::SeqCst);
                    }
                    connect_latch.decrement();
                    if !is_closed.load(Ordering::SeqCst) {
                        if let Some(cb) = callback {
                            Self::add_event_listener_str(
                                socket_class.clone(),
                                &ctx,
                                "connect",
                                cb,
                                true,
                                true,
                            )?;
                        }
                        Self::emit_str(socket_class, &ctx, "connect", vec![], false)?;
                    }
                },
                Err(err_msg) => {
                    connect_state.store(CONNECT_DISCONNECTED, Ordering::SeqCst);
                    *remote_addr.write().unwrap() = None;
                    connect_latch.decrement();
                    if let Some(cb) = callback {
                        let err = Exception::from_message(ctx.clone(), &err_msg)?;
                        cb.call::<_, ()>((err,))?;
                    } else {
                        let err = make_coded_exception(&ctx, "Error", "EINVAL", &err_msg)?;
                        Self::emit_str(socket_class, &ctx, "error", vec![err], false)?;
                    }
                },
            }
            Ok(())
        })?;

        Ok(())
    }

    pub fn disconnect(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        let borrow = this.borrow();
        if borrow.connect_state.load(Ordering::SeqCst) != CONNECT_CONNECTED {
            return Err(throw_coded(
                &ctx,
                "Error",
                "ERR_SOCKET_DGRAM_NOT_CONNECTED",
                "Not connected",
            ));
        }
        let Some(sock) = borrow.socket.as_deref() else {
            return Err(throw_coded(
                &ctx,
                "Error",
                "ERR_SOCKET_DGRAM_NOT_RUNNING",
                "Not running",
            ));
        };
        disconnect_udp_socket(sock).or_throw(&ctx)?;
        *borrow.remote_addr.write().unwrap() = None;
        borrow
            .connect_state
            .store(CONNECT_DISCONNECTED, Ordering::SeqCst);
        Ok(())
    }

    pub fn remote_address(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let borrow = this.borrow();
        let guard = borrow.remote_addr.read().unwrap();
        let Some(addr) = *guard else {
            return Err(throw_coded(
                &ctx,
                "Error",
                "ERR_SOCKET_DGRAM_NOT_CONNECTED",
                "Not connected",
            ));
        };
        let obj = Object::new(ctx)?;
        obj.set("address", addr.ip().to_string())?;
        obj.set("family", if addr.is_ipv4() { "IPv4" } else { "IPv6" })?;
        obj.set("port", addr.port())?;
        Ok(obj)
    }

    pub fn send(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        msg: Value<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<()> {
        let (is_ipv6, is_connected, is_closed, q_bytes, q_count) = {
            let borrow = this.borrow();
            (
                borrow.is_ipv6,
                borrow.connect_state.load(Ordering::SeqCst) != CONNECT_DISCONNECTED,
                borrow.is_closed.load(Ordering::SeqCst),
                borrow.send_queue_bytes.clone(),
                borrow.send_queue_count.clone(),
            )
        };

        let mut it = args.0.into_iter();
        let offset_val = it.next().unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let length_val = it.next().unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let mut port_val = it.next().unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let mut address_val = it.next().unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let mut cb_val = it.next().unwrap_or_else(|| Value::new_undefined(ctx.clone()));

        let slice = if !is_connected {
            let use_slice = (!address_val.is_undefined() && !address_val.is_null())
                || (!port_val.is_undefined() && !port_val.is_null() && !port_val.is_function());
            if use_slice {
                Some((offset_val, length_val))
            } else {
                cb_val = port_val;
                port_val = offset_val;
                address_val = length_val;
                None
            }
        } else {
            let slice = if length_val.is_number() || length_val.is_int() {
                if port_val.is_function() {
                    cb_val = port_val;
                    port_val = Value::new_undefined(ctx.clone());
                }
                Some((offset_val, length_val))
            } else {
                cb_val = offset_val;
                None
            };

            if (!port_val.is_undefined() && !port_val.is_null())
                || (!address_val.is_undefined() && !address_val.is_null())
            {
                return Err(throw_coded(
                    &ctx,
                    "Error",
                    "ERR_SOCKET_DGRAM_IS_CONNECTED",
                    "Already connected",
                ));
            }
            slice
        };

        let bytes = extract_send_bytes(&ctx, msg, slice)?;

        if is_closed {
            return Err(Exception::throw_message(&ctx, "Socket is closed"));
        }

        let mut cb: Option<Function<'js>> = cb_val.as_function().cloned();
        let dest = if !is_connected {
            let port = validate_port(&ctx, &port_val)?;
            let address = if let Some(func) = address_val.as_function() {
                cb = Some(func.clone());
                if is_ipv6 { "::1" } else { "127.0.0.1" }.to_string()
            } else if address_val.is_undefined() || address_val.is_null() {
                if is_ipv6 { "::1" } else { "127.0.0.1" }.to_string()
            } else if let Some(s) = address_val.as_string() {
                let s = s.to_string()?;
                if s.is_empty() {
                    if is_ipv6 { "::1" } else { "127.0.0.1" }.to_string()
                } else {
                    s
                }
            } else {
                return Err(throw_coded(
                    &ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_TYPE",
                    "The \"address\" argument must be of type string.",
                ));
            };
            Some((address, port))
        } else {
            None
        };

        let byte_len = bytes.len();
        q_count.fetch_add(1, Ordering::SeqCst);
        q_bytes.fetch_add(byte_len, Ordering::SeqCst);

        let ready_latch = match Self::ensure_listening(this.0.clone(), ctx.clone()) {
            Ok(l) => l,
            Err(e) => {
                q_count.fetch_sub(1, Ordering::SeqCst);
                q_bytes.fetch_sub(byte_len, Ordering::SeqCst);
                return Err(e);
            },
        };

        let socket_class = this.0.clone();
        let socket_class2 = this.0.clone();

        ctx.clone().spawn_exit(async move {
            let send_result: Result<()> = async {
                ready_latch.wait().await;

                let result_rx = {
                    let (result_tx, result_rx) = if cb.is_some() {
                        let (result_tx, result_rx) = oneshot::channel();
                        (Some(result_tx), Some(result_rx))
                    } else {
                        (None, None)
                    };

                    let send_res = {
                        let borrow = socket_class.borrow();
                        borrow
                            .send_tx
                            .as_ref()
                            .ok_or_else(|| Exception::throw_message(&ctx, "Socket is closed"))
                            .and_then(|tx| {
                                tx.send((bytes, dest, result_tx)).map_err(|_| {
                                    Exception::throw_message(&ctx, "Failed to send message")
                                })
                            })
                    };
                    if let Err(err) = send_res {
                        q_count.fetch_sub(1, Ordering::SeqCst);
                        q_bytes.fetch_sub(byte_len, Ordering::SeqCst);
                        return Err(err);
                    }
                    result_rx
                };

                let Some(result_rx) = result_rx else {
                    return Ok(());
                };

                let result = result_rx.await.unwrap_or(Err("Socket closed".to_string()));
                let Some(cb) = cb else {
                    result.or_throw(&ctx)?;
                    return Ok(());
                };

                match result {
                    Ok(sent) => {
                        cb.call::<_, ()>((Value::new_null(ctx.clone()), sent))?;
                    },
                    Err(e) => {
                        let err = Exception::from_message(ctx.clone(), &e)?;
                        cb.call::<_, ()>((err,))?;
                    },
                }

                Ok(())
            }
            .await;

            send_result.emit_error("send", &ctx, socket_class2)?;
            Ok(())
        })?;

        Ok(())
    }

    pub fn close(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        let already_closed = {
            let borrow = this.borrow();
            let was_closed = borrow
                .is_closed
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err();
            if !was_closed {
                let _ = borrow.close_tx.send(());
            }
            was_closed
        };

        if already_closed {
            return Err(throw_coded(
                &ctx,
                "Error",
                "ERR_SOCKET_DGRAM_NOT_RUNNING",
                "Not running",
            ));
        }

        if let Some(cb) = callback.0 {
            Self::add_event_listener_str(this.clone(), &ctx, "close", cb, true, true)?;
        }

        {
            let mut borrow = this.borrow_mut();
            borrow.socket = None;
            borrow.send_tx = None;
            *borrow.remote_addr.write().unwrap() = None;
            borrow
                .connect_state
                .store(CONNECT_DISCONNECTED, Ordering::SeqCst);
            borrow.send_queue_bytes.store(0, Ordering::SeqCst);
            borrow.send_queue_count.store(0, Ordering::SeqCst);
        }

        let this_clone = this.0.clone();
        ctx.clone().spawn_exit(async move {
            this_clone.borrow().loop_ref.unreference();
            Self::emit_str(this_clone, &ctx, "close", vec![], false)?;
            Ok(())
        })?;

        Ok(this.0)
    }

    pub fn address(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let borrow = this.borrow();

        let obj = Object::new(ctx)?;

        if let Some(addr) = &borrow.local_address {
            obj.set("address", addr.clone())?;
        }
        if let Some(port) = borrow.local_port {
            obj.set("port", port)?;
        }
        if let Some(family) = &borrow.local_family {
            obj.set("family", family.clone())?;
        }

        Ok(obj)
    }

    pub fn set_broadcast(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        flag: Value<'js>,
    ) -> Result<()> {
        let flag = !matches!(flag.as_bool(), Some(false))
            && !flag.is_undefined()
            && !flag.is_null()
            && flag.as_int() != Some(0);
        let borrow = this.borrow();
        sock_set_broadcast(borrow.socket.as_deref(), &ctx, flag)
    }

    #[qjs(rename = "setTTL")]
    pub fn set_ttl(this: This<Class<'js, Self>>, ctx: Ctx<'js>, ttl: i32) -> Result<i32> {
        let borrow = this.borrow();
        sock_set_ttl(borrow.socket.as_deref(), &ctx, borrow.is_ipv6, ttl)
    }

    #[qjs(rename = "setMulticastTTL")]
    pub fn set_multicast_ttl(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        ttl: i32,
    ) -> Result<i32> {
        let borrow = this.borrow();
        sock_set_multicast_ttl(borrow.socket.as_deref(), &ctx, borrow.is_ipv6, ttl)
    }

    pub fn set_multicast_loopback(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        flag: bool,
    ) -> Result<bool> {
        let borrow = this.borrow();
        sock_set_multicast_loopback(borrow.socket.as_deref(), &ctx, borrow.is_ipv6, flag)
    }

    pub fn set_multicast_interface(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        multicast_interface: String,
    ) -> Result<()> {
        let borrow = this.borrow();
        sock_set_multicast_interface(
            borrow.socket.as_deref(),
            &ctx,
            borrow.is_ipv6,
            &multicast_interface,
        )
    }

    pub fn add_membership(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        multicast_address: String,
        multicast_interface: Opt<String>,
    ) -> Result<()> {
        let borrow = this.borrow();
        sock_membership(
            borrow.socket.as_deref(),
            &ctx,
            borrow.is_ipv6,
            true,
            &multicast_address,
            multicast_interface.0.as_deref(),
        )
    }

    pub fn drop_membership(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        multicast_address: String,
        multicast_interface: Opt<String>,
    ) -> Result<()> {
        let borrow = this.borrow();
        sock_membership(
            borrow.socket.as_deref(),
            &ctx,
            borrow.is_ipv6,
            false,
            &multicast_address,
            multicast_interface.0.as_deref(),
        )
    }

    pub fn add_source_specific_membership(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        source_address: String,
        group_address: String,
        multicast_interface: Opt<String>,
    ) -> Result<()> {
        let borrow = this.borrow();
        sock_source_membership(
            borrow.socket.as_deref(),
            &ctx,
            borrow.is_ipv6,
            true,
            &source_address,
            &group_address,
            multicast_interface.0.as_deref(),
        )
    }

    pub fn drop_source_specific_membership(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        source_address: String,
        group_address: String,
        multicast_interface: Opt<String>,
    ) -> Result<()> {
        let borrow = this.borrow();
        sock_source_membership(
            borrow.socket.as_deref(),
            &ctx,
            borrow.is_ipv6,
            false,
            &source_address,
            &group_address,
            multicast_interface.0.as_deref(),
        )
    }

    pub fn get_recv_buffer_size(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<usize> {
        sock_get_recv_buffer_size(this.borrow().socket.as_deref(), &ctx)
    }

    pub fn set_recv_buffer_size(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        size: i64,
    ) -> Result<()> {
        sock_set_recv_buffer_size(this.borrow().socket.as_deref(), &ctx, size)
    }

    pub fn get_send_buffer_size(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<usize> {
        sock_get_send_buffer_size(this.borrow().socket.as_deref(), &ctx)
    }

    pub fn set_send_buffer_size(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        size: i64,
    ) -> Result<()> {
        sock_set_send_buffer_size(this.borrow().socket.as_deref(), &ctx, size)
    }

    pub fn get_send_queue_size(this: This<Class<'js, Self>>) -> usize {
        this.borrow().send_queue_bytes.load(Ordering::SeqCst)
    }

    pub fn get_send_queue_count(this: This<Class<'js, Self>>) -> usize {
        this.borrow().send_queue_count.load(Ordering::SeqCst)
    }

    pub fn unref(this: This<Class<'js, Self>>) -> Result<Class<'js, Self>> {
        this.borrow().loop_ref.unreference();
        Ok(this.0)
    }

    #[qjs(rename = "ref")]
    pub fn r#ref(this: This<Class<'js, Self>>) -> Result<Class<'js, Self>> {
        {
            let borrow = this.borrow();
            if !borrow.is_closed.load(Ordering::SeqCst) {
                borrow.loop_ref.reference();
            }
        }
        Ok(this.0)
    }
}
