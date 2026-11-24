// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::net::IpAddr;
use std::sync::{Arc, Mutex, RwLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::TrustAnchor;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error, RootCertStore, ServerConfig, SignatureScheme,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use webpki::{EndEntityCert, KeyUsage};

use crate::backend::{
    AcceptOptions, ClientTlsStream, ConnectOptions, ServerTlsStream, TlsConnectionInfo,
    VerifyRecord,
};
use crate::pem::pem_certs_to_der;
use crate::root_ca::load_rustls_root_store;
use crate::secure_context::SecureContext;
use crate::sni::SniRegistry;
use crate::version::rustls_protocol_versions;

pub fn build_client_config(
    context: &SecureContext,
    reject_unauthorized: bool,
    verify_record: Option<Arc<Mutex<VerifyRecord>>>,
) -> Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let provider = get_crypto_provider();
    let versions = rustls_protocol_versions(context.min_version, context.max_version);

    let roots = if context.ca.is_empty() {
        load_rustls_root_store(None)?
    } else {
        load_rustls_root_store(Some(&context.ca))?
    };

    let chain_verifier = ChainVerifier::new(provider.clone(), roots);

    let verifier: Arc<dyn ServerCertVerifier> = if reject_unauthorized {
        Arc::new(chain_verifier)
    } else {
        let record = verify_record.ok_or("verify_record required in permissive mode")?;
        Arc::new(RecordingVerifier::new(chain_verifier, record))
    };

    let mut config = if context.cert_chain.is_empty() {
        ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&versions)?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth()
    } else {
        let certs = parse_cert_chain(&context.cert_chain)?;
        let key = parse_private_key(context.key.as_deref())?;
        ClientConfig::builder_with_provider(get_crypto_provider())
            .with_protocol_versions(&versions)?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_client_auth_cert(certs, key)?
    };

    if let Some(alpn) = &context.alpn_protocols {
        config.alpn_protocols = decode_alpn(alpn);
    }
    Ok(config)
}

pub fn build_server_config(
    context: &SecureContext,
    request_cert: bool,
    reject_unauthorized: bool,
    sni_registry: Arc<RwLock<SniRegistry>>,
    selected_local_cert: Arc<Mutex<Option<Vec<u8>>>>,
) -> Result<ServerConfig, Box<dyn std::error::Error + Send + Sync>> {
    let provider = get_crypto_provider();
    let versions = rustls_protocol_versions(context.min_version, context.max_version);
    let certs = parse_cert_chain(&context.cert_chain)?;
    let key = parse_private_key(context.key.as_deref())?;

    let mut config = if request_cert {
        let roots = if context.ca.is_empty() {
            load_rustls_root_store(None)?
        } else {
            load_rustls_root_store(Some(&context.ca))?
        };
        let client_verifier = if reject_unauthorized {
            rustls::server::WebPkiClientVerifier::builder_with_provider(
                roots.into(),
                provider.clone(),
            )
            .build()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
        } else {
            rustls::server::WebPkiClientVerifier::builder_with_provider(
                RootCertStore::empty().into(),
                provider.clone(),
            )
            .allow_unauthenticated()
            .build()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
        };
        ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&versions)?
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs, key)?
    } else {
        ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&versions)?
            .with_no_client_auth()
            .with_single_cert(certs, key)?
    };

    if let Some(alpn) = &context.alpn_protocols {
        config.alpn_protocols = decode_alpn(alpn);
    }

    config.cert_resolver = Arc::new(SniResolver {
        registry: sni_registry,
        default: Arc::new(context.clone()),
        selected_local_cert,
    });

    Ok(config)
}

pub async fn connect<IO>(
    stream: IO,
    options: ConnectOptions,
) -> Result<ClientTlsStream<IO>, Box<dyn std::error::Error + Send + Sync>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send,
{
    let config = Arc::new(build_client_config(
        &options.context,
        options.reject_unauthorized,
        options.verify_record,
    )?);
    let connector = TlsConnector::from(config);
    let server_name = resolve_server_name(&options.identity_name, options.sni_name.as_deref())?;

    let connect = connector.connect(server_name, stream);
    if let Some(timeout) = options.timeout {
        match tokio::time::timeout(timeout, connect).await {
            Ok(result) => Ok(result?),
            Err(_) => Err("TLS handshake timeout".into()),
        }
    } else {
        Ok(connect.await?)
    }
}

pub async fn accept<IO>(
    stream: IO,
    options: AcceptOptions,
) -> Result<ServerTlsStream<IO>, Box<dyn std::error::Error + Send + Sync>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send,
{
    let config = Arc::new(build_server_config(
        &options.context,
        options.request_cert,
        options.reject_unauthorized,
        options.sni_registry,
        options.selected_local_cert,
    )?);
    let acceptor = TlsAcceptor::from(config);
    let accept = acceptor.accept(stream);
    if let Some(timeout) = options.timeout {
        match tokio::time::timeout(timeout, accept).await {
            Ok(result) => Ok(result?),
            Err(_) => Err("TLS handshake timeout".into()),
        }
    } else {
        Ok(accept.await?)
    }
}

pub fn inspect_client_connection<IO>(
    stream: &ClientTlsStream<IO>,
    verify_record: Option<&Arc<Mutex<VerifyRecord>>>,
) -> TlsConnectionInfo {
    let (_, session) = stream.get_ref();
    let peer_certs: Vec<Vec<u8>> = session
        .peer_certificates()
        .map(|certs| certs.iter().map(|c| c.to_vec()).collect())
        .unwrap_or_default();
    let cipher = session
        .negotiated_cipher_suite()
        .map(|c| c.suite().as_str().unwrap_or("unknown").to_string());
    let protocol = session.protocol_version().map(|v| format!("{v:?}"));
    let alpn = session.alpn_protocol().map(|p| p.to_vec());

    let (chain_authorized, chain_error) = if let Some(record) = verify_record {
        if let Ok(rec) = record.lock() {
            (rec.ok, rec.error.clone())
        } else {
            (false, None)
        }
    } else {
        (true, None)
    };

    TlsConnectionInfo {
        protocol,
        cipher: cipher.clone(),
        cipher_standard_name: cipher.clone(),
        alpn_protocol: alpn,
        chain_authorized,
        chain_error: chain_error.clone(),
        authorized: chain_authorized,
        authorization_error: chain_error,
        peer_certs,
        local_cert: None,
        client_servername: None,
    }
}

pub fn inspect_server_connection<IO>(
    stream: &ServerTlsStream<IO>,
    reject_unauthorized: bool,
    selected_local_cert: &Arc<Mutex<Option<Vec<u8>>>>,
) -> TlsConnectionInfo {
    let (_, session) = stream.get_ref();
    let peer_certs: Vec<Vec<u8>> = session
        .peer_certificates()
        .map(|certs| certs.iter().map(|c| c.to_vec()).collect())
        .unwrap_or_default();
    let cipher = session
        .negotiated_cipher_suite()
        .map(|c| c.suite().as_str().unwrap_or("unknown").to_string());
    let protocol = session.protocol_version().map(|v| format!("{v:?}"));
    let alpn = session.alpn_protocol().map(|p| p.to_vec());

    let (chain_authorized, chain_error) = if reject_unauthorized {
        (true, None)
    } else if !peer_certs.is_empty() {
        verify_peer_chain(&peer_certs, None)
    } else {
        (true, None)
    };

    TlsConnectionInfo {
        protocol,
        cipher: cipher.clone(),
        cipher_standard_name: cipher.clone(),
        alpn_protocol: alpn,
        chain_authorized,
        chain_error: chain_error.clone(),
        authorized: chain_authorized,
        authorization_error: chain_error,
        peer_certs,
        local_cert: selected_local_cert
            .lock()
            .ok()
            .and_then(|slot| slot.clone()),
        client_servername: session.server_name().map(str::to_string),
    }
}

pub fn extract_peer_chain_client<IO>(stream: &ClientTlsStream<IO>) -> Vec<Vec<u8>> {
    let (_, session) = stream.get_ref();
    session
        .peer_certificates()
        .map(|certs| certs.iter().map(|c| c.to_vec()).collect())
        .unwrap_or_default()
}

pub fn extract_peer_chain_server<IO>(stream: &ServerTlsStream<IO>) -> Vec<Vec<u8>> {
    let (_, session) = stream.get_ref();
    session
        .peer_certificates()
        .map(|certs| certs.iter().map(|c| c.to_vec()).collect())
        .unwrap_or_default()
}

pub fn supported_ciphers() -> Vec<String> {
    get_crypto_provider()
        .cipher_suites
        .iter()
        .map(|c| c.suite().as_str().unwrap_or("unknown").to_string())
        .collect()
}

#[cfg(feature = "tls-ring")]
fn get_crypto_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[cfg(feature = "tls-aws-lc")]
fn get_crypto_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

#[cfg(feature = "tls-graviola")]
fn get_crypto_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls_graviola::default_provider())
}

fn parse_cert_chain(
    chain: &[Vec<u8>],
) -> Result<Vec<CertificateDer<'static>>, Box<dyn std::error::Error + Send + Sync>> {
    let der = pem_certs_to_der(chain)?;
    Ok(der.into_iter().map(CertificateDer::from).collect())
}

fn parse_private_key(
    key_pem: Option<&[u8]>,
) -> Result<PrivateKeyDer<'static>, Box<dyn std::error::Error + Send + Sync>> {
    let key_pem = key_pem.ok_or("private key required")?;
    if key_pem.starts_with(b"-----BEGIN ENCRYPTED") {
        return Err(crate::error::ERR_TLS_OPTION_NOT_SUPPORTED.into());
    }
    Ok(PrivateKeyDer::from_pem_slice(key_pem)?)
}

fn resolve_server_name(
    identity_name: &str,
    sni_name: Option<&str>,
) -> Result<ServerName<'static>, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(sni) = sni_name {
        return Ok(ServerName::try_from(sni)
            .map_err(|_| "invalid server name")?
            .to_owned());
    }
    if let Ok(ip) = identity_name.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip.into()));
    }
    Ok(ServerName::try_from(identity_name)
        .map_err(|_| "invalid server name")?
        .to_owned())
}

fn build_certified_key(
    context: &SecureContext,
) -> Result<Arc<rustls::sign::CertifiedKey>, Box<dyn std::error::Error + Send + Sync>> {
    let certs = parse_cert_chain(&context.cert_chain)?;
    let key = parse_private_key(context.key.as_deref())?;
    let provider = get_crypto_provider();
    let signing_key = provider.key_provider.load_private_key(key.clone_key())?;
    Ok(Arc::new(rustls::sign::CertifiedKey::new(
        certs,
        signing_key,
    )))
}

fn decode_alpn(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut protocols = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let len = buf[pos] as usize;
        pos += 1;
        if pos + len > buf.len() {
            break;
        }
        protocols.push(buf[pos..pos + len].to_vec());
        pos += len;
    }
    protocols
}

fn verify_peer_chain(
    peer_certs: &[Vec<u8>],
    custom_ca: Option<&[Vec<u8>]>,
) -> (bool, Option<String>) {
    if peer_certs.is_empty() {
        return (false, Some("no peer certificate".to_string()));
    }

    let roots = match load_rustls_root_store(custom_ca) {
        Ok(store) => store,
        Err(err) => return (false, Some(err.to_string())),
    };

    let provider = get_crypto_provider();
    let end_entity = CertificateDer::from(peer_certs[0].clone());
    let end = match EndEntityCert::try_from(&end_entity) {
        Ok(end) => end,
        Err(err) => return (false, Some(err.to_string())),
    };

    let intermediates: Vec<CertificateDer<'_>> = peer_certs
        .iter()
        .skip(1)
        .map(|c| CertificateDer::from(c.clone()))
        .collect();

    match end.verify_for_usage(
        provider.signature_verification_algorithms.all,
        &roots.roots,
        &intermediates,
        UnixTime::now(),
        KeyUsage::server_auth(),
        None,
        None,
    ) {
        Ok(_) => (true, None),
        Err(err) => (false, Some(err.to_string())),
    }
}

#[derive(Debug)]
struct ChainVerifier {
    provider: Arc<CryptoProvider>,
    anchors: Vec<TrustAnchor<'static>>,
}

impl ChainVerifier {
    fn new(provider: Arc<CryptoProvider>, roots: RootCertStore) -> Self {
        Self {
            provider,
            anchors: roots.roots,
        }
    }
}

impl ServerCertVerifier for ChainVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let end = EndEntityCert::try_from(end_entity).map_err(|e| Error::General(e.to_string()))?;
        let algs = self.provider.signature_verification_algorithms.all;
        end.verify_for_usage(
            algs,
            &self.anchors,
            intermediates,
            now,
            KeyUsage::server_auth(),
            None,
            None,
        )
        .map_err(|e| Error::General(e.to_string()))?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[derive(Debug)]
struct RecordingVerifier {
    inner: ChainVerifier,
    recorded: Arc<Mutex<VerifyRecord>>,
}

impl RecordingVerifier {
    fn new(inner: ChainVerifier, recorded: Arc<Mutex<VerifyRecord>>) -> Self {
        Self { inner, recorded }
    }
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        match self
            .inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
        {
            Ok(assertion) => {
                if let Ok(mut rec) = self.recorded.lock() {
                    rec.ok = true;
                    rec.error = None;
                }
                Ok(assertion)
            },
            Err(e) => {
                let msg = e.to_string();
                if let Ok(mut rec) = self.recorded.lock() {
                    rec.ok = false;
                    rec.error = Some(msg);
                }
                Ok(ServerCertVerified::assertion())
            },
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

#[derive(Debug)]
struct SniResolver {
    registry: Arc<RwLock<SniRegistry>>,
    default: Arc<SecureContext>,
    selected_local_cert: Arc<Mutex<Option<Vec<u8>>>>,
}

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let context = client_hello
            .server_name()
            .and_then(|name| self.registry.read().ok()?.resolve(name))
            .unwrap_or_else(|| self.default.clone());
        if let Ok(mut selected) = self.selected_local_cert.lock() {
            *selected = pem_certs_to_der(&context.cert_chain)
                .ok()
                .and_then(|chain| chain.into_iter().next());
        }
        build_certified_key(&context).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::TlsVersion;

    #[test]
    fn decode_alpn_wire_format() {
        let decoded = decode_alpn(b"\x02h2\x08http/1.1");
        assert_eq!(decoded, vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
    }

    #[test]
    fn build_client_config_default() {
        let ctx = SecureContext {
            ca: Vec::new(),
            cert_chain: Vec::new(),
            key: None,
            passphrase: None,
            min_version: TlsVersion::V1_2,
            max_version: TlsVersion::V1_3,
            alpn_protocols: None,
        };
        assert!(build_client_config(&ctx, true, None).is_ok());
    }
}
