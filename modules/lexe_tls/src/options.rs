// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::time::Duration;

use rquickjs::{Ctx, Exception, Object, Result, Value};

use crate::error::{throw_invalid_arg_value, throw_option_not_supported};
use crate::secure_context::SecureContext;
use crate::version::{TlsVersion, DEFAULT_MAX_VERSION, DEFAULT_MIN_VERSION};

/// Parsed TLS options shared by connect/createServer/createSecureContext.
#[derive(Debug, Clone)]
pub struct TlsOptions {
    pub ca: Vec<Vec<u8>>,
    pub cert: Vec<Vec<u8>>,
    pub key: Option<Vec<u8>>,
    pub passphrase: Option<String>,
    pub min_version: TlsVersion,
    pub max_version: TlsVersion,
    pub alpn_protocols: Option<Vec<u8>>,
    pub servername: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub reject_unauthorized: bool,
    pub request_cert: bool,
    pub secure_context: Option<SecureContext>,
    pub timeout: Option<Duration>,
}

impl Default for TlsOptions {
    fn default() -> Self {
        Self {
            ca: Vec::new(),
            cert: Vec::new(),
            key: None,
            passphrase: None,
            min_version: DEFAULT_MIN_VERSION,
            max_version: DEFAULT_MAX_VERSION,
            alpn_protocols: None,
            servername: None,
            host: None,
            port: None,
            reject_unauthorized: true,
            request_cert: false,
            secure_context: None,
            timeout: None,
        }
    }
}

const UNSUPPORTED_OPTIONS: &[&str] = &[
    "pfx",
    "ciphers",
    "sigalgs",
    "dhparam",
    "crl",
    "psk",
    "ocsp",
    "session",
    "ticket",
    "SNICallback",
    "ALPNCallback",
];

pub fn parse_tls_options<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<TlsOptions> {
    if value.is_undefined() || value.is_null() {
        return Ok(TlsOptions::default());
    }

    let obj = value
        .as_object()
        .ok_or_else(|| Exception::throw_type(ctx, "options must be an object"))?;

    reject_unsupported_options(ctx, obj)?;

    let mut options = TlsOptions::default();

    if let Some(ca) = get_optional_pem_list(ctx, obj, "ca")? {
        options.ca = ca;
    }
    if let Some(cert) = get_optional_pem_list(ctx, obj, "cert")? {
        options.cert = cert;
    }
    if let Some(key) = get_optional_pem_bytes(ctx, obj, "key")? {
        options.key = Some(key);
    }
    options.passphrase = obj.get::<_, Option<String>>("passphrase")?;
    if let Some(min_version) = obj.get::<_, Option<String>>("minVersion")? {
        options.min_version = TlsVersion::from_str(ctx, &min_version, "minimum")?;
    }
    if let Some(max_version) = obj.get::<_, Option<String>>("maxVersion")? {
        options.max_version = TlsVersion::from_str(ctx, &max_version, "maximum")?;
    }
    if options.min_version > options.max_version {
        return Err(Exception::throw_range(
            ctx,
            "minVersion cannot be greater than maxVersion",
        ));
    }
    if let Some(alpn) = obj.get::<_, Option<Value>>("ALPNProtocols")? {
        options.alpn_protocols = Some(crate::alpn::convert_alpn_protocols(ctx, alpn)?);
    }
    if let Some(servername) = obj.get::<_, Option<String>>("servername")? {
        options.servername = Some(servername);
    }
    if let Some(host) = obj.get::<_, Option<String>>("host")? {
        options.host = Some(host);
    }
    if let Some(port_val) = obj.get::<_, Option<Value>>("port")? {
        options.port = lexe_net::parse_port_value(ctx, &port_val)?;
    }
    if let Some(reject) = obj.get::<_, Option<bool>>("rejectUnauthorized")? {
        options.reject_unauthorized = reject;
    }
    if let Some(request_cert) = obj.get::<_, Option<bool>>("requestCert")? {
        options.request_cert = request_cert;
    }
    if let Some(value) = obj.get::<_, Option<Value>>("checkServerIdentity")? {
        if !value.is_undefined() && !value.is_null() && !value.is_function() {
            return Err(Exception::throw_type(
                ctx,
                "checkServerIdentity must be a function",
            ));
        }
    }
    if let Some(timeout_ms) = obj.get::<_, Option<u64>>("timeout")? {
        options.timeout = Some(Duration::from_millis(timeout_ms));
    }
    if let Some(ctx_val) = obj.get::<_, Option<Value>>("secureContext")? {
        if !ctx_val.is_undefined() && !ctx_val.is_null() {
            options.secure_context = Some(SecureContext::from_js_value(ctx, ctx_val)?);
        }
    }

    if options.request_cert && !options.reject_unauthorized {
        return Err(throw_option_not_supported(
            ctx,
            "requestCert=true with rejectUnauthorized=false (soft mTLS)",
        ));
    }

    Ok(options)
}

pub fn parse_connect_options<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<TlsOptions> {
    parse_tls_options(ctx, value)
}

pub fn parse_server_options<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<TlsOptions> {
    parse_tls_options(ctx, value)
}

fn reject_unsupported_options<'js>(ctx: &Ctx<'js>, obj: &Object<'js>) -> Result<()> {
    for key in UNSUPPORTED_OPTIONS {
        if let Some(value) = obj.get::<_, Option<Value>>(*key)? {
            if !value.is_undefined() && !value.is_null() {
                return Err(throw_option_not_supported(ctx, key));
            }
        }
    }
    Ok(())
}

fn get_optional_pem_list<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    key: &str,
) -> Result<Option<Vec<Vec<u8>>>> {
    let Some(value) = obj.get::<_, Option<Value>>(key)? else {
        return Ok(None);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    Ok(Some(parse_pem_value(ctx, value)?))
}

fn get_optional_pem_bytes<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    key: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(value) = obj.get::<_, Option<Value>>(key)? else {
        return Ok(None);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let list = parse_pem_value(ctx, value)?;
    if list.len() == 1 {
        Ok(Some(list.into_iter().next().unwrap()))
    } else if list.is_empty() {
        Ok(None)
    } else {
        Err(throw_invalid_arg_value(
            ctx,
            key,
            "must be a single PEM block",
        ))
    }
}

pub fn parse_pem_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Vec<Vec<u8>>> {
    if let Some(s) = value.as_string() {
        let s = s.to_string()?;
        if s.is_empty() {
            return Ok(Vec::new());
        }
        return Ok(crate::pem::split_pem_chain(s.as_bytes()));
    }

    if let Some(array) = value.as_array() {
        let mut result = Vec::new();
        for item in array.iter::<Value>() {
            let item = item?;
            result.extend(parse_pem_value(ctx, item)?);
        }
        return Ok(result);
    }

    let bytes = lexe_utils::bytes::ObjectBytes::from(ctx, &value)?;
    let data: Vec<u8> = bytes
        .try_into()
        .map_err(|_| throw_invalid_arg_value(ctx, "value", "must be string, Buffer, or Array"))?;
    if data.is_empty() {
        return Ok(Vec::new());
    }
    Ok(crate::pem::split_pem_chain(&data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};

    fn with_ctx(f: impl FnOnce(&Ctx<'_>)) {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| f(&ctx));
    }

    #[test]
    fn rejects_soft_mtls() {
        with_ctx(|ctx| {
            let obj = Object::new(ctx.clone()).unwrap();
            obj.set("requestCert", true).unwrap();
            obj.set("rejectUnauthorized", false).unwrap();
            assert!(parse_tls_options(ctx, obj.into_value()).is_err());
        });
    }

    #[test]
    fn rejects_pfx() {
        with_ctx(|ctx| {
            let obj = Object::new(ctx.clone()).unwrap();
            obj.set("pfx", "data").unwrap();
            assert!(parse_tls_options(ctx, obj.into_value()).is_err());
        });
    }

    #[test]
    fn rejects_non_function_check_server_identity() {
        with_ctx(|ctx| {
            let obj = Object::new(ctx.clone()).unwrap();
            obj.set("checkServerIdentity", true).unwrap();
            assert!(parse_tls_options(ctx, obj.into_value()).is_err());
        });
    }

    #[test]
    fn allows_function_check_server_identity() {
        with_ctx(|ctx| {
            let result: bool = ctx
                .eval(
                    r#"
                    (() => {
                        const obj = { checkServerIdentity: () => undefined };
                        return typeof obj.checkServerIdentity === 'function';
                    })()
                "#,
                )
                .unwrap();
            assert!(result);
            let obj = Object::new(ctx.clone()).unwrap();
            ctx.eval::<(), _>("globalThis.__tlsTestCb = () => undefined;")
                .unwrap();
            let func: rquickjs::Function = ctx.globals().get("__tlsTestCb").unwrap();
            obj.set("checkServerIdentity", func).unwrap();
            assert!(parse_tls_options(ctx, obj.into_value()).is_ok());
        });
    }
}
