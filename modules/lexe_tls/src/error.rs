// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{Ctx, Exception, Object, Result, Value};

pub const ERR_TLS_OPTION_NOT_SUPPORTED: &str = "ERR_TLS_OPTION_NOT_SUPPORTED";
pub const ERR_TLS_CERT_ALTNAME_INVALID: &str = "ERR_TLS_CERT_ALTNAME_INVALID";
pub const ERR_TLS_CERT_ALTNAME_FORMAT: &str = "ERR_TLS_CERT_ALTNAME_FORMAT";
pub const ERR_INVALID_ARG_VALUE: &str = "ERR_INVALID_ARG_VALUE";

/// Create a Node-style Error object with a `.code` property.
pub fn tls_error<'js>(
    ctx: &Ctx<'js>,
    code: &str,
    message: &str,
    extra: &[(&str, Value<'js>)],
) -> Result<Exception<'js>> {
    let exception = Exception::from_message(ctx.clone(), message)?;
    let obj = exception.as_object();
    obj.set("code", code)?;
    for (key, value) in extra {
        obj.set(*key, value.clone())?;
    }
    Ok(exception)
}

pub fn throw_tls_error<'js>(
    ctx: &Ctx<'js>,
    code: &str,
    message: &str,
    extra: &[(&str, Value<'js>)],
) -> rquickjs::Error {
    match tls_error(ctx, code, message, extra) {
        Ok(exception) => exception.throw(),
        Err(error) => error,
    }
}

pub fn throw_option_not_supported(ctx: &Ctx<'_>, option: &str) -> rquickjs::Error {
    throw_tls_error(
        ctx,
        ERR_TLS_OPTION_NOT_SUPPORTED,
        &format!("The {option} option is not supported"),
        &[],
    )
}

pub fn throw_invalid_arg_value(ctx: &Ctx<'_>, name: &str, value: &str) -> rquickjs::Error {
    throw_tls_error(
        ctx,
        ERR_INVALID_ARG_VALUE,
        &format!("The property '{name}' {value}"),
        &[],
    )
}

pub fn cert_altname_invalid<'js>(
    ctx: &Ctx<'js>,
    reason: &str,
    host: &str,
    cert: Object<'js>,
) -> Result<Exception<'js>> {
    tls_error(
        ctx,
        ERR_TLS_CERT_ALTNAME_INVALID,
        &format!("Hostname/IP does not match certificate's altnames: {reason}"),
        &[("host", host.into_js(ctx)?), ("cert", cert.into_js(ctx)?)],
    )
}

use rquickjs::IntoJs;
