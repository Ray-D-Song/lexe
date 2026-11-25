//! The Node-compatible HTTP/1 server deliberately owns its Hyper listener
//! (with an opt-in lenient HTTP/1.1 parser when `insecureHTTPParser: true` is set).
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
use lexe_async_hooks::begin_http_incoming_message_scope;
use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_events::{Emitter, EventEmitter, EventList};
use lexe_net::Socket;
use lexe_timers::{poll_timers, resume_timer_loop_if_idle, ExecutingTimer};
use lexe_utils::{
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
#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions as NamedPipeServerOptions};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    net::TcpListener,
    sync::{broadcast, mpsc, oneshot},
};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

use crate::{
    headers::{
        header_value_to_list_with_kind, parse_write_or_end_args, throw_coded,
        throw_content_length_mismatch, throw_headers_sent, validate_header_name,
        validate_header_name_str, validate_header_value, validate_header_value_str, ErrorKind,
        HeaderStore,
    },
    io::{HttpServerIo, WriteCmd},
    lenient_http1::{serve_lenient_connection, LenientClientError, LenientConnConfig},
};

pub const MAX_HEADER_SIZE: usize = 16 * 1024;

pub const HTTP_METHODS: [&str; 35] = [
    "ACL",
    "BIND",
    "CHECKOUT",
    "CONNECT",
    "COPY",
    "DELETE",
    "GET",
    "HEAD",
    "LINK",
    "LOCK",
    "M-SEARCH",
    "MERGE",
    "MKACTIVITY",
    "MKCALENDAR",
    "MKCOL",
    "MOVE",
    "NOTIFY",
    "OPTIONS",
    "PATCH",
    "POST",
    "PROPFIND",
    "PROPPATCH",
    "PURGE",
    "PUT",
    "QUERY",
    "REBIND",
    "REPORT",
    "SEARCH",
    "SOURCE",
    "SUBSCRIBE",
    "TRACE",
    "UNBIND",
    "UNLINK",
    "UNLOCK",
    "UNSUBSCRIBE",
];

/// QuickJS `Ctx` is not re-entrant across concurrent Hyper handlers on the same runtime.
static HTTP_JS_ROOTS: LazyLock<std::sync::Mutex<HashMap<usize, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn http_js_root_for(rt_key: usize) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = HTTP_JS_ROOTS.lock().unwrap();
    map.entry(rt_key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

tokio::task_local! {
    pub(crate) static HTTP_DISPATCH_NEST: Cell<usize>;
}

pub(crate) struct HttpJsSerialGuard {
    _root: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl HttpJsSerialGuard {
    pub(crate) async fn acquire(rt_key: usize) -> Self {
        let nested = HTTP_DISPATCH_NEST.try_with(|cell| cell.get()).unwrap_or(0);
        let root = if nested == 0 {
            Some(http_js_root_for(rt_key).lock_owned().await)
        } else {
            None
        };
        let _ = HTTP_DISPATCH_NEST.try_with(|cell| cell.set(nested + 1));
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

pub(crate) fn build_incoming_headers_js<'js, I>(
    ctx: &Ctx<'js>,
    pairs: I,
) -> Result<(Object<'js>, Vec<String>)>
where
    I: IntoIterator<Item = (String, String)>,
{
    let headers_obj = Object::new(ctx.clone())?;
    let mut raw = Vec::new();
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for (name, value) in pairs {
        raw.push(name.clone());
        raw.push(value.clone());
        grouped
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(value);
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

pub(crate) fn create_server<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, Server<'js>>> {
    Server::new(ctx, args)
}

enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
    #[cfg(windows)]
    NamedPipe {
        path: String,
        server: NamedPipeServer,
    },
}

enum AcceptedStream {
    Tcp(tokio::net::TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    NamedPipe(NamedPipeServer),
}

impl AcceptedStream {
    fn drop_info<'js>(&self, ctx: &Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx.clone())?;
        if let Self::Tcp(stream) = self {
            if let Ok(local) = stream.local_addr() {
                obj.set("localAddress", local.ip().to_string())?;
                obj.set("localPort", local.port())?;
                obj.set("localFamily", if local.is_ipv4() { "IPv4" } else { "IPv6" })?;
            }
            if let Ok(remote) = stream.peer_addr() {
                obj.set("remoteAddress", remote.ip().to_string())?;
                obj.set("remotePort", remote.port())?;
                obj.set(
                    "remoteFamily",
                    if remote.is_ipv4() { "IPv4" } else { "IPv6" },
                )?;
            }
        }
        Ok(obj)
    }
}

pub(crate) enum ResponseFrame {
    Data(Bytes),
    Trailers(HeaderMap),
}

pub(crate) struct RawResponseHead {
    pub(crate) status_code: u16,
    pub(crate) status_message: Option<String>,
    pub(crate) headers: HeaderStore,
    pub(crate) body_rx: mpsc::UnboundedReceiver<ResponseFrame>,
    pub(crate) is_head: bool,
}

pub(crate) enum ResponseSink {
    Hyper(oneshot::Sender<HyperResponse<BoxBody<Bytes, Infallible>>>),
    Raw(oneshot::Sender<RawResponseHead>),
}

impl Listener {
    async fn accept(&mut self) -> std::io::Result<AcceptedStream> {
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
            #[cfg(windows)]
            Self::NamedPipe { path, server } => {
                server.connect().await?;
                let next = NamedPipeServerOptions::new().create(path.as_str())?;
                let connected = std::mem::replace(server, next);
                Ok(AcceptedStream::NamedPipe(connected))
            },
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
    aborted: bool,
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
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, socket: Opt<Value<'js>>) -> Result<Self> {
        let sock = socket.0.unwrap_or_else(|| Value::new_null(ctx.clone()));
        Ok(Self {
            emitter: EventEmitter::new(),
            method: String::new(),
            url: String::new(),
            headers: Object::new(ctx.clone())?,
            raw_headers: Vec::new(),
            trailers: Object::new(ctx)?,
            raw_trailers: Vec::new(),
            body: Vec::new(),
            complete: false,
            paused: false,
            destroyed: false,
            aborted: false,
            socket: sock,
            response_status_code: None,
            response_status_message: None,
        })
    }

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
    fn http_version_major(&self) -> u8 {
        1
    }
    #[qjs(get, enumerable)]
    fn http_version_minor(&self) -> u8 {
        1
    }
    #[qjs(get, enumerable)]
    fn complete(&self) -> bool {
        self.complete
    }
    #[qjs(get, enumerable)]
    fn aborted(&self) -> bool {
        self.aborted
    }
    #[qjs(get, enumerable)]
    fn socket(&self) -> Value<'js> {
        self.socket.clone()
    }
    #[qjs(get, enumerable)]
    fn connection(&self) -> Value<'js> {
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
    pub fn set_timeout(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        _msecs: Opt<Value<'js>>,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        if let Some(cb) = callback.0 {
            Self::add_event_listener_str(this.0.clone(), &ctx, "timeout", cb, true, false)?;
        }
        Ok(this.0)
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
    pub(crate) fn from_server_request(
        ctx: &Ctx<'js>,
        method: String,
        url: String,
        headers: Object<'js>,
        raw_headers: Vec<String>,
        socket: Value<'js>,
    ) -> Result<Self> {
        Ok(Self {
            emitter: EventEmitter::new(),
            method,
            url,
            headers,
            raw_headers,
            trailers: Object::new(ctx.clone())?,
            raw_trailers: Vec::new(),
            body: Vec::new(),
            complete: false,
            paused: false,
            destroyed: false,
            aborted: false,
            socket,
            response_status_code: None,
            response_status_message: None,
        })
    }

    pub(crate) fn from_client_response(
        ctx: &Ctx<'js>,
        status_code: u16,
        status_message: Option<String>,
        headers: Object<'js>,
        raw_headers: Vec<String>,
        socket: Value<'js>,
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
            aborted: false,
            socket,
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

    pub(crate) fn is_paused(&self) -> bool {
        self.paused
    }

    pub(crate) fn is_destroyed(&self) -> bool {
        self.destroyed
    }

    pub(crate) fn body_len(&self) -> usize {
        self.body.len()
    }

    pub(crate) fn push_trailer(&mut self, name: &str, value: String) {
        self.raw_trailers.push(name.to_string());
        self.raw_trailers.push(value.clone());
        let _ = self.trailers.set(name, value);
    }
}

#[rquickjs::class]
pub struct OutgoingMessage<'js> {
    emitter: EventEmitter<'js>,
    headers: HeaderStore,
    headers_sent: bool,
    writable_finished: bool,
    destroyed: bool,
    strict_content_length: bool,
    written_body_bytes: usize,
    trailers: Option<HeaderMap>,
    socket: Value<'js>,
}

unsafe impl<'js> JsLifetime<'js> for OutgoingMessage<'js> {
    type Changed<'to> = OutgoingMessage<'to>;
}

impl<'js> Trace<'js> for OutgoingMessage<'js> {
    fn trace<'a>(&self, t: Tracer<'a, 'js>) {
        self.emitter.trace(t);
        self.socket.trace(t);
    }
}

impl<'js> Emitter<'js> for OutgoingMessage<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> OutgoingMessage<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>) -> Self {
        Self {
            emitter: EventEmitter::new(),
            headers: HeaderStore::default(),
            headers_sent: false,
            writable_finished: false,
            destroyed: false,
            strict_content_length: false,
            written_body_bytes: 0,
            trailers: None,
            socket: Value::new_null(ctx),
        }
    }

    #[qjs(get, enumerable)]
    fn headers_sent(&self) -> bool {
        self.headers_sent
    }

    #[qjs(get, rename = "_header")]
    fn header_flag(&self) -> Option<bool> {
        self.headers_sent.then_some(true)
    }

    #[qjs(rename = "_implicitHeader")]
    fn implicit_header(&mut self) {
        self.headers_sent = true;
    }

    #[qjs(get, enumerable)]
    fn strict_content_length(&self) -> bool {
        self.strict_content_length
    }

    #[qjs(set, rename = "strictContentLength")]
    fn set_strict_content_length(&mut self, value: bool) {
        self.strict_content_length = value;
    }

    #[qjs(get, enumerable)]
    fn writable_finished(&self) -> bool {
        self.writable_finished
    }

    #[qjs(get, enumerable)]
    fn writable_ended(&self) -> bool {
        self.writable_finished
    }

    #[qjs(get, enumerable)]
    fn finished(&self) -> bool {
        self.writable_finished
    }

    #[qjs(get, enumerable)]
    fn destroyed(&self) -> bool {
        self.destroyed
    }

    #[qjs(get, enumerable)]
    fn socket(&self) -> Value<'js> {
        self.socket.clone()
    }

    #[qjs(get, enumerable)]
    fn connection(&self) -> Value<'js> {
        self.socket.clone()
    }

    pub fn set_header(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        name: String,
        value: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent {
            return Err(throw_headers_sent(&ctx, "set"));
        }
        validate_header_name_str(&ctx, &name)?;
        let (values, is_array) = header_value_to_list_with_kind(&ctx, &name, value)?;
        this.borrow_mut().headers.set(&name, values, is_array);
        Ok(this.0)
    }

    pub fn append_header(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        name: String,
        value: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent {
            return Err(throw_headers_sent(&ctx, "append"));
        }
        validate_header_name_str(&ctx, &name)?;
        let (values, is_array) = header_value_to_list_with_kind(&ctx, &name, value)?;
        this.borrow_mut().headers.append(&name, values, is_array);
        Ok(this.0)
    }

    pub fn get_header(&self, ctx: Ctx<'js>, name: String) -> Result<Value<'js>> {
        self.headers.get_js(&ctx, &name)
    }

    pub fn has_header(&self, name: String) -> bool {
        self.headers.has(&name)
    }

    pub fn get_header_names(&self) -> Vec<String> {
        self.headers.get_header_names()
    }

    pub fn get_raw_header_names(&self) -> Vec<String> {
        self.headers.get_raw_header_names()
    }

    pub fn get_headers(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        self.headers.get_headers_object(&ctx)
    }

    pub fn set_headers(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        headers: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent {
            return Err(throw_headers_sent(&ctx, "set"));
        }
        this.borrow_mut().headers.set_from_js(&ctx, headers)?;
        Ok(this.0)
    }

    pub fn remove_header(&mut self, ctx: Ctx<'js>, name: String) -> Result<()> {
        if self.headers_sent {
            return Err(throw_headers_sent(&ctx, "remove"));
        }
        self.headers.remove(&name);
        Ok(())
    }

    pub fn flush_headers(&mut self) {
        self.headers_sent = true;
    }

    pub fn add_trailers(&mut self, trailers: Object<'js>) -> Result<()> {
        let map = parse_trailers_object(trailers)?;
        self.trailers = Some(map);
        Ok(())
    }

    pub fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<bool> {
        let (bytes, cb) = parse_write_or_end_args(&ctx, &args.0, false)?;
        let bytes = bytes.unwrap_or_default();
        let mut msg = this.borrow_mut();
        if msg.strict_content_length {
            if let Some(expected) = msg.headers.get_content_length() {
                let next = msg.written_body_bytes.saturating_add(bytes.len());
                if next > expected {
                    return Err(throw_content_length_mismatch(&ctx, next, expected));
                }
            }
        }
        msg.headers_sent = true;
        msg.written_body_bytes = msg.written_body_bytes.saturating_add(bytes.len());
        drop(msg);
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
        let add_len = bytes.as_ref().map(|b| b.len()).unwrap_or(0);
        {
            let mut msg = this.borrow_mut();
            if msg.writable_finished || msg.destroyed {
                drop(msg);
                if let Some(cb) = cb {
                    let _ = cb.defer::<()>(());
                }
                return Ok(this.0);
            }
            if msg.strict_content_length {
                if let Some(expected) = msg.headers.get_content_length() {
                    let total = msg.written_body_bytes.saturating_add(add_len);
                    if total != expected {
                        return Err(throw_content_length_mismatch(&ctx, total, expected));
                    }
                }
            }
            msg.headers_sent = true;
            msg.written_body_bytes = msg.written_body_bytes.saturating_add(add_len);
            msg.writable_finished = true;
        }
        let _ = OutgoingMessage::emit_str(this.0.clone(), &ctx, "finish", vec![], false);
        if let Some(cb) = cb {
            cb.call::<_, ()>(())?;
        }
        Ok(this.0)
    }

    pub fn set_timeout(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        _msecs: Opt<Value<'js>>,
        cb: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        if let Some(cb) = cb.0 {
            Self::add_event_listener_str(this.0.clone(), &ctx, "timeout", cb, true, false)?;
        }
        Ok(this.0)
    }

    pub fn destroy(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        _err: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let already = {
            let mut msg = this.borrow_mut();
            if msg.destroyed {
                true
            } else {
                msg.destroyed = true;
                msg.writable_finished = true;
                false
            }
        };
        if !already {
            let _ = OutgoingMessage::emit_str(this.0.clone(), &ctx, "close", vec![], false);
        }
        Ok(this.0)
    }
}

fn parse_trailers_object<'js>(trailers: Object<'js>) -> Result<HeaderMap> {
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
    Ok(result)
}

#[rquickjs::class]
pub struct ServerResponse<'js> {
    emitter: EventEmitter<'js>,
    status_code: u16,
    status_message: Option<String>,
    headers: HeaderStore,
    headers_sent: bool,
    /// Mirrors Node `res.writableFinished` — true after `end()` completes.
    writable_finished: bool,
    /// Mirrors Node `res.destroyed`.
    destroyed: bool,
    /// Node OutgoingMessage internal: truthy once headers have been written.
    /// Compression middleware checks `if (!this._header) this._implicitHeader()`.
    header_sent_flag: bool,
    strict_content_length: bool,
    written_body_bytes: usize,
    sink: Option<ResponseSink>,
    body_tx: Option<mpsc::UnboundedSender<ResponseFrame>>,
    write_tx: Option<mpsc::UnboundedSender<WriteCmd>>,
    trailers: Option<HeaderMap>,
    continue_requested: bool,
    /// True when the request was a HEAD: Node never sends a body for it, and
    /// must not advertise a `Content-Length` for one either.
    is_head: bool,
    /// True once `write()` has queued body bytes. After that the framing is
    /// already committed (chunked), so `end()` must not add `Content-Length`.
    body_started: bool,
    req_value: Value<'js>,
    socket_value: Value<'js>,
    diag_server: Option<Value<'js>>,
    track_finish_diag: bool,
}

unsafe impl<'js> JsLifetime<'js> for ServerResponse<'js> {
    type Changed<'to> = ServerResponse<'to>;
}

impl<'js> Trace<'js> for ServerResponse<'js> {
    fn trace<'a>(&self, t: Tracer<'a, 'js>) {
        self.emitter.trace(t);
        self.req_value.trace(t);
        self.socket_value.trace(t);
        if let Some(server) = &self.diag_server {
            server.trace(t);
        }
    }
}

impl<'js> Emitter<'js> for ServerResponse<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

impl<'js> ServerResponse<'js> {
    pub(crate) fn new_internal(
        sink: ResponseSink,
        write_tx: Option<mpsc::UnboundedSender<WriteCmd>>,
        is_head: bool,
        req_value: Value<'js>,
        socket_value: Value<'js>,
        diag_server: Option<Value<'js>>,
        track_finish_diag: bool,
    ) -> Self {
        Self {
            emitter: EventEmitter::new(),
            status_code: StatusCode::OK.as_u16(),
            status_message: None,
            headers: HeaderStore::default(),
            headers_sent: false,
            writable_finished: false,
            destroyed: false,
            header_sent_flag: false,
            strict_content_length: false,
            written_body_bytes: 0,
            sink: Some(sink),
            body_tx: None,
            write_tx,
            trailers: None,
            continue_requested: false,
            is_head,
            body_started: false,
            req_value,
            socket_value,
            diag_server,
            track_finish_diag,
        }
    }

    pub(crate) fn is_bodyless(&self) -> bool {
        self.is_head
            || matches!(self.status_code, 204 | 304)
            || (100..200).contains(&self.status_code)
    }

    pub(crate) fn set_default_header(&mut self, name: &str, value: String) {
        if !self.headers.has(name) {
            self.headers.set(name, vec![value], false);
        }
    }

    pub(crate) fn has_continue_requested(&self) -> bool {
        self.continue_requested
    }

    pub(crate) fn is_headers_sent(&self) -> bool {
        self.headers_sent
    }

    fn start(&mut self) {
        if let Some(sink) = self.sink.take() {
            let (body_tx, body_rx) = mpsc::unbounded_channel();
            match sink {
                ResponseSink::Hyper(tx) => {
                    let mut builder = HyperResponse::builder().status(self.status_code);
                    if let Some(message) = &self.status_message {
                        if let Ok(message) = hyper::ext::ReasonPhrase::try_from(message.clone()) {
                            builder = builder.extension(message);
                        }
                    }
                    for (name, value) in self.headers.iter_pairs() {
                        builder = builder.header(name, value);
                    }
                    let body =
                        StreamBody::new(UnboundedReceiverStream::new(body_rx).map(|frame| {
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
                },
                ResponseSink::Raw(tx) => {
                    let _ = tx.send(RawResponseHead {
                        status_code: self.status_code,
                        status_message: self.status_message.clone(),
                        headers: self.headers.clone(),
                        body_rx,
                        is_head: self.is_head,
                    });
                },
            }
            self.body_tx = Some(body_tx);
            self.write_tx.take();
            self.headers_sent = true;
            self.header_sent_flag = true;
        } else {
            self.write_tx.take();
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
        self.write_tx.take();
        self.writable_finished = true;
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> ServerResponse<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, req: Opt<Value<'js>>) -> Result<Self> {
        let req_val = req.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let is_head = req_val
            .as_object()
            .and_then(|o| o.get_optional::<_, String>("method").ok().flatten())
            .is_some_and(|m| m.eq_ignore_ascii_case("HEAD"));
        let socket_value = req_val
            .as_object()
            .and_then(|o| o.get_optional::<_, Value>("socket").ok().flatten())
            .unwrap_or_else(|| Value::new_null(ctx.clone()));
        Ok(Self {
            emitter: EventEmitter::new(),
            status_code: StatusCode::OK.as_u16(),
            status_message: None,
            headers: HeaderStore::default(),
            headers_sent: false,
            writable_finished: false,
            destroyed: false,
            header_sent_flag: false,
            strict_content_length: false,
            written_body_bytes: 0,
            sink: None,
            body_tx: None,
            write_tx: None,
            trailers: None,
            continue_requested: false,
            is_head,
            body_started: false,
            req_value: req_val,
            socket_value,
            diag_server: None,
            track_finish_diag: false,
        })
    }

    #[qjs(get, enumerable)]
    fn status_code(&self) -> u16 {
        self.status_code
    }
    #[qjs(set, rename = "statusCode")]
    fn set_status_code(&mut self, value: u16) {
        if !self.headers_sent {
            self.status_code = value;
        }
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
    #[qjs(get, enumerable)]
    fn strict_content_length(&self) -> bool {
        self.strict_content_length
    }
    #[qjs(set, rename = "strictContentLength")]
    fn set_strict_content_length(&mut self, value: bool) {
        self.strict_content_length = value;
    }
    #[qjs(get, enumerable)]
    fn req(&self) -> Value<'js> {
        self.req_value.clone()
    }
    #[qjs(get, enumerable)]
    fn socket(&self) -> Value<'js> {
        self.socket_value.clone()
    }
    #[qjs(get, enumerable)]
    fn connection(&self) -> Value<'js> {
        self.socket_value.clone()
    }

    pub fn assign_socket(&mut self, socket: Value<'js>) {
        self.socket_value = socket;
    }

    pub fn detach_socket(&mut self, ctx: Ctx<'js>, _socket: Opt<Value<'js>>) {
        self.socket_value = Value::new_null(ctx);
    }

    /// Node `res.setHeader(name, value)` accepts string, number, or string[].
    pub fn set_header(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        name: String,
        value: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent {
            return Err(throw_headers_sent(&ctx, "set"));
        }
        validate_header_name_str(&ctx, &name)?;
        let (values, is_array) = header_value_to_list_with_kind(&ctx, &name, value)?;
        this.borrow_mut().headers.set(&name, values, is_array);
        Ok(this.0)
    }

    /// Node `res.appendHeader(name, value)` — adds without replacing prior values.
    pub fn append_header(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        name: String,
        value: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent {
            return Err(throw_headers_sent(&ctx, "append"));
        }
        validate_header_name_str(&ctx, &name)?;
        let (values, is_array) = header_value_to_list_with_kind(&ctx, &name, value)?;
        this.borrow_mut().headers.append(&name, values, is_array);
        Ok(this.0)
    }

    pub fn get_header(&self, ctx: Ctx<'js>, name: String) -> Result<Value<'js>> {
        self.headers.get_js(&ctx, &name)
    }

    pub fn has_header(&self, name: String) -> bool {
        self.headers.has(&name)
    }

    pub fn get_header_names(&self) -> Vec<String> {
        self.headers.get_header_names()
    }

    pub fn get_raw_header_names(&self) -> Vec<String> {
        self.headers.get_raw_header_names()
    }

    pub fn get_headers(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        self.headers.get_headers_object(&ctx)
    }

    pub fn set_headers(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        headers: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent {
            return Err(throw_headers_sent(&ctx, "set"));
        }
        this.borrow_mut().headers.set_from_js(&ctx, headers)?;
        Ok(this.0)
    }

    pub fn remove_header(&mut self, ctx: Ctx<'js>, name: String) -> Result<()> {
        if self.headers_sent {
            return Err(throw_headers_sent(&ctx, "remove"));
        }
        self.headers.remove(&name);
        Ok(())
    }

    pub fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<bool> {
        let (bytes, cb) = parse_write_or_end_args(&ctx, &args.0, false)?;
        let bytes = bytes.unwrap_or_default();
        let sent = {
            let mut res = this.borrow_mut();
            if res.writable_finished || res.destroyed {
                false
            } else {
                if res.strict_content_length && !res.is_bodyless() {
                    if let Some(expected) = res.headers.get_content_length() {
                        let next = res.written_body_bytes.saturating_add(bytes.len());
                        if next > expected {
                            return Err(throw_content_length_mismatch(&ctx, next, expected));
                        }
                    }
                }
                res.written_body_bytes = res.written_body_bytes.saturating_add(bytes.len());
                res.body_started = true;
                res.start();
                res.body_tx
                    .as_ref()
                    .is_some_and(|tx| tx.send(ResponseFrame::Data(Bytes::from(bytes))).is_ok())
            }
        };
        if let Some(cb) = cb {
            let _ = cb.defer::<()>(());
        }
        Ok(sent)
    }

    pub fn end(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let (raw_bytes, cb) = parse_write_or_end_args(&ctx, &args.0, true)?;
        let (diag_request, diag_socket, diag_server, track_finish) = {
            let mut res = this.borrow_mut();
            if res.writable_finished || res.destroyed {
                drop(res);
                if let Some(cb) = cb {
                    let _ = cb.defer::<()>(());
                }
                return Ok(this.0);
            }
            let bytes = raw_bytes.map(Bytes::from);
            let chunk_len = bytes.as_ref().map(|b| b.len()).unwrap_or(0);
            if res.strict_content_length && !res.is_bodyless() {
                if let Some(expected) = res.headers.get_content_length() {
                    let total = res.written_body_bytes.saturating_add(chunk_len);
                    if total != expected {
                        return Err(throw_content_length_mismatch(&ctx, total, expected));
                    }
                }
            }
            res.written_body_bytes = res.written_body_bytes.saturating_add(chunk_len);
            // Node decides the framing here: if headers are still unsent, nothing
            // has been written yet, and the body length is known, it emits an
            // exact `Content-Length` instead of degrading to chunked. A response
            // that already started streaming (`write()` called) or one that must
            // not carry a body (HEAD, 204, 304) keeps the existing behaviour.
            if !res.headers_sent
                && !res.body_started
                && !res.is_bodyless()
                && !res.headers.has("content-length")
            {
                let len = bytes.as_ref().map(|b| b.len()).unwrap_or(0);
                res.headers
                    .set("content-length", vec![len.to_string()], false);
            }
            res.start();
            if let Some(bytes) = bytes {
                if let Some(tx) = &res.body_tx {
                    let _ = tx.send(ResponseFrame::Data(bytes));
                }
            }
            res.finish();
            (
                res.track_finish_diag.then(|| res.req_value.clone()),
                res.track_finish_diag.then(|| res.socket_value.clone()),
                res.diag_server.take(),
                res.track_finish_diag,
            )
        };
        // Node emits `finish` after the response has been fully handed off.
        // Next's pipe-readable waits on this event before completing pipeTo.
        let _ = ServerResponse::emit_str(this.0.clone(), &ctx, "finish", vec![], false);
        if let Some(cb) = cb {
            cb.call::<_, ()>(())?;
        }
        if track_finish {
            let _ = lexe_utils::diagnostics::publish(&ctx, "http.server.response.finish", |ctx| {
                let msg = Object::new(ctx.clone())?;
                if let Some(req) = diag_request {
                    msg.set("request", req)?;
                }
                msg.set("response", this.0.clone().into_value())?;
                if let Some(sock) = diag_socket {
                    msg.set("socket", sock)?;
                }
                if let Some(srv) = diag_server {
                    msg.set("server", srv)?;
                }
                Ok(msg)
            });
        }
        let _ = ServerResponse::emit_str(this.0.clone(), &ctx, "close", vec![], false);
        Ok(this.0)
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
    #[qjs(get, enumerable)]
    fn finished(&self) -> bool {
        self.writable_finished
    }

    pub fn set_timeout(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        _msecs: Opt<Value<'js>>,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        if let Some(cb) = callback.0 {
            Self::add_event_listener_str(this.0.clone(), &ctx, "timeout", cb, true, false)?;
        }
        Ok(this.0)
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
                res.sink.take();
                res.diag_server.take();
                false
            }
        };
        if !already {
            let _ = ServerResponse::emit_str(this.0.clone(), &ctx, "close", vec![], false);
        }
        Ok(this.0)
    }

    pub fn add_trailers(&mut self, trailers: Object<'js>) -> Result<()> {
        self.trailers = Some(parse_trailers_object(trailers)?);
        Ok(())
    }

    pub fn write_head(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().headers_sent {
            return Err(throw_headers_sent(&ctx, "render"));
        }
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
                validate_header_name_str(&ctx, &name)?;
                let value: Value = headers.get(&name)?;
                let (values, is_array) = header_value_to_list_with_kind(&ctx, &name, value)?;
                response.headers.set(&name, values, is_array);
            }
        }
        response.start();
        drop(response);
        Ok(this.0)
    }

    pub fn write_header(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        Self::write_head(this, ctx, args)
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

    pub fn write_processing(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        cb: Opt<Value<'js>>,
    ) -> Result<()> {
        let cb_fn = cb.0.and_then(|v| v.into_function());
        let write_tx = {
            let res = this.borrow();
            if res.headers_sent || res.destroyed {
                None
            } else {
                res.write_tx.clone()
            }
        };
        if let Some(write_tx) = write_tx {
            let _ = write_tx.send(WriteCmd::Bytes(b"HTTP/1.1 102 Processing\r\n\r\n".to_vec()));
            if let Some(cb_fn) = cb_fn {
                let (ack_tx, ack_rx) = oneshot::channel();
                let _ = write_tx.send(WriteCmd::Flush(ack_tx));
                ctx.spawn_exit_simple(async move {
                    let _ = ack_rx.await;
                    let _ = cb_fn.call::<_, ()>(());
                    Ok(())
                });
            }
        } else if let Some(cb_fn) = cb_fn {
            let _ = cb_fn.defer::<()>(());
        }
        Ok(())
    }

    pub fn write_early_hints(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        hints: Value<'js>,
        cb: Opt<Value<'js>>,
    ) -> Result<()> {
        let cb_fn = cb.0.and_then(|v| v.into_function());
        if hints.is_null() || !hints.is_object() || hints.is_array() {
            let received = if hints.is_null() {
                "null"
            } else {
                hints.type_name()
            };
            return Err(throw_coded(
                &ctx,
                ErrorKind::Type,
                "ERR_INVALID_ARG_TYPE",
                &format!("The \"hints\" argument must be of type object. Received {received}"),
            ));
        }
        let hints_obj = hints.as_object().expect("hints checked object");
        let link_val: Value = hints_obj.get("link")?;
        if link_val.is_undefined() || link_val.is_null() {
            return Ok(());
        }
        let link_header = if let Some(s) = link_val.as_string() {
            let s = s.to_string()?;
            validate_header_value_str(&ctx, "link", &s)?;
            s
        } else if let Some(arr) = link_val.as_array() {
            let mut parts = Vec::with_capacity(arr.len());
            for item in arr.iter::<Value>() {
                let item = item?;
                let Some(s) = item.as_string() else {
                    return Err(throw_coded(
                        &ctx,
                        ErrorKind::Type,
                        "ERR_INVALID_ARG_VALUE",
                        "The argument 'hints' must be an array or string of format \"</styles.css>; rel=preload; as=style\".",
                    ));
                };
                let s = s.to_string()?;
                validate_header_value_str(&ctx, "link", &s)?;
                parts.push(s);
            }
            parts.join(", ")
        } else {
            return Err(throw_coded(
                &ctx,
                ErrorKind::Type,
                "ERR_INVALID_ARG_VALUE",
                "The argument 'hints' must be an array or string of format \"</styles.css>; rel=preload; as=style\".",
            ));
        };

        if link_header.is_empty() {
            return Ok(());
        }

        let mut head = format!("HTTP/1.1 103 Early Hints\r\nLink: {link_header}\r\n");
        for key in hints_obj.keys::<String>() {
            let key = key?;
            if key == "link" {
                continue;
            }
            validate_header_name_str(&ctx, &key)?;
            let val: Value = hints_obj.get(&key)?;
            if val.is_undefined() {
                continue;
            }
            let (values, _) = header_value_to_list_with_kind(&ctx, &key, val)?;
            for v in values {
                head.push_str(&key);
                head.push_str(": ");
                head.push_str(&v);
                head.push_str("\r\n");
            }
        }
        head.push_str("\r\n");

        let write_tx = {
            let res = this.borrow();
            if res.headers_sent || res.destroyed {
                None
            } else {
                res.write_tx.clone()
            }
        };
        if let Some(write_tx) = write_tx {
            let _ = write_tx.send(WriteCmd::Bytes(head.into_bytes()));
            if let Some(cb_fn) = cb_fn {
                let (ack_tx, ack_rx) = oneshot::channel();
                let _ = write_tx.send(WriteCmd::Flush(ack_tx));
                ctx.spawn_exit_simple(async move {
                    let _ = ack_rx.await;
                    let _ = cb_fn.call::<_, ()>(());
                    Ok(())
                });
            }
        } else if let Some(cb_fn) = cb_fn {
            let _ = cb_fn.defer::<()>(());
        }
        Ok(())
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
    max_requests_per_socket: usize,
    insecure_http_parser: bool,
    max_connections: Value<'js>,
    headers_timeout: u64,
    request_timeout: u64,
    keep_alive_timeout: u64,
    /// Node's `server.timeout`: per-socket inactivity limit in ms, `0` disables.
    /// Stored as the raw JS number so `setTimeout` keeps values like `-5` or
    /// `"50"` untouched, matching Node, which does not coerce or clamp.
    timeout: Value<'js>,
    loop_ref: Arc<EventLoopRef>,
}

unsafe impl<'js> JsLifetime<'js> for Server<'js> {
    type Changed<'to> = Server<'to>;
}

impl<'js> Trace<'js> for Server<'js> {
    fn trace<'a>(&self, t: Tracer<'a, 'js>) {
        self.emitter.trace(t);
        self.address.trace(t);
        self.max_connections.trace(t);
        self.timeout.trace(t);
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
    pub fn new(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Class<'js, Self>> {
        let mut server_options: Option<Object<'js>> = None;
        for value in &args.0 {
            if let Some(option) = value.as_object().filter(|_| value.as_function().is_none()) {
                server_options = Some(option.clone());
            }
        }
        let listener = args
            .0
            .into_iter()
            .rev()
            .find_map(|value| value.into_function());

        let (close_tx, _) = broadcast::channel(1);
        let (connection_close_tx, _) = broadcast::channel(1);
        let (idle_close_tx, _) = broadcast::channel(1);
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);
        let mut state = Self {
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
            max_header_size: MAX_HEADER_SIZE,
            max_request_body_size: 16 * 1024 * 1024,
            max_requests_per_socket: 0,
            insecure_http_parser: false,
            max_connections: Undefined.into_value(ctx.clone()),
            headers_timeout: 60_000,
            request_timeout: 300_000,
            keep_alive_timeout: 5_000,
            timeout: 0i32.into_js(&ctx)?,
            loop_ref,
        };
        if let Some(options) = server_options {
            if let Some(val) = options.get_optional::<_, Value>("maxHeadersCount")? {
                if val.is_null() {
                    state.max_headers_count = 0;
                } else if let Some(n) = val.as_number() {
                    if n >= 0.0 {
                        state.max_headers_count = n as usize;
                    }
                }
            }
            if let Some(value) = options.get_optional("headersTimeout")? {
                state.headers_timeout = value;
            }
            if let Some(value) = options.get_optional("requestTimeout")? {
                state.request_timeout = value;
            }
            if let Some(value) = options.get_optional("keepAliveTimeout")? {
                state.keep_alive_timeout = value;
            }
            if let Some(value) = options.get_optional("maxHeaderSize")? {
                state.max_header_size = value;
            }
            if let Some(value) = options.get_optional("maxBodySize")? {
                state.max_request_body_size = value;
            }
            if let Some(value) = options.get_optional("maxRequestsPerSocket")? {
                state.max_requests_per_socket = value;
            }
            if let Some(value) = options.get_optional("insecureHTTPParser")? {
                state.insecure_http_parser = value;
            }
            if let Some(value) = options.get_optional::<_, Value>("maxConnections")? {
                if !value.is_undefined() {
                    state.max_connections = value;
                }
            }
        }
        let instance = Class::instance(ctx.clone(), state)?;
        if let Some(listener) = listener {
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
    pub fn set_max_headers_count(&mut self, value: Value<'js>) {
        if value.is_null() {
            self.max_headers_count = 0;
        } else if let Some(n) = value.as_number() {
            if n >= 0.0 {
                self.max_headers_count = n as usize;
            }
        }
    }
    #[qjs(get, enumerable)]
    pub fn max_requests_per_socket(&self) -> usize {
        self.max_requests_per_socket
    }
    #[qjs(set, rename = "maxRequestsPerSocket")]
    pub fn set_max_requests_per_socket(&mut self, value: usize) {
        self.max_requests_per_socket = value;
    }
    #[qjs(get, enumerable, rename = "insecureHTTPParser")]
    pub fn insecure_http_parser(&self) -> bool {
        self.insecure_http_parser
    }
    #[qjs(set, rename = "insecureHTTPParser")]
    pub fn set_insecure_http_parser(&mut self, value: bool) {
        self.insecure_http_parser = value;
    }
    #[qjs(get, enumerable)]
    pub fn max_connections(&self) -> Value<'js> {
        self.max_connections.clone()
    }
    #[qjs(set, rename = "maxConnections")]
    pub fn set_max_connections(&mut self, value: Value<'js>) {
        self.max_connections = value;
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
    #[qjs(get, enumerable)]
    pub fn timeout(&self) -> Value<'js> {
        self.timeout.clone()
    }
    #[qjs(set, rename = "timeout")]
    pub fn set_timeout_value(&mut self, value: Value<'js>) {
        self.timeout = value;
    }
    /// `server.setTimeout(msecs[, callback])` returns the server itself, like
    /// Node. A callback is registered as a `'timeout'` listener rather than
    /// stored separately, so `listenerCount("timeout")` reflects it.
    pub fn set_timeout(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        msecs: Opt<Value<'js>>,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        if let Some(value) = msecs.0 {
            this.borrow_mut().timeout = value;
        }
        if let Some(callback) = callback.0 {
            Self::add_event_listener_str(this.0.clone(), &ctx, "timeout", callback, false, false)?;
        }
        Ok(this.0)
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
        // Node reports `close()` on a server that never started by invoking the
        // callback with ERR_SERVER_NOT_RUNNING, not by throwing.
        if !this.borrow().listening.load(Ordering::Relaxed)
            && !this.borrow().closing.load(Ordering::Relaxed)
        {
            // The constructor takes the loop reference, so a server that was
            // created but never listened still has to release it or the
            // process never drains.
            this.borrow().loop_ref.unreference();
            let error = not_running_error(&ctx)?;
            if let Some(callback) = callback.0 {
                callback.call::<_, ()>((error,))?;
            }
            return Ok(this.0);
        }

        let callback = callback.0;
        if let Some(callback) = callback.as_ref() {
            Self::add_event_listener_str(
                this.0.clone(),
                &ctx,
                "close",
                callback.clone(),
                true,
                true,
            )?;
        }

        let mut server = this.borrow_mut();
        // Node flips `listening` synchronously inside `close()`, so code that
        // polls it right after the call sees false even though the accept loop
        // has not unwound yet.
        server.listening.store(false, Ordering::Relaxed);
        server.address = Value::new_null(ctx.clone());
        let already_closing = server.closing.swap(true, Ordering::Relaxed);
        // The listening socket keeps the runtime referenced for as long as the
        // server is open (see `listen`). `close()` has to drop that reference,
        // or the event loop stays alive after the socket is gone and the
        // process never exits - while the `close` callback has already fired,
        // so it looks like close succeeded.
        server.loop_ref.unreference();
        let _ = server.close_tx.send(());
        // Idle keep-alive connections would otherwise sit in their select until
        // the keep-alive timer expires, holding `connections` above zero and
        // keeping the close callback from ever running. Connections with a
        // request in flight are left alone: Node waits for those and only lets
        // the connection go once the client ends it.
        let _ = server.idle_close_tx.send(());
        drop(server);

        if already_closing {
            // A second `close()` while one is pending reports NOT_RUNNING to its
            // own callback but must not emit a second `close` event.
            if let Some(callback) = callback.as_ref() {
                let error = not_running_error(&ctx)?;
                callback.call::<_, ()>((error,))?;
            }
            return Ok(this.0);
        }

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
            let (mut listener, address) = if let Some(path) = path {
                #[cfg(unix)]
                {
                    let listener = UnixListener::bind(&path)
                        .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                    (Listener::Unix(listener), path.into_js(&ctx2)?)
                }
                #[cfg(windows)]
                {
                    let pipe = NamedPipeServerOptions::new()
                        .first_pipe_instance(true)
                        .create(&path)
                        .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                    let addr = path.clone().into_js(&ctx2)?;
                    (Listener::NamedPipe { path, server: pipe }, addr)
                }
                #[cfg(all(not(unix), not(windows)))]
                {
                    let _ = path;
                    return Err(Exception::throw_type(
                        &ctx2,
                        "Unix domain sockets are not supported on this platform",
                    ));
                }
            } else {
                let listener = TcpListener::bind(format!("{host}:{port}"))
                    .await
                    .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                let addr = listener
                    .local_addr()
                    .map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                let address = Object::new(ctx2.clone())?;
                address.set("address", addr.ip().to_string())?;
                address.set("port", addr.port())?;
                address.set("family", if addr.is_ipv4() { "IPv4" } else { "IPv6" })?;
                (Listener::Tcp(listener), address.into_value())
            };
            server.borrow_mut().address = address;
            server.borrow().listening.store(true, Ordering::Relaxed);
            Self::emit_str(server.clone(), &ctx2, "listening", vec![], false)?;
            loop {
                if !lexe_utils::event_loop::keeps_event_loop_alive(&listen_loop_ref) {
                    break;
                }
                tokio::select! {
                    _ = lexe_utils::event_loop::wait_until_io_unref(listen_loop_ref.clone()) => {
                        break;
                    }
                    _ = close.recv() => break,
                    accepted = listener.accept() => {
                        let stream = accepted.map_err(|e| Exception::throw_message(&ctx2, &e.to_string()))?;
                        let drop_exceeded = {
                            let borrow = server.borrow();
                            if let Some(max) = borrow.max_connections.as_number() {
                                max.is_finite() && (borrow.connections.load(Ordering::Relaxed) as f64) >= max
                            } else {
                                false
                            }
                        };
                        if drop_exceeded {
                            let drop_info = stream.drop_info(&ctx2)?;
                            drop(stream);
                            Self::emit_str(server.clone(), &ctx2, "drop", vec![drop_info.into_value()], false)?;
                            continue;
                        }
                        let ctx3 = ctx2.clone();
                        let srv = server.clone();
                        match stream {
                            AcceptedStream::Tcp(stream) => spawn_tcp_connection(ctx3, srv, stream)?,
                            #[cfg(unix)]
                            AcceptedStream::Unix(stream) => spawn_unix_connection(ctx3, srv, stream)?,
                            #[cfg(windows)]
                            AcceptedStream::NamedPipe(pipe) => spawn_pipe_connection(ctx3, srv, pipe)?,
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

fn not_running_error<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let error = Exception::from_message(ctx.clone(), "Server is not running.")?;
    let value = error.into_value();
    if let Some(object) = value.as_object() {
        let code = rquickjs::String::from_str(ctx.clone(), "ERR_SERVER_NOT_RUNNING")?;
        object.set("code", code)?;
    }
    Ok(value)
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
    Socket::js_unref(This(socket.clone()));
    Socket::set_addresses(&socket, &ctx, &stream)?;
    Socket::mark_connected(&socket);
    let std_stream = stream
        .into_std()
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
    spawn_connection(ctx, server, socket.into_value(), stream)
}

#[cfg(unix)]
fn spawn_unix_connection<'js>(
    ctx: Ctx<'js>,
    server: Class<'js, Server<'js>>,
    stream: UnixStream,
) -> Result<()> {
    let socket = Socket::new(ctx.clone(), false)?;
    Socket::js_unref(This(socket.clone()));
    Socket::mark_connected(&socket);
    let std_stream = stream
        .into_std()
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
    spawn_connection(ctx, server, socket.into_value(), stream)
}

#[cfg(windows)]
fn spawn_pipe_connection<'js>(
    ctx: Ctx<'js>,
    server: Class<'js, Server<'js>>,
    pipe: NamedPipeServer,
) -> Result<()> {
    let socket = Socket::new(ctx.clone(), false)?;
    Socket::js_unref(This(socket.clone()));
    Socket::mark_connected(&socket);
    spawn_connection(ctx, server, socket.into_value(), pipe)
}

pub(crate) fn socket_class<'js>(
    ctx: &Ctx<'js>,
    socket: &Value<'js>,
) -> Result<Class<'js, Socket<'js>>> {
    let obj = socket
        .as_object()
        .ok_or_else(|| Exception::throw_type(ctx, "Invalid socket"))?;
    Class::<Socket>::from_object(obj)
        .ok_or_else(|| Exception::throw_type(ctx, "Expected net.Socket"))
}

pub(crate) fn attach_upgrade_writer<'js>(
    socket: &Class<'js, Socket<'js>>,
    write_tx: mpsc::UnboundedSender<WriteCmd>,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    Socket::attach_raw_writer(socket, tx);
    tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if write_tx.send(WriteCmd::Bytes(bytes)).is_err() {
                return;
            }
        }
        let (ack_tx, _ack_rx) = oneshot::channel();
        let _ = write_tx.send(WriteCmd::Shutdown(ack_tx));
    });
}

enum ConnectionOutcome {
    ClosedByServer,
    HyperResult(std::result::Result<(), hyper::Error>),
    LenientResult(std::result::Result<(), LenientClientError>),
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
    let insecure_http_parser = server_ref.insecure_http_parser;
    // `server.timeout` is kept as the raw JS value; only a positive finite
    // number arms the per-connection inactivity timer.
    let server_timeout = server_ref.timeout.as_number().unwrap_or(0.0);
    let server_timeout = if server_timeout > 0.0 {
        Duration::from_millis(server_timeout as u64)
    } else {
        Duration::ZERO
    };
    let timeout_listener = server_ref.emitter.has_listener_str("timeout");
    let timeout_server = server.clone();
    drop(server_ref);
    count.fetch_add(1, Ordering::Relaxed);
    let request_ctx = ctx.clone();
    let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let http_rt_key = event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
    let active_requests = Arc::new(AtomicUsize::new(0));
    let requests_on_socket = Arc::new(AtomicUsize::new(0));
    let (activity_tx, mut activity_rx) = mpsc::unbounded_channel();
    ctx.clone().spawn_exit_simple(async move {
        let _ = Server::emit_str(
            server.clone(),
            &ctx,
            "connection",
            vec![socket.clone()],
            false,
        );
        let (server_io, write_tx, _writer_task) = HttpServerIo::split_with_writer(stream);
        let err_write_tx = write_tx.clone();

        let idle_timeout = tokio::time::sleep(Duration::from_secs(86_400));
        tokio::pin!(idle_timeout);
        // `server.timeout` inactivity timer: armed at connect and re-armed on
        // every request, mirroring Node's per-socket `setTimeout`.
        let server_timeout_armed = !server_timeout.is_zero();
        // Start far in the future: with no `server.timeout` configured this
        // future must never be ready, or it would wake the select every tick.
        let server_idle = tokio::time::sleep(if server_timeout_armed {
            server_timeout
        } else {
            Duration::from_secs(86_400)
        });
        tokio::pin!(server_idle);
        // The `'timeout'` event carries the socket, as Node does.
        let request_socket_for_timeout = socket.clone();
        let mut keep_alive_armed = false;

        let outcome = if insecure_http_parser {
            let read_half = server_io.into_read_half();
            let lenient_conn = serve_lenient_connection(
                request_ctx,
                server.clone(),
                socket.clone(),
                read_half,
                write_tx,
                LenientConnConfig {
                    max_headers_count,
                    max_header_size,
                    max_request_body_size,
                    headers_timeout,
                    request_timeout,
                    keep_alive_timeout,
                },
                active_requests.clone(),
                activity_tx,
                requests_on_socket,
                http_rt_key,
            );
            tokio::pin!(lenient_conn);
            loop {
                tokio::select! {
                    _ = connection_close.recv() => break ConnectionOutcome::ClosedByServer,
                    _ = idle_close.recv() => {
                        if active_requests.load(Ordering::Relaxed) == 0 {
                            break ConnectionOutcome::ClosedByServer;
                        }
                    },
                    _ = &mut idle_timeout, if keep_alive_armed && keep_alive_timeout > 0 => {
                        if active_requests.load(Ordering::Relaxed) == 0 {
                            break ConnectionOutcome::ClosedByServer;
                        }
                        idle_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(keep_alive_timeout));
                    },
                    _ = activity_rx.recv() => {
                        keep_alive_armed = true;
                        if keep_alive_timeout > 0 {
                            idle_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(keep_alive_timeout));
                        }
                        if server_timeout_armed {
                            server_idle.as_mut().reset(tokio::time::Instant::now() + server_timeout);
                        }
                    },
                    _ = &mut server_idle, if server_timeout_armed => {
                        let _ = Server::emit_str(
                            timeout_server.clone(),
                            &ctx,
                            "timeout",
                            vec![request_socket_for_timeout.clone()],
                            false,
                        );
                        if !timeout_listener {
                            break ConnectionOutcome::ClosedByServer;
                        }
                        server_idle.as_mut().reset(tokio::time::Instant::now() + server_timeout);
                    },
                    result = &mut lenient_conn => break ConnectionOutcome::LenientResult(result),
                }
            }
        } else {
            let service_server = server.clone();
            let service_active_requests = active_requests.clone();
            let service_socket = socket.clone();
            let service_activity_tx = activity_tx;
            let service_write_tx = write_tx;
            let service_requests_on_socket = requests_on_socket;
            let service = service_fn(move |request: HyperRequest<Incoming>| {
                let active_requests = service_active_requests.clone();
                let request_ctx = request_ctx.clone();
                let server = service_server.clone();
                let socket = service_socket.clone();
                let activity_tx = service_activity_tx.clone();
                let write_tx = service_write_tx.clone();
                let requests_on_socket = service_requests_on_socket.clone();
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
                                keep_alive_timeout,
                                write_tx,
                                requests_on_socket,
                                http_rt_key,
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
                .serve_connection(TokioIo::new(server_io), service)
                .with_upgrades();
            tokio::pin!(connection);
            loop {
                tokio::select! {
                    _ = connection_close.recv() => break ConnectionOutcome::ClosedByServer,
                    _ = idle_close.recv() => {
                        if active_requests.load(Ordering::Relaxed) == 0 {
                            break ConnectionOutcome::ClosedByServer;
                        }
                    },
                    _ = &mut idle_timeout, if keep_alive_armed && keep_alive_timeout > 0 => {
                        if active_requests.load(Ordering::Relaxed) == 0 {
                            break ConnectionOutcome::ClosedByServer;
                        }
                        idle_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(keep_alive_timeout));
                    },
                    _ = activity_rx.recv() => {
                        keep_alive_armed = true;
                        if keep_alive_timeout > 0 {
                            idle_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(keep_alive_timeout));
                        }
                        if server_timeout_armed {
                            server_idle.as_mut().reset(tokio::time::Instant::now() + server_timeout);
                        }
                    },
                    _ = &mut server_idle, if server_timeout_armed => {
                        let _ = Server::emit_str(
                            timeout_server.clone(),
                            &ctx,
                            "timeout",
                            vec![request_socket_for_timeout.clone()],
                            false,
                        );
                        if !timeout_listener {
                            break ConnectionOutcome::ClosedByServer;
                        }
                        server_idle.as_mut().reset(tokio::time::Instant::now() + server_timeout);
                    },
                    result = &mut connection => break ConnectionOutcome::HyperResult(result),
                }
            }
        };

        match outcome {
            ConnectionOutcome::HyperResult(Err(error)) => {
                if let Ok(sock_cls) = socket_class(&ctx, &socket) {
                    attach_upgrade_writer(&sock_cls, err_write_tx);
                }
                emit_client_error(&ctx, &server, &socket, &error.to_string(), "HPE_INVALID_REQUEST")?;
            },
            ConnectionOutcome::LenientResult(Err(error)) => {
                if let Ok(sock_cls) = socket_class(&ctx, &socket) {
                    attach_upgrade_writer(&sock_cls, err_write_tx);
                }
                emit_client_error(&ctx, &server, &socket, &error.message, error.code)?;
            },
            _ => {},
        }
        count.fetch_sub(1, Ordering::Relaxed);
        emit_close_if_ready(&ctx, server)?;
        Ok(())
    });
    Ok(())
}

fn emit_client_error<'js>(
    ctx: &Ctx<'js>,
    server: &Class<'js, Server<'js>>,
    socket: &Value<'js>,
    message: &str,
    code: &str,
) -> Result<()> {
    let error_value = Object::new(ctx.clone())?.into_value();
    if let Some(error_object) = error_value.as_object() {
        error_object.set("message", message)?;
        error_object.set("code", code)?;
    }
    let _ = Server::emit_str(
        server.clone(),
        ctx,
        "clientError",
        vec![error_value, socket.clone()],
        false,
    );
    Ok(())
}

/// Drive microtasks while Hyper awaits async HTTP handlers.
pub(crate) struct JsEventLoopPump {
    rt: *mut qjs::JSRuntime,
    executing_timers: Vec<Option<ExecutingTimer>>,
}

impl JsEventLoopPump {
    pub(crate) fn new(ctx: &Ctx<'_>) -> Self {
        let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        Self {
            rt,
            executing_timers: Vec::new(),
        }
    }

    pub(crate) fn tick(&mut self, ctx: &Ctx<'_>) {
        while ctx.execute_pending_job() {}
        let _ = poll_timers(self.rt, &mut self.executing_timers, None, None);
    }
}

/// Drive microtasks while waiting for a response oneshot.
pub(crate) async fn await_oneshot_pumping<T>(
    ctx: &Ctx<'_>,
    pump: &mut JsEventLoopPump,
    rx: oneshot::Receiver<T>,
) -> Option<T> {
    use tokio::time::{interval, MissedTickBehavior};
    let mut tick = interval(Duration::from_millis(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tokio::pin!(rx);
    loop {
        tokio::select! {
            biased;
            response = &mut rx => return response.ok(),
            _ = tick.tick() => pump.tick(ctx),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch<'js>(
    ctx: Ctx<'js>,
    server: Class<'js, Server<'js>>,
    connection_socket: Value<'js>,
    mut request: HyperRequest<Incoming>,
    request_timeout: u64,
    max_request_body_size: usize,
    keep_alive_timeout: u64,
    write_tx: mpsc::UnboundedSender<WriteCmd>,
    requests_on_socket: Arc<AtomicUsize>,
    http_rt_key: usize,
) -> std::result::Result<HyperResponse<BoxBody<Bytes, Infallible>>, Infallible> {
    let _js_serial = HttpJsSerialGuard::acquire(http_rt_key).await;
    let empty = || {
        HyperResponse::builder()
            .status(500)
            .body(Full::new(Bytes::new()).boxed())
            .unwrap()
    };

    let req_num = requests_on_socket.fetch_add(1, Ordering::Relaxed) + 1;
    let max_req = server.borrow().max_requests_per_socket;
    if max_req > 0 && req_num > max_req {
        let pairs: Vec<(String, String)> = request
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        if let Ok((headers_obj, raw_headers)) = build_incoming_headers_js(&ctx, pairs) {
            if let Ok(msg) = IncomingMessage::from_server_request(
                &ctx,
                request.method().to_string(),
                request.uri().to_string(),
                headers_obj,
                raw_headers,
                connection_socket.clone(),
            ) {
                if let Ok(req) = Class::instance(ctx.clone(), msg) {
                    let _ = Server::emit_str(
                        server,
                        &ctx,
                        "dropRequest",
                        vec![req.into_value(), connection_socket],
                        false,
                    );
                }
            }
        }
        return Ok(HyperResponse::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(hyper::header::CONNECTION, "close")
            .header(hyper::header::TRANSFER_ENCODING, "chunked")
            .body(Full::new(Bytes::new()).boxed())
            .unwrap_or_else(|_| empty()));
    }

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
    let mut js_pump = JsEventLoopPump::new(&ctx);
    let _ = resume_timer_loop_if_idle(&ctx);

    let pairs: Vec<(String, String)> = parts
        .headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let Ok((headers, raw)) = build_incoming_headers_js(&ctx, pairs) else {
        return Ok(empty());
    };
    let req = match Class::instance(
        ctx.clone(),
        match IncomingMessage::from_server_request(
            &ctx,
            parts.method.to_string(),
            parts.uri.to_string(),
            headers,
            raw,
            connection_socket.clone(),
        ) {
            Ok(msg) => msg,
            Err(_) => return Ok(empty()),
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
        let (sock_write_tx, mut sock_write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        Socket::attach_raw_writer(&socket, sock_write_tx);
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
        let upgrade_write_tx = write_tx;
        tokio::spawn(async move {
            let Ok(upgraded) = on_upgrade.await else {
                return;
            };
            tokio::spawn(async move {
                while let Some(bytes) = sock_write_rx.recv().await {
                    if upgrade_write_tx.send(WriteCmd::Bytes(bytes)).is_err() {
                        return;
                    }
                }
                let (ack_tx, _ack_rx) = oneshot::channel();
                let _ = upgrade_write_tx.send(WriteCmd::Shutdown(ack_tx));
            });
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

    let expect_val = parts
        .headers
        .get(hyper::header::EXPECT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase());
    let expects_continue = expect_val.as_deref() == Some("100-continue");
    let has_other_expect = expect_val.as_deref().is_some_and(|v| v != "100-continue");
    if has_other_expect && !server.borrow().has_listener_str("checkExpectation") {
        return Ok(HyperResponse::builder()
            .status(StatusCode::EXPECTATION_FAILED)
            .header(hyper::header::CONNECTION, "close")
            .header(hyper::header::TRANSFER_ENCODING, "chunked")
            .body(Full::new(Bytes::new()).boxed())
            .unwrap_or_else(|_| empty()));
    }
    let event_name = if expects_continue && server.borrow().has_listener_str("checkContinue") {
        "checkContinue"
    } else if has_other_expect {
        "checkExpectation"
    } else {
        "request"
    };

    let (tx, mut rx) = oneshot::channel();
    let req_value = req.clone().into_value();
    let track_finish =
        lexe_utils::diagnostics::has_subscribers(&ctx, "http.server.response.finish");
    let mut response_inner = ServerResponse::new_internal(
        ResponseSink::Hyper(tx),
        Some(write_tx.clone()),
        parts.method == hyper::Method::HEAD,
        req_value.clone(),
        connection_socket.clone(),
        track_finish.then(|| server.clone().into_value()),
        track_finish,
    );
    if max_req > 0 && req_num < max_req {
        let secs = keep_alive_timeout / 1000;
        response_inner.set_default_header("keep-alive", format!("timeout={secs}, max={max_req}"));
    }
    let inject_close_on_wire = max_req > 0 && req_num == max_req;

    let res = match Class::instance(ctx.clone(), response_inner) {
        Ok(v) => v,
        Err(_) => return Ok(empty()),
    };
    let check_continue = event_name == "checkContinue";
    let res_value = res.clone().into_value();
    let _ = lexe_utils::diagnostics::publish(&ctx, "http.server.response.created", |ctx| {
        let msg = Object::new(ctx.clone())?;
        msg.set("request", req_value.clone())?;
        msg.set("response", res_value.clone())?;
        Ok(msg)
    });
    let _ = lexe_utils::diagnostics::publish(&ctx, "http.server.request.start", |ctx| {
        let msg = Object::new(ctx.clone())?;
        msg.set("request", req_value.clone())?;
        msg.set("response", res_value.clone())?;
        msg.set("socket", connection_socket.clone())?;
        msg.set("server", server.clone().into_value())?;
        Ok(msg)
    });
    if Server::emit_str(server, &ctx, event_name, vec![req_value, res_value], false).is_err() {
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        if !res.borrow().is_headers_sent() {
            return Ok(empty());
        }
    }
    for _ in 0..512 {
        if res.borrow().headers_sent {
            break;
        }
        js_pump.tick(&ctx);
    }

    let finish_hyper = |response: Option<HyperResponse<BoxBody<Bytes, Infallible>>>| {
        if inject_close_on_wire {
            let _ = write_tx.send(WriteCmd::InjectCloseNextHeader);
        }
        response.unwrap_or_else(empty)
    };

    // A synchronous handler may have already committed a final response.
    // Return it now instead of waiting for an abandoned or slow request body.
    if res.borrow().headers_sent {
        return Ok(finish_hyper(
            await_oneshot_pumping(&ctx, &mut js_pump, rx).await,
        ));
    }

    // Hyper owns the HTTP/1 encoder and deliberately sends `100 Continue`
    // when the body is first polled. This precisely matches Node's default
    // path. With a `checkContinue` listener Node lets userland decide; only
    // poll after `writeContinue()` so a final response is sent without 100.
    let consume_body = !check_continue || res.borrow().has_continue_requested();
    if consume_body {
        use tokio::time::{interval, MissedTickBehavior};
        let mut pump_tick = interval(Duration::from_millis(1));
        pump_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let request_deadline = (request_timeout > 0)
            .then(|| tokio::time::Instant::now() + Duration::from_millis(request_timeout));
        loop {
            while req.borrow().is_paused() && !req.borrow().is_destroyed() {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(1)) => js_pump.tick(&ctx),
                    _ = pump_tick.tick() => js_pump.tick(&ctx),
                }
            }
            if req.borrow().is_destroyed() {
                break;
            }
            // Do not make a response wait for a client that is still (or no
            // longer) sending its request body. This is especially important
            // for async handlers which call `end()` after the first body poll.
            let next_frame = if let Some(dl) = request_deadline {
                tokio::select! {
                    response = &mut rx => return Ok(finish_hyper(response.ok())),
                    result = tokio::time::timeout_at(dl, body.frame()) => match result {
                        Ok(frame) => frame,
                        Err(_) => return Ok(HyperResponse::builder()
                            .status(StatusCode::REQUEST_TIMEOUT)
                            .header(hyper::header::CONNECTION, "close")
                            .body(Full::new(Bytes::new()).boxed())
                            .unwrap_or_else(|_| empty())),
                    },
                    _ = pump_tick.tick() => {
                        js_pump.tick(&ctx);
                        continue;
                    }
                }
            } else {
                tokio::select! {
                    response = &mut rx => return Ok(finish_hyper(response.ok())),
                    frame = body.frame() => frame,
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
                    if req.borrow().body_len().saturating_add(data.len()) > max_request_body_size {
                        return Ok(HyperResponse::builder()
                            .status(StatusCode::PAYLOAD_TOO_LARGE)
                            .header(hyper::header::CONNECTION, "close")
                            .body(Full::new(Bytes::new()).boxed())
                            .unwrap_or_else(|_| empty()));
                    }
                    req.borrow_mut().append_body(&data);
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
                            message.push_trailer(name.as_str(), value);
                        }
                    }
                },
            }
        }
    }
    if consume_body {
        req.borrow_mut().mark_complete();
        let _ = IncomingMessage::emit_str(req, &ctx, "end", vec![], false);
    }
    Ok(finish_hyper(
        await_oneshot_pumping(&ctx, &mut js_pump, rx).await,
    ))
}

#[cfg(any(
    feature = "tls-ring",
    feature = "tls-aws-lc",
    feature = "tls-graviola",
    feature = "tls-openssl"
))]
pub(crate) fn handoff_tls_to_http_server<'js>(
    ctx: Ctx<'js>,
    http_server: Value<'js>,
    tls_socket: Class<'js, lexe_tls::tls_socket::TlsSocket<'js>>,
    stream: std::pin::Pin<Box<dyn lexe_tls::http_handoff::AsyncReadWrite + 'static>>,
) -> Result<()> {
    let http = Class::<Server>::from_object(
        http_server
            .as_object()
            .ok_or_else(|| Exception::throw_type(&ctx, "Invalid http.Server"))?,
    )
    .ok_or_else(|| Exception::throw_type(&ctx, "Invalid http.Server"))?;
    spawn_connection(ctx.clone(), http, tls_socket.clone().into_js(&ctx)?, stream)
}

/// Minimal `http.Agent` exposing Node agent properties for ecosystem compatibility.
fn create_http_agent_constructor<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    ctx.eval(
        r#"(function () {
  function Agent(options) {
    if (!(this instanceof Agent)) {
      return new Agent(options);
    }
    this.options = options && typeof options === "object" ? options : {};
    this.protocol = "http:";
    this.defaultPort = 80;
    this.keepAlive = Boolean(this.options.keepAlive);
    this.keepAliveMsecs = typeof this.options.keepAliveMsecs === "number" ? this.options.keepAliveMsecs : 1000;
    this.maxSockets = this.options.maxSockets !== undefined ? this.options.maxSockets : Infinity;
    this.maxFreeSockets = this.options.maxFreeSockets !== undefined ? this.options.maxFreeSockets : 256;
    this.maxTotalSockets = this.options.maxTotalSockets !== undefined ? this.options.maxTotalSockets : Infinity;
    this.sockets = {};
    this.freeSockets = {};
    this.requests = {};
  }
  Agent.prototype.getName = function (options) {
    options = options || {};
    let name = (options.host || options.hostname || "localhost") + ":";
    if (options.port) name += options.port;
    name += ":";
    if (options.localAddress) name += options.localAddress;
    return name;
  };
  Agent.prototype.destroy = function () {
    this.sockets = {};
    this.freeSockets = {};
    this.requests = {};
  };
  return Agent;
})()"#,
    )
}

fn set_max_idle_http_parsers(_max: Opt<Value<'_>>) {}

pub struct HttpModule;
impl ModuleDef for HttpModule {
    fn declare(declare: &Declarations) -> Result<()> {
        for name in [
            "createServer",
            "Server",
            "IncomingMessage",
            "OutgoingMessage",
            "ServerResponse",
            "ClientRequest",
            "Agent",
            "globalAgent",
            "METHODS",
            "STATUS_CODES",
            "maxHeaderSize",
            "setMaxIdleHTTPParsers",
            "validateHeaderName",
            "validateHeaderValue",
            // Node 22+ client-side WebSocket.
            "WebSocket",
            // Client entrypoints
            "get",
            "request",
            "default",
        ] {
            declare.declare(name)?;
        }
        Ok(())
    }
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        lexe_buffer::init(ctx)?;
        export_default(ctx, exports, |default| {
            Class::<Server>::define(default)?;
            Class::<IncomingMessage>::define(default)?;
            Class::<OutgoingMessage>::define(default)?;
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
            OutgoingMessage::add_event_emitter_prototype(ctx)?;
            ServerResponse::add_event_emitter_prototype(ctx)?;
            Socket::add_event_emitter_prototype(ctx)?;
            default.set("createServer", Func::from(create_server))?;
            let agent_ctor = create_http_agent_constructor(ctx)?;
            let global_opts = Object::new(ctx.clone())?;
            global_opts.set("keepAlive", true)?;
            let global_agent: Value = agent_ctor.call((global_opts,))?;
            default.set("Agent", agent_ctor)?;
            default.set("globalAgent", global_agent)?;
            default.set("METHODS", HTTP_METHODS.to_vec())?;
            default.set("maxHeaderSize", MAX_HEADER_SIZE as u32)?;
            default.set(
                "setMaxIdleHTTPParsers",
                Func::from(set_max_idle_http_parsers),
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

                let install_has_instance: Function = ctx.eval(
                    "(function (OM, SR, CR) { Object.defineProperty(OM, Symbol.hasInstance, { value(inst) { return inst !== null && typeof inst === 'object' && (OM.prototype.isPrototypeOf(inst) || SR.prototype.isPrototypeOf(inst) || (CR && CR.prototype.isPrototypeOf(inst))); } }); })",
                )?;
                let om: Value = default.get("OutgoingMessage")?;
                let sr: Value = default.get("ServerResponse")?;
                let cr: Value = default.get("ClientRequest")?;
                let _: Value = install_has_instance.call((om, sr, cr))?;
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
    throw new Error("http.get/request client API requires a TLS feature");
  };
})()"#,
                )?;
                default.set("ClientRequest", Value::new_undefined(ctx.clone()))?;
                default.set("get", client_stub.clone())?;
                default.set("request", client_stub)?;

                let install_has_instance: Function = ctx.eval(
                    "(function (OM, SR) { Object.defineProperty(OM, Symbol.hasInstance, { value(inst) { return inst !== null && typeof inst === 'object' && (OM.prototype.isPrototypeOf(inst) || SR.prototype.isPrototypeOf(inst)); } }); })",
                )?;
                let om: Value = default.get("OutgoingMessage")?;
                let sr: Value = default.get("ServerResponse")?;
                let _: Value = install_has_instance.call((om, sr))?;
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
