// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::sync::{Arc, OnceLock};

use once_cell::sync::Lazy;
use rustls::{ClientConfig, SupportedProtocolVersion};

use crate::no_verification::NoCertificateVerification;
use crate::root_ca::load_rustls_root_store;

pub use crate::root_ca::set_extra_ca_certs_bytes as set_extra_ca_certs;

// Select the crypto provider based on feature flags
#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub(crate) fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    get_crypto_provider()
}

#[cfg(feature = "tls-ring")]
fn get_crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[cfg(feature = "tls-aws-lc")]
fn get_crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

#[cfg(feature = "tls-graviola")]
fn get_crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls_graviola::default_provider())
}

static TLS_VERSIONS: OnceLock<Vec<&'static SupportedProtocolVersion>> = OnceLock::new();

pub fn set_tls_versions(versions: Vec<&'static SupportedProtocolVersion>) {
    _ = TLS_VERSIONS.set(versions);
}

pub fn get_tls_versions() -> Option<Vec<&'static SupportedProtocolVersion>> {
    let versions = TLS_VERSIONS.get_or_init(Vec::new).clone();
    if versions.is_empty() {
        None
    } else {
        Some(versions)
    }
}

pub static TLS_CONFIG: Lazy<Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>>> =
    Lazy::new(|| {
        build_client_config(BuildClientConfigOptions {
            reject_unauthorized: true,
            ca: None,
        })
    });

pub struct BuildClientConfigOptions {
    pub reject_unauthorized: bool,
    pub ca: Option<Vec<Vec<u8>>>,
}

pub fn build_client_config(
    options: BuildClientConfigOptions,
) -> Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let provider = get_crypto_provider();
    let builder = ClientConfig::builder_with_provider(provider.clone());

    let builder = match get_tls_versions() {
        Some(versions) => builder.with_protocol_versions(&versions),
        None => builder.with_safe_default_protocol_versions(),
    }?;

    let builder = if !options.reject_unauthorized {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification::new(provider)))
    } else if let Some(ca) = options.ca.as_deref() {
        let root_certificates = load_rustls_root_store(Some(ca))?;
        builder.with_root_certificates(root_certificates)
    } else {
        let root_certificates = load_rustls_root_store(None)?;
        builder.with_root_certificates(root_certificates)
    };

    Ok(builder.with_no_client_auth())
}

// Re-export for backward compatibility
pub use crate::root_ca::get_extra_ca_certs;
