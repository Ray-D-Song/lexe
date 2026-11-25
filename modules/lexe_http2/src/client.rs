// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{
    client::conn::http2,
    header::{HeaderName, HeaderValue},
    Request, Uri,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use lexe_context::CtxExtension;
use lexe_events::{Emitter, EventEmitter, EventList};
use lexe_tls::{
    alpn::convert_alpn_protocols,
    backend::{connect as tls_connect, ConnectOptions},
    options::parse_connect_options,
    secure_context::SecureContext,
};
use lexe_utils::{
    event_loop::EventLoopRef,
    perf::{self, PerfEntryKind},
    time,
};
use rquickjs::{
    class::Trace,
    prelude::{Rest, This},
    Array, Class, Ctx, Exception, Function, IntoJs, Object, Result, Undefined, Value,
};
use tokio::net::TcpStream;

use crate::stream::Http2Stream;

type H2Sender = hyper::client::conn::http2::SendRequest<Empty<Bytes>>;

#[rquickjs::class]
pub struct ClientHttp2Session<'js> {
    emitter: EventEmitter<'js>,
    request_tx: Option<Arc<tokio::sync::Mutex<H2Sender>>>,
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

    fn new_pending(ctx: Ctx<'js>) -> Result<Class<'js, Self>> {
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        Class::instance(
            ctx,
            Self {
                emitter: EventEmitter::new(),
                request_tx: None,
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
    pub fn request(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        headers: Object<'js>,
    ) -> Result<Class<'js, Http2Stream<'js>>> {
        let tx = this
            .borrow()
            .request_tx
            .clone()
            .ok_or_else(|| Exception::throw_message(&ctx, "session not connected"))?;
        let mut method = "GET".to_string();
        let mut path = "/".to_string();
        let mut scheme = "http".to_string();
        let mut authority = "localhost".to_string();
        let mut extra_headers = Vec::new();
        for entry in headers.props::<String, Value>() {
            let (key, val) = entry?;
            let val = value_to_string(&ctx, val)?;
            match key.as_str() {
                ":method" => method = val,
                ":path" => path = val,
                ":scheme" => scheme = val,
                ":authority" => authority = val,
                _ => extra_headers.push((key, val)),
            }
        }
        let req_stream = Http2Stream::new_request(
            &ctx,
            method.clone(),
            path.clone(),
            scheme.clone(),
            authority.clone(),
            hyper::HeaderMap::new(),
        )?;
        let ctx2 = ctx.clone();
        let stream = req_stream.clone();
        ctx.spawn_exit_simple(async move {
            let perf_start_ms =
                perf::has_observers(perf::rt_key_for_ctx(&ctx2), PerfEntryKind::Http2)
                    .then(time::monotonic_elapsed_ms);
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
            let req = builder
                .body(Empty::<Bytes>::new())
                .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
            let mut sender = tx.lock().await;
            let resp = sender
                .send_request(req)
                .await
                .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
            let (parts, body) = resp.into_parts();
            stream.borrow_mut().status_code = parts.status.as_u16();
            for (name, value) in parts.headers.iter() {
                let _ = stream
                    .borrow_mut()
                    .headers
                    .set(name.as_str(), value.to_str().unwrap_or("").to_string());
            }
            let header_obj = Object::new(ctx2.clone())?;
            for (name, value) in parts.headers.iter() {
                let _ = header_obj.set(name.as_str(), value.to_str().unwrap_or("").to_string());
            }
            let _ = Http2Stream::emit_str(
                stream.clone(),
                &ctx2,
                "response",
                vec![header_obj.into_value()],
                false,
            );
            let body_bytes = http_body_util::BodyExt::collect(body)
                .await
                .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?
                .to_bytes();
            let bytes_read = body_bytes.len() as u64;
            if !body_bytes.is_empty() {
                let chunk = lexe_buffer::Buffer(body_bytes.to_vec())
                    .into_js(&ctx2)
                    .unwrap_or_else(|_| Undefined.into_value(ctx2.clone()));
                let _ = Http2Stream::emit_str(stream.clone(), &ctx2, "data", vec![chunk], false);
            }
            let _ = Http2Stream::emit_str(stream, &ctx2, "end", vec![], false);
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
                        res_obj.set("statusCode", parts.status.as_u16())?;
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

    pub fn close(this: This<Class<'js, Self>>) -> Result<()> {
        this.borrow_mut().request_tx.take();
        Ok(())
    }
}

fn parse_authority<'js>(ctx: &Ctx<'js>, authority: Value<'js>) -> Result<(String, u16, bool)> {
    let s: String = if let Some(str) = authority.as_string() {
        str.to_string()?
    } else {
        return Err(Exception::throw_type(ctx, "authority must be a string"));
    };
    let trimmed = s.trim();
    let (secure, rest) = if let Some(rest) = trimmed.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        (false, rest)
    } else {
        (false, trimmed)
    };
    let (host, port) = if let Some((h, p)) = rest.rsplit_once(':') {
        if rest.starts_with('[') {
            (rest.to_string(), if secure { 443 } else { 80 })
        } else {
            (
                h.to_string(),
                p.parse::<u16>()
                    .map_err(|_| Exception::throw_type(ctx, "invalid port"))?,
            )
        }
    } else {
        (rest.to_string(), if secure { 443 } else { 80 })
    };
    Ok((host, port, secure))
}

async fn connect_io<IO>(
    io: TokioIo<IO>,
) -> std::result::Result<H2Sender, Box<dyn std::error::Error + Send + Sync>>
where
    IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (sender, conn) = http2::handshake(TokioExecutor::new(), io).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(sender)
}

pub fn connect<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, ClientHttp2Session<'js>>> {
    let mut authority = None;
    let mut options = None;
    let mut listener = None;
    for value in args.0 {
        if authority.is_none() && value.is_string() {
            authority = Some(value);
        } else if value.is_object() && !value.is_null() {
            options = Some(value.as_object().unwrap().clone());
        } else if value.is_function() {
            listener = value.into_function();
        }
    }
    let authority = authority.ok_or_else(|| Exception::throw_type(&ctx, "authority required"))?;
    let (host, port, secure) = parse_authority(&ctx, authority)?;
    let session = ClientHttp2Session::new_pending(ctx.clone())?;
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
    let tls_opts = if secure {
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
    if secure {
        if let Some(ref mut opts) = tls_opts {
            if opts.alpn_protocols.is_none() {
                let arr = Array::new(ctx.clone())?;
                arr.set(0, "h2")?;
                opts.alpn_protocols = Some(convert_alpn_protocols(&ctx, arr.into_value())?);
            }
        }
    }
    let ctx2 = ctx.clone();
    ctx.spawn_exit_simple(async move {
        let addr = format!("{host}:{port}");
        let tcp = TcpStream::connect(&addr)
            .await
            .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
        let sender = if secure {
            let mut tls_opts = tls_opts.unwrap();
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
            };
            let tls = tls_connect(tcp, connect_opts)
                .await
                .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
            connect_io(TokioIo::new(tls))
                .await
                .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?
        } else {
            connect_io(TokioIo::new(tcp))
                .await
                .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?
        };
        session
            .borrow_mut()
            .request_tx
            .replace(Arc::new(tokio::sync::Mutex::new(sender)));
        ClientHttp2Session::emit_str(session.clone(), &ctx2, "connect", vec![], false)?;
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
