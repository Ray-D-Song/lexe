// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    future::poll_fn,
    io,
    pin::Pin,
    sync::{atomic::Ordering, Arc},
    task::{Context, Poll},
    time::Instant,
};

use bytes::Bytes;
use h2::client::{Builder as H2Builder, SendRequest};
use http::{
    header::{HeaderName, HeaderValue},
    Request, Uri,
};
use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_events::{Emitter, EventEmitter, EventList};
use lexe_net::Socket as NetSocket;
use lexe_tls::{
    alpn::convert_alpn_protocols,
    backend::{connect as tls_connect, ConnectOptions},
    options::parse_connect_options,
    secure_context::SecureContext,
    tls_socket::TlsSocket,
};
use lexe_utils::{
    event_loop::EventLoopRef,
    perf::{self, PerfEntryKind},
    time,
};
use rquickjs::{
    class::Trace,
    prelude::{Opt, Rest, This},
    Array, Class, Ctx, Exception, Function, IntoJs, Object, Result, Undefined, Value,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    sync::{mpsc, oneshot, watch},
};

use crate::stream::{ClientStreamMsg, Http2Stream};

enum SessionCmd {
    Ping(oneshot::Sender<std::result::Result<f64, String>>),
    Close,
    Destroy,
}

enum HandoffTarget<'js> {
    Tls(Class<'js, TlsSocket<'js>>),
    Net(Class<'js, NetSocket<'js>>),
}

struct PrefixedSplitIo<R, W> {
    prefix: Vec<u8>,
    prefix_pos: usize,
    reader: R,
    writer: W,
}

impl<R, W> PrefixedSplitIo<R, W> {
    fn new(prefix: Vec<u8>, reader: R, writer: W) -> Self {
        Self {
            prefix,
            prefix_pos: 0,
            reader,
            writer,
        }
    }
}

impl<R: AsyncRead + Unpin, W: Unpin> AsyncRead for PrefixedSplitIo<R, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.prefix_pos < self.prefix.len() {
            let remaining = &self.prefix[self.prefix_pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            self.prefix_pos += to_copy;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl<R: Unpin, W: AsyncWrite + Unpin> AsyncWrite for PrefixedSplitIo<R, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

#[rquickjs::class]
pub struct ClientHttp2Session<'js> {
    emitter: EventEmitter<'js>,
    handshake_rx: watch::Receiver<Option<std::result::Result<SendRequest<Bytes>, String>>>,
    cmd_tx: Option<mpsc::UnboundedSender<SessionCmd>>,
    closed: bool,
    destroyed: bool,
    encrypted: bool,
    loop_ref: Arc<EventLoopRef>,
}

impl<'js> Trace<'js> for ClientHttp2Session<'js> {
    fn trace<'a>(&self, t: rquickjs::class::Tracer<'a, 'js>) {
        self.emitter.trace(t);
    }
}

impl<'js> Emitter<'js> for ClientHttp2Session<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

impl<'js> ClientHttp2Session<'js> {
    pub fn define_module(ctx: &Ctx<'js>, default: &Object<'js>) -> Result<()> {
        Class::<Self>::define(default)?;
        Self::add_event_emitter_prototype(ctx)?;
        Ok(())
    }

    fn new_pending(
        ctx: Ctx<'js>,
        encrypted: bool,
        handshake_rx: watch::Receiver<Option<std::result::Result<SendRequest<Bytes>, String>>>,
        cmd_tx: mpsc::UnboundedSender<SessionCmd>,
    ) -> Result<Class<'js, Self>> {
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        Class::instance(
            ctx,
            Self {
                emitter: EventEmitter::new(),
                handshake_rx,
                cmd_tx: Some(cmd_tx),
                closed: false,
                destroyed: false,
                encrypted,
                loop_ref: EventLoopRef::new_referenced(rt_key),
            },
        )
    }
}

unsafe impl<'js> rquickjs::JsLifetime<'js> for ClientHttp2Session<'js> {
    type Changed<'to> = ClientHttp2Session<'to>;
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> ClientHttp2Session<'js> {
    #[qjs(get, enumerable)]
    pub fn closed(&self) -> bool {
        self.closed
    }

    #[qjs(get, enumerable)]
    pub fn destroyed(&self) -> bool {
        self.destroyed
    }

    #[qjs(get, enumerable)]
    pub fn encrypted(&self) -> bool {
        self.encrypted
    }

    #[qjs(get, enumerable)]
    pub fn alpn_protocol(&self) -> &'static str {
        "h2"
    }

    #[qjs(rename = "ref")]
    pub fn js_ref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.reference();
        this.0
    }

    #[qjs(rename = "unref")]
    pub fn js_unref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.unreference();
        this.0
    }

    pub fn set_timeout(
        this: This<Class<'js, Self>>,
        _msecs: Opt<Value<'js>>,
        _cb: Opt<Function<'js>>,
    ) -> Class<'js, Self> {
        this.0
    }

    pub fn set_local_window_size(
        this: This<Class<'js, Self>>,
        _size: Opt<i32>,
    ) -> Class<'js, Self> {
        this.0
    }

    pub fn ping(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<bool> {
        let mut payload_bytes: Option<Vec<u8>> = None;
        let mut callback: Option<Function<'js>> = None;
        for arg in args.0 {
            if arg.is_function() {
                callback = arg.into_function();
            } else if payload_bytes.is_none() && !arg.is_undefined() && !arg.is_null() {
                payload_bytes =
                    Some(lexe_utils::bytes::ObjectBytes::from(&ctx, &arg)?.into_bytes(&ctx)?);
            }
        }
        let cmd_tx = {
            let borrow = this.borrow();
            if borrow.closed || borrow.destroyed {
                None
            } else {
                borrow.cmd_tx.clone()
            }
        };
        let Some(cmd_tx) = cmd_tx else {
            return Ok(false);
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        if cmd_tx.send(SessionCmd::Ping(reply_tx)).is_err() {
            return Ok(false);
        }
        let payload = payload_bytes.unwrap_or_else(|| vec![0u8; 8]);
        let ctx2 = ctx.clone();
        ctx.spawn_exit_simple(async move {
            let Some(cb) = callback else {
                let _ = reply_rx.await;
                return Ok(());
            };
            match reply_rx.await {
                Ok(Ok(duration_ms)) => {
                    let buf = Buffer(payload).into_js(&ctx2)?;
                    let null = rquickjs::Null.into_js(&ctx2)?;
                    let _ = cb.call::<_, ()>((null, duration_ms, buf));
                },
                Ok(Err(msg)) => {
                    let err = Exception::from_message(ctx2.clone(), &msg)?;
                    let _ = cb.call::<_, ()>((err,));
                },
                Err(_) => {
                    let err = Exception::from_message(ctx2.clone(), "session closed")?;
                    let _ = cb.call::<_, ()>((err,));
                },
            }
            Ok(())
        });
        Ok(true)
    }

    pub fn request(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        headers: Object<'js>,
        options: Opt<Object<'js>>,
    ) -> Result<Class<'js, Http2Stream<'js>>> {
        let (mut handshake_rx, is_encrypted, session_closed) = {
            let borrow = this.borrow();
            (
                borrow.handshake_rx.clone(),
                borrow.encrypted,
                borrow.closed || borrow.destroyed,
            )
        };
        if session_closed {
            let err = Exception::from_message(ctx.clone(), "The session has been destroyed")?;
            let err_val = err.into_value();
            if let Some(obj) = err_val.as_object() {
                let _ = obj.set("code", "ERR_HTTP2_INVALID_SESSION");
            }
            return Err(ctx.throw(err_val));
        }

        let end_stream = options
            .0
            .and_then(|o| o.get::<_, Option<bool>>("endStream").ok().flatten())
            .unwrap_or(false);

        let mut method = "GET".to_string();
        let mut path = "/".to_string();
        let mut scheme = if is_encrypted { "https" } else { "http" }.to_string();
        let mut authority = "localhost".to_string();
        let mut extra_headers: Vec<(String, String)> = Vec::new();
        for entry in headers.props::<String, Value>() {
            let (key, val) = entry?;
            let lower = key.to_ascii_lowercase();
            match lower.as_str() {
                ":method" => method = value_to_string(&ctx, val)?,
                ":path" => path = value_to_string(&ctx, val)?,
                ":scheme" => scheme = value_to_string(&ctx, val)?,
                ":authority" => authority = value_to_string(&ctx, val)?,
                "connection" | "keep-alive" | "transfer-encoding" | "upgrade"
                | "proxy-connection" => {},
                _ => {
                    if let Some(arr) = val.as_array() {
                        for item in arr.iter::<Value>() {
                            extra_headers.push((lower.clone(), value_to_string(&ctx, item?)?));
                        }
                    } else if !val.is_undefined() && !val.is_null() {
                        extra_headers.push((lower, value_to_string(&ctx, val)?));
                    }
                },
            }
        }

        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<ClientStreamMsg>();
        let (req_stream, paused, resume_notify) = Http2Stream::new_client_request(
            &ctx,
            method.clone(),
            path.clone(),
            scheme.clone(),
            authority.clone(),
            write_tx,
        )?;

        let ctx2 = ctx.clone();
        let stream = req_stream.clone();
        ctx.spawn_exit_simple(async move {
            let perf_start_ms =
                perf::has_observers(perf::rt_key_for_ctx(&ctx2), PerfEntryKind::Http2)
                    .then(time::monotonic_elapsed_ms);

            let sender = loop {
                if let Some(res) = handshake_rx.borrow().clone() {
                    match res {
                        Ok(s) => break s,
                        Err(msg) => {
                            emit_stream_error_and_close(&ctx2, &stream, &msg);
                            return Ok(());
                        },
                    }
                }
                if handshake_rx.changed().await.is_err() {
                    emit_stream_error_and_close(&ctx2, &stream, "HTTP/2 session closed");
                    return Ok(());
                }
            };

            let mut sender = match sender.ready().await {
                Ok(s) => s,
                Err(e) => {
                    emit_stream_error_and_close(&ctx2, &stream, &e.to_string());
                    return Ok(());
                },
            };

            let uri: Uri = format!("{scheme}://{authority}{path}")
                .parse()
                .unwrap_or_else(|_| Uri::from_static("/"));
            let perf_method = method.clone();
            let perf_url = uri.to_string();
            let mut builder = Request::builder().uri(uri).method(method.as_str());
            for (key, val) in extra_headers {
                if let Ok(name) = HeaderName::from_bytes(key.as_bytes()) {
                    if let Ok(value) = HeaderValue::from_str(&val) {
                        builder = builder.header(name, value);
                    }
                }
            }
            let req = match builder.body(()) {
                Ok(r) => r,
                Err(e) => {
                    emit_stream_error_and_close(&ctx2, &stream, &e.to_string());
                    return Ok(());
                },
            };

            let (resp_future, mut send_stream) = match sender.send_request(req, end_stream) {
                Ok(pair) => pair,
                Err(e) => {
                    emit_stream_error_and_close(&ctx2, &stream, &e.to_string());
                    return Ok(());
                },
            };

            if !end_stream {
                tokio::spawn(async move {
                    while let Some(msg) = write_rx.recv().await {
                        match msg {
                            ClientStreamMsg::Data(bytes) => {
                                if !send_h2_bytes(&mut send_stream, bytes, false).await {
                                    return;
                                }
                            },
                            ClientStreamMsg::End(maybe_bytes) => {
                                let payload = maybe_bytes.unwrap_or_default();
                                let _ = send_h2_bytes(&mut send_stream, payload, true).await;
                                return;
                            },
                            ClientStreamMsg::Reset(code) => {
                                send_stream.send_reset(h2::Reason::from(code));
                                return;
                            },
                        }
                    }
                    let _ = send_stream.send_data(Bytes::new(), true);
                });
            }

            let resp = match resp_future.await {
                Ok(r) => r,
                Err(e) => {
                    emit_stream_error_and_close(&ctx2, &stream, &e.to_string());
                    return Ok(());
                },
            };

            let (parts, mut body) = resp.into_parts();
            let status_u16 = parts.status.as_u16();
            {
                let mut borrow = stream.borrow_mut();
                borrow.status_code = status_u16;
            }
            let header_obj = headers_to_js_object(&ctx2, status_u16, &parts.headers)?;
            {
                let mut borrow = stream.borrow_mut();
                borrow.headers = header_obj.clone();
            }
            let _ = Http2Stream::emit_str(
                stream.clone(),
                &ctx2,
                "response",
                vec![header_obj.into_value(), 0i32.into_js(&ctx2)?],
                false,
            );

            let mut bytes_read = 0u64;
            while let Some(chunk_res) = body.data().await {
                while paused.load(Ordering::Acquire) {
                    if stream.borrow().destroyed || stream.borrow().closed {
                        break;
                    }
                    resume_notify.notified().await;
                }
                if stream.borrow().destroyed || stream.borrow().closed {
                    break;
                }
                match chunk_res {
                    Ok(chunk) => {
                        bytes_read += chunk.len() as u64;
                        let _ = body.flow_control().release_capacity(chunk.len());
                        if !chunk.is_empty() {
                            let buf = Buffer(chunk.to_vec())
                                .into_js(&ctx2)
                                .unwrap_or_else(|_| Undefined.into_value(ctx2.clone()));
                            let _ = Http2Stream::emit_str(
                                stream.clone(),
                                &ctx2,
                                "data",
                                vec![buf],
                                false,
                            );
                        }
                    },
                    Err(e) => {
                        emit_stream_error_and_close(&ctx2, &stream, &e.to_string());
                        return Ok(());
                    },
                }
            }

            if !stream.borrow().destroyed && !stream.borrow().closed {
                if let Ok(Some(trailers)) = body.trailers().await {
                    if let Ok(trailer_obj) = headers_to_js_object_no_status(&ctx2, &trailers) {
                        let _ = Http2Stream::emit_str(
                            stream.clone(),
                            &ctx2,
                            "trailers",
                            vec![trailer_obj.into_value()],
                            false,
                        );
                    }
                }
                let _ = Http2Stream::emit_str(stream.clone(), &ctx2, "end", vec![], false);
                let already_closed = {
                    let mut borrow = stream.borrow_mut();
                    let prev = borrow.closed;
                    borrow.closed = true;
                    prev
                };
                if !already_closed {
                    let _ = Http2Stream::emit_str(stream.clone(), &ctx2, "close", vec![], false);
                }
            }

            if let Some(start_ms) = perf_start_ms {
                let dur = (time::monotonic_elapsed_ms() - start_ms).max(0.0);
                perf::emit_node_entry(
                    &ctx2,
                    PerfEntryKind::Http2,
                    "Http2Stream",
                    start_ms,
                    dur,
                    |ctx| {
                        let detail = Object::new(ctx.clone())?;
                        detail.set("bytesRead", bytes_read as f64)?;
                        detail.set("bytesWritten", 0)?;
                        let req_obj = Object::new(ctx.clone())?;
                        req_obj.set("method", perf_method.as_str())?;
                        req_obj.set("url", perf_url.as_str())?;
                        let res_obj = Object::new(ctx.clone())?;
                        res_obj.set("statusCode", status_u16)?;
                        detail.set("req", req_obj)?;
                        detail.set("res", res_obj)?;
                        Ok(detail.into_value())
                    },
                );
            }
            Ok(())
        });

        Ok(req_stream)
    }

    pub fn close(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        cb: Opt<Function<'js>>,
    ) -> Result<()> {
        let already_closed = {
            let mut borrow = this.borrow_mut();
            let prev = borrow.closed;
            borrow.closed = true;
            if let Some(tx) = borrow.cmd_tx.take() {
                let _ = tx.send(SessionCmd::Close);
            }
            borrow.loop_ref.unreference();
            prev
        };
        if let Some(cb) = cb.0 {
            cb.defer(())?;
        }
        if !already_closed {
            Self::emit_str(this.0, &ctx, "close", vec![], false)?;
        }
        Ok(())
    }

    pub fn destroy(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        _err: Opt<Value<'js>>,
        _code: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let already_closed = {
            let mut borrow = this.borrow_mut();
            let prev = borrow.closed;
            borrow.destroyed = true;
            borrow.closed = true;
            if let Some(tx) = borrow.cmd_tx.take() {
                let _ = tx.send(SessionCmd::Destroy);
            }
            borrow.loop_ref.unreference();
            prev
        };
        if !already_closed {
            Self::emit_str(this.0.clone(), &ctx, "close", vec![], false)?;
        }
        Ok(this.0)
    }
}

async fn send_h2_bytes(
    send_stream: &mut h2::SendStream<Bytes>,
    bytes: Bytes,
    end_stream: bool,
) -> bool {
    if bytes.is_empty() {
        return send_stream.send_data(bytes, end_stream).is_ok();
    }
    let mut offset = 0;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        send_stream.reserve_capacity(remaining);
        match poll_fn(|cx| send_stream.poll_capacity(cx)).await {
            Some(Ok(cap)) if cap > 0 => {
                let take = cap.min(remaining);
                let chunk = bytes.slice(offset..offset + take);
                offset += take;
                let is_end = end_stream && offset == bytes.len();
                if send_stream.send_data(chunk, is_end).is_err() {
                    return false;
                }
            },
            _ => return false,
        }
    }
    true
}

fn emit_stream_error_and_close<'js>(
    ctx: &Ctx<'js>,
    stream: &Class<'js, Http2Stream<'js>>,
    message: &str,
) {
    let already_closed = stream.borrow().closed || stream.borrow().destroyed;
    if already_closed {
        return;
    }
    if stream.borrow().has_listener_str("error") {
        if let Ok(err) = Exception::from_message(ctx.clone(), message) {
            let _ =
                Http2Stream::emit_str(stream.clone(), ctx, "error", vec![err.into_value()], false);
        }
    }
    let already = {
        let mut borrow = stream.borrow_mut();
        let prev = borrow.closed;
        borrow.closed = true;
        prev
    };
    if !already {
        let _ = Http2Stream::emit_str(stream.clone(), ctx, "close", vec![], false);
    }
}

fn headers_to_js_object<'js>(
    ctx: &Ctx<'js>,
    status: u16,
    map: &http::HeaderMap,
) -> Result<Object<'js>> {
    let obj = headers_to_js_object_no_status(ctx, map)?;
    obj.set(":status", status)?;
    Ok(obj)
}

fn headers_to_js_object_no_status<'js>(
    ctx: &Ctx<'js>,
    map: &http::HeaderMap,
) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    let mut grouped: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, value) in map.iter() {
        grouped
            .entry(name.as_str())
            .or_default()
            .push(value.to_str().unwrap_or(""));
    }
    for (key, vals) in grouped {
        if vals.len() == 1 {
            obj.set(key, vals[0])?;
        } else {
            let arr = Array::new(ctx.clone())?;
            for (i, val) in vals.iter().enumerate() {
                arr.set(i, *val)?;
            }
            obj.set(key, arr)?;
        }
    }
    Ok(obj)
}

fn parse_authority<'js>(ctx: &Ctx<'js>, authority: Value<'js>) -> Result<(String, u16, bool)> {
    let s: String = if let Some(str) = authority.as_string() {
        str.to_string()?
    } else if let Some(obj) = authority.as_object() {
        if let Ok(Some(href)) = obj.get::<_, Option<String>>("href") {
            href
        } else {
            return Err(Exception::throw_type(
                ctx,
                "authority must be a string or URL",
            ));
        }
    } else {
        return Err(Exception::throw_type(
            ctx,
            "authority must be a string or URL",
        ));
    };
    let trimmed = s.trim();
    let (secure, rest) = if let Some(rest) = trimmed.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        (false, rest)
    } else {
        (false, trimmed)
    };
    let authority_part = rest.split('/').next().unwrap_or(rest);
    let authority_part = authority_part.split('?').next().unwrap_or(authority_part);
    let (host, port) = if let Some((h, p)) = authority_part.rsplit_once(':') {
        if authority_part.starts_with('[') && !authority_part.contains("]:") {
            (authority_part.to_string(), if secure { 443 } else { 80 })
        } else {
            (
                h.to_string(),
                p.parse::<u16>()
                    .map_err(|_| Exception::throw_type(ctx, "invalid port"))?,
            )
        }
    } else {
        (authority_part.to_string(), if secure { 443 } else { 80 })
    };
    Ok((host, port, secure))
}

async fn run_h2_session<'js, IO>(
    io: IO,
    session: Class<'js, ClientHttp2Session<'js>>,
    ctx: &Ctx<'js>,
    handshake_tx: watch::Sender<Option<std::result::Result<SendRequest<Bytes>, String>>>,
    mut cmd_rx: mpsc::UnboundedReceiver<SessionCmd>,
    socket_for_connect_event: Option<Value<'js>>,
    max_concurrent_cap: u32,
) where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut builder = H2Builder::new();
    builder.enable_push(false);
    let (send_request, mut conn) = match builder.handshake::<IO, Bytes>(io).await {
        Ok(pair) => pair,
        Err(err) => {
            let msg = err.to_string();
            let _ = handshake_tx.send(Some(Err(msg.clone())));
            if session.borrow().has_listener_str("error") {
                if let Ok(err_val) = Exception::from_message(ctx.clone(), &msg) {
                    let _ = ClientHttp2Session::emit_str(
                        session.clone(),
                        ctx,
                        "error",
                        vec![err_val.into_value()],
                        false,
                    );
                }
            }
            let already = {
                let mut b = session.borrow_mut();
                let prev = b.closed;
                b.closed = true;
                b.destroyed = true;
                b.cmd_tx = None;
                b.loop_ref.unreference();
                prev
            };
            if !already {
                let _ = ClientHttp2Session::emit_str(session, ctx, "close", vec![], false);
            }
            return;
        },
    };

    let remote_max = conn
        .max_concurrent_send_streams()
        .min(max_concurrent_cap as usize) as u32;
    let _ = handshake_tx.send(Some(Ok(send_request)));

    if let Ok(settings_obj) = Object::new(ctx.clone()) {
        let _ = settings_obj.set("maxConcurrentStreams", remote_max);
        let _ = settings_obj.set("enablePush", false);
        let _ = ClientHttp2Session::emit_str(
            session.clone(),
            ctx,
            "remoteSettings",
            vec![settings_obj.into_value()],
            false,
        );
    }

    let socket_val = socket_for_connect_event.unwrap_or_else(|| Undefined.into_value(ctx.clone()));
    let _ = ClientHttp2Session::emit_str(
        session.clone(),
        ctx,
        "connect",
        vec![session.clone().into_value(), socket_val],
        false,
    );

    let loop_ref = session.borrow().loop_ref.clone();
    let mut ping_pong = conn.ping_pong();
    let mut active_ping: Option<(Instant, oneshot::Sender<std::result::Result<f64, String>>)> =
        None;
    let mut closing = false;
    let mut conn_err: Option<String> = None;

    loop {
        if closing && session.borrow().destroyed {
            break;
        }
        tokio::select! {
            _ = lexe_utils::event_loop::wait_until_io_unref(loop_ref.clone()) => {
                break;
            }
            res = &mut conn => {
                if let Err(e) = res {
                    conn_err = Some(e.to_string());
                }
                break;
            }
            pong_res = async {
                if let Some(ref mut pp) = ping_pong {
                    poll_fn(|cx| pp.poll_pong(cx)).await
                } else {
                    std::future::pending().await
                }
            }, if active_ping.is_some() => {
                if let Some((started, reply_tx)) = active_ping.take() {
                    match pong_res {
                        Ok(_) => {
                            let ms = started.elapsed().as_secs_f64() * 1000.0;
                            let _ = reply_tx.send(Ok(ms));
                        },
                        Err(e) => {
                            let _ = reply_tx.send(Err(e.to_string()));
                        },
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(SessionCmd::Ping(reply_tx)) => {
                        if let Some(ref mut pp) = ping_pong {
                            if active_ping.is_some() {
                                let _ = reply_tx.send(Err("ping already in flight".into()));
                            } else if let Err(e) = pp.send_ping(h2::Ping::opaque()) {
                                let _ = reply_tx.send(Err(e.to_string()));
                            } else {
                                active_ping = Some((Instant::now(), reply_tx));
                            }
                        } else {
                            let _ = reply_tx.send(Err("ping unavailable".into()));
                        }
                    },
                    Some(SessionCmd::Close) => {
                        closing = true;
                    },
                    Some(SessionCmd::Destroy) | None => {
                        break;
                    },
                }
            }
        }
    }

    if let Some((_, reply_tx)) = active_ping.take() {
        let _ = reply_tx.send(Err("session closed".into()));
    }
    if let Some(msg) = conn_err {
        if session.borrow().has_listener_str("error") {
            if let Ok(err_val) = Exception::from_message(ctx.clone(), &msg) {
                let _ = ClientHttp2Session::emit_str(
                    session.clone(),
                    ctx,
                    "error",
                    vec![err_val.into_value()],
                    false,
                );
            }
        }
    }
    let already = {
        let mut borrow = session.borrow_mut();
        let prev = borrow.closed;
        borrow.closed = true;
        borrow.destroyed = true;
        borrow.cmd_tx = None;
        borrow.loop_ref.unreference();
        prev
    };
    if !already {
        let _ = ClientHttp2Session::emit_str(session, ctx, "close", vec![], false);
    }
}

pub fn connect<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, ClientHttp2Session<'js>>> {
    let mut authority: Option<Value<'js>> = None;
    let mut options: Option<Object<'js>> = None;
    let mut listener = None;
    for value in args.0 {
        if authority.is_none()
            && (value.is_string()
                || (value.is_object()
                    && !value.is_null()
                    && value
                        .as_object()
                        .and_then(|o| o.get::<_, Option<String>>("href").ok().flatten())
                        .is_some()))
        {
            authority = Some(value);
        } else if value.is_object() && !value.is_null() {
            options = Some(value.as_object().unwrap().clone());
        } else if value.is_function() {
            listener = value.into_function();
        }
    }
    let authority_val =
        authority.ok_or_else(|| Exception::throw_type(&ctx, "authority required"))?;
    let (host, port, secure) = parse_authority(&ctx, authority_val.clone())?;

    let create_connection = options.as_ref().and_then(|o| {
        o.get::<_, Option<Function>>("createConnection")
            .ok()
            .flatten()
    });
    let max_concurrent_cap = options
        .as_ref()
        .and_then(|o| {
            o.get::<_, Option<u32>>("peerMaxConcurrentStreams")
                .ok()
                .flatten()
        })
        .unwrap_or(100);

    let (handshake_tx, handshake_rx) = watch::channel(None);
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

    let mut handoff_target: Option<HandoffTarget<'js>> = None;
    let mut custom_socket_val: Option<Value<'js>> = None;
    let mut ready_rx: Option<oneshot::Receiver<std::result::Result<(), String>>> = None;
    let mut session_encrypted = secure;

    if let Some(create_conn) = create_connection {
        let opts_val = options
            .as_ref()
            .map(|o| o.clone().into_value())
            .unwrap_or_else(|| Undefined.into_value(ctx.clone()));
        let sock_val: Value = create_conn.call((authority_val, opts_val))?;
        let handle_val = sock_val
            .as_object()
            .and_then(|o| o.get::<_, Option<Value>>("_handle").ok().flatten())
            .unwrap_or_else(|| sock_val.clone());

        let (target, wait_event) = if let Ok(tls_cls) = Class::<TlsSocket>::from_value(&handle_val)
        {
            session_encrypted = true;
            let wait = tls_cls
                .borrow()
                .secure_connecting()
                .then_some("secureConnect");
            (HandoffTarget::Tls(tls_cls), wait)
        } else if let Ok(net_cls) = Class::<NetSocket>::from_value(&handle_val) {
            let wait = (net_cls.borrow().ready_state() != "open").then_some("connect");
            (HandoffTarget::Net(net_cls), wait)
        } else {
            return Err(Exception::throw_type(
                &ctx,
                "createConnection must return a net.Socket or tls.TLSSocket",
            ));
        };

        if let Some(ev_name) = wait_event {
            if let Some(sock_obj) = sock_val.as_object() {
                if let Ok(once_fn) = sock_obj.get::<_, Function>("once") {
                    let (tx, rx) = oneshot::channel();
                    let slot = Arc::new(std::sync::Mutex::new(Some(tx)));
                    let slot_ok = slot.clone();
                    let on_ready = Function::new(ctx.clone(), move || {
                        if let Some(tx) = slot_ok.lock().unwrap().take() {
                            let _ = tx.send(Ok(()));
                        }
                    })?;
                    let slot_err = slot;
                    let on_err = Function::new(ctx.clone(), move |err: Opt<Value<'js>>| {
                        let msg = err
                            .0
                            .and_then(|v| {
                                v.as_object()
                                    .and_then(|o| o.get::<_, String>("message").ok())
                            })
                            .unwrap_or_else(|| "createConnection socket error".to_string());
                        if let Some(tx) = slot_err.lock().unwrap().take() {
                            let _ = tx.send(Err(msg));
                        }
                    })?;
                    let _ = once_fn.call::<_, ()>((This(sock_obj.clone()), ev_name, on_ready));
                    let _ = once_fn.call::<_, ()>((This(sock_obj.clone()), "error", on_err));
                    ready_rx = Some(rx);
                }
            }
        }

        handoff_target = Some(target);
        custom_socket_val = Some(sock_val);
    }

    let session =
        ClientHttp2Session::new_pending(ctx.clone(), session_encrypted, handshake_rx, cmd_tx)?;
    if let Some(listener) = listener {
        ClientHttp2Session::add_event_listener_str(
            session.clone(),
            &ctx,
            "connect",
            listener,
            false,
            false,
        )?;
    }

    let tls_opts = if handoff_target.is_none() && secure {
        Some(if let Some(obj) = options {
            parse_connect_options(&ctx, obj.into_value())?
        } else {
            lexe_tls::options::TlsOptions::default()
        })
    } else {
        None
    };

    let session_out = session.clone();
    let mut tls_opts = tls_opts;
    if let Some(ref mut opts) = tls_opts {
        if opts.alpn_protocols.is_none() {
            let arr = Array::new(ctx.clone())?;
            arr.set(0, "h2")?;
            opts.alpn_protocols = Some(convert_alpn_protocols(&ctx, arr.into_value())?);
        }
    }

    let ctx2 = ctx.clone();
    ctx.spawn_exit_simple(async move {
        if let Some(rx) = ready_rx {
            match rx.await {
                Ok(Ok(())) => {},
                Ok(Err(msg)) => {
                    let _ = handshake_tx.send(Some(Err(msg.clone())));
                    return Err(Exception::throw_message(&ctx2, &msg));
                },
                Err(_) => {
                    let _ = handshake_tx.send(Some(Err("createConnection aborted".into())));
                    return Err(Exception::throw_message(&ctx2, "createConnection aborted"));
                },
            }
        }
        match handoff_target {
            Some(HandoffTarget::Tls(tls_cls)) => {
                let (reader, writer, prefix) =
                    match TlsSocket::begin_http2_handoff(&ctx2, tls_cls).await {
                        Ok(res) => res,
                        Err(e) => {
                            let _ = handshake_tx.send(Some(Err(e.to_string())));
                            return Err(e);
                        },
                    };
                let io = PrefixedSplitIo::new(prefix, reader, writer);
                run_h2_session(
                    io,
                    session,
                    &ctx2,
                    handshake_tx,
                    cmd_rx,
                    custom_socket_val,
                    max_concurrent_cap,
                )
                .await;
            },
            Some(HandoffTarget::Net(net_cls)) => {
                let (reader, writer, prefix) =
                    match NetSocket::begin_tls_handoff(&ctx2, net_cls).await {
                        Ok(res) => res,
                        Err(e) => {
                            let _ = handshake_tx.send(Some(Err(e.to_string())));
                            return Err(e);
                        },
                    };
                let io = PrefixedSplitIo::new(prefix, reader, writer);
                run_h2_session(
                    io,
                    session,
                    &ctx2,
                    handshake_tx,
                    cmd_rx,
                    custom_socket_val,
                    max_concurrent_cap,
                )
                .await;
            },
            None => {
                let addr = format!("{host}:{port}");
                let tcp = TcpStream::connect(&addr)
                    .await
                    .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                if secure {
                    let tls_opts = tls_opts.unwrap();
                    let context = if let Some(sc) = tls_opts.secure_context.clone() {
                        sc
                    } else {
                        SecureContext::from_options(&tls_opts)
                            .map_err(|e| Exception::throw_message(&ctx2, &e))?
                    };
                    let connect_opts = ConnectOptions {
                        context,
                        identity_name: host.clone(),
                        sni_name: Some(host.clone()),
                        reject_unauthorized: tls_opts.reject_unauthorized,
                        timeout: None,
                        verify_record: None,
                        session: tls_opts.session.clone(),
                        session_slot: None,
                        session_notify: None,
                    };
                    let tls = tls_connect(tcp, connect_opts)
                        .await
                        .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                    run_h2_session(
                        tls,
                        session,
                        &ctx2,
                        handshake_tx,
                        cmd_rx,
                        None,
                        max_concurrent_cap,
                    )
                    .await;
                } else {
                    run_h2_session(
                        tcp,
                        session,
                        &ctx2,
                        handshake_tx,
                        cmd_rx,
                        None,
                        max_concurrent_cap,
                    )
                    .await;
                }
            },
        }
        Ok(())
    });

    Ok(session_out)
}

fn value_to_string<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    if let Some(s) = value.as_string() {
        return s.to_string();
    }
    if let Some(n) = value.as_number() {
        return Ok(n.to_string());
    }
    let s: String = ctx.eval::<Function, _>("String")?.call((value,))?;
    Ok(s)
}
