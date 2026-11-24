// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::Duration;

use crate::backend::{accept, AcceptOptions};
use crate::options::parse_server_options;
use crate::secure_context::{secure_context_from_value, SecureContext};
use crate::sni::SniRegistry;
use crate::tls_socket::{rw_join, TlsSocket};
use lexe_context::CtxExtension;
use lexe_events::{EmitError, Emitter, EventEmitter, EventList};
use lexe_net::{get_address_parts, get_hostname};
use lexe_stream::SteamEvents;
use lexe_utils::{object::ObjectExt, result::ResultExt, reuse_list::ReuseList};
use rquickjs::{
    class::Trace,
    prelude::{Opt, Rest, This},
    Class, Ctx, Exception, Function, JsLifetime, Object, Result, Undefined, Value,
};
use tokio::{
    net::TcpListener,
    select,
    sync::{
        broadcast::{self, Sender},
        Notify,
    },
};

lexe_stream::impl_stream_events!(Server);

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(120);

fn parse_listen_port<'js>(ctx: &Ctx<'js>, port: i32) -> Result<u16> {
    u16::try_from(port).map_err(|_| Exception::throw_range(ctx, "port"))
}

#[rquickjs::class]
pub struct Server<'js> {
    emitter: EventEmitter<'js>,
    address: Value<'js>,
    close_tx: Sender<()>,
    allow_half_open: bool,
    already_listen: Arc<AtomicBool>,
    sockets: ReuseList<Class<'js, TlsSocket<'js>>>,
    should_close: Arc<AtomicBool>,
    secure_context: Arc<RwLock<Arc<SecureContext>>>,
    sni_registry: Arc<RwLock<SniRegistry>>,
    request_cert: bool,
    reject_unauthorized: bool,
    http_handoff_http_server: RefCell<Option<Value<'js>>>,
}

impl<'js> Trace<'js> for Server<'js> {
    fn trace<'a>(&self, tracer: rquickjs::class::Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
        self.address.trace(tracer);
        for socket_ref in self.sockets.iter() {
            socket_ref.trace(tracer);
        }
    }
}

unsafe impl<'js> JsLifetime<'js> for Server<'js> {
    type Changed<'to> = Server<'to>;
}

impl<'js> Emitter<'js> for Server<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Server<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Class<'js, Self>> {
        let mut args_iter = args.0.into_iter();

        let mut connection_listener = None;
        let mut allow_half_open = false;
        let mut options = None;

        if let Some(first) = args_iter.next() {
            if let Some(connection_listener_arg) = first.as_function() {
                connection_listener = Some(connection_listener_arg.clone());
            } else if let Some(opts_arg) = first.as_object() {
                options = Some(opts_arg.clone().into_value());
                allow_half_open = opts_arg.get_optional("allowHalfOpen")?.unwrap_or_default();
            }
        }
        if let Some(next) = args_iter.next() {
            connection_listener = next.into_function();
        }

        let (request_cert, reject_unauthorized, secure_context) = if let Some(opts) = options {
            let parsed = parse_server_options(&ctx, opts)?;
            let ctx_inner = if let Some(sc) = parsed.secure_context.clone() {
                sc
            } else {
                SecureContext::from_options(&parsed)
                    .map_err(|e| Exception::throw_message(&ctx, &e))?
            };
            (
                parsed.request_cert,
                parsed.reject_unauthorized,
                Arc::new(ctx_inner),
            )
        } else {
            (
                false,
                true,
                Arc::new(
                    SecureContext::from_options(&crate::options::TlsOptions::default()).map_err(
                        |_e| Exception::throw_message(&ctx, "default secure context required"),
                    )?,
                ),
            )
        };

        let emitter = EventEmitter::new();
        let (close_tx, _) = broadcast::channel::<()>(1);

        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter,
                address: Undefined.into_value(ctx.clone()),
                close_tx,
                allow_half_open,
                already_listen: Arc::new(AtomicBool::new(false)),
                sockets: ReuseList::with_capacity(8),
                should_close: Arc::new(AtomicBool::new(false)),
                secure_context: Arc::new(RwLock::new(secure_context)),
                sni_registry: Arc::new(RwLock::new(SniRegistry::new())),
                request_cert,
                reject_unauthorized,
                http_handoff_http_server: RefCell::new(None),
            },
        )?;

        if let Some(connection_listener) = connection_listener {
            Self::add_event_listener_str(
                instance.clone(),
                &ctx,
                "secureConnection",
                connection_listener,
                false,
                false,
            )?;
        }

        Ok(instance)
    }

    pub fn address(&self) -> Value<'js> {
        self.address.clone()
    }

    pub fn get_connections(&self, cb: Opt<Function<'js>>) -> Result<()> {
        if let Some(cb) = cb.0 {
            cb.call::<_, ()>((Undefined, self.sockets.len()))?;
        }
        Ok(())
    }

    pub fn set_secure_context(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        context: Value<'js>,
    ) -> Result<()> {
        let context = secure_context_from_value(&ctx, context)?;
        *this.borrow().secure_context.write().unwrap() = context;
        Ok(())
    }

    pub fn add_context(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        hostname: String,
        context: Value<'js>,
    ) -> Result<()> {
        let context = secure_context_from_value(&ctx, context)?;
        this.borrow()
            .sni_registry
            .write()
            .unwrap()
            .insert(hostname, context);
        Ok(())
    }

    pub fn listen(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let mut args_iter = args.0.into_iter();
        let mut port: Option<u16> = None;
        let mut host = None;
        let mut callback = None;

        if let Some(first) = args_iter.next() {
            if let Some(port_arg) = first.as_int() {
                port = Some(parse_listen_port(&ctx, port_arg)?);
                if let Some(second) = args_iter.next() {
                    if let Some(cb) = second.as_function() {
                        callback = Some(cb.clone());
                    } else if let Some(host_str) = second.as_string() {
                        host = Some(host_str.to_string()?);
                        callback = args_iter.next().and_then(|v| v.into_function());
                    } else {
                        return Err(Exception::throw_type(
                            &ctx,
                            "listen(port, host) requires host to be a string",
                        ));
                    }
                }
            } else if let Some(cb) = first.as_function() {
                callback = Some(cb.clone());
            } else if let Some(opts_arg) = first.as_object() {
                if let Some(port_val) = opts_arg.get_optional::<_, i32>("port")? {
                    port = Some(parse_listen_port(&ctx, port_val)?);
                }
                host = opts_arg.get_optional("host")?;
                callback = args_iter.next().and_then(|v| v.into_function());
            } else {
                return Err(Exception::throw_type(
                    &ctx,
                    "listen() first argument must be a port, options object, or callback",
                ));
            }
        }

        if port.is_none() {
            port = Some(0);
        }

        let borrow = this.borrow();
        let mut close_rx = borrow.close_tx.subscribe();
        let allow_half_open = borrow.allow_half_open;
        let already_running = borrow.already_listen.clone();
        let should_close = borrow.should_close.clone();
        let request_cert = borrow.request_cert;
        let reject_unauthorized = borrow.reject_unauthorized;
        let secure_context = borrow.secure_context.clone();
        let sni_registry = borrow.sni_registry.clone();
        drop(borrow);

        if already_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(Exception::throw_message(&ctx, "ERR_SERVER_ALREADY_LISTEN"));
        }

        let release_listen = already_running.clone();
        if let Some(callback) = callback {
            if let Err(err) =
                Self::add_event_listener_str(this.clone(), &ctx, "listening", callback, true, true)
            {
                release_listen.store(false, Ordering::SeqCst);
                return Err(err);
            }
        }

        let ctx2 = ctx.clone();
        let server = this.0.clone();
        if let Err(err) = ctx.spawn_exit(async move {
            let should_emit_close: Result<bool> = async {
                let listener =
                    match Self::bind(server.clone(), ctx2.clone(), port.map(|p| p as i32), host)
                        .await
                    {
                        Ok(listener) => listener,
                        Err(e) => {
                            Err::<(), _>(e).emit_error("listen", &ctx2, server.clone())?;
                            return Ok(false);
                        },
                    };

                Self::emit_str(server.clone(), &ctx2, "listening", vec![], false)?;

                let notify = Arc::new(Notify::new());
                let close_notify = notify.notified();

                loop {
                    let ctx3 = ctx2.clone();
                    let server2 = server.clone();

                    select! {
                        accept_result = listener.accept() => {
                            Self::handle_connection(
                                server2.clone(),
                                ctx3.clone(),
                                accept_result,
                                notify.clone(),
                                allow_half_open,
                                secure_context.clone(),
                                sni_registry.clone(),
                                request_cert,
                                reject_unauthorized,
                            ).emit_error("handle_connection", &ctx3, server2)?;
                        },
                        _ = close_rx.recv() => {
                            break;
                        }
                    }
                }

                if !server.borrow().sockets.is_empty() {
                    close_notify.await;
                }

                Ok(true)
            }
            .await;

            already_running.store(false, Ordering::Relaxed);
            should_close.store(false, Ordering::Relaxed);

            match should_emit_close {
                Ok(true) => Self::emit_str(server, &ctx2, "close", vec![], false),
                Ok(false) => Ok(()),
                Err(err) => Err(err),
            }
        }) {
            release_listen.store(false, Ordering::SeqCst);
            return Err(err);
        }

        Ok(this.0)
    }

    pub fn close(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        cb: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        if let Some(cb) = cb.0 {
            Self::add_event_listener_str(this.clone(), &ctx, "close", cb, true, true)?;
        }
        {
            let borrow = this.borrow_mut();
            borrow.should_close.store(true, Ordering::Relaxed);
            let _ = borrow.close_tx.send(());
        }
        Ok(this.0)
    }
}

impl<'js> Server<'js> {
    pub fn set_http_handoff_target(this: Class<'js, Self>, http_server: Value<'js>) {
        *this.borrow().http_handoff_http_server.borrow_mut() = Some(http_server);
    }

    async fn bind(
        this: Class<'js, Self>,
        ctx: Ctx<'js>,
        port: Option<i32>,
        host: Option<String>,
    ) -> Result<TcpListener> {
        let listener = TcpListener::bind(get_hostname(
            &host.unwrap_or_else(|| String::from("0.0.0.0")),
            port.unwrap_or(0) as u16,
        ))
        .await
        .or_throw(&ctx)?;

        let address_object = Object::new(ctx.clone())?;
        let (address, port, family) = get_address_parts(&ctx, listener.local_addr())?;
        address_object.set("address", address)?;
        address_object.set("port", port)?;
        address_object.set("family", family)?;
        this.borrow_mut().address = address_object.into_value();

        Ok(listener)
    }

    fn handle_connection(
        this: Class<'js, Self>,
        ctx: Ctx<'js>,
        accept_result: std::result::Result<
            (tokio::net::TcpStream, std::net::SocketAddr),
            std::io::Error,
        >,
        notify_close: Arc<Notify>,
        allow_half_open: bool,
        secure_context: Arc<RwLock<Arc<SecureContext>>>,
        sni_registry: Arc<RwLock<SniRegistry>>,
        request_cert: bool,
        reject_unauthorized: bool,
    ) -> Result<()> {
        let (tcp_stream, _) = accept_result.or_throw(&ctx)?;

        ctx.clone().spawn_exit(async move {
            // The listener must receive a real `tls.TLSSocket` (a net.Socket
            // subclass); the native handle is its `_handle` so both sides of
            // `process_split_io` operate on the same object.
            let tls_socket = TlsSocket::new(ctx.clone(), true)?;
            let socket_instance =
                crate::client::build_tls_socket_object(&ctx, &tls_socket, allow_half_open)?;
            let socket_index;
            {
                let mut server_borrow = this.borrow_mut();
                socket_index = server_borrow.sockets.append(tls_socket.clone());
            }

            let _cleanup = SocketCleanupGuard {
                server: this.clone(),
                socket_index,
                notify_close: notify_close.clone(),
            };

            TlsSocket::set_addresses_from_tcp(&tls_socket, &ctx, &tcp_stream)?;

            Self::emit_str(
                this.clone(),
                &ctx,
                "connection",
                vec![socket_instance.clone()],
                false,
            )?;

            let context = secure_context.read().unwrap().clone();
            let selected_local_cert = Arc::new(Mutex::new(None));
            let accept_opts = AcceptOptions {
                context: (*context).clone(),
                request_cert,
                reject_unauthorized,
                timeout: Some(HANDSHAKE_TIMEOUT),
                sni_registry,
                selected_local_cert: selected_local_cert.clone(),
            };

            let accept_result = accept(tcp_stream, accept_opts).await;

            match accept_result {
                Ok(tls_stream) => {
                    let tls_stream = match TlsSocket::finish_server_handshake(
                        tls_socket.clone(),
                        ctx.clone(),
                        tls_stream,
                        reject_unauthorized,
                        selected_local_cert,
                    )
                    .await
                    {
                        Ok(stream) => stream,
                        Err(err_value) => {
                            TlsSocket::fail_connect(tls_socket.clone(), &ctx, err_value)?;
                            return Ok(());
                        },
                    };

                    Self::emit_str(
                        this.clone(),
                        &ctx,
                        "secureConnection",
                        vec![socket_instance.clone()],
                        false,
                    )?;

                    if let Some(http_server) =
                        this.borrow().http_handoff_http_server.borrow().clone()
                    {
                        if crate::http_handoff::http_handoff_registered() {
                            crate::http_handoff::handoff_to_http(
                                ctx.clone(),
                                http_server,
                                tls_socket.clone(),
                                tls_stream,
                            )?;
                            return Ok(());
                        }
                    }

                    let (reader, writer) = tokio::io::split(tls_stream);
                    let (readable_done, writable_done) = TlsSocket::process_split_io(
                        &tls_socket,
                        &ctx,
                        reader,
                        writer,
                        allow_half_open,
                    )?;

                    let had_error = rw_join(&ctx, readable_done, writable_done).await?;
                    TlsSocket::emit_close(tls_socket, &ctx, had_error)?;
                },
                Err(e) => {
                    let err_msg = e.to_string();
                    if let Ok(exception) = Exception::from_message(ctx.clone(), &err_msg) {
                        Self::emit_str(
                            this.clone(),
                            &ctx,
                            "tlsClientError",
                            vec![exception.into_value(), socket_instance],
                            false,
                        )?;
                    }
                    TlsSocket::emit_close(tls_socket, &ctx, true)?;
                },
            }

            Ok(())
        })?;

        Ok(())
    }
}

struct SocketCleanupGuard<'js> {
    server: Class<'js, Server<'js>>,
    socket_index: usize,
    notify_close: Arc<Notify>,
}

impl<'js> Drop for SocketCleanupGuard<'js> {
    fn drop(&mut self) {
        if let Ok(mut server_borrow) = self.server.try_borrow_mut() {
            server_borrow.sockets.remove(self.socket_index);
            if server_borrow.sockets.is_empty()
                && server_borrow.should_close.load(Ordering::Relaxed)
            {
                self.notify_close.notify_one();
            }
        }
    }
}

pub fn create_server<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, Server<'js>>> {
    Server::new(ctx, args)
}
