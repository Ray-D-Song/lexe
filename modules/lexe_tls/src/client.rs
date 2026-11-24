// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::sync::{Arc, Mutex};

use lexe_context::CtxExtension;
use lexe_net::{check_network_access, get_hostname, PrefixedTcpStream, Socket};
use lexe_stream::SteamEvents;
use lexe_utils::{error::ErrorExtensions, result::ResultExt};
use rquickjs::{
    prelude::{Opt, Rest, This},
    Class, Ctx, Error, Exception, Function, Result, Value,
};
use tokio::net::TcpStream;

use crate::backend::{connect as tls_connect, ConnectOptions, VerifyRecord};
use crate::connect_args::normalize_connect_args;
use crate::error::throw_invalid_arg_value;
use crate::options::parse_connect_options;
use crate::secure_context::SecureContext;
use crate::tls_socket::{rw_join, TlsSocket};

const LOCALHOST: &str = "localhost";

fn handle_connect_error<'js>(
    ctx: &Ctx<'js>,
    tls_socket: Class<'js, TlsSocket<'js>>,
    err: Error,
) -> Result<()> {
    let err_value = err.into_value(ctx)?;
    TlsSocket::fail_connect(tls_socket, ctx, err_value)
}

/// Native backend behind a JS socket object.
///
/// `net.Socket` is a JS Duplex whose `_handle` is the Rust `Socket`; callers
/// hold the wrapper, so transport operations have to reach through it. A bare
/// native handle is still accepted for internal callers.
pub(crate) fn net_handle_of<'js>(value: &Value<'js>) -> Option<Class<'js, Socket<'js>>> {
    if let Ok(class) = Class::<Socket>::from_value(value) {
        return Some(class);
    }
    let obj = value.as_object()?;
    let handle: Value = obj.get("_handle").ok()?;
    Class::<Socket>::from_value(&handle).ok()
}

/// Wrap an already-created native TLS handle in the JS `tls.TLSSocket` object.
///
/// `tls.connect` builds the backend itself (it owns the handshake), so the JS
/// object is constructed around that handle rather than creating its own. The
/// object is still a real `net.Socket` subclass, which is what makes the three
/// `instanceof` checks hold for callers.
pub(crate) fn build_tls_socket_object<'js>(
    ctx: &Ctx<'js>,
    handle: &Class<'js, TlsSocket<'js>>,
    allow_half_open: bool,
) -> Result<Value<'js>> {
    let ctor: Value = ctx.globals().get("__lexeTlsSocketClass")?;
    let Some(ctor) = ctor.as_constructor().cloned() else {
        return Ok(handle.clone().into_value());
    };
    // The glue's constructor prefers `_handleFactory`, so hand it the handle we
    // already built instead of letting it create a second one.
    let opts = rquickjs::Object::new(ctx.clone())?;
    opts.set("allowHalfOpen", allow_half_open)?;
    let handle_value = handle.clone().into_value();
    let factory = Function::new(ctx.clone(), move |_o: Opt<Value<'js>>| handle_value.clone())?;
    opts.set("_handleFactory", factory)?;
    let instance: Value = ctor.construct((rquickjs::Undefined, opts))?;
    Ok(instance)
}

pub fn connect<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Value<'js>> {
    let (options_value, callback) = normalize_connect_args(&ctx, args.0)?;
    let options = parse_connect_options(&ctx, options_value.clone())?;

    let check_server_identity_cb = if let Some(obj) = options_value.as_object() {
        obj.get::<_, Option<Value>>("checkServerIdentity")?
            .and_then(|v| v.into_function())
    } else {
        None
    };

    let socket_opt = if let Some(obj) = options_value.as_object() {
        obj.get::<_, Option<Value>>("socket")?
    } else {
        None
    };

    let connect_host = options
        .host
        .clone()
        .unwrap_or_else(|| String::from(LOCALHOST));
    let identity_host = options
        .servername
        .clone()
        .unwrap_or_else(|| connect_host.clone());

    if let Some(ref servername) = options.servername {
        if servername.parse::<std::net::IpAddr>().is_ok() {
            return Err(throw_invalid_arg_value(
                &ctx,
                "servername",
                "must not be an IP address",
            ));
        }
    }

    let sni_name = if let Some(ref servername) = options.servername {
        Some(servername.clone())
    } else if connect_host.parse::<std::net::IpAddr>().is_ok() {
        None
    } else {
        Some(connect_host.clone())
    };

    let context = if let Some(ctx) = options.secure_context.clone() {
        ctx
    } else {
        SecureContext::from_options(&options).map_err(|e| Exception::throw_message(&ctx, &e))?
    };

    let reject_unauthorized = options.reject_unauthorized;
    let timeout = options.timeout;
    let port = options.port;
    // User-visible half-open policy. The native handle always stays half-open
    // for TLS; this option only drives the JS layer.
    let allow_half_open = options_value
        .as_object()
        .and_then(|obj| obj.get::<_, Option<bool>>("allowHalfOpen").ok().flatten())
        .unwrap_or(false);

    let verify_record = if !reject_unauthorized {
        Some(Arc::new(Mutex::new(VerifyRecord {
            ok: true,
            error: None,
        })))
    } else {
        None
    };

    // The native side is always half-open for TLS (one direction ends at a
    // time); the JS layer decides user-visible behaviour from its own option.
    let tls_socket = TlsSocket::new(ctx.clone(), true)?;
    let tls_js = build_tls_socket_object(&ctx, &tls_socket, allow_half_open)?;
    {
        let mut borrow = tls_socket.borrow_mut();
        borrow.servername = sni_name.clone();
        borrow.check_server_identity_cb = check_server_identity_cb;
        borrow.secure_connecting = true;
        borrow.pending = true;
    }

    if let Some(cb) = callback {
        // Register on the JS TLSSocket, not the native handle: events dispatch
        // through the owner, so a listener in the handle's own table never
        // fires (same rule as `net.Socket.connect`'s callback).
        if let Some(obj) = tls_js.as_object() {
            if let Some(once) = obj
                .get::<_, Value>("once")
                .ok()
                .and_then(|v| v.as_function().cloned())
            {
                once.call::<_, ()>((This(obj.clone()), "secureConnect", cb))?;
            }
        }
    }

    let tls_socket2 = tls_socket.clone();
    let tls_for_error = tls_socket.clone();
    let ctx2 = ctx.clone();
    let identity_host_for_handshake = identity_host.clone();

    ctx.spawn_exit(async move {
        let connect_result = async {
            if let Some(socket_val) = socket_opt {
                // The caller passes the JS `net.Socket` it holds. Its native
                // backend is `_handle`; taking the transport means operating on
                // that handle, not on the wrapper.
                let net_socket = net_handle_of(&socket_val)
                    .ok_or_else(|| Exception::throw_type(&ctx2, "socket must be a net.Socket"))?;

                let (read_half, write_half, prefix) =
                    Socket::begin_tls_handoff(&ctx2, net_socket.clone()).await?;

                let addresses = net_socket.borrow();
                let mut tls_borrow = tls_socket2.borrow_mut();
                tls_borrow.remote_address = addresses.remote_address().clone();
                tls_borrow.remote_port = addresses.remote_port();
                tls_borrow.remote_family = addresses.remote_family().clone();
                tls_borrow.local_address = addresses.local_address().clone();
                tls_borrow.local_port = addresses.local_port();
                tls_borrow.local_family = addresses.local_family().clone();
                drop(tls_borrow);
                drop(addresses);

                TlsSocket::mark_tcp_connected(&tls_socket2, &ctx2)?;

                let io = PrefixedTcpStream::new(prefix, read_half, write_half);
                let connect_opts = ConnectOptions {
                    context,
                    identity_name: identity_host_for_handshake.clone(),
                    sni_name: sni_name.clone(),
                    reject_unauthorized,
                    timeout,
                    verify_record: verify_record.clone(),
                };
                let tls_stream = tls_connect(io, connect_opts)
                    .await
                    .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;

                let tls_stream = match TlsSocket::finish_client_handshake(
                    tls_socket2.clone(),
                    ctx2.clone(),
                    tls_stream,
                    reject_unauthorized,
                    &identity_host_for_handshake,
                    verify_record.clone(),
                )
                .await
                {
                    Ok(stream) => stream,
                    Err(err_value) => {
                        TlsSocket::fail_connect(tls_socket2.clone(), &ctx2, err_value)?;
                        return Ok(());
                    },
                };

                let (reader, writer) = tokio::io::split(tls_stream);
                let (readable_done, writable_done) = TlsSocket::process_split_io(
                    &tls_socket2,
                    &ctx2,
                    reader,
                    writer,
                    allow_half_open,
                )?;

                let had_error = rw_join(&ctx2, readable_done, writable_done).await?;
                TlsSocket::emit_close(tls_socket2, &ctx2, had_error)?;
                Ok(())
            } else if let Some(port) = port {
                check_network_access(&ctx2, &connect_host, port)?;
                let addr = get_hostname(&connect_host, port);
                let tcp = TcpStream::connect(addr).await.or_throw(&ctx2)?;
                TlsSocket::set_addresses_from_tcp(&tls_socket2, &ctx2, &tcp)?;
                TlsSocket::mark_tcp_connected(&tls_socket2, &ctx2)?;

                let connect_opts = ConnectOptions {
                    context,
                    identity_name: identity_host_for_handshake.clone(),
                    sni_name: sni_name.clone(),
                    reject_unauthorized,
                    timeout,
                    verify_record: verify_record.clone(),
                };
                let tls_stream = tls_connect(tcp, connect_opts)
                    .await
                    .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;

                let tls_stream = match TlsSocket::finish_client_handshake(
                    tls_socket2.clone(),
                    ctx2.clone(),
                    tls_stream,
                    reject_unauthorized,
                    &identity_host_for_handshake,
                    verify_record,
                )
                .await
                {
                    Ok(stream) => stream,
                    Err(err_value) => {
                        TlsSocket::fail_connect(tls_socket2.clone(), &ctx2, err_value)?;
                        return Ok(());
                    },
                };

                let (reader, writer) = tokio::io::split(tls_stream);
                let (readable_done, writable_done) = TlsSocket::process_split_io(
                    &tls_socket2,
                    &ctx2,
                    reader,
                    writer,
                    allow_half_open,
                )?;

                let had_error = rw_join(&ctx2, readable_done, writable_done).await?;
                TlsSocket::emit_close(tls_socket2, &ctx2, had_error)?;
                Ok(())
            } else {
                Err(Exception::throw_type(&ctx2, "port or socket is required"))
            }
        }
        .await;

        if let Err(err) = connect_result {
            handle_connect_error(&ctx2, tls_for_error, err)?;
        }
        Ok(())
    })?;

    Ok(tls_js)
}
