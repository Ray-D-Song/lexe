// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

// Ensure only one TLS backend is selected
#[cfg(all(feature = "tls-ring", feature = "tls-aws-lc"))]
compile_error!("Features `tls-ring` and `tls-aws-lc` are mutually exclusive");

#[cfg(all(feature = "tls-ring", feature = "tls-graviola"))]
compile_error!("Features `tls-ring` and `tls-graviola` are mutually exclusive");

#[cfg(all(feature = "tls-ring", feature = "tls-openssl"))]
compile_error!("Features `tls-ring` and `tls-openssl` are mutually exclusive");

#[cfg(all(feature = "tls-aws-lc", feature = "tls-graviola"))]
compile_error!("Features `tls-aws-lc` and `tls-graviola` are mutually exclusive");

#[cfg(all(feature = "tls-aws-lc", feature = "tls-openssl"))]
compile_error!("Features `tls-aws-lc` and `tls-openssl` are mutually exclusive");

#[cfg(all(feature = "tls-graviola", feature = "tls-openssl"))]
compile_error!("Features `tls-graviola` and `tls-openssl` are mutually exclusive");

pub mod alpn;
pub mod backend;
pub mod certificate;
pub mod client;
pub mod connect_args;
pub mod error;
pub mod http_handoff;
pub mod identity;
pub mod module;
pub mod options;
pub mod pem;
pub mod root_ca;
pub mod secure_context;
pub mod server;
pub mod sni;
pub mod tls_socket;
pub mod version;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
mod rustls_config;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub use rustls_config::*;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
mod no_verification;

#[cfg(feature = "tls-openssl")]
mod openssl_config;

#[cfg(feature = "tls-openssl")]
pub use openssl_config::*;

pub use alpn::{convert_alpn_protocols, convert_alpn_protocols_export};
pub use certificate::{parse_cert_chain_der, parse_cert_der, parse_cert_pem, CertObject};
pub use error::{
    cert_altname_invalid, throw_invalid_arg_value, throw_option_not_supported, throw_tls_error,
    tls_error, ERR_INVALID_ARG_VALUE, ERR_TLS_CERT_ALTNAME_FORMAT, ERR_TLS_CERT_ALTNAME_INVALID,
    ERR_TLS_OPTION_NOT_SUPPORTED,
};
pub use identity::{canonicalize_ip, check_server_identity, CnValue};
pub use module::TlsModule;
pub use options::{
    parse_connect_options, parse_pem_value, parse_server_options, parse_tls_options, TlsOptions,
};
pub use secure_context::{create_secure_context, JsSecureContext, SecureContext};
pub use sni::SniRegistry;
pub use version::{TlsVersion, DEFAULT_MAX_VERSION, DEFAULT_MIN_VERSION};
