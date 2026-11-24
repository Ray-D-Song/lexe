// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::sync::OnceLock;

use once_cell::sync::Lazy;
use openssl::ssl::{SslConnectorBuilder, SslMethod, SslVerifyMode};

use crate::root_ca::{
    get_extra_ca_certs_openssl, load_openssl_ca_store, set_extra_ca_certs_openssl,
};

pub use crate::root_ca::set_extra_ca_certs_bytes as set_extra_ca_certs;

pub fn set_extra_ca_certs_der(certs: Vec<Vec<u8>>) {
    set_extra_ca_certs_openssl(certs);
}

pub fn get_extra_ca_certs() -> Option<Vec<Vec<u8>>> {
    get_extra_ca_certs_openssl()
}

static TLS_VERSION: OnceLock<Option<openssl::ssl::SslVersion>> = OnceLock::new();

pub fn set_tls_version(version: Option<openssl::ssl::SslVersion>) {
    _ = TLS_VERSION.set(version);
}

pub fn get_tls_version() -> Option<openssl::ssl::SslVersion> {
    *TLS_VERSION.get_or_init(|| None)
}

pub static TLS_CONFIG: Lazy<Result<SslConnectorBuilder, Box<dyn std::error::Error + Send + Sync>>> =
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
) -> Result<SslConnectorBuilder, Box<dyn std::error::Error + Send + Sync>> {
    let mut builder = openssl::ssl::SslConnector::builder(SslMethod::tls_client())?;

    // TLS version
    if let Some(version) = get_tls_version() {
        builder.set_min_proto_version(Some(version))?;
    }

    // Certificate verification
    if !options.reject_unauthorized {
        builder.set_verify(SslVerifyMode::NONE);
    } else if let Some(ca) = options.ca.as_deref() {
        load_openssl_ca_store(&mut builder, Some(ca))?;
    } else {
        load_openssl_ca_store(&mut builder, None)?;
    }

    Ok(builder)
}
