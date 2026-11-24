// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_events::Emitter;
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Class, Ctx, Result,
};

pub use self::config::*;
#[cfg(any(
    feature = "tls-ring",
    feature = "tls-aws-lc",
    feature = "tls-graviola",
    feature = "tls-openssl"
))]
mod client_request;
#[cfg(any(
    feature = "tls-ring",
    feature = "tls-aws-lc",
    feature = "tls-graviola",
    feature = "tls-openssl"
))]
mod https_server;
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
        #[cfg(any(
            feature = "tls-ring",
            feature = "tls-aws-lc",
            feature = "tls-graviola",
            feature = "tls-openssl"
        ))]
        declare.declare(stringify!(Agent))?;
        declare.declare("createServer")?;
        declare.declare("get")?;
        declare.declare("request")?;
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
                lexe_buffer::init(ctx)?;
                lexe_tls::http_handoff::register_http_handoff(
                    crate::server::handoff_tls_to_http_server,
                );
                rquickjs::Class::<Agent>::define(default)?;
                // `https` shares the http request/response classes, so their
                // constructors and event methods must be registered here too:
                // they are not inherited from the `http` module.
                Class::<crate::server::IncomingMessage>::define(default)?;
                crate::server::IncomingMessage::add_event_emitter_prototype(ctx)?;
                use crate::client_request::{https_get, https_request, ClientRequest};
                Class::<ClientRequest>::define(default)?;
                ClientRequest::add_event_emitter_prototype(ctx)?;
                default.set("request", Func::from(https_request))?;
                default.set("get", Func::from(https_get))?;
                default.set(
                    "createServer",
                    Func::from(crate::https_server::create_https_server),
                )?;
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
    throw new Error("https.createServer is not implemented in lexe yet");
  };
})()"#,
                )?;
                default.set("createServer", stub)?;
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
