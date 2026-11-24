// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::sync::OnceLock;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
use rustls::pki_types::{pem::PemObject, CertificateDer};

#[cfg(feature = "tls-openssl")]
use openssl::x509::X509;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
static EXTRA_CA_CERTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();

#[cfg(feature = "tls-openssl")]
static EXTRA_CA_CERTS_OPENSSL: OnceLock<Vec<Vec<u8>>> = OnceLock::new();

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub fn set_extra_ca_certs(certs: Vec<CertificateDer<'static>>) {
    _ = EXTRA_CA_CERTS.set(certs);
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub fn get_extra_ca_certs() -> Option<Vec<CertificateDer<'static>>> {
    let certs = EXTRA_CA_CERTS.get_or_init(Vec::new).clone();
    if certs.is_empty() {
        None
    } else {
        Some(certs)
    }
}

#[cfg(feature = "tls-openssl")]
pub fn set_extra_ca_certs_openssl(certs: Vec<Vec<u8>>) {
    _ = EXTRA_CA_CERTS_OPENSSL.set(certs);
}

#[cfg(feature = "tls-openssl")]
pub fn get_extra_ca_certs_openssl() -> Option<Vec<Vec<u8>>> {
    let certs = EXTRA_CA_CERTS_OPENSSL.get_or_init(Vec::new).clone();
    if certs.is_empty() {
        None
    } else {
        Some(certs)
    }
}

/// Unified entry point used by both config modules.
pub fn set_extra_ca_certs_bytes(certs: Vec<Vec<u8>>) {
    #[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
    {
        let parsed: Vec<CertificateDer<'static>> = certs
            .iter()
            .filter_map(|pem| CertificateDer::from_pem_slice(pem).ok())
            .collect();
        set_extra_ca_certs(parsed);
    }
    #[cfg(feature = "tls-openssl")]
    {
        set_extra_ca_certs_openssl(certs);
    }
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub fn load_rustls_root_store(
    custom_ca: Option<&[Vec<u8>]>,
) -> Result<rustls::RootCertStore, Box<dyn std::error::Error + Send + Sync>> {
    use rustls::RootCertStore;

    let mut root_certificates = RootCertStore::empty();

    if let Some(ca) = custom_ca {
        for cert in ca {
            root_certificates.add(CertificateDer::from_pem_slice(cert)?)?;
        }
        return Ok(root_certificates);
    }

    #[cfg(feature = "webpki-roots")]
    {
        use webpki_roots::TLS_SERVER_ROOTS;
        for cert in TLS_SERVER_ROOTS.iter().cloned() {
            root_certificates.roots.push(cert);
        }
    }

    #[cfg(feature = "native-roots")]
    {
        let load_results = rustls_native_certs::load_native_certs();
        for cert in load_results.certs {
            if let Err(err) = root_certificates.add(cert) {
                tracing::debug!("rustls failed to parse DER certificate: {err:?}");
            }
        }
    }

    if let Some(extra_ca_certs) = get_extra_ca_certs() {
        root_certificates.add_parsable_certificates(extra_ca_certs);
    }

    Ok(root_certificates)
}

#[cfg(feature = "tls-openssl")]
pub fn load_openssl_ca_store(
    builder: &mut openssl::ssl::SslConnectorBuilder,
    custom_ca: Option<&[Vec<u8>]>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(ca) = custom_ca {
        for cert_pem in ca {
            let cert = X509::from_pem(cert_pem)?;
            builder.cert_store_mut().add_cert(cert)?;
        }
    } else {
        builder.set_default_verify_paths()?;
        if let Some(extra_certs) = get_extra_ca_certs_openssl() {
            for cert_der in extra_certs {
                if let Ok(cert) = X509::from_der(&cert_der) {
                    let _ = builder.cert_store_mut().add_cert(cert);
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "tls-openssl")]
pub fn load_openssl_server_ca(
    builder: &mut openssl::ssl::SslAcceptorBuilder,
    custom_ca: Option<&[Vec<u8>]>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(ca) = custom_ca {
        for cert_pem in ca {
            let cert = X509::from_pem(cert_pem)?;
            builder.cert_store_mut().add_cert(cert)?;
        }
    }
    Ok(())
}
