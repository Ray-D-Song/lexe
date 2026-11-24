// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::uninlined_format_args)]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

use lexe_context::CtxExtension;
use lexe_events::{EmitError, Emitter, EventEmitter, EventList};
use lexe_stream::{impl_stream_events, SteamEvents};
use lexe_utils::{
    event_loop::EventLoopRef, object::ObjectExt, result::ResultExt, reuse_list::ReuseList,
};
#[cfg(unix)]
use rquickjs::IntoJs;
use rquickjs::{
    class::Trace,
    prelude::{Opt, Rest, This},
    Class, Ctx, Exception, Function, JsLifetime, Null, Object, Result, Undefined, Value,
};
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::{
    net::TcpListener,
    select,
    sync::{
        broadcast::{self, Sender},
        Notify,
    },
};
use tracing::trace;

use super::{get_address_parts, get_hostname, socket::Socket, Listener, NetStream};

impl_stream_events!(Server);

#[rquickjs::class]
pub struct Server<'js> {
    emitter: EventEmitter<'js>,
    address: Value<'js>,
    close_tx: Sender<()>,
    allow_half_open: bool,
    already_listen: Arc<AtomicBool>,
    sockets: ReuseList<Class<'js, Socket<'js>>>,
    should_close: Arc<AtomicBool>,
    loop_ref: Arc<EventLoopRef>,
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

        if let Some(first) = args_iter.next() {
            if let Some(connection_listener_arg) = first.as_function() {
                connection_listener = Some(connection_listener_arg.clone());
            }
            if let Some(opts_arg) = first.as_object() {
                allow_half_open = opts_arg.get_optional("allowHalfOpen")?.unwrap_or_default();
            }
        }
        if let Some(next) = args_iter.next() {
            connection_listener = next.into_function();
        }

        let emitter = EventEmitter::new();
        let (close_tx, _) = broadcast::channel::<()>(1);
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);

        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter,
                address: Null.into_value(ctx.clone()),
                close_tx,
                allow_half_open,
                already_listen: Arc::new(AtomicBool::new(false)),
                sockets: ReuseList::with_capacity(8),
                should_close: Arc::new(AtomicBool::new(false)),
                loop_ref,
            },
        )?;

        if let Some(connection_listener) = connection_listener {
            Self::add_event_listener_str(
                instance.clone(),
                &ctx,
                "connection",
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

    #[allow(unused_assignments)]
    ///TODO add backlog support
    pub fn listen(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<()> {
        let mut args_iter = args.0.into_iter();
        let mut port = None;
        let mut path = None;
        let mut host = None;
        #[allow(unused_variables)] //TODO add backlog support
        let mut backlog = None;
        let mut callback = None;

        let borrow = this.borrow();
        let mut close_rx = borrow.close_tx.subscribe();
        let allow_half_open = borrow.allow_half_open;
        let already_running = borrow.already_listen.clone();
        let should_close = borrow.should_close.clone();
        drop(borrow);

        if already_running.load(Ordering::Relaxed) {
            return Err(Exception::throw_message(&ctx, "ERR_SERVER_ALREADY_LISTEN"));
        }

        if let Some(first) = args_iter.next() {
            if let Some(callback_arg) = first.as_function() {
                callback = Some(callback_arg.clone());
            } else {
                if let Some(port_arg) = first.as_int() {
                    if port_arg > 0xFFFF {
                        return Err(Exception::throw_range(
                            &ctx,
                            "port should be between 0 and 65535",
                        ));
                    }
                    port = Some(port_arg);
                }
                if let Some(path_arg) = first.as_string() {
                    path = Some(path_arg.to_string()?);
                }
                if let Some(opts_arg) = first.as_object() {
                    port = opts_arg.get_optional("port")?;
                    path = opts_arg.get_optional("path")?;
                    host = opts_arg.get_optional("host")?;
                    backlog = opts_arg.get_optional("backlog")?;
                }

                let path = first.into_string();

                if let Some(second) = args_iter.next() {
                    if let Some(callback_arg) = second.as_function() {
                        callback = Some(callback_arg.clone());
                    }
                    if let Some(host_arg) = second.as_string() {
                        host = Some(host_arg.to_string()?);
                    }
                    if path.is_some() {
                        if let Some(backlog_arg) = second.as_int() {
                            backlog = Some(backlog_arg);
                        }
                    }
                    if let Some(third) = args_iter.next() {
                        if let Some(callback_arg) = third.as_function() {
                            callback = Some(callback_arg.clone());
                        }
                        if port.is_some() {
                            if let Some(backlog_arg) = third.as_int() {
                                backlog = Some(backlog_arg);

                                callback = args_iter.next().and_then(|v| v.into_function());
                            }
                        }
                    }
                }
            }
        }

        if let Some(callback) = callback {
            Self::add_event_listener_str(this.clone(), &ctx, "listening", callback, true, true)?;
        }

        let ctx2 = ctx.clone();
        let listen_loop_ref = this.borrow().loop_ref.clone();

        if port.is_none() && path.is_none() {
            port = Some(0)
        }

        ctx.spawn_exit(async move {
            already_running.store(true, Ordering::Relaxed);
            let listener = match Self::bind(this.clone(), ctx2.clone(), port, host, path).await {
                Ok(listener) => listener,
                Err(e) => {
                    already_running.store(false, Ordering::Relaxed);
                    Err::<(), _>(e).emit_error("listen", &ctx2, this.clone())?;
                    return Ok(()); // Don't stop the VM if failed to bind
                },
            };

            Self::emit_str(this.clone(), &ctx2, "listening", vec![], false)?;

            let notify = Arc::new(Notify::new());
            let close_notify = notify.notified();

            loop {
                let ctx3 = ctx2.clone();
                let this2 = this.clone();

                if !lexe_utils::event_loop::keeps_event_loop_alive(&listen_loop_ref) {
                    break;
                }

                select! {
                    _ = lexe_utils::event_loop::wait_until_io_unref(listen_loop_ref.clone()) => {
                        break;
                    }
                    socket = listener.accept(&ctx3) => {
                        Self::handle_socket_connection(
                            this2.clone(),
                            ctx3.clone(),
                            socket,
                            notify.clone(),
                            allow_half_open,
                        ).emit_error("handle_socket_connection",&ctx3, this2)?;
                    },
                    _ = close_rx.recv() => {
                        break;
                    }
                }
            }

            if !this.borrow().sockets.is_empty() {
                trace!("Waiting for sockets to finish");
                close_notify.await;
                trace!("Sockets finished");
            } else {
                trace!("No sockets to wait for, closing");
            }

            already_running.store(false, Ordering::Relaxed);
            should_close.store(false, Ordering::Relaxed);

            // Node resets `server.address()` to null once the listener is gone,
            // so code that probes it after `close` gets the documented value
            // instead of the last bound address.
            this.borrow_mut().address = Null.into_value(ctx2.clone());

            // The listening handle is gone: release its event-loop reference.
            this.borrow().loop_ref.unreference();

            Self::emit_str(this.0, &ctx2, "close", vec![], false)?;

            Ok(())
        })?;

        Ok(())
    }

    fn close(this: This<Class<'js, Self>>, ctx: Ctx<'js>, cb: Opt<Function<'js>>) -> Result<()> {
        trace!("Closing server");
        if let Some(cb) = cb.0 {
            Self::add_event_listener_str(this.clone(), &ctx, "close", cb, true, true)?;
        }
        let borrow = this.borrow_mut();
        borrow.should_close.store(true, Ordering::Relaxed);
        let _ = borrow.close_tx.send(());
        Ok(())
    }
}

impl<'js> Server<'js> {
    async fn bind(
        this: Class<'js, Self>,
        ctx: Ctx<'js>,
        port: Option<i32>,
        host: Option<String>,
        path: Option<String>,
    ) -> Result<Listener> {
        let listener = if let Some(port) = port {
            let listener = TcpListener::bind(get_hostname(
                &host.unwrap_or_else(|| String::from("0.0.0.0")),
                port as u16,
            ))
            .await
            .or_throw(&ctx)?;

            let address_object = Object::new(ctx.clone())?;

            let (address, port, family) = get_address_parts(&ctx, listener.local_addr())?;
            address_object.set("address", address)?;
            address_object.set("port", port)?;
            address_object.set("family", family)?;

            this.borrow_mut().address = address_object.into_value();

            Listener::Tcp(listener)
        } else if let Some(path) = path {
            #[cfg(unix)]
            {
                let listener: UnixListener = UnixListener::bind(&path).or_throw(&ctx)?;
                this.borrow_mut().address = path.into_js(&ctx)?;
                Listener::Unix(listener)
            }
            #[cfg(not(unix))]
            {
                _ = path;
                return Err(Exception::throw_type(
                    &ctx,
                    "Unix domain sockets are not supported on this platform",
                ));
            }
        } else {
            panic!("unreachable")
        };
        Ok(listener)
    }

    fn handle_socket_connection(
        this: Class<'js, Self>,
        ctx: Ctx<'js>,
        stream_result: Result<NetStream>,
        notify_close: Arc<Notify>,
        allow_half_open: bool,
    ) -> Result<()> {
        let net_stream = stream_result.or_throw(&ctx)?;

        ctx.clone().spawn_exit(async move {
            // The listener must receive the JS `net.Socket` (a Duplex), and the
            // native read/write loops must run on *that same object's* handle.
            // Building a bare handle here would give the two sides different
            // objects and the pushed bytes would never reach the stream.
            let (js_socket, socket_instance) = build_accepted_socket(&ctx, allow_half_open)?;
            let socket_index;
            {
                let mut sever_borrow = this.borrow_mut();
                socket_index = sever_borrow.sockets.append(socket_instance.clone());
            }

            Self::emit_str(this.clone(), &ctx, "connection", vec![js_socket], false)?;

            let had_error = net_stream
                .process(&socket_instance, &ctx, allow_half_open)
                .await?;

            Socket::emit_close(socket_instance, &ctx, had_error)?;
            {
                let mut sever_borrow = this.borrow_mut();
                sever_borrow.sockets.remove(socket_index);

                if sever_borrow.sockets.is_empty()
                    && sever_borrow.should_close.load(Ordering::Relaxed)
                {
                    trace!("Sockets empty, notify close");
                    notify_close.notify_one();
                }
            }

            Ok(())
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lexe_buffer as buffer;
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    use crate::NetModule;

    async fn call_tcp(port: u16) {
        // Connect to server
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        stream.set_nodelay(true).unwrap();

        // Write
        let msg = b"Hello, world!";
        stream.write_all(msg).await.unwrap();
        stream.flush().await.unwrap();

        // Read
        let mut buf = vec![0; 1024];
        let n = stream.read(&mut buf).await.unwrap();

        assert_eq!(&buf[..n], msg);
    }

    #[tokio::test]
    async fn test_server_echo() {
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
                        import { createServer } from 'net';

                        export async function test() {
                            const server = createServer(socket => {
                                socket.on('data', data => {
                                    socket.write(data, () => server.close());
                                });
                            });

                            // Use port 0 to let the OS assign an available port
                            server.listen(0, '127.0.0.1');

                            return new Promise((resolve, reject) => {
                                server.on('listening', () => {
                                    const port = server.address().port;
                                    resolve(port);
                                });
                                server.on('error', (err) => reject(err));
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                // Get the OS-assigned port from the server
                let port: u16 = call_test(&ctx, &module, ()).await;

                // Now connect to the server and run the echo test
                call_tcp(port).await;
            })
        })
        .await;
    }
}

/// Build an accepted connection as the JS `net.Socket` class.
///
/// Returns the user-facing object plus the native handle taken out of it, so
/// both `process_stream` and the listener operate on one socket. Falls back to a
/// bare handle when the JS class is not registered yet (module still loading).
fn build_accepted_socket<'js>(
    ctx: &Ctx<'js>,
    allow_half_open: bool,
) -> Result<(Value<'js>, Class<'js, Socket<'js>>)> {
    let ctor: Value = ctx.globals().get("__lexeNetSocketClass")?;
    let ctor = ctor.as_constructor().cloned();
    if let Some(ctor) = ctor {
        let opts = Object::new(ctx.clone())?;
        opts.set("allowHalfOpen", allow_half_open)?;
        let instance: Value = ctor.construct((opts,))?;
        let handle = instance
            .as_object()
            .and_then(|obj| obj.get::<_, Value>("_handle").ok())
            .and_then(|v| v.as_object().cloned())
            .and_then(|obj| Class::<Socket>::from_object(&obj));
        if let Some(handle) = handle {
            return Ok((instance, handle));
        }
    }
    let handle = Socket::new(ctx.clone(), allow_half_open)?;
    Ok((handle.clone().into_value(), handle))
}
