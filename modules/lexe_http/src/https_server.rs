// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_tls::server::{create_server as create_tls_server, Server as TlsServer};
use rquickjs::{prelude::Rest, Class, Ctx, Exception, Function, IntoJs, Object, Result, Value};

use crate::server::create_server as create_http_server;

fn parse_create_args<'js>(
    ctx: &Ctx<'js>,
    args: &Rest<Value<'js>>,
) -> Result<(Option<Object<'js>>, Option<Function<'js>>)> {
    let mut options: Option<Object<'js>> = None;
    let mut request_listener: Option<Function<'js>> = None;

    for value in &args.0 {
        if value.is_function() {
            request_listener = Some(value.as_function().unwrap().clone());
        } else if value.is_object() {
            options = Some(value.as_object().unwrap().clone());
        }
    }

    if options.is_none() {
        return Err(Exception::throw_type(
            ctx,
            "https.createServer requires TLS options (cert/key)",
        ));
    }

    Ok((options, request_listener))
}

pub fn create_https_server<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Class<'js, TlsServer<'js>>> {
    let (options, request_listener) = parse_create_args(&ctx, &args)?;

    let mut http_args = Vec::new();
    if let Some(listener) = request_listener {
        http_args.push(listener.into_js(&ctx)?);
    }
    let http = create_http_server(ctx.clone(), Rest(http_args))?;

    let mut tls_args: Vec<Value> = Vec::new();
    if let Some(opts) = options {
        tls_args.push(opts.into_value());
    }
    let tls = create_tls_server(ctx.clone(), Rest(tls_args))?;

    let http_value = http.clone().into_js(&ctx)?;
    TlsServer::set_http_handoff_target(tls.clone(), http_value);

    Ok(tls)
}
