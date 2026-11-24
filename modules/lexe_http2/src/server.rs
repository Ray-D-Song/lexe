// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};

use bytes::Bytes;
use http::Response as HttpResponse;
use http_body_util::{BodyExt, Full};
use hyper::{Response, StatusCode};
use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_events::{Emitter, EventEmitter, EventList};
use lexe_timers::{poll_timers, ExecutingTimer};
use lexe_tls::{
    alpn::convert_alpn_protocols,
    backend::{accept, AcceptOptions},
    options::parse_server_options,
    secure_context::SecureContext,
    sni::SniRegistry,
};
use lexe_utils::{
    event_loop::{self, EventLoopRef},
    object::ObjectExt,
};
use rquickjs::{
    class::Trace,
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, Opt, Rest, This},
    qjs, Array, Class, Ctx, Exception, Function, IntoJs, Object, Result, Undefined, Value,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
    sync::{broadcast, oneshot},
};

use crate::stream::Http2Stream;

static HTTP2_JS_ROOT: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

struct Http2JsSerialGuard {
    _root: tokio::sync::MutexGuard<'static, ()>,
}

impl Http2JsSerialGuard {
    async fn acquire() -> Self {
        Self {
            _root: HTTP2_JS_ROOT.lock().await,
        }
    }
}

#[rquickjs::class]
pub struct Http2Server<'js> {
    emitter: EventEmitter<'js>,
    address: Value<'js>,
    close_tx: broadcast::Sender<()>,
    listening: Arc<AtomicBool>,
    loop_ref: Arc<EventLoopRef>,
    secure_context: Option<Arc<SecureContext>>,
    request_cert: bool,
    reject_unauthorized: bool,
    sni_registry: Option<Arc<std::sync::RwLock<SniRegistry>>>,
}

unsafe impl<'js> rquickjs::JsLifetime<'js> for Http2Server<'js> {
    type Changed<'to> = Http2Server<'to>;
}

impl<'js> Trace<'js> for Http2Server<'js> {
    fn trace<'a>(&self, t: rquickjs::class::Tracer<'a, 'js>) {
        self.emitter.trace(t);
        self.address.trace(t);
    }
}

impl<'js> Emitter<'js> for Http2Server<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Http2Server<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, listener: Opt<Function<'js>>) -> Result<Class<'js, Self>> {
        let server = Self::alloc(ctx.clone(), None)?;
        if let Some(listener) = listener.0 {
            Self::add_event_listener_str(server.clone(), &ctx, "request", listener, false, false)?;
        }
        Ok(server)
    }

    pub fn address(&self) -> Value<'js> {
        self.address.clone()
    }

    #[qjs(get, enumerable)]
    pub fn listening(&self) -> bool {
        self.listening.load(Ordering::Relaxed)
    }

    pub fn close(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        if let Some(callback) = callback.0 {
            Self::add_event_listener_str(this.0.clone(), &ctx, "close", callback, true, true)?;
        }
        let _ = this.borrow().close_tx.send(());
        Ok(this.0)
    }

    pub fn listen(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let mut port = 0u16;
        let mut host = "127.0.0.1".to_string();
        let mut callback = None;
        let mut saw_port = false;
        for (index, arg) in args.0.into_iter().enumerate() {
            if let Some(p) = arg.as_int() {
                port = p as u16;
                saw_port = true;
            } else if let Some(s) = arg.as_string() {
                let value = s.to_string()?;
                if index == 0 && !saw_port {
                    host = value;
                } else {
                    host = value;
                }
            } else if let Some(f) = arg.as_function() {
                callback = Some(f.clone());
            } else if let Some(o) = arg.as_object() {
                if let Some(p) = o.get_optional::<_, u16>("port")? {
                    port = p;
                }
                if let Some(h) = o.get_optional::<_, String>("host")? {
                    host = h;
                }
            }
        }
        if let Some(callback) = callback {
            Self::add_event_listener_str(this.0.clone(), &ctx, "listening", callback, true, true)?;
        }
        let return_server = this.0.clone();
        let server = this.0.clone();
        let ctx2 = ctx.clone();
        let mut close = server.borrow().close_tx.subscribe();
        let listen_loop_ref = server.borrow().loop_ref.clone();
        let secure_context = server.borrow().secure_context.clone();
        let request_cert = server.borrow().request_cert;
        let reject_unauthorized = server.borrow().reject_unauthorized;
        let sni_registry = server.borrow().sni_registry.clone();
        ctx.spawn_exit_simple(async move {
            let listener = TcpListener::bind(format!("{host}:{port}"))
                .await
                .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
            let addr = listener
                .local_addr()
                .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
            let address = Object::new(ctx2.clone())?;
            address.set("address", addr.ip().to_string())?;
            address.set("port", addr.port())?;
            address.set(
                "family",
                if addr.is_ipv4() { "IPv4" } else { "IPv6" },
            )?;
            server.borrow_mut().address = address.into_value();
            server
                .borrow()
                .listening
                .store(true, Ordering::Relaxed);
            Self::emit_str(server.clone(), &ctx2, "listening", vec![], false)?;
            loop {
                if !event_loop::keeps_event_loop_alive(&listen_loop_ref) {
                    break;
                }
                tokio::select! {
                    _ = event_loop::wait_until_io_unref(listen_loop_ref.clone()) => break,
                    _ = close.recv() => break,
                    accepted = listener.accept() => {
                        let (tcp, _) = accepted.map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                        let ctx3 = ctx2.clone();
                        let srv = server.clone();
                        let tls_ctx = secure_context.clone();
                        let sni = sni_registry.clone();
                        let ctx4 = ctx3.clone();
                        let _ = ctx3.spawn_exit_simple(async move {
                            let _ = serve_connection(
                                ctx4,
                                srv,
                                tcp,
                                tls_ctx,
                                request_cert,
                                reject_unauthorized,
                                sni,
                            )
                            .await;
                            Ok(())
                        });
                    }
                }
            }
            server
                .borrow()
                .listening
                .store(false, Ordering::Relaxed);
            Self::emit_str(server, &ctx2, "close", vec![], false)?;
            Ok(())
        });
        Ok(return_server)
    }
}

impl<'js> Http2Server<'js> {
    fn alloc(
        ctx: Ctx<'js>,
        secure: Option<(Arc<SecureContext>, bool, bool)>,
    ) -> Result<Class<'js, Self>> {
        let (secure_context, request_cert, reject_unauthorized, sni_registry) = match secure {
            Some((ctx, req, rej)) => (
                Some(ctx),
                req,
                rej,
                Some(Arc::new(std::sync::RwLock::new(SniRegistry::new()))),
            ),
            None => (None, false, true, None),
        };
        let (close_tx, _) = broadcast::channel(1);
        let rt_ptr = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);
        Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                address: Undefined.into_value(ctx.clone()),
                close_tx,
                listening: Arc::new(AtomicBool::new(false)),
                loop_ref,
                secure_context,
                request_cert,
                reject_unauthorized,
                sni_registry,
            },
        )
    }
}

async fn serve_connection<'js>(
    ctx: Ctx<'js>,
    server: Class<'js, Http2Server<'js>>,
    tcp: tokio::net::TcpStream,
    secure_context: Option<Arc<SecureContext>>,
    request_cert: bool,
    reject_unauthorized: bool,
    sni_registry: Option<Arc<std::sync::RwLock<SniRegistry>>>,
) -> Result<()> {
    if let Some(context) = secure_context {
        let sni_registry =
            sni_registry.unwrap_or_else(|| Arc::new(std::sync::RwLock::new(SniRegistry::new())));
        let selected_local_cert = Arc::new(Mutex::new(None));
        let accept_opts = AcceptOptions {
            context: (*context).clone(),
            request_cert,
            reject_unauthorized,
            timeout: Some(Duration::from_secs(120)),
            sni_registry,
            selected_local_cert,
        };
        let tls_stream = accept(tcp, accept_opts)
            .await
            .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
        let (_, session) = tls_stream.get_ref();
        if session.alpn_protocol() != Some(b"h2") {
            return Err(Exception::throw_message(
                &ctx,
                "TLS connection did not negotiate HTTP/2 (ALPN h2)",
            ));
        }
        serve_h2_io(ctx, server, tls_stream).await
    } else {
        serve_h2_io(ctx, server, tcp).await
    }
}

async fn serve_h2_io<'js, IO>(
    ctx: Ctx<'js>,
    server: Class<'js, Http2Server<'js>>,
    io: IO,
) -> Result<()>
where
    IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut connection = h2::server::handshake(io)
        .await
        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
    while let Some(result) = connection.accept().await {
        let (request, mut respond) =
            result.map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
        let (parts, mut recv) = request.into_parts();
        let response = dispatch_h2(ctx.clone(), server.clone(), parts).await;
        let (resp_parts, resp_body) = response.into_parts();
        let mut send = respond
            .send_response(
                http::Response::builder()
                    .status(resp_parts.status)
                    .body(())
                    .unwrap(),
                false,
            )
            .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
        let body_bytes = http_body_util::BodyExt::collect(resp_body)
            .await
            .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?
            .to_bytes();
        send.send_data(body_bytes, true)
            .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
        tokio::spawn(async move { while recv.data().await.is_some() {} });
    }
    Ok(())
}

struct JsEventLoopPump {
    rt: *mut qjs::JSRuntime,
    executing_timers: Vec<Option<ExecutingTimer>>,
}

impl JsEventLoopPump {
    fn new(ctx: &Ctx<'_>) -> Self {
        let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        Self {
            rt,
            executing_timers: Vec::new(),
        }
    }

    fn tick(&mut self, ctx: &Ctx<'_>) {
        while ctx.execute_pending_job() {}
        let _ = poll_timers(self.rt, &mut self.executing_timers, None, None);
    }
}

async fn await_response_pumping(
    ctx: &Ctx<'_>,
    pump: &mut JsEventLoopPump,
    rx: oneshot::Receiver<Response<http_body_util::combinators::BoxBody<Bytes, Infallible>>>,
) -> Response<http_body_util::combinators::BoxBody<Bytes, Infallible>> {
    use tokio::time::{interval, MissedTickBehavior};
    let mut tick = interval(Duration::from_millis(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tokio::pin!(rx);
    loop {
        tokio::select! {
            biased;
            response = &mut rx => {
                return response.unwrap_or_else(|_| {
                    Response::builder()
                        .status(500)
                        .body(Full::new(Bytes::new()).boxed())
                        .unwrap()
                });
            }
            _ = tick.tick() => pump.tick(ctx),
        }
    }
}

async fn dispatch_h2<'js>(
    ctx: Ctx<'js>,
    server: Class<'js, Http2Server<'js>>,
    parts: hyper::http::request::Parts,
) -> Response<http_body_util::combinators::BoxBody<Bytes, Infallible>> {
    let _guard = Http2JsSerialGuard::acquire().await;
    let method = parts.method.to_string();
    let path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let scheme = parts.uri.scheme_str().unwrap_or("http").to_string();
    let authority = parts
        .uri
        .authority()
        .map(|a| a.as_str().to_string())
        .unwrap_or_else(|| "localhost".into());

    let empty_500 = || {
        Response::builder()
            .status(500)
            .body(Full::new(Bytes::new()).boxed())
            .unwrap()
    };

    let (response_tx, response_rx) = oneshot::channel();

    let (req_stream, res_stream) = match (
        Http2Stream::new_request(&ctx, method, path, scheme, authority, parts.headers),
        Http2Stream::new_response(&ctx, response_tx),
    ) {
        (Ok(req), Ok(res)) => (req, res),
        _ => return empty_500(),
    };

    let mut pump = JsEventLoopPump::new(&ctx);
    pump.tick(&ctx);

    if server.borrow().has_listener_str("request") {
        let _ = Http2Server::emit_str(
            server.clone(),
            &ctx,
            "request",
            vec![
                req_stream.clone().into_value(),
                res_stream.clone().into_value(),
            ],
            false,
        );
    }

    let ctx_pump = ctx.clone();
    tokio::pin!(response_rx);
    loop {
        use tokio::time::{interval, MissedTickBehavior};
        let mut tick = interval(Duration::from_millis(1));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        tokio::select! {
            biased;
            response = &mut response_rx => {
                return response.unwrap_or_else(|_| empty_500());
            }
            _ = tick.tick() => pump.tick(&ctx_pump),
        }
    }
}

pub fn create_server<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, Http2Server<'js>>> {
    let mut listener = None;
    for value in args.0 {
        if value.is_function() {
            listener = value.into_function();
        }
    }
    let server = Http2Server::alloc(ctx.clone(), None)?;
    if let Some(listener) = listener {
        Http2Server::add_event_listener_str(
            server.clone(),
            &ctx,
            "request",
            listener,
            false,
            false,
        )?;
    }
    Ok(server)
}

pub fn create_secure_server<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, Http2Server<'js>>> {
    let mut options: Option<Object<'js>> = None;
    let mut listener = None;
    for value in args.0 {
        if value.is_function() {
            listener = value.into_function();
        } else if value.is_object() {
            options = value.into_object();
        }
    }
    let options = options.ok_or_else(|| {
        Exception::throw_type(&ctx, "createSecureServer requires TLS options (cert/key)")
    })?;
    let mut parsed = parse_server_options(&ctx, options.clone().into_value())?;
    if parsed.alpn_protocols.is_none() {
        let arr = Array::new(ctx.clone())?;
        arr.set(0, "h2")?;
        parsed.alpn_protocols = Some(convert_alpn_protocols(&ctx, arr.into_value())?);
    }
    let secure_context = Arc::new(if let Some(sc) = parsed.secure_context.clone() {
        sc
    } else {
        SecureContext::from_options(&parsed).map_err(|e| Exception::throw_message(&ctx, &e))?
    });
    let server = Http2Server::alloc(
        ctx.clone(),
        Some((
            secure_context,
            parsed.request_cert,
            parsed.reject_unauthorized,
        )),
    )?;
    if let Some(listener) = listener {
        Http2Server::add_event_listener_str(
            server.clone(),
            &ctx,
            "request",
            listener,
            false,
            false,
        )?;
    }
    Ok(server)
}

fn get_default_settings_export<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    crate::constants::get_default_settings(ctx)
}

fn get_packed_settings_export<'js>(ctx: Ctx<'js>, settings: Value<'js>) -> Result<Buffer> {
    crate::constants::get_packed_settings(&ctx, settings)
}

fn get_unpacked_settings_export<'js>(ctx: Ctx<'js>, buf: Value<'js>) -> Result<Object<'js>> {
    crate::constants::get_unpacked_settings(ctx, buf)
}

pub struct Http2Module;

impl ModuleDef for Http2Module {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare(stringify!(Http2Server))?;
        declare.declare(stringify!(Http2Stream))?;
        declare.declare("createServer")?;
        declare.declare("createSecureServer")?;
        declare.declare("connect")?;
        declare.declare("getDefaultSettings")?;
        declare.declare("getPackedSettings")?;
        declare.declare("getUnpackedSettings")?;
        declare.declare("constants")?;
        declare.declare("sensitiveHeaders")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        lexe_utils::module::export_default(ctx, exports, |default| {
            lexe_buffer::init(ctx)?;
            Class::<Http2Server>::define(default)?;
            Http2Server::add_event_emitter_prototype(ctx)?;
            Class::<Http2Stream>::define(default)?;
            Http2Stream::add_event_emitter_prototype(ctx)?;
            crate::client::ClientHttp2Session::define_module(ctx, default)?;

            default.set("createServer", Func::from(create_server))?;
            default.set("createSecureServer", Func::from(create_secure_server))?;
            default.set("connect", Func::from(crate::client::connect))?;
            default.set(
                "getDefaultSettings",
                Func::from(get_default_settings_export),
            )?;
            default.set("getPackedSettings", Func::from(get_packed_settings_export))?;
            default.set(
                "getUnpackedSettings",
                Func::from(get_unpacked_settings_export),
            )?;
            crate::constants::define_constants(ctx, default)?;
            default.set(
                "sensitiveHeaders",
                ctx.eval::<Value, _>("Symbol.for('nodejs.http2.sensitiveHeaders')")?,
            )?;
            Ok(())
        })?;
        Ok(())
    }
}

impl From<Http2Module> for lexe_utils::module::ModuleInfo<Http2Module> {
    fn from(val: Http2Module) -> Self {
        lexe_utils::module::ModuleInfo {
            name: "http2",
            module: val,
        }
    }
}
