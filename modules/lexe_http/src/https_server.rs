// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_events::Emitter;
use lexe_tls::server::Server as TlsServer;
use rquickjs::{
    class::{Trace, Tracer},
    prelude::{Opt, Rest, This},
    Array, Class, Ctx, Function, IntoJs, JsLifetime, Object, Result, Value,
};

use crate::server::{create_server as create_http_server, Server as HttpServer};

fn is_http_event(event: &Value<'_>) -> bool {
    let Some(s) = event.as_string() else {
        return false;
    };
    let Ok(name) = s.to_string() else {
        return false;
    };
    matches!(
        name.as_str(),
        "request"
            | "checkContinue"
            | "checkExpectation"
            | "clientError"
            | "upgrade"
            | "connect"
            | "dropRequest"
            | "timeout"
    )
}

fn parse_create_args<'js>(args: &Rest<Value<'js>>) -> (Option<Object<'js>>, Option<Function<'js>>) {
    let mut options: Option<Object<'js>> = None;
    let mut request_listener: Option<Function<'js>> = None;

    for value in &args.0 {
        if let Some(cb) = value.as_function() {
            request_listener = Some(cb.clone());
        } else if let Some(obj) = value.as_object() {
            options = Some(obj.clone());
        }
    }

    (options, request_listener)
}

#[rquickjs::class(rename = "Server")]
pub struct HttpsServer<'js> {
    tls: Class<'js, TlsServer<'js>>,
    http: Class<'js, HttpServer<'js>>,
}

unsafe impl<'js> JsLifetime<'js> for HttpsServer<'js> {
    type Changed<'to> = HttpsServer<'to>;
}

impl<'js> Trace<'js> for HttpsServer<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.tls.trace(tracer);
        self.http.trace(tracer);
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> HttpsServer<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Class<'js, Self>> {
        let (options, request_listener) = parse_create_args(&args);

        let mut http_args: Vec<Value<'js>> = Vec::new();
        if let Some(ref opts) = options {
            http_args.push(opts.clone().into_value());
        }
        if let Some(listener) = request_listener {
            http_args.push(listener.into_js(&ctx)?);
        }
        let http = create_http_server(ctx.clone(), Rest(http_args))?;
        // `tls` owns the listening socket lifetime; unref internal `http` until explicitly reffed.
        let _ = HttpServer::js_unref(This(http.clone()));

        let mut tls_args: Vec<Value<'js>> = Vec::new();
        if let Some(opts) = options {
            tls_args.push(opts.into_value());
        }
        let tls = TlsServer::new(ctx.clone(), Rest(tls_args))?;

        let http_value = http.clone().into_js(&ctx)?;
        TlsServer::set_http_handoff_target(tls.clone(), http_value);

        Class::instance(ctx, Self { tls, http })
    }

    pub fn address(&self) -> Value<'js> {
        self.tls.borrow().address()
    }

    #[qjs(get, enumerable)]
    pub fn listening(&self) -> bool {
        self.tls.borrow().is_listening()
    }

    pub fn listen(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let tls = this.borrow().tls.clone();
        TlsServer::listen(This(tls), ctx, args)?;
        Ok(this.0)
    }

    pub fn close(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        cb: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        let (tls, http) = {
            let b = this.borrow();
            (b.tls.clone(), b.http.clone())
        };
        http.borrow().close_idle_connections();
        TlsServer::close(This(tls), ctx, cb)?;
        Ok(this.0)
    }

    pub fn close_all_connections(&self) {
        self.http.borrow().close_all_connections();
    }

    pub fn close_idle_connections(&self) {
        self.http.borrow().close_idle_connections();
    }

    pub fn get_connections(&self, cb: Opt<Function<'js>>) -> Result<()> {
        self.tls.borrow().get_connections(cb)
    }

    pub fn set_secure_context(&self, ctx: Ctx<'js>, context: Value<'js>) -> Result<()> {
        TlsServer::set_secure_context(This(self.tls.clone()), ctx, context)
    }

    pub fn add_context(&self, ctx: Ctx<'js>, hostname: String, context: Value<'js>) -> Result<()> {
        TlsServer::add_context(This(self.tls.clone()), ctx, hostname, context)
    }

    pub fn set_timeout(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        msecs: Opt<Value<'js>>,
        callback: Opt<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        let http = this.borrow().http.clone();
        HttpServer::set_timeout(This(http), ctx, msecs, callback)?;
        Ok(this.0)
    }

    #[qjs(rename = "ref")]
    pub fn js_ref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        let http = this.borrow().http.clone();
        let _ = HttpServer::js_ref(This(http));
        this.0
    }

    #[qjs(rename = "unref")]
    pub fn js_unref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        let http = this.borrow().http.clone();
        let _ = HttpServer::js_unref(This(http));
        this.0
    }

    #[qjs(get, enumerable)]
    pub fn timeout(&self) -> Value<'js> {
        self.http.borrow().timeout()
    }
    #[qjs(set, rename = "timeout")]
    pub fn set_timeout_val(&self, value: Value<'js>) {
        self.http.borrow_mut().set_timeout_value(value);
    }

    #[qjs(get, enumerable)]
    pub fn keep_alive_timeout(&self) -> u64 {
        self.http.borrow().keep_alive_timeout()
    }
    #[qjs(set, rename = "keepAliveTimeout")]
    pub fn set_keep_alive_timeout(&self, value: u64) {
        self.http.borrow_mut().set_keep_alive_timeout(value);
    }

    #[qjs(get, enumerable)]
    pub fn headers_timeout(&self) -> u64 {
        self.http.borrow().headers_timeout()
    }
    #[qjs(set, rename = "headersTimeout")]
    pub fn set_headers_timeout(&self, value: u64) {
        self.http.borrow_mut().set_headers_timeout(value);
    }

    #[qjs(get, enumerable)]
    pub fn request_timeout(&self) -> u64 {
        self.http.borrow().request_timeout()
    }
    #[qjs(set, rename = "requestTimeout")]
    pub fn set_request_timeout(&self, value: u64) {
        self.http.borrow_mut().set_request_timeout(value);
    }

    #[qjs(get, enumerable)]
    pub fn max_headers_count(&self) -> usize {
        self.http.borrow().max_headers_count()
    }
    #[qjs(set, rename = "maxHeadersCount")]
    pub fn set_max_headers_count(&self, value: Value<'js>) {
        self.http.borrow_mut().set_max_headers_count(value);
    }

    #[qjs(get, enumerable)]
    pub fn max_requests_per_socket(&self) -> usize {
        self.http.borrow().max_requests_per_socket()
    }
    #[qjs(set, rename = "maxRequestsPerSocket")]
    pub fn set_max_requests_per_socket(&self, value: usize) {
        self.http.borrow_mut().set_max_requests_per_socket(value);
    }

    #[qjs(get, enumerable, rename = "insecureHTTPParser")]
    pub fn insecure_http_parser(&self) -> bool {
        self.http.borrow().insecure_http_parser()
    }
    #[qjs(set, rename = "insecureHTTPParser")]
    pub fn set_insecure_http_parser(&self, value: bool) {
        self.http.borrow_mut().set_insecure_http_parser(value);
    }

    #[qjs(get, enumerable)]
    pub fn max_connections(&self) -> Value<'js> {
        self.http.borrow().max_connections()
    }
    #[qjs(set, rename = "maxConnections")]
    pub fn set_max_connections(&self, value: Value<'js>) {
        self.http.borrow_mut().set_max_connections(value);
    }

    // EventEmitter routing between HttpServer (HTTP lifecycle) and TlsServer (TLS/TCP lifecycle)

    pub fn on(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
    ) -> Result<Class<'js, Self>> {
        Self::add_listener_impl(&this.0, ctx, event, listener, false, false)?;
        Ok(this.0)
    }

    pub fn add_listener(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
    ) -> Result<Class<'js, Self>> {
        Self::add_listener_impl(&this.0, ctx, event, listener, false, false)?;
        Ok(this.0)
    }

    pub fn once(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
    ) -> Result<Class<'js, Self>> {
        Self::add_listener_impl(&this.0, ctx, event, listener, false, true)?;
        Ok(this.0)
    }

    pub fn prepend_listener(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
    ) -> Result<Class<'js, Self>> {
        Self::add_listener_impl(&this.0, ctx, event, listener, true, false)?;
        Ok(this.0)
    }

    pub fn prepend_once_listener(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
    ) -> Result<Class<'js, Self>> {
        Self::add_listener_impl(&this.0, ctx, event, listener, true, true)?;
        Ok(this.0)
    }

    pub fn off(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
    ) -> Result<Class<'js, Self>> {
        Self::remove_listener_impl(&this.0, ctx, event, listener)?;
        Ok(this.0)
    }

    pub fn remove_listener(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
    ) -> Result<Class<'js, Self>> {
        Self::remove_listener_impl(&this.0, ctx, event, listener)?;
        Ok(this.0)
    }

    pub fn remove_all_listeners(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        event: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let (http, tls) = {
            let b = this.borrow();
            (b.http.clone(), b.tls.clone())
        };
        match event.0.filter(|v| !v.is_undefined()) {
            Some(evt) => {
                if is_http_event(&evt) {
                    let obj = Object::from_value(http.into_value())?;
                    <HttpServer as Emitter>::remove_all_listeners(This(obj), ctx, Opt(Some(evt)))?;
                } else {
                    let obj = Object::from_value(tls.into_value())?;
                    <TlsServer as Emitter>::remove_all_listeners(This(obj), ctx, Opt(Some(evt)))?;
                }
            },
            None => {
                let h_obj = Object::from_value(http.into_value())?;
                <HttpServer as Emitter>::remove_all_listeners(This(h_obj), ctx.clone(), Opt(None))?;
                let t_obj = Object::from_value(tls.into_value())?;
                <TlsServer as Emitter>::remove_all_listeners(This(t_obj), ctx, Opt(None))?;
            },
        }
        Ok(this.0)
    }

    pub fn listeners(&self, ctx: Ctx<'js>, event: Value<'js>) -> Result<Vec<Function<'js>>> {
        if is_http_event(&event) {
            let obj = Object::from_value(self.http.clone().into_value())?;
            <HttpServer as Emitter>::listeners(This(obj), ctx, event)
        } else {
            let obj = Object::from_value(self.tls.clone().into_value())?;
            <TlsServer as Emitter>::listeners(This(obj), ctx, event)
        }
    }

    pub fn raw_listeners(&self, ctx: Ctx<'js>, event: Value<'js>) -> Result<Vec<Function<'js>>> {
        if is_http_event(&event) {
            let obj = Object::from_value(self.http.clone().into_value())?;
            <HttpServer as Emitter>::raw_listeners(This(obj), ctx, event)
        } else {
            let obj = Object::from_value(self.tls.clone().into_value())?;
            <TlsServer as Emitter>::raw_listeners(This(obj), ctx, event)
        }
    }

    pub fn listener_count(&self, ctx: Ctx<'js>, event: Value<'js>) -> Result<usize> {
        if is_http_event(&event) {
            let obj = Object::from_value(self.http.clone().into_value())?;
            <HttpServer as Emitter>::listener_count(This(obj), ctx, event)
        } else {
            let obj = Object::from_value(self.tls.clone().into_value())?;
            <TlsServer as Emitter>::listener_count(This(obj), ctx, event)
        }
    }

    pub fn event_names(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let t_obj = Object::from_value(self.tls.clone().into_value())?;
        let h_obj = Object::from_value(self.http.clone().into_value())?;
        let t_names = <TlsServer as Emitter>::event_names(This(t_obj), ctx.clone())?;
        let h_names = <HttpServer as Emitter>::event_names(This(h_obj), ctx.clone())?;
        let out = Array::new(ctx)?;
        for (idx, name) in t_names.into_iter().chain(h_names).enumerate() {
            out.set(idx, name)?;
        }
        Ok(out)
    }

    pub fn emit(&self, ctx: Ctx<'js>, event: Value<'js>, args: Rest<Value<'js>>) -> Result<bool> {
        if is_http_event(&event) {
            let obj = Object::from_value(self.http.clone().into_value())?;
            <HttpServer as Emitter>::emit(This(obj), ctx, event, args)
        } else {
            let obj = Object::from_value(self.tls.clone().into_value())?;
            <TlsServer as Emitter>::emit(This(obj), ctx, event, args)
        }
    }

    pub fn set_max_listeners(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        n: Value<'js>,
    ) -> Result<Class<'js, Self>> {
        let h_obj = Object::from_value(this.borrow().http.clone().into_value())?;
        <HttpServer as Emitter>::set_max_listeners(This(h_obj), ctx.clone(), n.clone())?;
        let t_obj = Object::from_value(this.borrow().tls.clone().into_value())?;
        <TlsServer as Emitter>::set_max_listeners(This(t_obj), ctx, n)?;
        Ok(this.0)
    }

    pub fn get_max_listeners(&self, ctx: Ctx<'js>) -> Result<u32> {
        let t_obj = Object::from_value(self.tls.clone().into_value())?;
        <TlsServer as Emitter>::get_max_listeners(This(t_obj), ctx)
    }
}

impl<'js> HttpsServer<'js> {
    fn add_listener_impl(
        this: &Class<'js, Self>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
        prepend: bool,
        once: bool,
    ) -> Result<()> {
        let (http, tls) = {
            let b = this.borrow();
            (b.http.clone(), b.tls.clone())
        };
        if is_http_event(&event) {
            let obj = Object::from_value(http.into_value())?;
            <HttpServer as Emitter>::add_event_listener(
                This(obj),
                ctx,
                event,
                listener,
                prepend,
                once,
            )?;
        } else {
            let obj = Object::from_value(tls.into_value())?;
            <TlsServer as Emitter>::add_event_listener(
                This(obj),
                ctx,
                event,
                listener,
                prepend,
                once,
            )?;
        }
        Ok(())
    }

    fn remove_listener_impl(
        this: &Class<'js, Self>,
        ctx: Ctx<'js>,
        event: Value<'js>,
        listener: Function<'js>,
    ) -> Result<()> {
        let (http, tls) = {
            let b = this.borrow();
            (b.http.clone(), b.tls.clone())
        };
        if is_http_event(&event) {
            let obj = Object::from_value(http.into_value())?;
            <HttpServer as Emitter>::remove_event_listener(This(obj), ctx, event, listener)?;
        } else {
            let obj = Object::from_value(tls.into_value())?;
            <TlsServer as Emitter>::remove_event_listener(This(obj), ctx, event, listener)?;
        }
        Ok(())
    }
}

pub fn create_https_server<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, HttpsServer<'js>>> {
    HttpsServer::new(ctx, args)
}
