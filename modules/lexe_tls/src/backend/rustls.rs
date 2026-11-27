// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, RwLock};

use once_cell::sync::Lazy;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{
    ClientSessionStore, ResolvesClientCert, Resumption, Tls12ClientSessionValue,
    Tls13ClientSessionValue,
};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::TrustAnchor;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{
    ClientHello, ProducesTickets, ResolvesServerCert, ServerSessionMemoryCache,
    StoresServerSessions,
};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, Error, HandshakeKind, NamedGroup,
    RootCertStore, ServerConfig, SignatureScheme,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{server::StartHandshake, LazyConfigAcceptor, TlsAcceptor, TlsConnector};
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

#[derive(Default)]
struct ClientSessionQueue {
    kx_hint: Option<NamedGroup>,
    tls12: Option<Tls12ClientSessionValue>,
    tls13: VecDeque<Tls13ClientSessionValue>,
    reject_unauthorized: bool,
    ca: Vec<Vec<u8>>,
    cert_chain: Vec<Vec<u8>>,
    verifier: Option<Arc<dyn ServerCertVerifier>>,
    client_creds: Option<Arc<dyn ResolvesClientCert>>,
}

type ClientSessionEntry = (Vec<u8>, Arc<Mutex<ClientSessionQueue>>);

static CLIENT_SESSIONS: Lazy<Mutex<VecDeque<ClientSessionEntry>>> =
    Lazy::new(|| Mutex::new(VecDeque::with_capacity(256)));

fn lookup_client_session(token: &[u8]) -> Option<Arc<Mutex<ClientSessionQueue>>> {
    let guard = CLIENT_SESSIONS.lock().ok()?;
    guard
        .iter()
        .find(|(k, _)| k.as_slice() == token)
        .map(|(_, v)| Arc::clone(v))
}

fn save_client_session(token: Vec<u8>, queue: Arc<Mutex<ClientSessionQueue>>) {
    if let Ok(mut guard) = CLIENT_SESSIONS.lock() {
        if let Some((_, existing)) = guard.iter_mut().find(|(k, _)| *k == token) {
            *existing = queue;
            return;
        }
        if guard.len() >= 256 {
            guard.pop_front();
        }
        guard.push_back((token, queue));
    }
}

#[derive(Debug)]
struct LexeClientSessionStore {
    queue: Arc<Mutex<ClientSessionQueue>>,
    token: Vec<u8>,
    session_slot: Option<Arc<Mutex<Option<Vec<u8>>>>>,
    session_notify: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
}

impl std::fmt::Debug for ClientSessionQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientSessionQueue")
            .field("tls13_len", &self.tls13.len())
            .field("has_tls12", &self.tls12.is_some())
            .finish()
    }
}

impl LexeClientSessionStore {
    fn new(
        resume_token: Option<&[u8]>,
        session_slot: Option<Arc<Mutex<Option<Vec<u8>>>>>,
        session_notify: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
    ) -> Self {
        let queue = resume_token
            .and_then(lookup_client_session)
            .unwrap_or_else(|| Arc::new(Mutex::new(ClientSessionQueue::default())));
        let token = match resume_token {
            Some(existing) if !existing.is_empty() => existing.to_vec(),
            _ => {
                use std::time::{SystemTime, UNIX_EPOCH};
                static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                let mut tok = Vec::with_capacity(16);
                tok.extend_from_slice(&seq.to_be_bytes());
                tok.extend_from_slice(&nanos.to_be_bytes());
                tok
            },
        };
        Self {
            queue,
            token,
            session_slot,
            session_notify,
        }
    }

    fn record_session_available(&self) {
        save_client_session(self.token.clone(), Arc::clone(&self.queue));
        if let Some(slot) = &self.session_slot {
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(self.token.clone());
            }
        }
        if let Some(tx) = &self.session_notify {
            let _ = tx.send(self.token.clone());
        }
    }
}

impl ClientSessionStore for LexeClientSessionStore {
    fn set_kx_hint(&self, _server_name: ServerName<'static>, group: NamedGroup) {
        if let Ok(mut q) = self.queue.lock() {
            q.kx_hint = Some(group);
        }
    }

    fn kx_hint(&self, _server_name: &ServerName<'_>) -> Option<NamedGroup> {
        self.queue.lock().ok().and_then(|q| q.kx_hint)
    }

    fn set_tls12_session(&self, _server_name: ServerName<'static>, value: Tls12ClientSessionValue) {
        if let Ok(mut q) = self.queue.lock() {
            q.tls12 = Some(value);
        }
        self.record_session_available();
    }

    fn tls12_session(&self, _server_name: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
        self.queue.lock().ok().and_then(|q| q.tls12.clone())
    }

    fn remove_tls12_session(&self, _server_name: &ServerName<'_>) {
        if let Ok(mut q) = self.queue.lock() {
            q.tls12 = None;
        }
    }

    fn insert_tls13_ticket(
        &self,
        _server_name: ServerName<'static>,
        value: Tls13ClientSessionValue,
    ) {
        if let Ok(mut q) = self.queue.lock() {
            if q.tls13.len() >= 8 {
                q.tls13.pop_front();
            }
            q.tls13.push_back(value);
        }
        self.record_session_available();
    }

    fn take_tls13_ticket(&self, _server_name: &ServerName<'_>) -> Option<Tls13ClientSessionValue> {
        self.queue.lock().ok().and_then(|mut q| q.tls13.pop_front())
    }
}

pub struct ServerSharedState {
    pub session_storage: Arc<dyn StoresServerSessions>,
    pub ticketer: Option<Arc<dyn ProducesTickets>>,
}

pub fn new_server_shared_state() -> Arc<ServerSharedState> {
    #[cfg(feature = "tls-graviola")]
    let ticketer = rustls_graviola::Ticketer::new().ok();
    #[cfg(feature = "tls-ring")]
    let ticketer = rustls::crypto::ring::Ticketer::new().ok();
    #[cfg(feature = "tls-aws-lc")]
    let ticketer = rustls::crypto::aws_lc_rs::Ticketer::new().ok();

    Arc::new(ServerSharedState {
        session_storage: ServerSessionMemoryCache::new(1024),
        ticketer,
    })
}

pub fn build_client_config(
    context: &SecureContext,
    reject_unauthorized: bool,
    verify_record: Option<Arc<Mutex<VerifyRecord>>>,
) -> Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    build_client_config_inner(context, reject_unauthorized, verify_record, None).map(|(c, _)| c)
}

fn build_client_config_inner(
    context: &SecureContext,
    reject_unauthorized: bool,
    verify_record: Option<Arc<Mutex<VerifyRecord>>>,
    override_verifier: Option<Arc<dyn ServerCertVerifier>>,
) -> Result<(ClientConfig, Arc<dyn ServerCertVerifier>), Box<dyn std::error::Error + Send + Sync>> {
    let provider = get_crypto_provider();
    let versions = rustls_protocol_versions(context.min_version, context.max_version);

    let verifier: Arc<dyn ServerCertVerifier> = if let Some(v) = override_verifier {
        v
    } else {
        let roots = if context.ca.is_empty() {
            load_rustls_root_store(None)?
        } else {
            load_rustls_root_store(Some(&context.ca))?
        };

        let chain_verifier = ChainVerifier::new(provider.clone(), roots);
        if reject_unauthorized {
            Arc::new(chain_verifier)
        } else {
            let record = verify_record.ok_or("verify_record required in permissive mode")?;
            Arc::new(RecordingVerifier::new(chain_verifier, record))
        }
    };

    let mut config = if context.cert_chain.is_empty() {
        ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&versions)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::clone(&verifier))
            .with_no_client_auth()
    } else {
        let certs = parse_cert_chain(&context.cert_chain)?;
        let key = parse_private_key(context.key.as_deref())?;
        ClientConfig::builder_with_provider(get_crypto_provider())
            .with_protocol_versions(&versions)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::clone(&verifier))
            .with_client_auth_cert(certs, key)?
    };

    if let Some(alpn) = &context.alpn_protocols {
        config.alpn_protocols = decode_alpn(alpn);
    }
    Ok((config, verifier))
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
        let client_verifier: Arc<dyn ClientCertVerifier> = if reject_unauthorized {
            rustls::server::WebPkiClientVerifier::builder_with_provider(
                roots.into(),
                provider.clone(),
            )
            .build()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
        } else {
            Arc::new(SoftClientCertVerifier {
                provider: provider.clone(),
                subjects: roots.subjects(),
            })
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

#[allow(clippy::too_many_arguments)]
pub fn build_server_config_with_options(
    context: &SecureContext,
    request_cert: bool,
    reject_unauthorized: bool,
    sni_registry: Arc<RwLock<SniRegistry>>,
    selected_local_cert: Arc<Mutex<Option<Vec<u8>>>>,
    server_state: Option<Arc<ServerSharedState>>,
    chosen_alpn: Option<Vec<u8>>,
    use_sni_resolver: bool,
) -> Result<ServerConfig, Box<dyn std::error::Error + Send + Sync>> {
    let mut config = build_server_config(
        context,
        request_cert,
        reject_unauthorized,
        sni_registry,
        selected_local_cert.clone(),
    )?;
    if !use_sni_resolver {
        let certs = parse_cert_chain(&context.cert_chain)?;
        let key = parse_private_key(context.key.as_deref())?;
        if let Ok(mut slot) = selected_local_cert.lock() {
            *slot = certs.first().map(|c| c.to_vec());
        }
        let provider = get_crypto_provider();
        let signing_key = provider.key_provider.load_private_key(key)?;
        config.cert_resolver = Arc::new(SingleCertResolver(Arc::new(
            rustls::sign::CertifiedKey::new(certs, signing_key),
        )));
    }
    if let Some(proto) = chosen_alpn {
        config.alpn_protocols = vec![proto];
    }
    if let Some(state) = server_state {
        config.session_storage = Arc::clone(&state.session_storage);
        if let Some(ref ticketer) = state.ticketer {
            config.ticketer = Arc::clone(ticketer);
        }
    }
    Ok(config)
}

#[derive(Debug)]
struct SingleCertResolver(Arc<rustls::sign::CertifiedKey>);

impl ResolvesServerCert for SingleCertResolver {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

pub async fn connect<IO>(
    stream: IO,
    options: ConnectOptions,
) -> Result<ClientTlsStream<IO>, Box<dyn std::error::Error + Send + Sync>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send,
{
    let store = Arc::new(LexeClientSessionStore::new(
        options.session.as_deref(),
        options.session_slot,
        options.session_notify,
    ));
    let cached_auth = store.queue.lock().ok().and_then(|q| {
        if q.reject_unauthorized == options.reject_unauthorized
            && q.ca == options.context.ca
            && q.cert_chain == options.context.cert_chain
        {
            Some((
                Arc::clone(q.verifier.as_ref()?),
                Arc::clone(q.client_creds.as_ref()?),
            ))
        } else {
            None
        }
    });
    let (mut config, verifier) = build_client_config_inner(
        &options.context,
        options.reject_unauthorized,
        options.verify_record,
        cached_auth.as_ref().map(|(v, _)| Arc::clone(v)),
    )?;
    if let Some((_, c)) = cached_auth {
        config.client_auth_cert_resolver = c;
    } else if let Ok(mut q) = store.queue.lock() {
        q.reject_unauthorized = options.reject_unauthorized;
        q.ca = options.context.ca.clone();
        q.cert_chain = options.context.cert_chain.clone();
        q.verifier = Some(verifier);
        q.client_creds = Some(Arc::clone(&config.client_auth_cert_resolver));
    }
    config.resumption = Resumption::store(store);

    let connector = TlsConnector::from(Arc::new(config));
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
    let config = Arc::new(build_server_config_with_options(
        &options.context,
        options.request_cert,
        options.reject_unauthorized,
        options.sni_registry,
        options.selected_local_cert,
        options.server_state,
        options.chosen_alpn,
        true,
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

pub async fn accept_client_hello<IO>(
    stream: IO,
    timeout: Option<std::time::Duration>,
) -> Result<StartHandshake<IO>, Box<dyn std::error::Error + Send + Sync>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send,
{
    let lazy = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), stream);
    if let Some(t) = timeout {
        match tokio::time::timeout(t, lazy).await {
            Ok(res) => Ok(res?),
            Err(_) => Err("TLS handshake timeout".into()),
        }
    } else {
        Ok(lazy.await?)
    }
}

pub async fn finish_lazy_accept<IO>(
    start: StartHandshake<IO>,
    options: AcceptOptions,
) -> Result<ServerTlsStream<IO>, Box<dyn std::error::Error + Send + Sync>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send,
{
    let config = Arc::new(build_server_config_with_options(
        &options.context,
        options.request_cert,
        options.reject_unauthorized,
        options.sni_registry,
        options.selected_local_cert,
        options.server_state,
        options.chosen_alpn,
        false,
    )?);
    let fut = start.into_stream(config);
    if let Some(t) = options.timeout {
        match tokio::time::timeout(t, fut).await {
            Ok(res) => Ok(res?),
            Err(_) => Err("TLS handshake timeout".into()),
        }
    } else {
        Ok(fut.await?)
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
    let session_reused = session.handshake_kind() == Some(HandshakeKind::Resumed);

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
        session_reused,
    }
}

pub fn inspect_server_connection<IO>(
    stream: &ServerTlsStream<IO>,
    request_cert: bool,
    reject_unauthorized: bool,
    custom_ca: &[Vec<u8>],
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
    let session_reused = session.handshake_kind() == Some(HandshakeKind::Resumed);

    let (chain_authorized, chain_error) = if reject_unauthorized {
        (true, None)
    } else if !peer_certs.is_empty() {
        let ca_opt = (!custom_ca.is_empty()).then_some(custom_ca);
        verify_peer_chain(&peer_certs, ca_opt, KeyUsage::client_auth())
    } else if request_cert {
        (false, Some("UNABLE_TO_GET_ISSUER_CERT".to_string()))
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
        session_reused,
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
    key_usage: KeyUsage,
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
        key_usage,
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
struct SoftClientCertVerifier {
    provider: Arc<CryptoProvider>,
    subjects: Vec<DistinguishedName>,
}

impl ClientCertVerifier for SoftClientCertVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &self.subjects
    }

    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        Ok(ClientCertVerified::assertion())
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

    #[tokio::test]
    async fn tls13_session_resumption_works() {
        let root_pem = include_bytes!("../../../../libs/lexe_test_tls/data/root.pem").to_vec();
        let cert_pem = include_bytes!("../../../../libs/lexe_test_tls/data/server.pem").to_vec();
        let key_pem = include_bytes!("../../../../libs/lexe_test_tls/data/server.key").to_vec();

        let server_ctx = SecureContext {
            ca: Vec::new(),
            cert_chain: vec![cert_pem],
            key: Some(key_pem),
            passphrase: None,
            min_version: TlsVersion::V1_2,
            max_version: TlsVersion::V1_3,
            alpn_protocols: None,
        };
        let client_ctx = SecureContext {
            ca: vec![root_pem],
            cert_chain: Vec::new(),
            key: None,
            passphrase: None,
            min_version: TlsVersion::V1_2,
            max_version: TlsVersion::V1_3,
            alpn_protocols: None,
        };

        let server_state = new_server_shared_state();
        assert!(server_state.ticketer.is_some(), "ticketer must be Some");
        let sni_registry = Arc::new(RwLock::new(SniRegistry::new()));
        let selected_local_cert = Arc::new(Mutex::new(None));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let srv_state = Arc::clone(&server_state);
        let srv_ctx = server_ctx.clone();
        let srv_sni = Arc::clone(&sni_registry);
        let srv_sel = Arc::clone(&selected_local_cert);
        let server_task = tokio::spawn(async move {
            let mut results = Vec::new();
            for _ in 0..2 {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut stream = accept(
                    tcp,
                    AcceptOptions {
                        context: srv_ctx.clone(),
                        request_cert: false,
                        reject_unauthorized: true,
                        timeout: None,
                        sni_registry: Arc::clone(&srv_sni),
                        selected_local_cert: Arc::clone(&srv_sel),
                        server_state: Some(Arc::clone(&srv_state)),
                        chosen_alpn: None,
                    },
                )
                .await
                .unwrap();
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                results.push(stream.get_ref().1.handshake_kind());
                let mut buf = [0u8; 4];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(b"pong").await;
                let _ = stream.shutdown().await;
            }
            results
        });

        let session_slot = Arc::new(Mutex::new(None));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tcp1 = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut s1 = connect(
            tcp1,
            ConnectOptions {
                context: client_ctx.clone(),
                identity_name: "localhost".to_string(),
                sni_name: Some("localhost".to_string()),
                reject_unauthorized: true,
                timeout: None,
                verify_record: None,
                session: None,
                session_slot: Some(Arc::clone(&session_slot)),
                session_notify: Some(tx),
            },
        )
        .await
        .unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        s1.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        let _ = s1.read_exact(&mut buf).await;
        let token = rx.recv().await.unwrap();
        drop(s1);

        let tcp2 = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut s2 = connect(
            tcp2,
            ConnectOptions {
                context: client_ctx,
                identity_name: "localhost".to_string(),
                sni_name: Some("localhost".to_string()),
                reject_unauthorized: true,
                timeout: None,
                verify_record: None,
                session: Some(token),
                session_slot: None,
                session_notify: None,
            },
        )
        .await
        .unwrap();
        s2.write_all(b"ping").await.unwrap();
        let _ = s2.read_exact(&mut buf).await;
        let client_hk2 = s2.get_ref().1.handshake_kind();
        let server_results = server_task.await.unwrap();
        assert_eq!(
            client_hk2,
            Some(HandshakeKind::Resumed),
            "server_results={server_results:?}"
        );
    }
}
