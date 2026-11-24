// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::sync::Arc;

use rquickjs::{class::Trace, Ctx, Exception, JsLifetime, Result, Value};

use crate::options::parse_tls_options;
use crate::pem::pem_certs_to_der;
use crate::version::TlsVersion;

type SecureResult<T> = std::result::Result<T, String>;

/// Normalized TLS configuration held by a SecureContext.
#[derive(Debug, Clone)]
pub struct SecureContext {
    pub ca: Vec<Vec<u8>>,
    pub cert_chain: Vec<Vec<u8>>,
    pub key: Option<Vec<u8>>,
    pub passphrase: Option<String>,
    pub min_version: TlsVersion,
    pub max_version: TlsVersion,
    pub alpn_protocols: Option<Vec<u8>>,
}

impl<'js> Trace<'js> for SecureContext {
    fn trace<'a>(&self, _: rquickjs::class::Tracer<'a, 'js>) {}
}

impl SecureContext {
    pub fn from_options(options: &crate::options::TlsOptions) -> SecureResult<Self> {
        let cert_chain = options.cert.clone();

        let key = options.key.clone();

        if cert_chain.is_empty() != key.is_none() {
            return Err("cert and key must both be provided or both omitted".to_string());
        }

        if !cert_chain.is_empty() {
            validate_cert_key_match(&cert_chain, key.as_deref(), options.passphrase.as_deref())?;
        }

        Ok(Self {
            ca: options.ca.clone(),
            cert_chain,
            key,
            passphrase: options.passphrase.clone(),
            min_version: options.min_version,
            max_version: options.max_version,
            alpn_protocols: options.alpn_protocols.clone(),
        })
    }

    pub fn from_js_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        if let Ok(class) = rquickjs::Class::<JsSecureContext>::from_value(&value) {
            return Ok(class.borrow().inner.clone());
        }
        let options = parse_tls_options(ctx, value)?;
        Self::from_options(&options).map_err(|e| Exception::throw_message(ctx, &e))
    }
}

unsafe impl<'js> JsLifetime<'js> for JsSecureContext {
    type Changed<'to> = JsSecureContext;
}

#[rquickjs::class(rename = "SecureContext")]
pub struct JsSecureContext {
    pub inner: SecureContext,
}

impl<'js> Trace<'js> for JsSecureContext {
    fn trace<'a>(&self, _: rquickjs::class::Tracer<'a, 'js>) {}
}

#[rquickjs::methods]
impl<'js> JsSecureContext {
    #[qjs(constructor)]
    pub fn ctor(ctx: Ctx<'js>, options: Value<'js>) -> Result<rquickjs::Class<'js, Self>> {
        let inner = create_secure_context(&ctx, options)?;
        rquickjs::Class::instance(ctx, Self { inner })
    }
}

pub fn create_secure_context<'js>(ctx: &Ctx<'js>, options: Value<'js>) -> Result<SecureContext> {
    let options = parse_tls_options(ctx, options)?;
    SecureContext::from_options(&options).map_err(|e| Exception::throw_message(ctx, &e))
}

pub fn create_js_secure_context<'js>(
    ctx: &Ctx<'js>,
    options: Value<'js>,
) -> Result<rquickjs::Class<'js, JsSecureContext>> {
    let inner = create_secure_context(ctx, options)?;
    rquickjs::Class::instance(ctx.clone(), JsSecureContext { inner })
}

pub fn secure_context_from_value<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> Result<Arc<SecureContext>> {
    Ok(Arc::new(SecureContext::from_js_value(ctx, value)?))
}

fn validate_cert_key_match(
    cert_chain: &[Vec<u8>],
    key_pem: Option<&[u8]>,
    passphrase: Option<&str>,
) -> SecureResult<()> {
    let _passphrase = passphrase;
    let key_pem = key_pem.ok_or_else(|| "key is required when cert is provided".to_string())?;

    let cert_der = pem_certs_to_der(cert_chain)?;
    let leaf = cert_der
        .first()
        .ok_or_else(|| "certificate chain is empty".to_string())?;

    #[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
    {
        use rustls::pki_types::{pem::PemObject, PrivateKeyDer};
        use x509_parser::prelude::*;

        if key_pem.starts_with(b"-----BEGIN ENCRYPTED") {
            return Err(crate::error::ERR_TLS_OPTION_NOT_SUPPORTED.to_string());
        }

        let key_der = PrivateKeyDer::from_pem_slice(key_pem)
            .map_err(|e| format!("failed to parse private key: {e}"))?;

        let provider = crate::rustls_config::crypto_provider();
        let signing_key = provider
            .key_provider
            .load_private_key(key_der.clone_key())
            .map_err(|e| format!("failed to load private key: {e}"))?;

        let (_, cert) = X509Certificate::from_der(leaf).map_err(|e| e.to_string())?;
        let cert_spki = cert.public_key().raw;

        let key_pub = signing_key
            .public_key()
            .ok_or_else(|| "failed to extract public key from private key".to_string())?;

        if cert_spki != key_pub.as_ref() {
            return Err("key values mismatch".to_string());
        }
        return Ok(());
    }

    #[cfg(all(
        feature = "tls-openssl",
        not(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))
    ))]
    {
        use openssl::pkey::PKey;
        use openssl::x509::X509;
        let cert = X509::from_der(leaf).map_err(|e| e.to_string())?;
        let pkey = if key_pem.starts_with(b"-----BEGIN ENCRYPTED") {
            let pass = passphrase.unwrap_or("");
            PKey::private_key_from_pem_passphrase(key_pem, pass.as_bytes())
        } else {
            PKey::private_key_from_pem(key_pem)
        }
        .map_err(|e| e.to_string())?;
        let cert_pubkey = cert.public_key().map_err(|e| e.to_string())?;
        if !pkey.public_eq(&cert_pubkey) {
            return Err("key values mismatch".to_string());
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::TlsOptions;

    #[test]
    fn requires_paired_cert_key() {
        let mut options = TlsOptions::default();
        options.cert =
            vec![b"-----BEGIN CERTIFICATE-----\nYQ==\n-----END CERTIFICATE-----\n".to_vec()];
        assert!(SecureContext::from_options(&options).is_err());
    }

    #[test]
    fn accepts_matching_fixture_cert_key() {
        let mut options = TlsOptions::default();
        options.cert = vec![include_bytes!("../../../libs/lexe_test_tls/data/server.pem").to_vec()];
        options.key = Some(include_bytes!("../../../libs/lexe_test_tls/data/server.key").to_vec());
        assert!(SecureContext::from_options(&options).is_ok());
    }
}
