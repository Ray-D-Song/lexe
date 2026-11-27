// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use base64::{engine::general_purpose::STANDARD, Engine as _};
use p12_keystore::{KeyStore, KeyStoreEntry, Pkcs12ImportPolicy};
use pkcs8::EncryptedPrivateKeyInfoRef;

/// Split a PEM blob into individual PEM blocks (certificates or keys).
pub fn split_pem_chain(data: &[u8]) -> Vec<Vec<u8>> {
    let Ok(text) = std::str::from_utf8(data) else {
        return vec![data.to_vec()];
    };

    let mut blocks = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.starts_with("-----BEGIN ") && !current.is_empty() {
            blocks.push(current.into_bytes());
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
        if line.starts_with("-----END ") {
            blocks.push(current.into_bytes());
            current = String::new();
        }
    }

    if !current.is_empty() {
        blocks.push(current.into_bytes());
    }

    if blocks.is_empty() && !data.is_empty() {
        blocks.push(data.to_vec());
    }

    blocks
}

/// Return DER-encoded certificates from a PEM chain.
pub fn pem_certs_to_der(pem_blocks: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, String> {
    pem_blocks
        .iter()
        .filter(|block| is_cert_pem(block))
        .map(|block| pem_block_to_der(block))
        .collect()
}

pub fn is_cert_pem(block: &[u8]) -> bool {
    block.starts_with(b"-----BEGIN CERTIFICATE-----")
}

pub fn is_key_pem(block: &[u8]) -> bool {
    block.starts_with(b"-----BEGIN ")
        && (block.starts_with(b"-----BEGIN PRIVATE KEY-----")
            || block.starts_with(b"-----BEGIN RSA PRIVATE KEY-----")
            || block.starts_with(b"-----BEGIN EC PRIVATE KEY-----")
            || block.starts_with(b"-----BEGIN ENCRYPTED PRIVATE KEY-----"))
}

pub fn pem_block_to_der(block: &[u8]) -> Result<Vec<u8>, String> {
    let body: String = std::str::from_utf8(block)
        .map_err(|e| e.to_string())?
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    STANDARD.decode(body).map_err(|e| e.to_string())
}

pub fn der_to_pem(label: &str, der: &[u8]) -> Vec<u8> {
    let encoded = STANDARD.encode(der);
    let mut pem = format!("-----BEGIN {label}-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str(&format!("-----END {label}-----\n"));
    pem.into_bytes()
}

pub type ParsedPfx = (Option<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>);

pub fn parse_pfx(pfx: &[u8], passphrase: Option<&str>) -> Result<ParsedPfx, String> {
    let pass = passphrase.unwrap_or("");
    let store = KeyStore::from_pkcs12(pfx, pass, Pkcs12ImportPolicy::default())
        .map_err(|e| format!("failed to parse PKCS#12 pfx: {e}"))?;
    let mut key_pem = None;
    let mut certs = Vec::new();
    let mut ca = Vec::new();
    for (_alias, entry) in store.entries() {
        match entry {
            KeyStoreEntry::PrivateKeyChain(chain) => {
                if key_pem.is_none() {
                    key_pem = Some(der_to_pem("PRIVATE KEY", chain.key().as_der()));
                    for c in chain.certs() {
                        certs.push(der_to_pem("CERTIFICATE", c.as_der()));
                    }
                }
            },
            KeyStoreEntry::Certificate(cert) => {
                ca.push(der_to_pem("CERTIFICATE", cert.as_der()));
            },
            KeyStoreEntry::Secret(_) => {},
        }
    }
    Ok((key_pem, certs, ca))
}

pub fn decrypt_private_key_pem_if_needed(
    key_pem: &[u8],
    passphrase: Option<&str>,
) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(key_pem).unwrap_or("");
    if !text.contains("BEGIN ENCRYPTED PRIVATE KEY") {
        return Ok(key_pem.to_vec());
    }
    let pass =
        passphrase.ok_or_else(|| "passphrase required for encrypted private key".to_string())?;
    let der = pem_block_to_der(key_pem)?;
    let info = EncryptedPrivateKeyInfoRef::try_from(der.as_slice())
        .map_err(|e| format!("invalid encrypted private key: {e}"))?;
    let doc = info
        .decrypt(pass.as_bytes())
        .map_err(|_| "bad decrypt: invalid passphrase or unsupported key cipher".to_string())?;
    Ok(der_to_pem("PRIVATE KEY", doc.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN: &str = "-----BEGIN CERTIFICATE-----\nY2VydDE=\n-----END CERTIFICATE-----\n\
-----BEGIN CERTIFICATE-----\nY2VydDI=\n-----END CERTIFICATE-----\n";

    #[test]
    fn split_multi_pem_chain() {
        let blocks = split_pem_chain(CHAIN.as_bytes());
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].starts_with(b"-----BEGIN CERTIFICATE-----"));
        assert!(blocks[1].starts_with(b"-----BEGIN CERTIFICATE-----"));
    }

    #[test]
    fn pem_certs_to_der_extracts_der() {
        let blocks = split_pem_chain(CHAIN.as_bytes());
        let der = pem_certs_to_der(&blocks).unwrap();
        assert_eq!(der.len(), 2);
        assert_eq!(der[0], b"cert1");
        assert_eq!(der[1], b"cert2");
    }
}
