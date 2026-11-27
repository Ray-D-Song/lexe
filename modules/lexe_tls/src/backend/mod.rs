// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
mod rustls;

#[cfg(feature = "tls-openssl")]
mod openssl;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub use rustls::*;

#[cfg(all(
    feature = "tls-openssl",
    not(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))
))]
pub use openssl::*;

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::secure_context::SecureContext;
use crate::sni::SniRegistry;

/// Result of certificate chain verification recorded in permissive client mode.
#[derive(Debug, Default, Clone)]
pub struct VerifyRecord {
    pub ok: bool,
    pub error: Option<String>,
}

/// Information about an established TLS connection.
#[derive(Debug, Clone)]
pub struct TlsConnectionInfo {
    pub protocol: Option<String>,
    pub cipher: Option<String>,
    pub cipher_standard_name: Option<String>,
    pub alpn_protocol: Option<Vec<u8>>,
    pub chain_authorized: bool,
    pub chain_error: Option<String>,
    pub authorized: bool,
    pub authorization_error: Option<String>,
    pub peer_certs: Vec<Vec<u8>>,
    pub local_cert: Option<Vec<u8>>,
    pub client_servername: Option<String>,
    pub session_reused: bool,
}

pub struct ConnectOptions {
    pub context: SecureContext,
    pub identity_name: String,
    pub sni_name: Option<String>,
    pub reject_unauthorized: bool,
    pub timeout: Option<Duration>,
    pub verify_record: Option<Arc<Mutex<VerifyRecord>>>,
    pub session: Option<Vec<u8>>,
    pub session_slot: Option<Arc<Mutex<Option<Vec<u8>>>>>,
    pub session_notify: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
}

pub struct AcceptOptions {
    pub context: SecureContext,
    pub request_cert: bool,
    pub reject_unauthorized: bool,
    pub timeout: Option<Duration>,
    pub sni_registry: Arc<RwLock<SniRegistry>>,
    pub selected_local_cert: Arc<Mutex<Option<Vec<u8>>>>,
    pub server_state: Option<Arc<ServerSharedState>>,
    pub chosen_alpn: Option<Vec<u8>>,
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub type ClientTlsStream<IO> = tokio_rustls::client::TlsStream<IO>;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub type ServerTlsStream<IO> = tokio_rustls::server::TlsStream<IO>;

#[cfg(all(
    feature = "tls-openssl",
    not(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))
))]
pub type ClientTlsStream<IO> = tokio_openssl::SslStream<IO>;

#[cfg(all(
    feature = "tls-openssl",
    not(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))
))]
pub type ServerTlsStream<IO> = tokio_openssl::SslStream<IO>;
