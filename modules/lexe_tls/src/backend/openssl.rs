// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use openssl::ssl::{
    NameType, SniError, Ssl, SslAcceptor, SslAcceptorBuilder, SslConnector, SslMethod, SslRef,
    SslVerifyMode,
};
use openssl::x509::X509;
use std::sync::{Arc, Mutex, RwLock};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::backend::{
    AcceptOptions, ClientTlsStream, ConnectOptions, ServerTlsStream, TlsConnectionInfo,
    VerifyRecord,
};
use crate::root_ca::load_openssl_ca_store;
use crate::secure_context::SecureContext;
use crate::sni::SniRegistry;
use crate::version::TlsVersion;

pub fn build_client_config(
    context: &SecureContext,
    _reject_unauthorized: bool,
) -> Result<SslConnector, Box<dyn std::error::Error + Send + Sync>> {
    let mut builder = SslConnector::builder(SslMethod::tls_client())?;
    apply_version(&mut builder, context.min_version, context.max_version)?;

    builder.set_verify(SslVerifyMode::PEER);
    let custom_ca = if context.ca.is_empty() {
        None
    } else {
        Some(context.ca.as_slice())
    };
    load_openssl_ca_store(&mut builder, custom_ca)?;

    if !context.cert_chain.is_empty() {
        let cert = X509::from_pem(&context.cert_chain[0])?;
        builder.set_certificate(&cert)?;
        if let Some(key_pem) = &context.key {
            let pkey = if let Some(pass) = &context.passphrase {
                openssl::pkey::PKey::private_key_from_pem_passphrase(key_pem, pass.as_bytes())?
            } else {
                openssl::pkey::PKey::private_key_from_pem(key_pem)?
            };
            builder.set_private_key(&pkey)?;
            builder.check_private_key()?;
        }
        for extra in context.cert_chain.iter().skip(1) {
            let chain_cert = X509::from_pem(extra)?;
            builder.add_extra_chain_cert(chain_cert)?;
        }
    }

    if let Some(alpn) = &context.alpn_protocols {
        builder.set_alpn_protos(alpn)?;
    }

    Ok(builder.build())
}

pub fn build_server_config(
    context: &SecureContext,
    request_cert: bool,
    reject_unauthorized: bool,
    sni_registry: Arc<RwLock<SniRegistry>>,
    selected_local_cert: Arc<Mutex<Option<Vec<u8>>>>,
) -> Result<SslAcceptor, Box<dyn std::error::Error + Send + Sync>> {
    let default_context = Arc::new(context.clone());
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;
    apply_server_version(&mut builder, context.min_version, context.max_version)?;
    apply_server_identity(&mut builder, context)?;

    if request_cert {
        let mode = if reject_unauthorized {
            SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT
        } else {
            SslVerifyMode::PEER
        };
        builder.set_verify(mode);
        if !context.ca.is_empty() {
            let mut store = openssl::x509::store::X509StoreBuilder::new()?;
            for ca in &context.ca {
                store.add_cert(X509::from_pem(ca)?)?;
            }
            builder.set_verify_cert_store(store.build())?;
        }
    }

    if let Some(alpn) = &context.alpn_protocols {
        set_server_alpn(&mut builder, alpn)?;
    }

    let registry = sni_registry.clone();
    let selected = selected_local_cert.clone();
    builder.set_servername_callback(move |ssl, _alert| {
        let context = ssl
            .servername(NameType::HOST_NAME)
            .and_then(|name| registry.read().ok().and_then(|r| r.resolve(name)))
            .unwrap_or_else(|| default_context.clone());
        if let Ok(mut slot) = selected.lock() {
            *slot = crate::pem::pem_certs_to_der(&context.cert_chain)
                .ok()
                .and_then(|chain| chain.into_iter().next());
        }
        apply_secure_context_to_ssl(ssl, &context)
    });

    Ok(builder.build())
}

fn apply_server_identity(
    builder: &mut SslAcceptorBuilder,
    context: &SecureContext,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if context.cert_chain.is_empty() {
        return Err("server certificate is required".into());
    }
    let key_pem = context
        .key
        .as_deref()
        .ok_or("server private key is required")?;
    let cert = X509::from_pem(&context.cert_chain[0])?;
    builder.set_certificate(&cert)?;
    let pkey = if let Some(pass) = &context.passphrase {
        openssl::pkey::PKey::private_key_from_pem_passphrase(key_pem, pass.as_bytes())?
    } else {
        openssl::pkey::PKey::private_key_from_pem(key_pem)?
    };
    builder.set_private_key(&pkey)?;
    builder.check_private_key()?;
    for extra in context.cert_chain.iter().skip(1) {
        let chain_cert = X509::from_pem(extra)?;
        builder.add_extra_chain_cert(chain_cert)?;
    }
    Ok(())
}

fn apply_secure_context_to_ssl(ssl: &mut SslRef, context: &SecureContext) -> Result<(), SniError> {
    if context.cert_chain.is_empty() {
        return Ok(());
    }

    let cert = X509::from_pem(&context.cert_chain[0]).map_err(|_| SniError::NOACK)?;
    ssl.set_certificate(&cert).map_err(|_| SniError::NOACK)?;
    if let Some(key_pem) = &context.key {
        let pkey = if let Some(pass) = &context.passphrase {
            openssl::pkey::PKey::private_key_from_pem_passphrase(key_pem, pass.as_bytes())
        } else {
            openssl::pkey::PKey::private_key_from_pem(key_pem)
        }
        .map_err(|_| SniError::NOACK)?;
        ssl.set_private_key(&pkey).map_err(|_| SniError::NOACK)?;
    }
    for extra in context.cert_chain.iter().skip(1) {
        let chain_cert = X509::from_pem(extra).map_err(|_| SniError::NOACK)?;
        ssl.add_chain_cert(chain_cert)
            .map_err(|_| SniError::NOACK)?;
    }
    Ok(())
}

fn select_alpn_from_client<'a>(server: &[u8], client: &'a [u8]) -> Option<&'a [u8]> {
    let mut server_offset = 0;
    while server_offset < server.len() {
        let len = server[server_offset] as usize;
        server_offset += 1;
        if server_offset + len > server.len() {
            break;
        }
        let proto = &server[server_offset..server_offset + len];
        server_offset += len;

        let mut client_offset = 0;
        while client_offset < client.len() {
            let clen = client[client_offset] as usize;
            client_offset += 1;
            if client_offset + clen > client.len() {
                break;
            }
            if &client[client_offset..client_offset + clen] == proto {
                return Some(&client[client_offset..client_offset + clen]);
            }
            client_offset += clen;
        }
    }
    None
}

fn set_server_alpn(
    builder: &mut SslAcceptorBuilder,
    alpn: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server_protos = alpn.to_vec();
    builder.set_alpn_select_callback(move |_, client| {
        select_alpn_from_client(&server_protos, client).ok_or(openssl::ssl::AlpnError::NOACK)
    });
    Ok(())
}

pub async fn connect<IO>(
    stream: IO,
    options: ConnectOptions,
) -> Result<ClientTlsStream<IO>, Box<dyn std::error::Error + Send + Sync>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send,
{
    let connector = build_client_config(&options.context, options.reject_unauthorized)?;
    let mut ssl = Ssl::new(connector.context())?;
    if let Some(sni) = &options.sni_name {
        ssl.set_hostname(sni)?;
    }

    if options.reject_unauthorized {
        ssl.set_verify(SslVerifyMode::PEER);
    } else if let Some(record) = &options.verify_record {
        let record = Arc::clone(record);
        ssl.set_verify_callback(SslVerifyMode::PEER, move |preverify_ok, x509_ctx| {
            if !preverify_ok {
                if let Ok(mut recorded) = record.lock() {
                    recorded.ok = false;
                    if recorded.error.is_none() {
                        recorded.error = Some(x509_ctx.error().to_string());
                    }
                }
            }
            true
        });
    }

    let mut tls_stream = tokio_openssl::SslStream::new(ssl, stream)?;
    let handshake = std::pin::Pin::new(&mut tls_stream).connect();
    if let Some(timeout) = options.timeout {
        match tokio::time::timeout(timeout, handshake).await {
            Ok(result) => result?,
            Err(_) => return Err("TLS handshake timeout".into()),
        }
    } else {
        handshake.await?;
    }
    Ok(tls_stream)
}

pub async fn accept<IO>(
    stream: IO,
    options: AcceptOptions,
) -> Result<ServerTlsStream<IO>, Box<dyn std::error::Error + Send + Sync>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send,
{
    let acceptor = build_server_config(
        &options.context,
        options.request_cert,
        options.reject_unauthorized,
        options.sni_registry,
        options.selected_local_cert,
    )?;
    let ssl = Ssl::new(acceptor.context())?;
    let mut tls_stream = tokio_openssl::SslStream::new(ssl, stream)?;
    let accept = std::pin::Pin::new(&mut tls_stream).accept();
    if let Some(timeout) = options.timeout {
        match tokio::time::timeout(timeout, accept).await {
            Ok(result) => result?,
            Err(_) => return Err("TLS handshake timeout".into()),
        }
    } else {
        accept.await?;
    }
    Ok(tls_stream)
}

pub fn inspect_client_connection<IO>(
    stream: &ClientTlsStream<IO>,
    verify_record: Option<&Arc<Mutex<VerifyRecord>>>,
) -> TlsConnectionInfo {
    let (chain_authorized, chain_error) = if let Some(record) = verify_record {
        if let Ok(recorded) = record.lock() {
            (recorded.ok, recorded.error.clone())
        } else {
            chain_status_from_verify_result(stream.ssl())
        }
    } else {
        chain_status_from_verify_result(stream.ssl())
    };
    build_connection_info(stream, chain_authorized, chain_error, false, None)
}

pub fn inspect_server_connection<IO>(
    stream: &ServerTlsStream<IO>,
    reject_unauthorized: bool,
    selected_local_cert: &Arc<Mutex<Option<Vec<u8>>>>,
) -> TlsConnectionInfo {
    let (chain_authorized, chain_error) = if reject_unauthorized {
        chain_status_from_verify_result(stream.ssl())
    } else {
        (true, None)
    };
    build_connection_info(
        stream,
        chain_authorized,
        chain_error,
        true,
        Some(selected_local_cert),
    )
}

fn build_connection_info<IO>(
    stream: &tokio_openssl::SslStream<IO>,
    chain_authorized: bool,
    chain_error: Option<String>,
    server: bool,
    selected_local_cert: Option<&Arc<Mutex<Option<Vec<u8>>>>>,
) -> TlsConnectionInfo {
    let ssl = stream.ssl();
    let protocol = Some(ssl.version_str().to_string());
    let cipher = ssl.current_cipher().map(|c| c.name().to_string());
    let cipher_standard_name = ssl
        .current_cipher()
        .and_then(|c| c.standard_name().map(str::to_string));
    let alpn = ssl.selected_alpn_protocol().map(|p| p.to_vec());
    let peer_certs = extract_peer_chain(stream);
    let local_cert = if server {
        extract_local_certificate(stream).or_else(|| {
            selected_local_cert.and_then(|arc| arc.lock().ok().and_then(|slot| slot.clone()))
        })
    } else {
        None
    };
    let client_servername = if server {
        ssl.servername(NameType::HOST_NAME).map(str::to_string)
    } else {
        None
    };

    TlsConnectionInfo {
        protocol,
        cipher,
        cipher_standard_name,
        alpn_protocol: alpn,
        chain_authorized,
        chain_error: chain_error.clone(),
        authorized: chain_authorized,
        authorization_error: chain_error,
        peer_certs,
        local_cert,
        client_servername,
    }
}

fn chain_status_from_verify_result(ssl: &openssl::ssl::SslRef) -> (bool, Option<String>) {
    let result = ssl.verify_result();
    if result == openssl::x509::X509VerifyResult::OK {
        (true, None)
    } else {
        (false, Some(result.to_string()))
    }
}

pub fn extract_peer_chain_client<IO>(stream: &ClientTlsStream<IO>) -> Vec<Vec<u8>> {
    extract_peer_chain(stream)
}

pub fn extract_peer_chain_server<IO>(stream: &ServerTlsStream<IO>) -> Vec<Vec<u8>> {
    extract_peer_chain(stream)
}

pub fn extract_peer_chain<IO>(stream: &tokio_openssl::SslStream<IO>) -> Vec<Vec<u8>> {
    stream
        .ssl()
        .peer_cert_chain()
        .map(|chain| chain.iter().filter_map(|c| c.to_der().ok()).collect())
        .unwrap_or_default()
}

pub fn extract_local_certificate<IO>(stream: &ServerTlsStream<IO>) -> Option<Vec<u8>> {
    stream.ssl().certificate().and_then(|c| c.to_der().ok())
}

pub fn supported_ciphers() -> Vec<String> {
    vec![
        "TLS_AES_256_GCM_SHA384".to_string(),
        "TLS_AES_128_GCM_SHA256".to_string(),
        "TLS_CHACHA20_POLY1305_SHA256".to_string(),
        "ECDHE-RSA-AES128-GCM-SHA256".to_string(),
        "ECDHE-RSA-AES256-GCM-SHA384".to_string(),
    ]
}

fn apply_version(
    builder: &mut openssl::ssl::SslConnectorBuilder,
    min: TlsVersion,
    max: TlsVersion,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    builder.set_min_proto_version(Some(min.to_openssl()))?;
    builder.set_max_proto_version(Some(max.to_openssl()))?;
    Ok(())
}

fn apply_server_version(
    builder: &mut openssl::ssl::SslAcceptorBuilder,
    min: TlsVersion,
    max: TlsVersion,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    builder.set_min_proto_version(Some(min.to_openssl()))?;
    builder.set_max_proto_version(Some(max.to_openssl()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(build_client_config(&ctx, true).is_ok());
        assert!(build_client_config(&ctx, false).is_ok());
    }

    #[test]
    fn server_alpn_callback_drops_with_builder() {
        let mut builder =
            SslAcceptor::mozilla_intermediate(SslMethod::tls()).expect("acceptor builder");
        let alpn = [2, b'h', b'2', 2, b'h', b'3'];
        set_server_alpn(&mut builder, &alpn).expect("alpn callback");
        drop(builder);
    }
}
