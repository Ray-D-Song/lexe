// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_events::Emitter;
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Class, Ctx, Object, Result,
};

pub use self::config::*;
#[cfg(any(
    feature = "tls-ring",
    feature = "tls-aws-lc",
    feature = "tls-graviola",
    feature = "tls-openssl"
))]
mod client_request;
mod headers;
#[cfg(any(
    feature = "tls-ring",
    feature = "tls-aws-lc",
    feature = "tls-graviola",
    feature = "tls-openssl"
))]
mod https_server;
mod io;
mod lenient_http1;
mod websocket;
pub use self::server::HttpModule;

#[cfg(any(
    feature = "tls-ring",
    feature = "tls-aws-lc",
    feature = "tls-graviola",
    feature = "tls-openssl"
))]
mod client;
mod config;
mod server;

#[cfg(any(
    feature = "tls-ring",
    feature = "tls-aws-lc",
    feature = "tls-graviola",
    feature = "tls-openssl"
))]
mod agent;

#[cfg(any(
    feature = "tls-ring",
    feature = "tls-aws-lc",
    feature = "tls-graviola",
    feature = "tls-openssl"
))]
pub use self::{agent::Agent, client::*};

pub struct HttpsModule;

impl ModuleDef for HttpsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("Agent")?;
        declare.declare("Server")?;
        declare.declare("createServer")?;
        declare.declare("get")?;
        declare.declare("request")?;
        declare.declare("globalAgent")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            #[cfg(any(
                feature = "tls-ring",
                feature = "tls-aws-lc",
                feature = "tls-graviola",
                feature = "tls-openssl"
            ))]
            {
                use crate::client_request::{https_get, https_request, ClientRequest};
                use crate::https_server::{create_https_server, HttpsServer};
                use crate::server::{IncomingMessage, Server as HttpServer, ServerResponse};
                use lexe_net::Socket;
                use lexe_tls::server::Server as TlsServer;

                lexe_buffer::init(ctx)?;
                lexe_tls::http_handoff::register_http_handoff(
                    crate::server::handoff_tls_to_http_server,
                );
                // Ensure internal/shared HTTP/TLS/Socket class prototypes & EventEmitter
                // methods exist on this context even if `node:http`/`node:tls` weren't imported,
                // without leaking extra named exports onto `https`.
                let scratch = Object::new(ctx.clone())?;
                Class::<HttpServer>::define(&scratch)?;
                HttpServer::add_event_emitter_prototype(ctx)?;
                Class::<TlsServer>::define(&scratch)?;
                TlsServer::add_event_emitter_prototype(ctx)?;
                Class::<IncomingMessage>::define(&scratch)?;
                IncomingMessage::add_event_emitter_prototype(ctx)?;
                Class::<ServerResponse>::define(&scratch)?;
                ServerResponse::add_event_emitter_prototype(ctx)?;
                Class::<ClientRequest>::define(&scratch)?;
                ClientRequest::add_event_emitter_prototype(ctx)?;
                Class::<Socket>::define(&ctx.globals())?;
                Socket::add_event_emitter_prototype(ctx)?;

                Class::<Agent>::define(default)?;
                Class::<HttpsServer>::define(default)?;
                let global_agent = Agent::new_global(ctx)?;
                default.set("globalAgent", global_agent)?;
                default.set("request", Func::from(https_request))?;
                default.set("get", Func::from(https_get))?;
                default.set("createServer", Func::from(create_https_server))?;
            }
            #[cfg(not(any(
                feature = "tls-ring",
                feature = "tls-aws-lc",
                feature = "tls-graviola",
                feature = "tls-openssl"
            )))]
            {
                let stub: rquickjs::Function = ctx.eval(
                    r#"(function(){
  return function httpsStub() {
    throw new Error("https module requires a TLS feature build");
  };
})"#,
                )?;
                default.set("Agent", rquickjs::Value::new_undefined(ctx.clone()))?;
                default.set("Server", rquickjs::Value::new_undefined(ctx.clone()))?;
                default.set("globalAgent", rquickjs::Value::new_undefined(ctx.clone()))?;
                default.set("createServer", stub.clone())?;
                default.set("get", stub.clone())?;
                default.set("request", stub)?;
            }

            Ok(())
        })
    }
}

impl From<HttpsModule> for ModuleInfo<HttpsModule> {
    fn from(val: HttpsModule) -> Self {
        ModuleInfo {
            name: "https",
            module: val,
        }
    }
}
