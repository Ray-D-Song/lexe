// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, HeaderMap, Response};
use lexe_buffer::Buffer;
use lexe_events::{Emitter, EventEmitter, EventList};
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{
    class::{Trace, Tracer},
    prelude::{Opt, Rest, This},
    Class, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Result, Undefined, Value,
};
use tokio::sync::{mpsc, oneshot, Notify};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>;

pub enum ClientStreamMsg {
    Data(Bytes),
    End(Option<Bytes>),
    Reset(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StreamRole {
    Request,
    Response,
}

#[rquickjs::class]
pub struct Http2Stream<'js> {
    emitter: EventEmitter<'js>,
    role: StreamRole,
    method: String,
    path: String,
    scheme: String,
    authority: String,
    pub(crate) headers: Object<'js>,
    pub(crate) status_code: u16,
    response_tx: Option<oneshot::Sender<Response<BoxBody>>>,
    response_started: bool,
    pub(crate) closed: bool,
    pub(crate) destroyed: bool,
    paused: Arc<AtomicBool>,
    resume_notify: Arc<Notify>,
    write_tx: Option<mpsc::UnboundedSender<ClientStreamMsg>>,
}

unsafe impl<'js> JsLifetime<'js> for Http2Stream<'js> {
    type Changed<'to> = Http2Stream<'to>;
}

impl<'js> Trace<'js> for Http2Stream<'js> {
    fn trace<'a>(&self, t: Tracer<'a, 'js>) {
        self.emitter.trace(t);
        self.headers.trace(t);
    }
}

impl<'js> Emitter<'js> for Http2Stream<'js> {
    fn get_event_list(&self) -> Arc<std::sync::RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

impl<'js> Http2Stream<'js> {
    pub fn new_request(
        ctx: &Ctx<'js>,
        method: String,
        path: String,
        scheme: String,
        authority: String,
        header_map: HeaderMap,
    ) -> Result<Class<'js, Self>> {
        let headers = headers_to_object(ctx, &header_map)?;
        Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                role: StreamRole::Request,
                method,
                path,
                scheme,
                authority,
                headers,
                status_code: 200,
                response_tx: None,
                response_started: false,
                closed: false,
                destroyed: false,
                paused: Arc::new(AtomicBool::new(false)),
                resume_notify: Arc::new(Notify::new()),
                write_tx: None,
            },
        )
    }

    pub fn new_client_request(
        ctx: &Ctx<'js>,
        method: String,
        path: String,
        scheme: String,
        authority: String,
        write_tx: mpsc::UnboundedSender<ClientStreamMsg>,
    ) -> Result<(Class<'js, Self>, Arc<AtomicBool>, Arc<Notify>)> {
        let paused = Arc::new(AtomicBool::new(false));
        let resume_notify = Arc::new(Notify::new());
        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                role: StreamRole::Request,
                method,
                path,
                scheme,
                authority,
                headers: Object::new(ctx.clone())?,
                status_code: 200,
                response_tx: None,
                response_started: false,
                closed: false,
                destroyed: false,
                paused: paused.clone(),
                resume_notify: resume_notify.clone(),
                write_tx: Some(write_tx),
            },
        )?;
        Ok((instance, paused, resume_notify))
    }

    pub fn new_response(
        ctx: &Ctx<'js>,
        tx: oneshot::Sender<Response<BoxBody>>,
    ) -> Result<Class<'js, Self>> {
        Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                role: StreamRole::Response,
                method: String::new(),
                path: String::new(),
                scheme: String::new(),
                authority: String::new(),
                headers: Object::new(ctx.clone())?,
                status_code: 200,
                response_tx: Some(tx),
                response_started: false,
                closed: false,
                destroyed: false,
                paused: Arc::new(AtomicBool::new(false)),
                resume_notify: Arc::new(Notify::new()),
                write_tx: None,
            },
        )
    }
}

fn headers_to_object<'js>(ctx: &Ctx<'js>, map: &HeaderMap) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    for (name, value) in map.iter() {
        let key = name.as_str();
        let val = value.to_str().unwrap_or("");
        if let Ok(existing) = obj.get::<_, String>(key) {
            let _ = obj.set(key, format!("{existing}, {val}"));
        } else {
            let _ = obj.set(key, val);
        }
    }
    Ok(obj)
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Http2Stream<'js> {
    #[qjs(get, enumerable)]
    fn method(&self) -> String {
        self.method.clone()
    }

    #[qjs(get, enumerable)]
    fn url(&self) -> String {
        self.path.clone()
    }

    #[qjs(get, enumerable)]
    fn scheme(&self) -> String {
        self.scheme.clone()
    }

    #[qjs(get, enumerable)]
    fn authority(&self) -> String {
        self.authority.clone()
    }

    #[qjs(get, enumerable)]
    fn headers(&self) -> Object<'js> {
        self.headers.clone()
    }

    #[qjs(get, enumerable)]
    fn http_version(&self) -> &'static str {
        "2.0"
    }

    #[qjs(get, enumerable)]
    fn closed(&self) -> bool {
        self.closed
    }

    #[qjs(get, enumerable)]
    fn destroyed(&self) -> bool {
        self.destroyed
    }

    #[qjs(get, rename = "statusCode")]
    fn status_code_get(&self) -> u16 {
        self.status_code
    }

    #[qjs(set, rename = "statusCode")]
    fn status_code_set(&mut self, code: u16) {
        self.status_code = code;
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn pause(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().paused.store(true, Ordering::Release);
        this.0
    }

    pub fn resume(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        let notify = {
            let borrow = this.borrow();
            borrow.paused.store(false, Ordering::Release);
            borrow.resume_notify.clone()
        };
        notify.notify_waiters();
        this.0
    }

    pub fn set_timeout(
        this: This<Class<'js, Self>>,
        _msecs: Opt<Value<'js>>,
        _cb: Opt<Function<'js>>,
    ) -> Class<'js, Self> {
        this.0
    }

    pub fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        chunk: Value<'js>,
        encoding_or_cb: Opt<Value<'js>>,
        cb: Opt<Function<'js>>,
    ) -> Result<bool> {
        let callback =
            cb.0.or_else(|| encoding_or_cb.0.and_then(|v| v.into_function()));
        let bytes = value_to_bytes(&ctx, chunk)?;
        let tx = this.borrow().write_tx.clone();
        if let Some(tx) = tx {
            let _ = tx.send(ClientStreamMsg::Data(Bytes::from(bytes)));
        }
        if let Some(cb) = callback {
            cb.defer(())?;
        }
        Ok(true)
    }

    pub fn write_head(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        status: Value<'js>,
        headers: Opt<Value<'js>>,
    ) -> Result<()> {
        if this.borrow().role != StreamRole::Response {
            return Err(Exception::throw_type(
                &ctx,
                "writeHead on non-response stream",
            ));
        }
        let mut stream = this.borrow_mut();
        let code = status.as_int().unwrap_or(stream.status_code as i32) as u16;
        stream.status_code = code;
        if let Some(extra) = headers.0 {
            apply_header_value(&ctx, &mut stream, extra)?;
        }
        Ok(())
    }

    pub fn end(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        if this.borrow().role == StreamRole::Response {
            let first = args.0.into_iter().next();
            let mut stream = this.borrow_mut();
            if stream.response_started {
                return Ok(this.0.clone());
            }
            stream.response_started = true;
            let body_bytes = first
                .filter(|v| !v.is_undefined() && !v.is_null() && !v.is_function())
                .map(|v| value_to_bytes(&ctx, v))
                .transpose()?
                .unwrap_or_default();
            let mut builder = Response::builder().status(stream.status_code);
            for entry in stream.headers.props::<String, Value>() {
                let (key, val) = entry?;
                if let Some(val) = val.as_string() {
                    if let Ok(s) = val.to_string() {
                        builder = builder.header(key.as_str(), s.as_str());
                    }
                }
            }
            let response = builder
                .body(Full::new(Bytes::from(body_bytes)).boxed())
                .unwrap_or_else(|_| Response::new(Full::new(Bytes::new()).boxed()));
            if let Some(tx) = stream.response_tx.take() {
                let _ = tx.send(response);
            }
            return Ok(this.0.clone());
        }

        let mut data_val = None;
        let mut callback = None;
        for arg in args.0 {
            if arg.is_function() {
                callback = arg.into_function();
            } else if !arg.is_undefined()
                && !arg.is_null()
                && data_val.is_none()
                && !arg.is_string()
            {
                data_val = Some(arg);
            } else if arg.is_string() && data_val.is_none() {
                data_val = Some(arg);
            }
        }
        let body = data_val
            .map(|v| value_to_bytes(&ctx, v))
            .transpose()?
            .map(Bytes::from);
        let tx = this.borrow_mut().write_tx.take();
        if let Some(tx) = tx {
            let _ = tx.send(ClientStreamMsg::End(body));
        }
        if let Some(cb) = callback {
            cb.defer(())?;
        }
        Ok(this.0)
    }

    pub fn close(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        code_or_cb: Opt<Value<'js>>,
        cb: Opt<Function<'js>>,
    ) -> Result<()> {
        let mut code = 0u32;
        let mut callback = cb.0;
        if let Some(first) = code_or_cb.0 {
            if first.is_function() {
                callback = first.into_function();
            } else if let Some(n) = first.as_int() {
                code = n.max(0) as u32;
            }
        }
        let already_closed = {
            let mut borrow = this.borrow_mut();
            let prev = borrow.closed;
            borrow.closed = true;
            borrow.paused.store(false, Ordering::Release);
            borrow.resume_notify.notify_waiters();
            if let Some(tx) = borrow.write_tx.take() {
                let _ = tx.send(ClientStreamMsg::Reset(code));
            }
            prev
        };
        if let Some(cb) = callback {
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
    ) -> Result<Class<'js, Self>> {
        let already_closed = {
            let mut borrow = this.borrow_mut();
            let prev = borrow.closed;
            borrow.destroyed = true;
            borrow.closed = true;
            borrow.paused.store(false, Ordering::Release);
            borrow.resume_notify.notify_waiters();
            if let Some(tx) = borrow.write_tx.take() {
                let _ = tx.send(ClientStreamMsg::Reset(0x8)); // CANCEL
            }
            prev
        };
        if !already_closed {
            Self::emit_str(this.0.clone(), &ctx, "close", vec![], false)?;
        }
        Ok(this.0)
    }
}

fn apply_header_value<'js>(
    ctx: &Ctx<'js>,
    stream: &mut Http2Stream<'js>,
    value: Value<'js>,
) -> Result<()> {
    if let Some(obj) = value.as_object() {
        for entry in obj.props::<String, Value>() {
            let (key, val) = entry?;
            set_header_field(ctx, stream, key, val)?;
        }
    } else if let Some(arr) = value.as_array() {
        for item in arr.iter::<Value>() {
            let item = item?;
            if let Some(pair) = item.as_array() {
                let mut iter = pair.iter::<Value>();
                let name = iter
                    .next()
                    .transpose()?
                    .and_then(|v| v.as_string().map(|s| s.to_string()))
                    .transpose()?
                    .unwrap_or_default();
                let val = iter
                    .next()
                    .transpose()?
                    .unwrap_or(Undefined.into_value(ctx.clone()));
                set_header_field(ctx, stream, name, val)?;
            }
        }
    }
    Ok(())
}

fn set_header_field<'js>(
    ctx: &Ctx<'js>,
    stream: &mut Http2Stream<'js>,
    name: String,
    value: Value<'js>,
) -> Result<()> {
    if value.is_array() {
        let arr = value.as_array().unwrap();
        let mut parts = Vec::new();
        for item in arr.iter::<Value>() {
            parts.push(value_to_string(ctx, item?)?);
        }
        let _ = stream.headers.set(name, parts.join(", "));
    } else {
        let _ = stream.headers.set(name, value_to_string(ctx, value)?);
    }
    Ok(())
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

fn value_to_bytes<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Vec<u8>> {
    ObjectBytes::from(ctx, &value)?.into_bytes(ctx)
}

pub async fn pump_request_body<'a>(
    ctx: Ctx<'a>,
    req: Class<'a, Http2Stream<'a>>,
    mut body: Incoming,
) {
    while let Some(frame) = body.frame().await {
        match frame {
            Ok(frame) => {
                if let Ok(data) = frame.into_data() {
                    let chunk = Buffer(data.to_vec())
                        .into_js(&ctx)
                        .unwrap_or_else(|_| Undefined.into_value(ctx.clone()));
                    let _ = Http2Stream::emit_str(req.clone(), &ctx, "data", vec![chunk], false);
                }
            },
            Err(_) => break,
        }
    }
    let _ = Http2Stream::emit_str(req, &ctx, "end", vec![], false);
}
