// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, Uri};
use raster_runtime_buffer::Buffer;
use raster_runtime_context::CtxExtension;
use raster_runtime_events::{Emitter, EventEmitter, EventList};
use raster_runtime_net::Socket;
use raster_runtime_timers::resume_timer_loop_if_idle;
use raster_runtime_utils::{
    event_loop::{self, EventLoopRef},
    object::ObjectExt,
    result::ResultExt,
    VERSION,
};
use rquickjs::{
    function::{Opt, This},
    prelude::Rest,
    Array, Class, Ctx, Exception, IntoJs, Object, Result, Undefined, Value,
};

use crate::server::{header_value_to_list, IncomingMessage};
use crate::HyperClient;

struct ParsedRequest<'js> {
    uri: Uri,
    method: Method,
    headers: Vec<(String, String)>,
    callback: Option<rquickjs::Function<'js>>,
    /// Client to dispatch on. Built per request when TLS options were given,
    /// otherwise the shared pooled one.
    client: HyperClient,
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

fn parse_request_args<'js>(
    ctx: &Ctx<'js>,
    args: &Rest<Value<'js>>,
    force_method: Option<&str>,
    default_protocol: &str,
) -> Result<ParsedRequest<'js>> {
    let mut url: Option<String> = None;
    let mut options: Option<Object<'js>> = None;
    let mut callback: Option<rquickjs::Function<'js>> = None;

    for value in &args.0 {
        if value.is_function() {
            callback = Some(
                value
                    .as_function()
                    .ok_or_else(|| Exception::throw_type(ctx, "Callback must be a function"))?
                    .clone(),
            );
        } else if value.is_string() {
            url = Some(value.as_string().unwrap().to_string()?);
        } else if value.is_object() {
            options = Some(value.as_object().unwrap().clone());
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

    let mut headers = Vec::new();
    if let Some(opts) = &options {
        if let Some(hdrs) = opts.get_optional::<_, Object>("headers")? {
            for key in hdrs.keys::<String>() {
                let key = key?;
                let value: Value = hdrs.get(&key)?;
                for part in header_value_to_list(ctx, value)? {
                    headers.push((key.to_ascii_lowercase(), part));
                }
            }
        }
    }

    let uri = if let Some(url) = url {
        url.parse::<Uri>()
            .map_err(|_| Exception::throw_type(ctx, "Invalid URL"))?
    } else {
        build_uri_from_options(ctx, options.as_ref(), default_protocol)?
    };

    // TLS options are per request (`https.get({ ca })`, `rejectUnauthorized`),
    // exactly as in Node. A request that supplies either needs its own client;
    // everything else shares the pooled one built from `TLS_CONFIG`.
    let tls = parse_request_tls_options(ctx, options.as_ref())?;
    let client = match tls {
        Some(tls) => crate::build_client_for_request(ctx, tls)?,
        None => hyper_client(ctx)?,
    };

    Ok(ParsedRequest {
        uri,
        method,
        headers,
        callback,
        client,
    })
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
        Some(value) => Some(pem_list_from_value(ctx, value)?),
        None => None,
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
    raster_runtime_utils::bytes::ObjectBytes::from(ctx, &value)?.into_bytes(ctx)
}

fn build_uri_from_options<'js>(
    ctx: &Ctx<'js>,
    options: Option<&Object<'js>>,
    default_protocol: &str,
) -> Result<Uri> {
    let options = options.ok_or_else(|| Exception::throw_type(ctx, "URL or options required"))?;
    let protocol: String = options
        .get_optional("protocol")?
        .unwrap_or_else(|| default_protocol.into());
    let default_port = if protocol == "https:" { 443 } else { 80 };
    let port: u16 = options.get_optional("port")?.unwrap_or(default_port);
    let hostname: String = options
        .get_optional::<_, String>("hostname")?
        .or_else(|| options.get_optional("host").ok().flatten())
        .ok_or_else(|| Exception::throw_type(ctx, "hostname is required"))?;
    let hostname = hostname.split(':').next().unwrap_or(&hostname).to_string();
    let path: String = options.get_optional("path")?.unwrap_or_else(|| "/".into());
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    let uri = format!("{protocol}//{hostname}:{port}{path}");
    uri.parse::<Uri>()
        .map_err(|_| Exception::throw_type(ctx, "Invalid URL"))
}

#[rquickjs::class]
#[derive(rquickjs::JsLifetime, rquickjs::class::Trace)]
pub struct ClientRequest<'js> {
    emitter: EventEmitter<'js>,
    #[qjs(skip_trace)]
    client: HyperClient,
    #[qjs(skip_trace)]
    uri: Uri,
    #[qjs(skip_trace)]
    method: Method,
    #[qjs(skip_trace)]
    headers: Vec<(String, String)>,
    #[qjs(skip_trace)]
    body: RefCell<Vec<u8>>,
    #[qjs(skip_trace)]
    ended: RefCell<bool>,
    /// Node exposes these as plain booleans on the request. `aborted` is only
    /// set by `abort()`; `destroy()` leaves it `false`.
    #[qjs(skip_trace)]
    aborted: Cell<bool>,
    #[qjs(skip_trace)]
    destroyed: Cell<bool>,
    /// Set once the response finished normally, so a late `destroy()` is a
    /// no-op like Node's.
    #[qjs(skip_trace)]
    completed: Cell<bool>,
    /// Fires when `destroy()`/`abort()` runs, so an in-flight hyper request is
    /// actually dropped instead of holding the loop reference until it settles.
    #[qjs(skip_trace)]
    cancel: Arc<tokio::sync::Notify>,
    #[qjs(skip_trace)]
    loop_ref: Arc<EventLoopRef>,
}

impl<'js> Emitter<'js> for ClientRequest<'js> {
    fn get_event_list(&self) -> std::sync::Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

impl<'js> ClientRequest<'js> {
    fn start(
        ctx: &Ctx<'js>,
        parsed: ParsedRequest<'js>,
        auto_end: bool,
    ) -> Result<Class<'js, Self>> {
        let client = parsed.client.clone();
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key =
            raster_runtime_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);

        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                client,
                uri: parsed.uri,
                method: parsed.method,
                headers: parsed.headers,
                body: RefCell::new(Vec::new()),
                ended: RefCell::new(false),
                aborted: Cell::new(false),
                destroyed: Cell::new(false),
                completed: Cell::new(false),
                cancel: Arc::new(tokio::sync::Notify::new()),
                loop_ref,
            },
        )?;

        if let Some(cb) = parsed.callback {
            Self::add_event_listener_str(instance.clone(), ctx, "response", cb, false, false)?;
        }

        if auto_end {
            ClientRequest::end(This(instance.clone()), ctx.clone(), Opt(None))?;
        }

        Ok(instance)
    }

    async fn dispatch(ctx: Ctx<'js>, this: Class<'js, Self>) -> Result<()> {
        let (client, uri, method, headers, body, rt_key) = {
            let borrow = this.borrow();
            let request_body = borrow.body.borrow().clone();
            (
                borrow.client.clone(),
                borrow.uri.clone(),
                borrow.method.clone(),
                borrow.headers.clone(),
                request_body,
                borrow.loop_ref.rt(),
            )
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
                uri,
                method,
                headers,
                body,
            ) => result,
        };
        event_loop::adjust_http_inflight(rt_key, -1);
        result
    }

    async fn dispatch_inner(
        ctx: Ctx<'js>,
        this: Class<'js, Self>,
        client: HyperClient,
        uri: Uri,
        method: Method,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<()> {
        let _ = resume_timer_loop_if_idle(&ctx);

        let mut builder = Request::builder().method(method).uri(uri);
        let mut has_user_agent = false;
        for (name, value) in &headers {
            if name.eq_ignore_ascii_case("user-agent") {
                has_user_agent = true;
            }
            builder = builder.header(name.as_str(), value.as_str());
        }
        if !has_user_agent {
            builder = builder.header("user-agent", ["raster_runtime ", VERSION].concat());
        }
        builder = builder.header("accept", "*/*");

        let request = if body.is_empty() {
            builder
                .body(Full::new(Bytes::new()).boxed())
                .or_throw(&ctx)?
        } else {
            builder
                .body(Full::new(Bytes::from(body)).boxed())
                .or_throw(&ctx)?
        };

        // Node reports transport failures through `request.emit("error")`.
        // Throwing here instead would escape as an uncaught exception before
        // any `error` listener could see it, and would bypass the usual
        // `req.on("error")` handling.
        let response = match client.request(request).await {
            Ok(response) => response,
            Err(err) => {
                let error = Exception::from_message(ctx.clone(), &err.to_string())?;
                ClientRequest::emit_str(
                    this.clone(),
                    &ctx,
                    "error",
                    vec![error.into_value()],
                    false,
                )?;
                return Ok(());
            },
        };
        let status = response.status();
        let (parts, incoming_body) = response.into_parts();

        let socket = Socket::new(ctx.clone(), false)?;
        let (headers_obj, raw_headers) = headers_from_parts(&ctx, &parts.headers)?;

        let message = Class::instance(
            ctx.clone(),
            IncomingMessage::from_client_response(
                &ctx,
                status.as_u16(),
                status.canonical_reason().map(str::to_string),
                headers_obj,
                raw_headers,
                socket,
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
            message.borrow_mut().append_body(&bytes);
            let chunk = Buffer(bytes.to_vec())
                .into_js(&ctx)
                .unwrap_or_else(|_| Undefined.into_value(ctx.clone()));
            IncomingMessage::emit_str(message.clone(), &ctx, "data", vec![chunk], false)?;
        }
        message.borrow_mut().mark_complete();
        IncomingMessage::emit_str(message, &ctx, "end", vec![], false)?;
        // Node marks the request destroyed once the response is fully read.
        {
            let borrow = this.borrow();
            borrow.completed.set(true);
            borrow.destroyed.set(true);
        }
        ClientRequest::emit_str(this.clone(), &ctx, "close", vec![], false)?;
        this.borrow().loop_ref.unreference();
        Ok(())
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> ClientRequest<'js> {
    pub fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        chunk: Value<'js>,
        _encoding: Opt<Value<'js>>,
        _cb: Opt<rquickjs::Function<'js>>,
    ) -> Result<bool> {
        if *this.borrow().ended.borrow() {
            return Err(Exception::throw_type(&ctx, "Request already ended"));
        }
        let bytes = chunk_to_bytes(&ctx, chunk)?;
        this.borrow().body.borrow_mut().extend_from_slice(&bytes);
        Ok(true)
    }

    pub fn end(this: This<Class<'js, Self>>, ctx: Ctx<'js>, chunk: Opt<Value<'js>>) -> Result<()> {
        if let Some(chunk) = chunk.0 {
            let bytes = chunk_to_bytes(&ctx, chunk)?;
            this.borrow().body.borrow_mut().extend_from_slice(&bytes);
        }
        if *this.borrow().ended.borrow() {
            return Ok(());
        }
        *this.borrow().ended.borrow_mut() = true;
        let req = this.clone();
        let ctx2 = ctx.clone();
        ctx.spawn_exit_simple(async move {
            ClientRequest::dispatch(ctx2, req).await?;
            Ok(())
        });
        Ok(())
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
        *borrow.ended.borrow_mut() = true;
    }

    if completed {
        // The response already finished; Node has emitted `close` by now.
        return Ok(());
    }

    let error_value = match error {
        Some(value) => value,
        None => {
            let err = Exception::from_message(ctx.clone(), "socket hang up")?;
            let value = err.into_value();
            if let Some(object) = value.as_object() {
                let code = rquickjs::String::from_str(ctx.clone(), "ECONNRESET")?;
                object.set("code", code)?;
            }
            value
        },
    };
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

fn headers_from_parts<'js>(
    ctx: &Ctx<'js>,
    headers: &hyper::HeaderMap,
) -> Result<(Object<'js>, Vec<String>)> {
    let headers_obj = Object::new(ctx.clone())?;
    let mut raw = Vec::new();
    let mut grouped: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (name, value) in headers {
        let value = value.to_str().unwrap_or("").to_string();
        let name = name.as_str().to_string();
        raw.push(name.clone());
        raw.push(value.clone());
        grouped.entry(name).or_default().push(value);
    }
    for (name, values) in grouped {
        if name == "set-cookie" {
            let cookies = Array::new(ctx.clone())?;
            for (index, value) in values.into_iter().enumerate() {
                cookies.set(index, value)?;
            }
            headers_obj.set(name, cookies)?;
        } else {
            let separator = if name == "cookie" { "; " } else { ", " };
            headers_obj.set(name, values.join(separator))?;
        }
    }
    Ok((headers_obj, raw))
}
