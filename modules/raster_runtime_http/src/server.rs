//! The Node-compatible HTTP/1 server deliberately owns its Hyper listener.
//! This mirrors Bun's architecture: `node:http` is an HTTP server, not a
//! parser bolted onto the public `node:net` server.
use std::{
    cell::Cell,
    collections::HashMap,
    convert::Infallible,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, LazyLock,
    },
    time::Duration,
};

use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::{
    body::{Frame, Incoming},
    header::{HeaderName, HeaderValue},
    server::conn::http1,
    service::service_fn,
    HeaderMap, Request as HyperRequest, Response as HyperResponse, StatusCode,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use raster_runtime_async_hooks::begin_http_incoming_message_scope;
use raster_runtime_buffer::Buffer;
use raster_runtime_context::CtxExtension;
use raster_runtime_events::{Emitter, EventEmitter, EventList};
use raster_runtime_net::Socket;
use raster_runtime_timers::{poll_timers, resume_timer_loop_if_idle, ExecutingTimer};
use raster_runtime_utils::{
    bytes::ObjectBytes,
    event_loop::{self, EventLoopRef},
    module::{export_default, ModuleInfo},
    object::ObjectExt,
};
use rquickjs::{
    class::{Trace, Tracer},
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, Opt, Rest, This},
    qjs, Array, Class, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Result, Undefined,
    Value,
};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::{broadcast, mpsc, oneshot},
};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

/// QuickJS `Ctx` is not re-entrant across concurrent Hyper handlers.
static HTTP_JS_ROOT: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

tokio::task_local! {
    static HTTP_DISPATCH_NEST: Cell<usize>;
}

struct HttpJsSerialGuard {
    _root: Option<tokio::sync::MutexGuard<'static, ()>>,
}

impl HttpJsSerialGuard {
    async fn acquire() -> Self {
        let nested = HTTP_DISPATCH_NEST.with(|cell| cell.get());
        let root = if nested == 0 {
            Some(HTTP_JS_ROOT.lock().await)
        } else {
            None
        };
        HTTP_DISPATCH_NEST.with(|cell| cell.set(nested + 1));
        Self { _root: root }
    }
}

impl Drop for HttpJsSerialGuard {
    fn drop(&mut self) {
        let _ = HTTP_DISPATCH_NEST.try_with(|cell| {
            cell.set(cell.get().saturating_sub(1));
        });
    }
}

fn validate_header_name(ctx: Ctx<'_>, name: String) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
    {
        return Err(Exception::throw_type(&ctx, "Invalid HTTP header name"));
    }
    Ok(())
}

fn validate_header_value(ctx: Ctx<'_>, _name: String, value: String) -> Result<()> {
    if value
        .bytes()
        .any(|byte| byte == b'\r' || byte == b'\n' || byte == 0)
    {
        return Err(Exception::throw_type(&ctx, "Invalid HTTP header value"));
    }
    Ok(())
}

/// Coerce Node-style header values into a list of strings (arrays expand).
pub(super) fn header_value_to_list<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Vec<String>> {
    if value.is_undefined() || value.is_null() {
        return Ok(vec![String::new()]);
    }
    if let Some(s) = value.as_string() {
        return Ok(vec![s.to_string()?]);
    }
    if let Some(n) = value.as_number() {
        // Match Node: numbers are stringified without scientific notation for integers.
        if n.fract() == 0.0 && n.abs() < 1e15 {
            return Ok(vec![format!("{}", n as i64)]);
        }
        return Ok(vec![n.to_string()]);
    }
    if let Some(b) = value.as_bool() {
        return Ok(vec![if b { "true" } else { "false" }.into()]);
    }
    if let Some(arr) = value.as_array() {
        let mut parts = Vec::new();
        for item in arr.iter::<Value>() {
            let item = item?;
            if item.is_undefined() || item.is_null() {
                continue;
            }
            parts.extend(header_value_to_list(ctx, item)?);
        }
        return Ok(parts);
    }
    // Fallback: JS ToString
    let s: String = ctx.eval::<Function, _>("String")?.call((value,))?;
    Ok(vec![s])
}

pub(crate) fn create_server<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, Server<'js>>> {
    let mut server_options: Option<Object<'js>> = None;
    for value in &args.0 {
        if let Some(option) = value.as_object().filter(|_| value.as_function().is_none()) {
            if option
                .get_optional::<_, bool>("insecureHTTPParser")?
                .unwrap_or(false)
            {
                return Err(Exception::throw_message(
                    &ctx,
                    "insecureHTTPParser is not supported",
                ));
            }
            server_options = Some(option.clone());
        }
    }
    let listener = args
        .0
        .into_iter()
        .rev()
        .find_map(|value| value.into_function());
    let server = Server::new(ctx, Opt(listener))?;
    if let Some(options) = server_options {
        let mut instance = server.borrow_mut();
        if let Some(value) = options.get_optional("maxHeadersCount")? {
            instance.max_headers_count = value;
        }
        if let Some(value) = options.get_optional("headersTimeout")? {
            instance.headers_timeout = value;
        }
        if let Some(value) = options.get_optional("requestTimeout")? {
            instance.request_timeout = value;
        }
        if let Some(value) = options.get_optional("keepAliveTimeout")? {
            instance.keep_alive_timeout = value;
        }
        if let Some(value) = options.get_optional("maxHeaderSize")? {
            instance.max_header_size = value;
        }
        if let Some(value) = options.get_optional("maxBodySize")? {
            instance.max_request_body_size = value;
        }
    }
    Ok(server)
}

enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

enum AcceptedStream {
    Tcp(tokio::net::TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

enum ResponseFrame {
    Data(Bytes),
    Trailers(HeaderMap),
}

impl Listener {
    async fn accept(&self) -> std::io::Result<AcceptedStream> {
        match self {
            Self::Tcp(listener) => listener
                .accept()
                .await
                .map(|(stream, _)| AcceptedStream::Tcp(stream)),
            #[cfg(unix)]
            Self::Unix(listener) => listener
                .accept()
                .await
                .map(|(stream, _)| AcceptedStream::Unix(stream)),
        }
    }
}

#[rquickjs::class]
pub struct IncomingMessage<'js> {
    emitter: EventEmitter<'js>,
    method: String,
    url: String,
    headers: Object<'js>,
    raw_headers: Vec<String>,
    trailers: Object<'js>,
    raw_trailers: Vec<String>,
    body: Vec<u8>,
    complete: bool,
    paused: bool,
    destroyed: bool,
    socket: Value<'js>,
    response_status_code: Option<u16>,
    response_status_message: Option<String>,
}
unsafe impl<'js> JsLifetime<'js> for IncomingMessage<'js> {
    type Changed<'to> = IncomingMessage<'to>;
}
impl<'js> Trace<'js> for IncomingMessage<'js> {
    fn trace<'a>(&self, t: Tracer<'a, 'js>) {
        self.emitter.trace(t);
        self.headers.trace(t);
        self.trailers.trace(t);
        self.socket.trace(t);
    }
}
impl<'js> Emitter<'js> for IncomingMessage<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}
#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> IncomingMessage<'js> {
    #[qjs(get, enumerable)]
    fn method(&self) -> String {
        self.method.clone()
    }
    #[qjs(set, rename = "method")]
    fn set_method(&mut self, value: String) {
        self.method = value;
    }
    #[qjs(get, enumerable)]
    fn url(&self) -> String {
        self.url.clone()
    }
    /// Node allows rewriting `req.url` during routing/middleware.
    #[qjs(set, rename = "url")]
    fn set_url(&mut self, value: String) {
        self.url = value;
    }
    #[qjs(get, enumerable)]
    fn headers(&self) -> Object<'js> {
        self.headers.clone()
    }
    #[qjs(get, enumerable)]
    fn raw_headers(&self) -> Vec<String> {
        self.raw_headers.clone()
    }
    #[qjs(get, enumerable)]
    fn trailers(&self) -> Object<'js> {
        self.trailers.clone()
    }
    #[qjs(get, enumerable)]
    fn raw_trailers(&self) -> Vec<String> {
        self.raw_trailers.clone()
    }
    #[qjs(get, enumerable)]
    fn http_version(&self) -> &'static str {
        "1.1"
    }
    #[qjs(get, enumerable)]
    fn complete(&self) -> bool {
        self.complete
    }
    #[qjs(get, enumerable)]
    fn socket(&self) -> Value<'js> {
        self.socket.clone()
    }
    pub fn read(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Value<'js>> {
        ObjectBytes::Vec(std::mem::take(&mut this.borrow_mut().body)).into_js(&ctx)
    }
    pub fn pause(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow_mut().paused = true;
        this.0
    }
    pub fn resume(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow_mut().paused = false;
        this.0
    }
    pub fn destroy(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow_mut().destroyed = true;
        this.0
    }
    #[qjs(get, rename = "statusCode")]
    fn status_code(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match self.response_status_code {
            Some(code) => code.into_js(&ctx),
            None => Ok(Undefined.into_value(ctx)),
        }
    }
    #[qjs(get, rename = "statusMessage")]
    fn status_message(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match self.response_status_message.as_ref() {
            Some(msg) => msg.clone().into_js(&ctx),
            None => Ok(Undefined.into_value(ctx)),
        }
    }
}

impl<'js> IncomingMessage<'js> {
    pub(crate) fn from_client_response(
        ctx: &Ctx<'js>,
        status_code: u16,
        status_message: Option<String>,
        headers: Object<'js>,
        raw_headers: Vec<String>,
        socket: Class<'js, Socket<'js>>,
    ) -> Result<Self> {
        Ok(Self {
            emitter: EventEmitter::new(),
            method: String::new(),
            url: String::new(),
            headers,
            raw_headers,
            trailers: Object::new(ctx.clone())?,
            raw_trailers: Vec::new(),
            body: Vec::new(),
            complete: false,
            paused: false,
            destroyed: false,
            socket: socket.into_value(),
            response_status_code: Some(status_code),
            response_status_message: status_message,
        })
    }

    pub(crate) fn append_body(&mut self, data: &[u8]) {
        self.body.extend_from_slice(data);
    }

    pub(crate) fn mark_complete(&mut self) {
        self.complete = true;
    }
}

#[rquickjs::class]
pub struct ServerResponse<'js> {
    emitter: EventEmitter<'js>,
    status_code: u16,
    status_message: Option<String>,
    /// Header values as lists so `appendHeader` can emit multiple fields
    /// (Node joins most headers with `, ` and repeats `set-cookie`).
    headers: HashMap<String, Vec<String>>,
    headers_sent: bool,
    /// Mirrors Node `res.writableFinished` — true after `end()` completes.
    writable_finished: bool,
    /// Mirrors Node `res.destroyed`.
    destroyed: bool,
    /// Node OutgoingMessage internal: truthy once headers have been written.
    /// Compression middleware checks `if (!this._header) this._implicitHeader()`.
    header_sent_flag: bool,
    tx: Option<oneshot::Sender<HyperResponse<BoxBody<Bytes, Infallible>>>>,
    body_tx: Option<mpsc::UnboundedSender<ResponseFrame>>,
    trailers: Option<HeaderMap>,
    continue_requested: bool,
    /// True when the request was a HEAD: Node never sends a body for it, and
    /// must not advertise a `Content-Length` for one either.
    is_head: bool,
    /// True once `write()` has queued body bytes. After that the framing is
    /// already committed (chunked), so `end()` must not add `Content-Length`.
    body_started: bool,
}
unsafe impl<'js> JsLifetime<'js> for ServerResponse<'js> {
    type Changed<'to> = ServerResponse<'to>;
}
impl<'js> Trace<'js> for ServerResponse<'js> {
    fn trace<'a>(&self, t: Tracer<'a, 'js>) {
        self.emitter.trace(t);
    }
}
impl<'js> Emitter<'js> for ServerResponse<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}
impl<'js> ServerResponse<'js> {
    fn start(&mut self) {
        if let Some(tx) = self.tx.take() {
            let (body_tx, body_rx) = mpsc::unbounded_channel();
            let mut builder = HyperResponse::builder().status(self.status_code);
            if let Some(message) = &self.status_message {
                if let Ok(message) = hyper::ext::ReasonPhrase::try_from(message.clone()) {
                    builder = builder.extension(message);
                }
            }
            for (name, values) in &self.headers {
                for value in values {
                    builder = builder.header(name.as_str(), value.as_str());
                }
            }
            let body = StreamBody::new(UnboundedReceiverStream::new(body_rx).map(|frame| {
                Ok::<_, Infallible>(match frame {
                    ResponseFrame::Data(bytes) => Frame::data(bytes),
                    ResponseFrame::Trailers(trailers) => Frame::trailers(trailers),
                })
            }))
            .boxed();
            let response = builder
                .body(body)
                .unwrap_or_else(|_| HyperResponse::new(Full::new(Bytes::new()).boxed()));
            let _ = tx.send(response);
            self.body_tx = Some(body_tx);
            self.headers_sent = true;
            self.header_sent_flag = true;
        }
    }
    fn finish(&mut self) {
        self.start();
        if let (Some(tx), Some(trailers)) = (&self.body_tx, self.trailers.take()) {
            let _ = tx.send(ResponseFrame::Trailers(trailers));
        }
        self.body_tx.take();
        self.writable_finished = true;
    }
}
#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> ServerResponse<'js> {
    #[qjs(get, enumerable)]
    fn status_code(&self) -> u16 {
        self.status_code
    }
    #[qjs(set, rename = "statusCode")]
    fn set_status_code(&mut self, value: u16) {
        self.status_code = value;
    }
    #[qjs(get, enumerable)]
    fn headers_sent(&self) -> bool {
        self.headers_sent
    }
    /// Node OutgoingMessage internal flag (used by `compression` and similar).
    #[qjs(get, rename = "_header")]
    fn header_flag(&self) -> Option<bool> {
        if self.header_sent_flag {
            Some(true)
        } else {
            None
        }
    }
    /// Node OutgoingMessage internal — send headers if not already sent.
    #[qjs(rename = "_implicitHeader")]
    pub fn implicit_header(this: This<Class<'js, Self>>) {
        let mut res = this.borrow_mut();
        if !res.headers_sent {
            res.start();
        }
    }
    #[qjs(get, enumerable)]
    fn status_message(&self) -> String {
        self.status_message.clone().unwrap_or_else(|| {
            StatusCode::from_u16(self.status_code)
                .ok()
                .and_then(|status| status.canonical_reason().map(str::to_owned))
                .unwrap_or_default()
        })
    }
    #[qjs(set, rename = "statusMessage")]
    fn set_status_message(&mut self, value: String) {
        if !self.headers_sent {
            self.status_message = Some(value);
        }
    }
    /// Node `res.setHeader(name, value)` accepts string, number, or string[].
    pub fn set_header(&mut self, ctx: Ctx<'js>, name: String, value: Value<'js>) -> Result<()> {
        validate_header_name(ctx.clone(), name.clone())?;
        let values = header_value_to_list(&ctx, value)?;
        for v in &values {
            validate_header_value(ctx.clone(), name.clone(), v.clone())?;
        }
        if !self.headers_sent {
            self.headers.insert(name.to_ascii_lowercase(), values);
        }
        Ok(())
    }
    /// Node `res.appendHeader(name, value)` — adds without replacing prior values.
    pub fn append_header(&mut self, ctx: Ctx<'js>, name: String, value: Value<'js>) -> Result<()> {
        validate_header_name(ctx.clone(), name.clone())?;
        let mut values = header_value_to_list(&ctx, value)?;
        for v in &values {
            validate_header_value(ctx.clone(), name.clone(), v.clone())?;
        }
        if !self.headers_sent {
            let key = name.to_ascii_lowercase();
            self.headers.entry(key).or_default().append(&mut values);
        }
        Ok(())
    }
    pub fn get_header(&self, name: String) -> Option<String> {
        self.headers.get(&name.to_ascii_lowercase()).map(|values| {
            if name.eq_ignore_ascii_case("set-cookie") {
                // Node returns the first set-cookie string from getHeader for
                // historical reasons when a single value exists; multi returns
                // joined — keep simple join for non-array consumers.
                values.join(", ")
            } else {
                values.join(", ")
            }
        })
    }
    pub fn has_header(&self, name: String) -> bool {
        self.headers.contains_key(&name.to_ascii_lowercase())
    }
    pub fn get_headers(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let result = Object::new(ctx.clone())?;
        for (name, values) in &self.headers {
            if name == "set-cookie" {
                let arr = rquickjs::Array::new(ctx.clone())?;
                for (i, v) in values.iter().enumerate() {
                    arr.set(i, v.clone())?;
                }
                result.set(name, arr)?;
            } else if values.len() == 1 {
                result.set(name, values[0].clone())?;
            } else {
                result.set(name, values.join(", "))?;
            }
        }
        Ok(result)
    }
    pub fn remove_header(&mut self, name: String) {
        self.headers.remove(&name.to_ascii_lowercase());
    }
    pub fn write(this: This<Class<'js, Self>>, ctx: Ctx<'js>, value: Value<'js>) -> Result<bool> {
        let bytes = Bytes::from(ObjectBytes::from(&ctx, &value)?.into_bytes(&ctx)?);
        let mut res = this.borrow_mut();
        res.body_started = true;
        res.start();
        Ok(res
            .body_tx
            .as_ref()
            .is_some_and(|tx| tx.send(ResponseFrame::Data(bytes)).is_ok()))
    }
    pub fn end(this: This<Class<'js, Self>>, ctx: Ctx<'js>, value: Opt<Value<'js>>) -> Result<()> {
        {
            let mut res = this.borrow_mut();
            if res.writable_finished || res.destroyed {
                return Ok(());
            }
            let bytes = match value.0 {
                Some(value) => Some(Bytes::from(
                    ObjectBytes::from(&ctx, &value)?.into_bytes(&ctx)?,
                )),
                None => None,
            };
            // Node decides the framing here: if headers are still unsent, nothing
            // has been written yet, and the body length is known, it emits an
            // exact `Content-Length` instead of degrading to chunked. A response
            // that already started streaming (`write()` called) or one that must
            // not carry a body (HEAD, 204, 304) keeps the existing behaviour.
            let bodyless_status =
                matches!(res.status_code, 204 | 304) || (100..200).contains(&res.status_code);
            if !res.headers_sent
                && !res.body_started
                && !res.is_head
                && !bodyless_status
                && !res
                    .headers
                    .keys()
                    .any(|k| k.eq_ignore_ascii_case("content-length"))
            {
                let len = bytes.as_ref().map(|b| b.len()).unwrap_or(0);
                res.headers
                    .insert("content-length".to_string(), vec![len.to_string()]);
            }
            res.start();
            if let Some(bytes) = bytes {
                if let Some(tx) = &res.body_tx {
                    let _ = tx.send(ResponseFrame::Data(bytes));
                }
            }
            res.finish();
        }
        // Node emits `finish` after the response has been fully handed off.
        // Next's pipe-readable waits on this event before completing pipeTo.
        let _ = ServerResponse::emit_str(this.0.clone(), &ctx, "finish", vec![], false);
        let _ = ServerResponse::emit_str(this.0, &ctx, "close", vec![], false);
        Ok(())
    }
    #[qjs(get, enumerable)]
    fn writable_finished(&self) -> bool {
        self.writable_finished
    }
    #[qjs(get, enumerable)]
    fn destroyed(&self) -> bool {
        self.destroyed
    }
    #[qjs(get, enumerable)]
    fn writable_ended(&self) -> bool {
        self.writable_finished
    }
    /// Minimal Node Writable destroy — closes the response without a body frame.
    pub fn destroy(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        _err: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let already = {
            let mut res = this.borrow_mut();
            if res.destroyed {
                true
            } else {
                res.destroyed = true;
                res.writable_finished = true;
                res.body_tx.take();
                res.tx.take();
                false
            }
        };
        if !already {
            let _ = ServerResponse::emit_str(this.0.clone(), &ctx, "close", vec![], false);
        }
        Ok(this.0)
    }
    pub fn add_trailers(&mut self, trailers: Object<'js>) -> Result<()> {
        let mut result = HeaderMap::new();
        for name in trailers.keys::<String>() {
            let name = name?;
            let value = trailers.get::<_, String>(&name)?;
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                rquickjs::Error::new_from_js_message(
                    "header name",
                    "HeaderName",
                    "Invalid trailer name",
                )
            })?;
            let value = HeaderValue::from_str(&value).map_err(|_| {
                rquickjs::Error::new_from_js_message(
                    "header value",
                    "HeaderValue",
                    "Invalid trailer value",
                )
            })?;
            result.append(name, value);
        }
        self.trailers = Some(result);
        Ok(())
    }
    pub fn write_head(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let mut args = args.0.into_iter();
        let status_code = args
            .next()
            .and_then(|value| value.as_int())
            .ok_or_else(|| Exception::throw_type(&ctx, "statusCode is required"))?
            as u16;
        let next = args.next();
        let (status_message, headers) = match next {
            Some(value) if value.is_string() => (
                value
                    .as_string()
                    .map(|value| value.to_string())
                    .transpose()?,
                args.next().and_then(|value| value.as_object().cloned()),
            ),
            Some(value) => (None, value.as_object().cloned()),
            None => (None, None),
        };
        let mut response = this.borrow_mut();
        response.status_code = status_code;
        if let Some(status_message) = status_message {
            response.status_message = Some(status_message);
        }
        if let Some(headers) = headers {
            for name in headers.keys::<String>() {
                let name = name?;
                let value: Value = headers.get(&name)?;
                let values = header_value_to_list(&ctx, value)?;
                for v in &values {
                    validate_header_name(ctx.clone(), name.clone())?;
                    validate_header_value(ctx.clone(), name.clone(), v.clone())?;
                }
                response.headers.insert(name.to_ascii_lowercase(), values);
            }
        }
        response.start();
        drop(response);
        Ok(this.0)
    }
    pub fn flush_headers(&mut self) {
        self.start();
    }
    /// Hyper emits the actual interim response when its request body is first
    /// polled. `dispatch` uses this flag to decide whether that poll is
    /// permitted for a `checkContinue` request.
    pub fn write_continue(&mut self) {
        self.continue_requested = true;
    }
}

#[rquickjs::class]
pub struct Server<'js> {
    emitter: EventEmitter<'js>,
    address: Value<'js>,
    close_tx: broadcast::Sender<()>,
    connection_close_tx: broadcast::Sender<()>,
    idle_close_tx: broadcast::Sender<()>,
    listening: Arc<AtomicBool>,
    connections: Arc<AtomicUsize>,
    closing: Arc<AtomicBool>,
    close_emitted: Arc<AtomicBool>,
    max_headers_count: usize,
    max_header_size: usize,
    max_request_body_size: usize,
    headers_timeout: u64,
    request_timeout: u64,
    keep_alive_timeout: u64,
    loop_ref: Arc<EventLoopRef>,
}
unsafe impl<'js> JsLifetime<'js> for Server<'js> {
    type Changed<'to> = Server<'to>;
}
impl<'js> Trace<'js> for Server<'js> {
    fn trace<'a>(&self, t: Tracer<'a, 'js>) {
        self.emitter.trace(t);
        self.address.trace(t);
    }
}
impl<'js> Emitter<'js> for Server<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}
#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Server<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, listener: Opt<Function<'js>>) -> Result<Class<'js, Self>> {
        let (close_tx, _) = broadcast::channel(1);
        let (connection_close_tx, _) = broadcast::channel(1);
        let (idle_close_tx, _) = broadcast::channel(1);
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key =
            raster_runtime_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);
        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                address: Undefined.into_value(ctx.clone()),
                close_tx,
                connection_close_tx,
                idle_close_tx,
                listening: Arc::new(AtomicBool::new(false)),
                connections: Arc::new(AtomicUsize::new(0)),
                closing: Arc::new(AtomicBool::new(false)),
                close_emitted: Arc::new(AtomicBool::new(false)),
                max_headers_count: 100,
                max_header_size: 16 * 1024,
                max_request_body_size: 16 * 1024 * 1024,
                headers_timeout: 60_000,
                request_timeout: 300_000,
                keep_alive_timeout: 65_000,
                loop_ref,
            },
        )?;
        if let Some(listener) = listener.0 {
            Self::add_event_listener_str(
                instance.clone(),
                &ctx,
                "request",
                listener,
                false,
                false,
            )?;
        }
        Ok(instance)
    }
    pub fn address(&self) -> Value<'js> {
        self.address.clone()
    }
    #[qjs(get, enumerable)]
    pub fn listening(&self) -> bool {
        self.listening.load(Ordering::Relaxed)
    }
    #[qjs(get, enumerable)]
    pub fn max_headers_count(&self) -> usize {
        self.max_headers_count
    }
    #[qjs(set, rename = "maxHeadersCount")]
    pub fn set_max_headers_count(&mut self, value: usize) {
        self.max_headers_count = value;
    }
    #[qjs(get, enumerable)]
    pub fn headers_timeout(&self) -> u64 {
        self.headers_timeout
    }
    #[qjs(set, rename = "headersTimeout")]
    pub fn set_headers_timeout(&mut self, value: u64) {
        self.headers_timeout = value;
    }
    #[qjs(get, enumerable)]
    pub fn request_timeout(&self) -> u64 {
        self.request_timeout
    }
    #[qjs(set, rename = "requestTimeout")]
    pub fn set_request_timeout(&mut self, value: u64) {
        self.request_timeout = value;
    }
    #[qjs(get, enumerable)]
    pub fn keep_alive_timeout(&self) -> u64 {
        self.keep_alive_timeout
    }
    #[qjs(set, rename = "keepAliveTimeout")]
    pub fn set_keep_alive_timeout(&mut self, value: u64) {
        self.keep_alive_timeout = value;
    }
    pub fn get_connections(&self, cb: Opt<Function<'js>>) -> Result<()> {
        if let Some(cb) = cb.0 {
            cb.call::<_, ()>((Undefined, self.connections.load(Ordering::Relaxed)))?;
        }
        Ok(())
    }
    pub fn close(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        if let Some(callback) = callback.0 {
            Self::add_event_listener_str(this.0.clone(), &ctx, "close", callback, true, true)?;
        }
        let server = this.borrow();
        server.closing.store(true, Ordering::Relaxed);
        let _ = server.close_tx.send(());
        drop(server);
        emit_close_if_ready(&ctx, this.0.clone())?;
        Ok(this.0)
    }
    pub fn close_all_connections(&self) {
        let _ = self.connection_close_tx.send(());
    }
    pub fn close_idle_connections(&self) {
        let _ = self.idle_close_tx.send(());
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
    pub fn listen(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let mut port = 0u16;
        let mut host = "0.0.0.0".to_string();
        let mut path = None;
        let mut callback = None;
        let mut saw_port = false;
        for (index, arg) in args.0.into_iter().enumerate() {
            if let Some(p) = arg.as_int() {
                port = p as u16;
                saw_port = true;
            } else if let Some(s) = arg.as_string() {
                let value = s.to_string()?;
                if index == 0 && !saw_port {
                    path = Some(value);
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
                path = o.get_optional::<_, String>("path")?;
            }
        }
        if let Some(callback) = callback {
            Self::add_event_listener_str(this.0.clone(), &ctx, "listening", callback, true, true)?;
        }
        let return_server = this.0.clone();
        let server = this.0;
        let ctx2 = ctx.clone();
        let mut close = server.borrow().close_tx.subscribe();
        let listen_loop_ref = server.borrow().loop_ref.clone();
        ctx.spawn_exit_simple(async move {
            let (listener, address) = if let Some(path) = path {
                #[cfg(unix)]
                { let listener = UnixListener::bind(&path).map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?; (Listener::Unix(listener), path.into_js(&ctx2)?) }
                #[cfg(not(unix))]
                { return Err(Exception::throw_type(&ctx2, "Unix domain sockets are not supported on this platform")); }
            } else {
                let listener = TcpListener::bind(format!("{host}:{port}")).await.map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                let addr = listener.local_addr().map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                let address = Object::new(ctx2.clone())?; address.set("address", addr.ip().to_string())?; address.set("port", addr.port())?; address.set("family", if addr.is_ipv4() { "IPv4" } else { "IPv6" })?; (Listener::Tcp(listener), address.into_value())
            };
            server.borrow_mut().address = address; server.borrow().listening.store(true, Ordering::Relaxed); Self::emit_str(server.clone(), &ctx2, "listening", vec![], false)?;
            loop {
                if !raster_runtime_utils::event_loop::keeps_event_loop_alive(&listen_loop_ref) {
                    break;
                }
                tokio::select! {
                    _ = raster_runtime_utils::event_loop::wait_until_io_unref(listen_loop_ref.clone()) => {
                        break;
                    }
                    _ = close.recv() => break,
                    accepted = listener.accept() => {
                        let stream = accepted.map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                        let ctx3 = ctx2.clone();
                        let srv = server.clone();
                        match stream {
                            AcceptedStream::Tcp(stream) => spawn_tcp_connection(ctx3, srv, stream)?,
                            #[cfg(unix)]
                            AcceptedStream::Unix(stream) => spawn_unix_connection(ctx3, srv, stream)?,
                        }
                    }
                }
            }
            server.borrow().listening.store(false, Ordering::Relaxed);
            emit_close_if_ready(&ctx2, server)?;
            Ok(())
        });
        Ok(return_server)
    }
}

fn emit_close_if_ready<'js>(ctx: &Ctx<'js>, server: Class<'js, Server<'js>>) -> Result<()> {
    let state = server.borrow();
    let ready = state.closing.load(Ordering::Relaxed)
        && !state.listening.load(Ordering::Relaxed)
        && state.connections.load(Ordering::Relaxed) == 0
        && !state.close_emitted.swap(true, Ordering::Relaxed);
    drop(state);
    if ready {
        Server::emit_str(server, ctx, "close", vec![], false)?;
    }
    Ok(())
}

fn spawn_tcp_connection<'js>(
    ctx: Ctx<'js>,
    server: Class<'js, Server<'js>>,
    stream: tokio::net::TcpStream,
) -> Result<()> {
    let socket = Socket::new(ctx.clone(), false)?;
    Socket::set_addresses(&socket, &ctx, &stream)?;
    Socket::mark_connected(&socket);
    let std_stream = stream
        .into_std()
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let writer_std = std_stream
        .try_clone()
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    Socket::attach_raw_tcp_shutdown(
        &socket,
        Arc::new(
            std_stream
                .try_clone()
                .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?,
        ),
    );
    let stream = tokio::net::TcpStream::from_std(std_stream)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let writer_stream = tokio::net::TcpStream::from_std(writer_std)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let (writer, mut writes) = mpsc::unbounded_channel();
    Socket::attach_raw_writer(&socket, writer);
    tokio::spawn(async move {
        let mut writer_stream = writer_stream;
        while let Some(bytes) = writes.recv().await {
            if writer_stream.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = writer_stream.shutdown().await;
    });
    spawn_connection(ctx, server, socket.into_value(), stream)
}

#[cfg(unix)]
fn spawn_unix_connection<'js>(
    ctx: Ctx<'js>,
    server: Class<'js, Server<'js>>,
    stream: UnixStream,
) -> Result<()> {
    let socket = Socket::new(ctx.clone(), false)?;
    Socket::mark_connected(&socket);
    let std_stream = stream
        .into_std()
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let writer_std = std_stream
        .try_clone()
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    Socket::attach_raw_unix_shutdown(
        &socket,
        Arc::new(
            std_stream
                .try_clone()
                .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?,
        ),
    );
    let stream = UnixStream::from_std(std_stream)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let writer_stream = UnixStream::from_std(writer_std)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let (writer, mut writes) = mpsc::unbounded_channel();
    Socket::attach_raw_writer(&socket, writer);
    tokio::spawn(async move {
        let mut writer_stream = writer_stream;
        while let Some(bytes) = writes.recv().await {
            if writer_stream.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = writer_stream.shutdown().await;
    });
    spawn_connection(ctx, server, socket.into_value(), stream)
}

fn socket_class<'js>(ctx: &Ctx<'js>, socket: &Value<'js>) -> Result<Class<'js, Socket<'js>>> {
    let obj = socket
        .as_object()
        .ok_or_else(|| Exception::throw_type(ctx, "Invalid socket"))?;
    Class::<Socket>::from_object(obj)
        .ok_or_else(|| Exception::throw_type(ctx, "Expected net.Socket"))
}

fn spawn_connection<'js, T>(
    ctx: Ctx<'js>,
    server: Class<'js, Server<'js>>,
    socket: Value<'js>,
    stream: T,
) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let server_ref = server.borrow();
    let count = server_ref.connections.clone();
    let mut connection_close = server_ref.connection_close_tx.subscribe();
    let mut idle_close = server_ref.idle_close_tx.subscribe();
    let max_headers_count = server_ref.max_headers_count;
    let max_header_size = server_ref.max_header_size;
    let max_request_body_size = server_ref.max_request_body_size;
    let headers_timeout = server_ref.headers_timeout;
    let request_timeout = server_ref.request_timeout;
    let keep_alive_timeout = server_ref.keep_alive_timeout;
    drop(server_ref);
    count.fetch_add(1, Ordering::Relaxed);
    let request_ctx = ctx.clone();
    let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let http_rt_key = event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
    let service_server = server.clone();
    let active_requests = Arc::new(AtomicUsize::new(0));
    let service_active_requests = active_requests.clone();
    let (activity_tx, mut activity_rx) = mpsc::unbounded_channel();
    let service_activity_tx = activity_tx.clone();
    ctx.clone().spawn_exit_simple(async move {
        let _ = Server::emit_str(
            server.clone(),
            &ctx,
            "connection",
            vec![socket.clone()],
            false,
        );
        let service_socket = socket.clone();
        let service = service_fn(move |request: HyperRequest<Incoming>| {
            let active_requests = service_active_requests.clone();
            let request_ctx = request_ctx.clone();
            let server = service_server.clone();
            let socket = service_socket.clone();
            let activity_tx = service_activity_tx.clone();
            async move {
                HTTP_DISPATCH_NEST
                    .scope(Cell::new(0), async {
                        active_requests.fetch_add(1, Ordering::Relaxed);
                        event_loop::adjust_http_inflight(http_rt_key, 1);
                        let response = dispatch(
                            request_ctx,
                            server,
                            socket,
                            request,
                            request_timeout,
                            max_request_body_size,
                        )
                        .await;
                        event_loop::adjust_http_inflight(http_rt_key, -1);
                        active_requests.fetch_sub(1, Ordering::Relaxed);
                        let _ = activity_tx.send(());
                        response
                    })
                    .await
            }
        });
        let mut builder = http1::Builder::new();
        builder.timer(TokioTimer::new());
        if max_headers_count > 0 {
            builder.max_headers(max_headers_count);
        }
        builder.max_buf_size(max_header_size.max(8192));
        if headers_timeout > 0 {
            builder.header_read_timeout(Duration::from_millis(headers_timeout));
        }
        let connection = builder
            .serve_connection(TokioIo::new(stream), service)
            .with_upgrades();
        tokio::pin!(connection);
        let idle_timeout = tokio::time::sleep(Duration::from_secs(86_400));
        tokio::pin!(idle_timeout);
        let mut keep_alive_armed = false;
        let result = loop {
            if keep_alive_timeout == 0 {
                tokio::select! {
                    _ = connection_close.recv() => break None,
                    _ = idle_close.recv() => {
                        if active_requests.load(Ordering::Relaxed) == 0 {
                            break None;
                        }
                    },
                    result = &mut connection => break Some(result),
                }
            } else {
                tokio::select! {
                    _ = connection_close.recv() => break None,
                    _ = idle_close.recv() => {
                        if active_requests.load(Ordering::Relaxed) == 0 {
                            break None;
                        }
                    },
                    _ = &mut idle_timeout, if keep_alive_armed => {
                        if active_requests.load(Ordering::Relaxed) == 0 {
                            break None;
                        }
                        idle_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(keep_alive_timeout));
                    },
                    _ = activity_rx.recv() => {
                        keep_alive_armed = true;
                        idle_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(keep_alive_timeout));
                    },
                    result = &mut connection => break Some(result),
                }
            }
        };
        if let Some(Err(error)) = result {
            let error_message = error.to_string();
            let error_value = Object::new(ctx.clone())?.into_value();
            if let Some(error_object) = error_value.as_object() {
                error_object.set("message", error_message)?;
                error_object.set("code", "HPE_INVALID_REQUEST")?;
            }
            let _ = Server::emit_str(
                server.clone(),
                &ctx,
                "clientError",
                vec![error_value],
                false,
            );
        }
        count.fetch_sub(1, Ordering::Relaxed);
        emit_close_if_ready(&ctx, server)?;
        Ok(())
    });
    Ok(())
}

/// Drive microtasks while Hyper awaits async HTTP handlers.
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

/// Drive microtasks while waiting for the response oneshot.
async fn await_response_pumping(
    ctx: &Ctx<'_>,
    pump: &mut JsEventLoopPump,
    rx: oneshot::Receiver<HyperResponse<BoxBody<Bytes, Infallible>>>,
    empty: impl Fn() -> HyperResponse<BoxBody<Bytes, Infallible>>,
) -> HyperResponse<BoxBody<Bytes, Infallible>> {
    use tokio::time::{interval, MissedTickBehavior};
    let mut tick = interval(Duration::from_millis(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tokio::pin!(rx);
    loop {
        tokio::select! {
            biased;
            response = &mut rx => return response.unwrap_or_else(|_| empty()),
            _ = tick.tick() => pump.tick(ctx),
        }
    }
}

async fn dispatch<'js>(
    ctx: Ctx<'js>,
    server: Class<'js, Server<'js>>,
    connection_socket: Value<'js>,
    request: HyperRequest<Incoming>,
    request_timeout: u64,
    max_request_body_size: usize,
) -> std::result::Result<HyperResponse<BoxBody<Bytes, Infallible>>, Infallible> {
    let _js_serial = HttpJsSerialGuard::acquire().await;
    let mut request = request;
    let is_connect = request.method() == hyper::Method::CONNECT;
    let is_upgrade = request.headers().contains_key(hyper::header::UPGRADE);
    let upgrade_event = if is_connect { "connect" } else { "upgrade" };
    if (is_connect || is_upgrade) && !server.borrow().has_listener_str(upgrade_event) {
        return Ok(HyperResponse::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(hyper::header::CONNECTION, "close")
            .body(Full::new(Bytes::new()).boxed())
            .unwrap_or_else(|_| HyperResponse::new(Full::new(Bytes::new()).boxed())));
    }
    let on_upgrade = (is_connect || is_upgrade).then(|| hyper::upgrade::on(&mut request));
    let (parts, mut body) = request.into_parts();
    let empty = || {
        HyperResponse::builder()
            .status(500)
            .body(Full::new(Bytes::new()).boxed())
            .unwrap()
    };
    let mut js_pump = JsEventLoopPump::new(&ctx);
    let _ = resume_timer_loop_if_idle(&ctx);
    let headers = match Object::new(ctx.clone()) {
        Ok(v) => v,
        Err(_) => return Ok(empty()),
    };
    let trailers = match Object::new(ctx.clone()) {
        Ok(v) => v,
        Err(_) => return Ok(empty()),
    };
    let mut raw = Vec::new();
    let mut grouped_headers: HashMap<String, Vec<String>> = HashMap::new();
    for (name, value) in &parts.headers {
        let value = value.to_str().unwrap_or("").to_string();
        let name = name.to_string();
        raw.push(name.clone());
        raw.push(value.clone());
        grouped_headers.entry(name).or_default().push(value);
    }
    for (name, values) in grouped_headers {
        if name == "set-cookie" {
            let Ok(cookies) = Array::new(ctx.clone()) else {
                return Ok(empty());
            };
            for (index, value) in values.into_iter().enumerate() {
                if cookies.set(index, value).is_err() {
                    return Ok(empty());
                }
            }
            if headers.set(name, cookies).is_err() {
                return Ok(empty());
            }
        } else {
            let separator = if name == "cookie" { "; " } else { ", " };
            if headers.set(name, values.join(separator)).is_err() {
                return Ok(empty());
            }
        }
    }
    let req = match Class::instance(
        ctx.clone(),
        IncomingMessage {
            emitter: EventEmitter::new(),
            method: parts.method.to_string(),
            url: parts.uri.to_string(),
            headers,
            raw_headers: raw,
            trailers,
            raw_trailers: Vec::new(),
            body: Vec::new(),
            complete: false,
            paused: false,
            destroyed: false,
            socket: connection_socket.clone(),
            response_status_code: None,
            response_status_message: None,
        },
    ) {
        Ok(v) => v,
        Err(_) => return Ok(empty()),
    };
    let req_object = match req.clone().into_value().as_object() {
        Some(object) => object.clone(),
        None => return Ok(empty()),
    };
    let _http_scope = match begin_http_incoming_message_scope(ctx.clone(), req_object) {
        Ok(scope) => scope,
        Err(_) => return Ok(empty()),
    };
    if let Some(on_upgrade) = on_upgrade {
        let socket_value = connection_socket.clone();
        let socket = match socket_class(&ctx, &socket_value) {
            Ok(socket) => socket,
            Err(_) => return Ok(empty()),
        };
        let reader = Arc::new(std::sync::Mutex::new(Vec::new()));
        Socket::attach_raw_reader(&socket, reader.clone());
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let event_socket = socket.clone();
        let event_ctx = ctx.clone();
        ctx.spawn_exit_simple(async move {
            while let Some(bytes) = event_rx.recv().await {
                let value = Buffer(bytes).into_js(&event_ctx)?;
                Socket::emit_str(event_socket.clone(), &event_ctx, "data", vec![value], false)?;
            }
            Socket::emit_str(event_socket, &event_ctx, "end", vec![], false)?;
            Ok(())
        });
        let event = if is_connect { "connect" } else { "upgrade" };
        let head = ObjectBytes::Vec(Vec::new())
            .into_js(&ctx)
            .unwrap_or_else(|_| Undefined.into_value(ctx.clone()));
        let _ = Server::emit_str(
            server,
            &ctx,
            event,
            vec![req.into_value(), socket_value, head],
            false,
        );
        tokio::spawn(async move {
            let Ok(upgraded) = on_upgrade.await else {
                return;
            };
            let stream = TokioIo::new(upgraded);
            let (mut reader_stream, _writer_stream) = tokio::io::split(stream);
            let reader_buffer = reader;
            tokio::spawn(async move {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader_stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            let bytes = buffer[..count].to_vec();
                            reader_buffer.lock().unwrap().extend_from_slice(&bytes);
                            let _ = event_tx.send(bytes);
                        },
                    }
                }
            });
        });
        let mut response = HyperResponse::builder();
        if is_connect {
            response = response.status(StatusCode::OK);
        } else {
            response = response
                .status(StatusCode::SWITCHING_PROTOCOLS)
                .header(hyper::header::CONNECTION, "upgrade");
            if let Some(value) = parts.headers.get(hyper::header::UPGRADE) {
                response = response.header(hyper::header::UPGRADE, value);
            }
        }
        return Ok(response
            .body(Full::new(Bytes::new()).boxed())
            .unwrap_or_else(|_| empty()));
    }
    let (tx, mut rx) = oneshot::channel();
    let res = match Class::instance(
        ctx.clone(),
        ServerResponse {
            emitter: EventEmitter::new(),
            status_code: StatusCode::OK.as_u16(),
            status_message: None,
            headers: HashMap::new(),
            headers_sent: false,
            writable_finished: false,
            destroyed: false,
            header_sent_flag: false,
            tx: Some(tx),
            body_tx: None,
            trailers: None,
            continue_requested: false,
            is_head: parts.method == hyper::Method::HEAD,
            body_started: false,
        },
    ) {
        Ok(v) => v,
        Err(_) => return Ok(empty()),
    };
    let expects_continue = parts
        .headers
        .get(hyper::header::EXPECT)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"100-continue"));
    let check_continue = expects_continue && server.borrow().has_listener_str("checkContinue");
    let req_value = req.clone().into_value();
    let res_value = res.clone().into_value();
    let _ = Server::emit_str(
        server,
        &ctx,
        if check_continue {
            "checkContinue"
        } else {
            "request"
        },
        vec![req_value, res_value],
        false,
    );
    for _ in 0..512 {
        if res.borrow().headers_sent {
            break;
        }
        js_pump.tick(&ctx);
    }

    // A synchronous handler may have already committed a final response.
    // Return it now instead of waiting for an abandoned or slow request body.
    if res.borrow().headers_sent {
        return Ok(await_response_pumping(&ctx, &mut js_pump, rx, empty).await);
    }

    // Hyper owns the HTTP/1 encoder and deliberately sends `100 Continue`
    // when the body is first polled. This precisely matches Node's default
    // path. With a `checkContinue` listener Node lets userland decide; only
    // poll after `writeContinue()` so a final response is sent without 100.
    let consume_body = !check_continue || res.borrow().continue_requested;
    if consume_body {
        use tokio::time::{interval, MissedTickBehavior};
        let mut pump_tick = interval(Duration::from_millis(1));
        pump_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            while req.borrow().paused && !req.borrow().destroyed {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(1)) => js_pump.tick(&ctx),
                    _ = pump_tick.tick() => js_pump.tick(&ctx),
                }
            }
            if req.borrow().destroyed {
                break;
            }
            // Do not make a response wait for a client that is still (or no
            // longer) sending its request body. This is especially important
            // for async handlers which call `end()` after the first body poll.
            let next_frame = if request_timeout == 0 {
                tokio::select! {
                    response = &mut rx => return Ok(response.unwrap_or_else(|_| empty())),
                    frame = body.frame() => frame,
                    _ = pump_tick.tick() => {
                        js_pump.tick(&ctx);
                        continue;
                    }
                }
            } else {
                tokio::select! {
                    response = &mut rx => return Ok(response.unwrap_or_else(|_| empty())),
                    result = tokio::time::timeout(Duration::from_millis(request_timeout), body.frame()) => match result {
                        Ok(frame) => frame,
                        Err(_) => return Ok(HyperResponse::builder()
                            .status(StatusCode::REQUEST_TIMEOUT)
                            .body(Full::new(Bytes::new()).boxed())
                            .unwrap_or_else(|_| empty())),
                    },
                    _ = pump_tick.tick() => {
                        js_pump.tick(&ctx);
                        continue;
                    }
                }
            };
            let Some(frame) = next_frame else {
                break;
            };
            let Ok(frame) = frame else {
                break;
            };
            match frame.into_data() {
                Ok(data) => {
                    if req.borrow().body.len().saturating_add(data.len()) > max_request_body_size {
                        return Ok(HyperResponse::builder()
                            .status(StatusCode::PAYLOAD_TOO_LARGE)
                            .header(hyper::header::CONNECTION, "close")
                            .body(Full::new(Bytes::new()).boxed())
                            .unwrap_or_else(|_| empty()));
                    }
                    req.borrow_mut().body.extend_from_slice(&data);
                    let chunk = Buffer(data.to_vec())
                        .into_js(&ctx)
                        .unwrap_or_else(|_| Undefined.into_value(ctx.clone()));
                    let _ =
                        IncomingMessage::emit_str(req.clone(), &ctx, "data", vec![chunk], false);
                },
                Err(frame) => {
                    if let Ok(trailers) = frame.into_trailers() {
                        let mut message = req.borrow_mut();
                        for (name, value) in &trailers {
                            let value = value.to_str().unwrap_or("").to_string();
                            message.raw_trailers.push(name.to_string());
                            message.raw_trailers.push(value.clone());
                            let _ = message.trailers.set(name.as_str(), value);
                        }
                    }
                },
            }
        }
    }
    if consume_body {
        req.borrow_mut().complete = true;
        let _ = IncomingMessage::emit_str(req, &ctx, "end", vec![], false);
    }
    Ok(await_response_pumping(&ctx, &mut js_pump, rx, empty).await)
}

pub(crate) fn handoff_tls_to_http_server<'js>(
    ctx: Ctx<'js>,
    http_server: Value<'js>,
    tls_socket: Class<'js, raster_runtime_tls::tls_socket::TlsSocket<'js>>,
    stream: std::pin::Pin<Box<dyn raster_runtime_tls::http_handoff::AsyncReadWrite + 'static>>,
) -> Result<()> {
    let http = Class::<Server>::from_object(
        http_server
            .as_object()
            .ok_or_else(|| Exception::throw_type(&ctx, "Invalid http.Server"))?,
    )
    .ok_or_else(|| Exception::throw_type(&ctx, "Invalid http.Server"))?;
    spawn_connection(ctx.clone(), http, tls_socket.clone().into_js(&ctx)?, stream)
}

/// Minimal `http.Agent` so Next can run `new http.Agent(httpAgentOptions)`.
/// Not a full connection-pool agent; options are accepted for compatibility.
fn create_http_agent_constructor<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    ctx.eval(
        r#"(function () {
  function Agent(options) {
    if (!(this instanceof Agent)) {
      return new Agent(options);
    }
    this.options = options && typeof options === "object" ? options : {};
    this.keepAlive = Boolean(this.options.keepAlive);
    this.maxSockets = this.options.maxSockets;
    this.maxFreeSockets = this.options.maxFreeSockets;
  }
  Agent.prototype.destroy = function () {};
  return Agent;
})()"#,
    )
}

pub struct HttpModule;
impl ModuleDef for HttpModule {
    fn declare(declare: &Declarations) -> Result<()> {
        for name in [
            "createServer",
            "Server",
            "IncomingMessage",
            "ServerResponse",
            "ClientRequest",
            "Agent",
            "globalAgent",
            "METHODS",
            "STATUS_CODES",
            "validateHeaderName",
            "validateHeaderValue",
            // Node 22+ client-side WebSocket.
            "WebSocket",
            // Client stubs — present for ESM named-import graphs (e.g. vite).
            "get",
            "request",
            "default",
        ] {
            declare.declare(name)?;
        }
        Ok(())
    }
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        raster_runtime_buffer::init(ctx)?;
        export_default(ctx, exports, |default| {
            Class::<Server>::define(default)?;
            Class::<IncomingMessage>::define(default)?;
            Class::<ServerResponse>::define(default)?;
            // Socket is an implementation detail of upgrade/connect events;
            // Node exposes it from `node:net`, not `node:http`.
            Class::<Socket>::define(&ctx.globals())?;
            // `http.WebSocket` (Node 22+). Also published globally, as Node does.
            Class::<crate::websocket::WebSocket>::define(default)?;
            Class::<crate::websocket::WebSocket>::define(&ctx.globals())?;
            if let Some(proto) = Class::<crate::websocket::WebSocket>::prototype(ctx)? {
                proto.set("CONNECTING", crate::websocket::CONNECTING)?;
                proto.set("OPEN", crate::websocket::OPEN)?;
                proto.set("CLOSING", crate::websocket::CLOSING)?;
                proto.set("CLOSED", crate::websocket::CLOSED)?;
            }
            if let Some(ctor) = Class::<crate::websocket::WebSocket>::create_constructor(ctx)? {
                crate::websocket::WebSocket::add_event_emitter_prototype(ctx)?;
                ctor.set("CONNECTING", crate::websocket::CONNECTING)?;
                ctor.set("OPEN", crate::websocket::OPEN)?;
                ctor.set("CLOSING", crate::websocket::CLOSING)?;
                ctor.set("CLOSED", crate::websocket::CLOSED)?;
                // Node's WebSocket is an EventTarget: handlers are set as
                // `onopen`/`onmessage`/... properties, not through `on()`.
                // Route those onto the emitter the class already has.
                let install: rquickjs::Function = ctx.eval(
                    r#"(function (WebSocketClass) {
  for (const name of ["open", "message", "close", "error"]) {
    const slot = "_on" + name;
    Object.defineProperty(WebSocketClass.prototype, "on" + name, {
      configurable: true,
      enumerable: true,
      get() { return this[slot] || null; },
      set(handler) {
        const previous = this[slot];
        if (typeof previous === "function") this.removeListener(name, previous);
        this[slot] = typeof handler === "function" ? handler : null;
        if (typeof handler === "function") this.on(name, handler);
      },
    });
  }
  return WebSocketClass;
})"#,
                )?;
                let _: Value = install.call((ctor,))?;
            }
            Server::add_event_emitter_prototype(ctx)?;
            IncomingMessage::add_event_emitter_prototype(ctx)?;
            ServerResponse::add_event_emitter_prototype(ctx)?;
            Socket::add_event_emitter_prototype(ctx)?;
            default.set("createServer", Func::from(create_server))?;
            let agent_ctor = create_http_agent_constructor(ctx)?;
            let global_agent: Value = agent_ctor.call((Object::new(ctx.clone())?,))?;
            default.set("Agent", agent_ctor)?;
            default.set("globalAgent", global_agent)?;
            default.set(
                "METHODS",
                vec![
                    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT", "TRACE",
                ],
            )?;
            let status_codes = Object::new(ctx.clone())?;
            for code in 100..=599 {
                if let Ok(status) = StatusCode::from_u16(code) {
                    if let Some(reason) = status.canonical_reason() {
                        status_codes.set(code, reason)?;
                    }
                }
            }
            default.set("STATUS_CODES", status_codes)?;
            default.set("validateHeaderName", Func::from(validate_header_name))?;
            default.set("validateHeaderValue", Func::from(validate_header_value))?;
            #[cfg(any(
                feature = "tls-ring",
                feature = "tls-aws-lc",
                feature = "tls-graviola",
                feature = "tls-openssl"
            ))]
            {
                use crate::client_request::{http_get, http_request, ClientRequest};
                Class::<ClientRequest>::define(default)?;
                ClientRequest::add_event_emitter_prototype(ctx)?;
                default.set("request", Func::from(http_request))?;
                default.set("get", Func::from(http_get))?;
            }
            #[cfg(not(any(
                feature = "tls-ring",
                feature = "tls-aws-lc",
                feature = "tls-graviola",
                feature = "tls-openssl"
            )))]
            {
                let client_stub: rquickjs::Function = ctx.eval(
                    r#"(function(){
  return function httpClientStub() {
    throw new Error("http.get/request client API is not implemented in raster_runtime yet");
  };
})()"#,
                )?;
                default.set("get", client_stub.clone())?;
                default.set("request", client_stub)?;
            }
            Ok(())
        })?;
        Ok(())
    }
}
impl From<HttpModule> for ModuleInfo<HttpModule> {
    fn from(module: HttpModule) -> Self {
        ModuleInfo {
            name: "http",
            module,
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod server_tests;
