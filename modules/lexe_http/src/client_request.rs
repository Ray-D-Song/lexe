// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    cell::{Cell, RefCell},
    convert::Infallible,
    sync::Arc,
    time::Duration,
};

use base64::Engine as _;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::{
    body::Frame,
    header::{HeaderName, HeaderValue},
    Method, Request, Uri,
};
use lexe_context::CtxExtension;
use lexe_events::{Emitter, EventEmitter, EventList};
use lexe_net::Socket;
use lexe_timers::resume_timer_loop_if_idle;
use lexe_utils::{
    event_loop::{self, EventLoopRef},
    object::ObjectExt,
    perf::{self, PerfEntryKind},
    result::ResultExt,
    time, VERSION,
};
use rquickjs::{
    class::{Trace, Tracer},
    function::{Opt, Rest, This},
    Class, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Result, Undefined, Value,
};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

use crate::{
    headers::{
        header_value_to_list_with_kind, parse_write_or_end_args, throw_headers_sent,
        validate_header_name_str, HeaderStore,
    },
    server::{build_incoming_headers_js, IncomingMessage},
    Agent, HyperClient,
};

struct ParsedRequest<'js> {
    scheme: String,
    authority: String,
    host: String,
    path: String,
    protocol: String,
    method: Method,
    headers: HeaderStore,
    callback: Option<Function<'js>>,
    client: HyperClient,
    socket_path: Option<String>,
    timeout_ms: Option<f64>,
    max_headers_count: Value<'js>,
    signal: Option<Value<'js>>,
}

fn hyper_client(ctx: &Ctx<'_>) -> Result<HyperClient> {
    match crate::HTTP_CLIENT.as_ref() {
        Ok(client) => Ok(client.clone()),
        Err(err) => Err(Exception::throw_message(ctx, &err.to_string())),
    }
}

pub fn http_request<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, ClientRequest<'js>>> {
    let parsed = parse_request_args(&ctx, &args, None, "http:")?;
    ClientRequest::start(&ctx, parsed, false)
}

pub fn http_get<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, ClientRequest<'js>>> {
    let parsed = parse_request_args(&ctx, &args, Some("GET"), "http:")?;
    ClientRequest::start(&ctx, parsed, true)
}

/// `https.get` / `https.request`.
///
/// Node's `https` client defaults to TLS, so an options object without an
/// explicit `protocol` must still produce an `https:` URI. The plain `http`
/// entry points keep their `http:` default.
pub fn https_request<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, ClientRequest<'js>>> {
    let parsed = parse_request_args(&ctx, &args, None, "https:")?;
    ClientRequest::start(&ctx, parsed, false)
}

pub fn https_get<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, ClientRequest<'js>>> {
    let parsed = parse_request_args(&ctx, &args, Some("GET"), "https:")?;
    ClientRequest::start(&ctx, parsed, true)
}

fn is_url_object(obj: &Object<'_>) -> bool {
    obj.get_optional::<_, String>("href")
        .ok()
        .flatten()
        .is_some()
        && obj
            .get_optional::<_, String>("protocol")
            .ok()
            .flatten()
            .is_some()
        && obj
            .get_optional::<_, String>("hostname")
            .ok()
            .flatten()
            .is_some()
}

fn parse_port_value(val: &Value<'_>) -> Option<u16> {
    if let Some(n) = val.as_number() {
        if n.is_finite() && (0.0..=65535.0).contains(&n) {
            return Some(n as u16);
        }
    }
    if let Some(s) = val.as_string() {
        if let Ok(text) = s.to_string() {
            if let Ok(p) = text.trim().parse::<u16>() {
                return Some(p);
            }
        }
    }
    None
}

fn split_host_and_port(host: &str) -> (String, Option<u16>) {
    if let Some(rest) = host.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let addr = &rest[..end];
            let after = &rest[end + 1..];
            let port = after.strip_prefix(':').and_then(|p| p.parse::<u16>().ok());
            return (addr.to_string(), port);
        }
    }
    if let Some((h, p)) = host.rsplit_once(':') {
        if !h.contains(':') {
            if let Ok(port) = p.parse::<u16>() {
                return (h.to_string(), Some(port));
            }
        }
    }
    (host.to_string(), None)
}

fn format_authority_host(hostname: &str) -> String {
    if hostname.contains(':') && !hostname.starts_with('[') {
        format!("[{hostname}]")
    } else {
        hostname.to_string()
    }
}

fn parse_request_args<'js>(
    ctx: &Ctx<'js>,
    args: &Rest<Value<'js>>,
    force_method: Option<&str>,
    default_protocol: &str,
) -> Result<ParsedRequest<'js>> {
    let mut url_str: Option<String> = None;
    let mut url_obj: Option<Object<'js>> = None;
    let mut options: Option<Object<'js>> = None;
    let mut callback: Option<Function<'js>> = None;

    for value in &args.0 {
        if value.is_function() {
            callback = Some(
                value
                    .as_function()
                    .ok_or_else(|| Exception::throw_type(ctx, "Callback must be a function"))?
                    .clone(),
            );
        } else if value.is_string() && url_str.is_none() && url_obj.is_none() {
            url_str = Some(value.as_string().unwrap().to_string()?);
        } else if let Some(obj) = value.as_object() {
            if url_obj.is_none() && url_str.is_none() && is_url_object(obj) {
                url_obj = Some(obj.clone());
            } else {
                options = Some(obj.clone());
            }
        }
    }

    let mut parsed_url: Option<Uri> = None;
    let mut url_auth: Option<String> = None;
    if let Some(ref raw_url) = url_str {
        let uri = raw_url
            .parse::<Uri>()
            .map_err(|_| Exception::throw_type(ctx, "Invalid URL"))?;
        parsed_url = Some(uri);
    } else if let Some(ref uobj) = url_obj {
        let href: String = uobj.get("href")?;
        let uri = href
            .parse::<Uri>()
            .map_err(|_| Exception::throw_type(ctx, "Invalid URL"))?;
        parsed_url = Some(uri);
        let uname = uobj
            .get_optional::<_, String>("username")?
            .unwrap_or_default();
        let pass = uobj
            .get_optional::<_, String>("password")?
            .unwrap_or_default();
        if !uname.is_empty() || !pass.is_empty() {
            url_auth = Some(format!("{uname}:{pass}"));
        }
    }

    let mut method = force_method
        .map(str::to_string)
        .or_else(|| {
            options
                .as_ref()
                .and_then(|o| o.get_optional::<_, String>("method").ok().flatten())
        })
        .unwrap_or_else(|| "GET".into());
    method.make_ascii_uppercase();
    let method = Method::from_bytes(method.as_bytes())
        .map_err(|_| Exception::throw_type(ctx, "Invalid HTTP method"))?;

    let protocol = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, String>("protocol").ok().flatten())
        .or_else(|| {
            parsed_url
                .as_ref()
                .and_then(|u| u.scheme_str().map(|s| format!("{s}:")))
        })
        .unwrap_or_else(|| default_protocol.to_string());
    let scheme = protocol.trim_end_matches(':').to_string();
    let default_port: u16 = if scheme == "https" { 443 } else { 80 };

    let mut port: Option<u16> = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, Value>("port").ok().flatten())
        .and_then(|v| parse_port_value(&v));
    if port.is_none() {
        if let Some(u) = parsed_url.as_ref() {
            port = u.port_u16();
        }
    }

    let raw_host_opt = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, String>("hostname").ok().flatten())
        .or_else(|| {
            options
                .as_ref()
                .and_then(|o| o.get_optional::<_, String>("host").ok().flatten())
        })
        .or_else(|| {
            parsed_url
                .as_ref()
                .and_then(|u| u.host().map(str::to_string))
        });

    let (hostname, host_port) = match raw_host_opt {
        Some(h) if !h.is_empty() => split_host_and_port(&h),
        _ => ("localhost".to_string(), None),
    };
    let port = port.or(host_port).unwrap_or(default_port);

    let path = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, String>("path").ok().flatten())
        .or_else(|| {
            parsed_url
                .as_ref()
                .and_then(|u| u.path_and_query().map(|pq| pq.as_str().to_string()))
        })
        .unwrap_or_else(|| "/".to_string());
    let path = if path.is_empty() || !path.starts_with('/') {
        format!("/{path}")
    } else {
        path
    };

    let formatted_host = format_authority_host(&hostname);
    let authority = format!("{formatted_host}:{port}");
    let default_host_header = if port == default_port {
        formatted_host.clone()
    } else {
        format!("{formatted_host}:{port}")
    };

    let mut headers = HeaderStore::default();
    if let Some(opts) = &options {
        if let Some(hdrs) = opts.get_optional::<_, Object>("headers")? {
            for key in hdrs.keys::<String>() {
                let key = key?;
                validate_header_name_str(ctx, &key)?;
                let value: Value = hdrs.get(&key)?;
                let (values, is_array) = header_value_to_list_with_kind(ctx, &key, value)?;
                headers.set(&key, values, is_array);
            }
        }
    }
    if !headers.has("host") {
        headers.set("host", vec![default_host_header], false);
    }
    let auth = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, String>("auth").ok().flatten())
        .or(url_auth);
    if let Some(auth) = auth {
        if !auth.is_empty() && !headers.has("authorization") {
            let token = base64::engine::general_purpose::STANDARD.encode(auth.as_bytes());
            headers.set("authorization", vec![format!("Basic {token}")], false);
        }
    }

    let socket_path = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, String>("socketPath").ok().flatten());
    let timeout_ms = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, f64>("timeout").ok().flatten())
        .filter(|n| n.is_finite() && *n > 0.0);
    let max_headers_count = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, Value>("maxHeadersCount").ok().flatten())
        .filter(|v| !v.is_undefined())
        .unwrap_or_else(|| Value::new_null(ctx.clone()));
    let signal = options
        .as_ref()
        .and_then(|o| o.get_optional::<_, Value>("signal").ok().flatten())
        .filter(|v| !v.is_undefined() && !v.is_null());

    // TLS options are per request (`https.get({ ca })`, `rejectUnauthorized`),
    // or supplied via `options.agent`. Everything else shares the pooled client.
    let tls = parse_request_tls_options(ctx, options.as_ref())?;
    let client = if let Some(tls) = tls {
        crate::build_client_for_request(ctx, tls)?
    } else if let Some(agent_client) = extract_agent_client(options.as_ref()) {
        agent_client
    } else {
        hyper_client(ctx)?
    };

    Ok(ParsedRequest {
        scheme,
        authority,
        host: hostname,
        path,
        protocol,
        method,
        headers,
        callback,
        client,
        socket_path,
        timeout_ms,
        max_headers_count,
        signal,
    })
}

fn extract_agent_client(options: Option<&Object<'_>>) -> Option<HyperClient> {
    let opts = options?;
    let agent_val: Value = opts.get_optional("agent").ok().flatten()?;
    let obj = agent_val.as_object()?;
    Class::<Agent>::from_object(obj).map(|agent| agent.borrow().client())
}

pub struct RequestTlsOptions {
    pub reject_unauthorized: Option<bool>,
    pub ca: Option<Vec<Vec<u8>>>,
}

fn parse_request_tls_options<'js>(
    ctx: &Ctx<'js>,
    options: Option<&Object<'js>>,
) -> Result<Option<RequestTlsOptions>> {
    let Some(options) = options else {
        return Ok(None);
    };
    let reject_unauthorized: Option<bool> = options.get_optional("rejectUnauthorized")?;
    let ca = match options.get_optional::<_, Value>("ca")? {
        Some(value) if !value.is_undefined() && !value.is_null() => {
            Some(pem_list_from_value(ctx, value)?)
        },
        _ => None,
    };
    if reject_unauthorized.is_none() && ca.is_none() {
        return Ok(None);
    }
    Ok(Some(RequestTlsOptions {
        reject_unauthorized,
        ca,
    }))
}

/// Accept the forms Node accepts for `ca`: a string, a Buffer, or an array of
/// either.
fn pem_list_from_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Vec<Vec<u8>>> {
    if value.is_array() {
        let array = value.as_array().expect("array");
        let mut out = Vec::with_capacity(array.len());
        for item in array.iter::<Value>() {
            out.push(pem_bytes_from_value(ctx, item?)?);
        }
        return Ok(out);
    }
    Ok(vec![pem_bytes_from_value(ctx, value)?])
}

fn pem_bytes_from_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Vec<u8>> {
    if let Some(string) = value.as_string() {
        return Ok(string.to_string()?.into_bytes());
    }
    lexe_utils::bytes::ObjectBytes::from(ctx, &value)?.into_bytes(ctx)
}

#[rquickjs::class]
pub struct ClientRequest<'js> {
    emitter: EventEmitter<'js>,
    socket: RefCell<Value<'js>>,
    max_headers_count: RefCell<Value<'js>>,
    client: HyperClient,
    socket_path: Option<String>,
    scheme: String,
    authority: String,
    host: String,
    protocol: String,
    path: RefCell<String>,
    method: RefCell<Method>,
    headers: RefCell<HeaderStore>,
    body: RefCell<Vec<u8>>,
    body_stream_tx: RefCell<Option<mpsc::UnboundedSender<Bytes>>>,
    dispatched: Cell<bool>,
    headers_sent: Cell<bool>,
    ended: Cell<bool>,
    writable_finished: Cell<bool>,
    /// Node exposes these as plain booleans on the request. `aborted` is only
    /// set by `abort()`; `destroy()` leaves it `false`.
    aborted: Cell<bool>,
    destroyed: Cell<bool>,
    /// Set once the response finished normally, so a late `destroy()` is a
    /// no-op like Node's.
    completed: Cell<bool>,
    reused_socket: Cell<bool>,
    pending_no_delay: Cell<Option<bool>>,
    pending_keep_alive: Cell<Option<(bool, u64)>>,
    timeout_cancel: RefCell<Option<Arc<(std::sync::atomic::AtomicBool, tokio::sync::Notify)>>>,
    /// Fires when `destroy()`/`abort()` runs, so an in-flight hyper request is
    /// actually dropped instead of holding the loop reference until it settles.
    cancel: Arc<tokio::sync::Notify>,
    loop_ref: Arc<EventLoopRef>,
}

unsafe impl<'js> JsLifetime<'js> for ClientRequest<'js> {
    type Changed<'to> = ClientRequest<'to>;
}

impl<'js> Trace<'js> for ClientRequest<'js> {
    fn trace<'a>(&self, t: Tracer<'a, 'js>) {
        self.emitter.trace(t);
        self.socket.borrow().trace(t);
        self.max_headers_count.borrow().trace(t);
    }
}

impl<'js> Emitter<'js> for ClientRequest<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

impl<'js> ClientRequest<'js> {
    fn start(
        ctx: &Ctx<'js>,
        parsed: ParsedRequest<'js>,
        auto_end: bool,
    ) -> Result<Class<'js, Self>> {
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);

        let socket = Socket::new(ctx.clone(), false)?;
        Socket::js_unref(This(socket.clone()));
        let socket_val = socket.into_value();

        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                socket: RefCell::new(socket_val),
                max_headers_count: RefCell::new(parsed.max_headers_count),
                client: parsed.client,
                socket_path: parsed.socket_path,
                scheme: parsed.scheme,
                authority: parsed.authority,
                host: parsed.host,
                protocol: parsed.protocol,
                path: RefCell::new(parsed.path),
                method: RefCell::new(parsed.method),
                headers: RefCell::new(parsed.headers),
                body: RefCell::new(Vec::new()),
                body_stream_tx: RefCell::new(None),
                dispatched: Cell::new(false),
                headers_sent: Cell::new(false),
                ended: Cell::new(false),
                writable_finished: Cell::new(false),
                aborted: Cell::new(false),
                destroyed: Cell::new(false),
                completed: Cell::new(false),
                reused_socket: Cell::new(false),
                pending_no_delay: Cell::new(None),
                pending_keep_alive: Cell::new(None),
                timeout_cancel: RefCell::new(None),
                cancel: Arc::new(tokio::sync::Notify::new()),
                loop_ref,
            },
        )?;

        if let Some(cb) = parsed.callback {
            Self::add_event_listener_str(instance.clone(), ctx, "response", cb, false, false)?;
        }

        if let Some(ms) = parsed.timeout_ms {
            Self::arm_timeout(&instance, ctx, ms);
        }

        if let Some(signal) = parsed.signal {
            Self::wire_abort_signal(&instance, ctx, signal)?;
        }

        let _ = lexe_utils::diagnostics::publish(ctx, "http.client.request.created", |ctx| {
            let msg = Object::new(ctx.clone())?;
            msg.set("request", instance.clone().into_value())?;
            Ok(msg)
        });

        // Emit `'socket'` asynchronously (so sync `.on("socket", ...)` hooks catch it).
        let sock_req = instance.clone();
        let sock_ctx = ctx.clone();
        ctx.spawn_exit_simple(async move {
            tokio::task::yield_now().await;
            Self::emit_assigned_socket(&sock_req, &sock_ctx)?;
            Ok(())
        });

        if auto_end {
            ClientRequest::end(This(instance.clone()), ctx.clone(), Rest(Vec::new()))?;
        }

        Ok(instance)
    }

    fn emit_assigned_socket(this: &Class<'js, Self>, ctx: &Ctx<'js>) -> Result<()> {
        let (destroyed, socket_val, no_delay, keep_alive) = {
            let borrow = this.borrow();
            let res = (
                borrow.destroyed.get(),
                borrow.socket.borrow().clone(),
                borrow.pending_no_delay.take(),
                borrow.pending_keep_alive.take(),
            );
            res
        };
        if destroyed {
            return Ok(());
        }
        if let Some(obj) = socket_val.as_object() {
            if let Some(sock_cls) = Class::<Socket>::from_object(obj) {
                if let Some(nd) = no_delay {
                    let _ =
                        Socket::set_no_delay(This(sock_cls.clone()), ctx.clone(), Opt(Some(nd)));
                }
                if let Some((enable, delay)) = keep_alive {
                    let _ = Socket::set_keep_alive(
                        This(sock_cls),
                        ctx.clone(),
                        Opt(Some(Value::new_bool(ctx.clone(), enable))),
                        Opt(Some(delay.into_js(ctx)?)),
                    );
                }
            }
        }
        Self::emit_str(this.clone(), ctx, "socket", vec![socket_val], false)?;
        Ok(())
    }

    fn wire_abort_signal(
        this: &Class<'js, Self>,
        ctx: &Ctx<'js>,
        signal: Value<'js>,
    ) -> Result<()> {
        let Some(sig_obj) = signal.as_object() else {
            return Ok(());
        };
        let aborted = sig_obj.get_optional::<_, bool>("aborted")?.unwrap_or(false);
        let reason = sig_obj
            .get_optional::<_, Value>("reason")?
            .filter(|v| !v.is_undefined());
        if aborted {
            let err = reason.unwrap_or_else(|| {
                Exception::from_message(ctx.clone(), "The operation was aborted")
                    .map(|e| e.into_value())
                    .unwrap_or_else(|_| Undefined.into_value(ctx.clone()))
            });
            let req = this.clone();
            let ctx2 = ctx.clone();
            ctx.spawn_exit_simple(async move {
                teardown(req, &ctx2, Some(err))?;
                Ok(())
            });
            return Ok(());
        }
        if let Some(add_fn) = sig_obj.get_optional::<_, Function>("addEventListener")? {
            let req_for_cb = this.clone();
            let ctx_for_cb = ctx.clone();
            let sig_for_cb = signal.clone();
            let cb = rquickjs::Function::new(ctx.clone(), move || {
                let err = sig_for_cb
                    .as_object()
                    .and_then(|o| o.get_optional::<_, Value>("reason").ok().flatten())
                    .filter(|v| !v.is_undefined())
                    .or_else(|| {
                        Exception::from_message(ctx_for_cb.clone(), "The operation was aborted")
                            .ok()
                            .map(|e| e.into_value())
                    });
                let _ = teardown(req_for_cb.clone(), &ctx_for_cb, err);
            })?;
            let opts = Object::new(ctx.clone())?;
            opts.set("once", true)?;
            let _: Value = add_fn.call((This(sig_obj.clone()), "abort", cb, opts))?;
        }
        Ok(())
    }

    fn arm_timeout(this: &Class<'js, Self>, ctx: &Ctx<'js>, msecs: f64) {
        this.borrow().clear_timeout_timer();
        if !msecs.is_finite() || msecs <= 0.0 {
            return;
        }
        let borrow = this.borrow();
        if borrow.destroyed.get() || borrow.completed.get() {
            return;
        }
        let pair = Arc::new((
            std::sync::atomic::AtomicBool::new(false),
            tokio::sync::Notify::new(),
        ));
        *borrow.timeout_cancel.borrow_mut() = Some(pair.clone());
        drop(borrow);

        let req = this.clone();
        let ctx2 = ctx.clone();
        let dur = Duration::from_millis(msecs as u64);
        ctx.spawn_exit_simple(async move {
            if pair.0.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            tokio::select! {
                biased;
                _ = pair.1.notified() => Ok(()),
                _ = tokio::time::sleep(dur) => {
                    if pair.0.load(std::sync::atomic::Ordering::Acquire) {
                        return Ok(());
                    }
                    let active = {
                        let b = req.borrow();
                        !b.destroyed.get() && !b.completed.get()
                    };
                    if active {
                        let _ = Self::emit_str(req, &ctx2, "timeout", vec![], false);
                    }
                    Ok(())
                },
            }
        });
    }

    fn clear_timeout_timer(&self) {
        if let Some(prev) = self.timeout_cancel.borrow_mut().take() {
            prev.0.store(true, std::sync::atomic::Ordering::Release);
            prev.1.notify_one();
        }
    }

    fn start_dispatch(
        this: &Class<'js, Self>,
        ctx: &Ctx<'js>,
        body_rx: Option<mpsc::UnboundedReceiver<Bytes>>,
    ) {
        if this.borrow().dispatched.replace(true) {
            return;
        }
        this.borrow().headers_sent.set(true);
        let req = this.clone();
        let ctx2 = ctx.clone();
        ctx.spawn_exit_simple(async move {
            ClientRequest::dispatch(ctx2, req, body_rx).await?;
            Ok(())
        });
    }

    fn begin_streaming(
        &self,
        ctx: &Ctx<'js>,
        this: &Class<'js, Self>,
    ) -> mpsc::UnboundedSender<Bytes> {
        if let Some(tx) = self.body_stream_tx.borrow().as_ref() {
            return tx.clone();
        }
        let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
        let initial = std::mem::take(&mut *self.body.borrow_mut());
        if !initial.is_empty() {
            let _ = tx.send(Bytes::from(initial));
        }
        *self.body_stream_tx.borrow_mut() = Some(tx.clone());
        Self::start_dispatch(this, ctx, Some(rx));
        tx
    }

    fn current_uri(&self, ctx: &Ctx<'_>) -> Result<Uri> {
        let path = self.path.borrow();
        if self.socket_path.is_some() {
            return path
                .as_str()
                .parse::<Uri>()
                .map_err(|_| Exception::throw_type(ctx, "Invalid URL"));
        }
        format!("{}://{}{}", self.scheme, self.authority, *path)
            .parse::<Uri>()
            .map_err(|_| Exception::throw_type(ctx, "Invalid URL"))
    }

    async fn dispatch(
        ctx: Ctx<'js>,
        this: Class<'js, Self>,
        body_rx: Option<mpsc::UnboundedReceiver<Bytes>>,
    ) -> Result<()> {
        let (client, socket_path, uri, method, headers, buffered_body, rt_key, socket_val) = {
            let borrow = this.borrow();
            let request_body = if body_rx.is_some() {
                Vec::new()
            } else {
                borrow.body.borrow().clone()
            };
            let res = (
                borrow.client.clone(),
                borrow.socket_path.clone(),
                borrow.current_uri(&ctx)?,
                borrow.method.borrow().clone(),
                borrow
                    .headers
                    .borrow()
                    .iter_pairs()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect::<Vec<_>>(),
                request_body,
                borrow.loop_ref.rt(),
                borrow.socket.borrow().clone(),
            );
            res
        };

        let cancel = this.borrow().cancel.clone();
        event_loop::adjust_http_inflight(rt_key, 1);
        // `destroy()`/`abort()` must actually drop the in-flight request:
        // otherwise the response future keeps the runtime referenced and the
        // process never drains.
        let result = tokio::select! {
            biased;
            _ = cancel.notified() => Ok(()),
            result = Self::dispatch_inner(
                ctx.clone(),
                this.clone(),
                client,
                socket_path,
                uri,
                method,
                headers,
                buffered_body,
                body_rx,
                socket_val,
            ) => result,
        };
        event_loop::adjust_http_inflight(rt_key, -1);
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn dispatch_inner(
        ctx: Ctx<'js>,
        this: Class<'js, Self>,
        client: HyperClient,
        socket_path: Option<String>,
        uri: Uri,
        method: Method,
        headers: Vec<(String, String)>,
        buffered_body: Vec<u8>,
        body_rx: Option<mpsc::UnboundedReceiver<Bytes>>,
        socket_val: Value<'js>,
    ) -> Result<()> {
        let _ = resume_timer_loop_if_idle(&ctx);

        let perf_start_ms = perf::has_observers(perf::rt_key_for_ctx(&ctx), PerfEntryKind::Http)
            .then(time::monotonic_elapsed_ms);
        let perf_method = method.to_string();
        let perf_url = uri.to_string();
        let mut builder = Request::builder().method(method).uri(uri);
        let mut has_user_agent = false;
        let mut has_accept = false;
        for (name, value) in &headers {
            if name.eq_ignore_ascii_case("user-agent") {
                has_user_agent = true;
            }
            if name.eq_ignore_ascii_case("accept") {
                has_accept = true;
            }
            let hname = HeaderName::from_bytes(name.as_bytes()).or_throw(&ctx)?;
            let hval = HeaderValue::from_str(value).or_throw(&ctx)?;
            builder = builder.header(hname, hval);
        }
        if !has_user_agent {
            builder = builder.header("user-agent", ["lexe ", VERSION].concat());
        }
        if !has_accept {
            builder = builder.header("accept", "*/*");
        }

        let body: BoxBody<Bytes, Infallible> = if let Some(rx) = body_rx {
            StreamBody::new(
                UnboundedReceiverStream::new(rx).map(|b| Ok::<_, Infallible>(Frame::data(b))),
            )
            .boxed()
        } else if buffered_body.is_empty() {
            Full::new(Bytes::new()).boxed()
        } else {
            Full::new(Bytes::from(buffered_body)).boxed()
        };
        let request = builder.body(body).or_throw(&ctx)?;

        // Node reports transport failures through `request.emit("error")`.
        // Throwing here instead would escape as an uncaught exception before
        // any `error` listener could see it.
        let _ = lexe_utils::diagnostics::publish(&ctx, "http.client.request.start", |ctx| {
            let msg = Object::new(ctx.clone())?;
            msg.set("request", this.clone().into_value())?;
            Ok(msg)
        });

        let response_result = send_request_via_transport(client, socket_path, request).await;
        let response = match response_result {
            Ok(response) => response,
            Err(err) => {
                this.borrow().clear_timeout_timer();
                let error = Exception::from_message(ctx.clone(), &err)?;
                let error_value = error.into_value();
                let _ =
                    lexe_utils::diagnostics::publish(&ctx, "http.client.request.error", |ctx| {
                        let msg = Object::new(ctx.clone())?;
                        msg.set("request", this.clone().into_value())?;
                        msg.set("error", error_value.clone())?;
                        Ok(msg)
                    });
                ClientRequest::emit_str(this.clone(), &ctx, "error", vec![error_value], false)?;
                this.borrow().loop_ref.unreference();
                return Ok(());
            },
        };
        let status = response.status();
        let (parts, incoming_body) = response.into_parts();

        let pairs: Vec<(String, String)> = parts
            .headers
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let (headers_obj, raw_headers) = build_incoming_headers_js(&ctx, pairs)?;
        let perf_res_headers = perf_start_ms.map(|_| headers_obj.clone());

        let message = Class::instance(
            ctx.clone(),
            IncomingMessage::from_client_response(
                &ctx,
                status.as_u16(),
                status.canonical_reason().map(str::to_string),
                headers_obj,
                raw_headers,
                socket_val,
            )?,
        )?;

        ClientRequest::emit_str(
            this.clone(),
            &ctx,
            "response",
            vec![message.clone().into_value()],
            false,
        )?;

        let collected = incoming_body.collect().await.or_throw(&ctx)?;
        let bytes = collected.to_bytes();
        if !bytes.is_empty() {
            IncomingMessage::push_body_chunk(&message, &ctx, &bytes);
        }
        IncomingMessage::finish_body(&message, &ctx);
        let _ = lexe_utils::diagnostics::publish(&ctx, "http.client.response.finish", |ctx| {
            let msg = Object::new(ctx.clone())?;
            msg.set("request", this.clone().into_value())?;
            msg.set("response", message.clone().into_value())?;
            Ok(msg)
        });
        if let Some(start_ms) = perf_start_ms {
            let dur = (time::monotonic_elapsed_ms() - start_ms).max(0.0);
            perf::emit_node_entry(
                &ctx,
                PerfEntryKind::Http,
                "HttpClient",
                start_ms,
                dur,
                |ctx| {
                    let detail = Object::new(ctx.clone())?;
                    let req_obj = Object::new(ctx.clone())?;
                    req_obj.set("method", perf_method.as_str())?;
                    req_obj.set("url", perf_url.as_str())?;
                    let req_headers = this.borrow().headers.borrow().get_headers_object(ctx)?;
                    req_obj.set("headers", req_headers)?;

                    let res_obj = Object::new(ctx.clone())?;
                    res_obj.set("statusCode", status.as_u16())?;
                    res_obj.set("statusMessage", status.canonical_reason().unwrap_or(""))?;
                    if let Some(h) = &perf_res_headers {
                        res_obj.set("headers", h.clone())?;
                    } else {
                        res_obj.set("headers", Object::new(ctx.clone())?)?;
                    }
                    detail.set("req", req_obj)?;
                    detail.set("res", res_obj)?;
                    Ok(detail.into_value())
                },
            );
        }
        // Node marks the request destroyed once the response is fully read.
        {
            let borrow = this.borrow();
            borrow.clear_timeout_timer();
            borrow.completed.set(true);
            borrow.destroyed.set(true);
        }
        ClientRequest::emit_str(this.clone(), &ctx, "close", vec![], false)?;
        this.borrow().loop_ref.unreference();
        Ok(())
    }
}

async fn send_request_via_transport(
    client: HyperClient,
    socket_path: Option<String>,
    request: Request<BoxBody<Bytes, Infallible>>,
) -> std::result::Result<hyper::Response<hyper::body::Incoming>, String> {
    if let Some(path) = socket_path {
        #[cfg(unix)]
        {
            let stream = tokio::net::UnixStream::connect(&path)
                .await
                .map_err(|e| e.to_string())?;
            let (mut sender, conn) =
                hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
                    .await
                    .map_err(|e| e.to_string())?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            return sender
                .send_request(request)
                .await
                .map_err(|e| e.to_string());
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            return Err("Unix socketPath is not supported on this platform".to_string());
        }
    }
    client.request(request).await.map_err(|e| e.to_string())
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> ClientRequest<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Class<'js, Self>> {
        http_request(ctx, args)
    }

    #[qjs(get, enumerable)]
    pub fn method(&self) -> String {
        self.method.borrow().to_string()
    }
    #[qjs(set, rename = "method")]
    pub fn set_method(&self, ctx: Ctx<'js>, value: String) -> Result<()> {
        if self.headers_sent.get() {
            return Err(throw_headers_sent(&ctx, "modify"));
        }
        let upper = value.to_ascii_uppercase();
        let parsed = Method::from_bytes(upper.as_bytes())
            .map_err(|_| Exception::throw_type(&ctx, "Invalid HTTP method"))?;
        *self.method.borrow_mut() = parsed;
        Ok(())
    }

    #[qjs(get, enumerable)]
    pub fn path(&self) -> String {
        self.path.borrow().clone()
    }
    #[qjs(set, rename = "path")]
    pub fn set_path(&self, value: String) {
        if !self.headers_sent.get() {
            *self.path.borrow_mut() = value;
        }
    }

    #[qjs(get, enumerable)]
    pub fn host(&self) -> String {
        self.host.clone()
    }

    #[qjs(get, enumerable)]
    pub fn protocol(&self) -> String {
        self.protocol.clone()
    }

    #[qjs(get, enumerable)]
    pub fn socket(&self) -> Value<'js> {
        self.socket.borrow().clone()
    }

    #[qjs(get, enumerable)]
    pub fn connection(&self) -> Value<'js> {
        self.socket.borrow().clone()
    }

    #[qjs(get, enumerable)]
    pub fn reused_socket(&self) -> bool {
        self.reused_socket.get()
    }

    #[qjs(get, enumerable)]
    pub fn max_headers_count(&self) -> Value<'js> {
        self.max_headers_count.borrow().clone()
    }
    #[qjs(set, rename = "maxHeadersCount")]
    pub fn set_max_headers_count(&self, value: Value<'js>) {
        *self.max_headers_count.borrow_mut() = value;
    }

    #[qjs(get, enumerable)]
    pub fn headers_sent(&self) -> bool {
        self.headers_sent.get()
    }
    #[qjs(get, rename = "_header")]
    pub fn header_flag(&self) -> Option<bool> {
        self.headers_sent.get().then_some(true)
    }
    #[qjs(get, enumerable)]
    pub fn writable_ended(&self) -> bool {
        self.ended.get()
    }
    #[qjs(get, enumerable)]
    pub fn writable_finished(&self) -> bool {
        self.writable_finished.get()
    }
    #[qjs(get, enumerable)]
    pub fn finished(&self) -> bool {
        self.ended.get()
    }

    pub fn set_header(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        name: String,
        value: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent.get() {
            return Err(throw_headers_sent(&ctx, "set"));
        }
        validate_header_name_str(&ctx, &name)?;
        let (values, is_array) = header_value_to_list_with_kind(&ctx, &name, value)?;
        this.borrow()
            .headers
            .borrow_mut()
            .set(&name, values, is_array);
        Ok(this.0)
    }

    pub fn append_header(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        name: String,
        value: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent.get() {
            return Err(throw_headers_sent(&ctx, "append"));
        }
        validate_header_name_str(&ctx, &name)?;
        let (values, is_array) = header_value_to_list_with_kind(&ctx, &name, value)?;
        this.borrow()
            .headers
            .borrow_mut()
            .append(&name, values, is_array);
        Ok(this.0)
    }

    pub fn get_header(&self, ctx: Ctx<'js>, name: String) -> Result<Value<'js>> {
        self.headers.borrow().get_js(&ctx, &name)
    }

    pub fn get_header_names(&self) -> Vec<String> {
        self.headers.borrow().get_header_names()
    }

    pub fn get_raw_header_names(&self) -> Vec<String> {
        self.headers.borrow().get_raw_header_names()
    }

    pub fn get_headers(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        self.headers.borrow().get_headers_object(&ctx)
    }

    pub fn has_header(&self, name: String) -> bool {
        self.headers.borrow().has(&name)
    }

    pub fn remove_header(&self, ctx: Ctx<'js>, name: String) -> Result<()> {
        if self.headers_sent.get() {
            return Err(throw_headers_sent(&ctx, "remove"));
        }
        self.headers.borrow_mut().remove(&name);
        Ok(())
    }

    pub fn set_headers(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        headers: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent.get() {
            return Err(throw_headers_sent(&ctx, "set"));
        }
        this.borrow()
            .headers
            .borrow_mut()
            .set_from_js(&ctx, headers)?;
        Ok(this.0)
    }

    pub fn flush_headers(this: This<Class<'js, Self>>, ctx: Ctx<'js>) {
        if this.borrow().headers_sent.get() || this.borrow().destroyed.get() {
            return;
        }
        this.borrow().begin_streaming(&ctx, &this.0);
    }

    pub fn set_timeout(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        msecs: f64,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().ended.get() {
            return Ok(this.0);
        }
        if let Some(cb) = callback.0 {
            Self::add_event_listener_str(this.0.clone(), &ctx, "timeout", cb, true, false)?;
        }
        Self::arm_timeout(&this.0, &ctx, msecs);
        Ok(this.0)
    }

    pub fn clear_timeout(&self) {
        self.clear_timeout_timer();
    }

    pub fn set_no_delay(&self, ctx: Ctx<'js>, no_delay: Opt<Value<'js>>) -> Result<()> {
        let enable = match no_delay.0 {
            Some(v) if !v.is_undefined() => v.as_bool().unwrap_or(true),
            _ => true,
        };
        self.pending_no_delay.set(Some(enable));
        let sock = self.socket.borrow().clone();
        if let Some(obj) = sock.as_object() {
            if let Some(sock_cls) = Class::<Socket>::from_object(obj) {
                let _ = Socket::set_no_delay(This(sock_cls), ctx, Opt(Some(enable)));
            }
        }
        Ok(())
    }

    pub fn set_socket_keep_alive(
        &self,
        ctx: Ctx<'js>,
        enable: Opt<Value<'js>>,
        initial_delay: Opt<Value<'js>>,
    ) -> Result<()> {
        let en = match enable.0 {
            Some(v) if !v.is_undefined() => v.as_bool().unwrap_or(false),
            _ => false,
        };
        let delay = match initial_delay.0 {
            Some(v) if !v.is_undefined() => v
                .as_number()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(|n| n as u64)
                .unwrap_or(0),
            _ => 0,
        };
        self.pending_keep_alive.set(Some((en, delay)));
        let sock = self.socket.borrow().clone();
        if let Some(obj) = sock.as_object() {
            if let Some(sock_cls) = Class::<Socket>::from_object(obj) {
                let _ = Socket::set_keep_alive(
                    This(sock_cls),
                    ctx.clone(),
                    Opt(Some(Value::new_bool(ctx.clone(), en))),
                    Opt(Some(delay.into_js(&ctx)?)),
                );
            }
        }
        Ok(())
    }

    pub fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<bool> {
        if this.borrow().ended.get() {
            return Err(Exception::throw_type(&ctx, "Request already ended"));
        }
        let (bytes, cb) = parse_write_or_end_args(&ctx, &args.0, false)?;
        let bytes = bytes.unwrap_or_default();
        {
            let borrow = this.borrow();
            let tx = borrow.begin_streaming(&ctx, &this.0);
            if !bytes.is_empty() {
                let _ = tx.send(Bytes::from(bytes));
            }
        }
        if let Some(cb) = cb {
            let _ = cb.defer::<()>(());
        }
        Ok(true)
    }

    pub fn end(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let (bytes, cb) = parse_write_or_end_args(&ctx, &args.0, true)?;
        {
            let borrow = this.borrow();
            if borrow.ended.get() {
                drop(borrow);
                if let Some(cb) = cb {
                    let _ = cb.defer::<()>(());
                }
                return Ok(this.0);
            }
            borrow.ended.set(true);
            borrow.writable_finished.set(true);
            borrow.headers_sent.set(true);
            if let Some(tx) = borrow.body_stream_tx.borrow_mut().take() {
                if let Some(b) = bytes {
                    if !b.is_empty() {
                        let _ = tx.send(Bytes::from(b));
                    }
                }
                drop(tx);
            } else {
                if let Some(b) = bytes {
                    borrow.body.borrow_mut().extend_from_slice(&b);
                }
                Self::start_dispatch(&this.0, &ctx, None);
            };
        }
        let _ = Self::emit_str(this.0.clone(), &ctx, "finish", vec![], false);
        if let Some(cb) = cb {
            cb.call::<_, ()>(())?;
        }
        Ok(this.0)
    }

    /// Node's `destroy()`: stop the request, emit `error` (ECONNRESET unless a
    /// cause was given) then `close`, and mark the request destroyed. It does
    /// not set `aborted` - only `abort()` does.
    pub fn destroy(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        error: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        teardown(this.0.clone(), &ctx, error.0)?;
        Ok(this.0)
    }

    /// Node deprecated `abort()` but kept it: it is `destroy()` plus an `abort`
    /// event emitted before `error`, and it sets `aborted`.
    pub fn abort(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Class<'js, Self>> {
        this.borrow().aborted.set(true);
        Self::emit_str(this.clone(), &ctx, "abort", vec![], false)?;
        teardown(this.0.clone(), &ctx, None)?;
        Ok(this.0)
    }

    #[qjs(get, enumerable)]
    pub fn aborted(&self) -> bool {
        self.aborted.get()
    }

    #[qjs(get, enumerable)]
    pub fn destroyed(&self) -> bool {
        self.destroyed.get()
    }
}

/// Shared tail of `destroy()` / `abort()`: idempotent, no-op once the response
/// has completed (Node leaves a finished request alone).
fn teardown<'js>(
    this: Class<'js, ClientRequest<'js>>,
    ctx: &Ctx<'js>,
    error: Option<Value<'js>>,
) -> Result<()> {
    let (already_destroyed, completed) = {
        let borrow = this.borrow();
        (borrow.destroyed.get(), borrow.completed.get())
    };
    if already_destroyed {
        return Ok(());
    }
    {
        let borrow = this.borrow();
        borrow.destroyed.set(true);
        borrow.ended.set(true);
        borrow.clear_timeout_timer();
        borrow.body_stream_tx.borrow_mut().take();
    }

    if completed {
        // The response already finished; Node has emitted `close` by now.
        return Ok(());
    }

    let error_value = match error {
        Some(value) if !value.is_undefined() => value,
        _ => {
            let err = Exception::from_message(ctx.clone(), "socket hang up")?;
            let value = err.into_value();
            if let Some(object) = value.as_object() {
                let code = rquickjs::String::from_str(ctx.clone(), "ECONNRESET")?;
                object.set("code", code)?;
            }
            value
        },
    };
    let _ = lexe_utils::diagnostics::publish(ctx, "http.client.request.error", |ctx| {
        let msg = Object::new(ctx.clone())?;
        msg.set("request", this.clone().into_value())?;
        msg.set("error", error_value.clone())?;
        Ok(msg)
    });
    ClientRequest::emit_str(this.clone(), ctx, "error", vec![error_value], false)?;
    ClientRequest::emit_str(this.clone(), ctx, "close", vec![], false)?;
    this.borrow().cancel.notify_waiters();
    this.borrow().loop_ref.unreference();
    Ok(())
}

pub(crate) fn chunk_to_bytes<'js>(ctx: &Ctx<'js>, chunk: Value<'js>) -> Result<Vec<u8>> {
    if let Some(s) = chunk.as_string() {
        return Ok(s.to_string()?.into_bytes());
    }
    if let Ok(buf) = rquickjs::TypedArray::<u8>::from_value(chunk.clone()) {
        return Ok(buf.as_bytes().map(|b| b.to_vec()).unwrap_or_default());
    }
    if let Some(obj) = chunk.as_object() {
        if let Some(ab) = obj.as_array_buffer() {
            return Ok(ab.as_bytes().map(|b| b.to_vec()).unwrap_or_default());
        }
    }
    Err(Exception::throw_type(ctx, "Chunk must be string or Buffer"))
}
