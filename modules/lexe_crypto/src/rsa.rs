// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `crypto.publicEncrypt` (RSA-OAEP only).

use lexe_buffer::Buffer;
use lexe_encoding::bytes_from_b64;
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{Ctx, Exception, IntoJs, Object, Result, Value};

use crate::hash::HashAlgorithm;
use crate::provider::{CryptoError, CryptoProvider};
use crate::CRYPTO_PROVIDER;

/// Node `crypto.constants` RSA & EC point conversion constants.
pub const RSA_PKCS1_PADDING: i32 = 1;
pub const RSA_NO_PADDING: i32 = 3;
pub const RSA_PKCS1_OAEP_PADDING: i32 = 4;
pub const RSA_PKCS1_PSS_PADDING: i32 = 6;
pub const RSA_PSS_SALTLEN_DIGEST: i32 = -1;
pub const RSA_PSS_SALTLEN_MAX_SIGN: i32 = -2;
pub const RSA_PSS_SALTLEN_AUTO: i32 = -2;
pub const POINT_CONVERSION_COMPRESSED: i32 = 2;
pub const POINT_CONVERSION_UNCOMPRESSED: i32 = 4;
pub const POINT_CONVERSION_HYBRID: i32 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RsaKeyError {
    InvalidKey,
    UnsupportedAlgorithm,
}

impl std::fmt::Display for RsaKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RsaKeyError::InvalidKey => write!(f, "Invalid RSA public key"),
            RsaKeyError::UnsupportedAlgorithm => {
                write!(f, "RSA-OAEP is not supported by the active crypto provider")
            },
        }
    }
}

struct ParsedPublicEncryptOptions {
    key: Vec<u8>,
    padding: i32,
    hash: HashAlgorithm,
    label: Option<Vec<u8>>,
}

fn is_direct_key(value: &Value<'_>) -> bool {
    if value.as_string().is_some() {
        return true;
    }
    let Some(obj) = value.as_object() else {
        return false;
    };
    // TypedArray / DataView
    if ObjectBytes::from_array_buffer_view(obj)
        .ok()
        .flatten()
        .is_some()
    {
        return true;
    }
    // Bare ArrayBuffer
    rquickjs::ArrayBuffer::from_object(obj.clone()).is_some()
}

fn binary_like_to_bytes<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    arg_name: &str,
) -> Result<Vec<u8>> {
    if let Some(s) = value.as_string() {
        return Ok(s.to_string()?.into_bytes());
    }
    if let Some(obj) = value.as_object() {
        if let Some(bytes) = ObjectBytes::from_array_buffer(obj)? {
            return Ok(bytes.as_bytes(ctx)?.to_vec());
        }
    }
    Err(Exception::throw_type(
        ctx,
        &format!(
            "The \"{}\" argument must be of type string or an instance of Buffer, TypedArray, or DataView.",
            arg_name
        ),
    ))
}

fn key_bytes_from_value<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<Vec<u8>> {
    binary_like_to_bytes(ctx, value, "key")
}

fn parse_public_encrypt_options<'js>(
    ctx: &Ctx<'js>,
    key_or_options: Value<'js>,
) -> Result<ParsedPublicEncryptOptions> {
    // Direct key: string / ArrayBufferView / ArrayBuffer.
    if is_direct_key(&key_or_options) {
        let key = key_bytes_from_value(ctx, &key_or_options)?;
        return Ok(ParsedPublicEncryptOptions {
            key,
            padding: RSA_PKCS1_OAEP_PADDING,
            hash: HashAlgorithm::Sha1,
            label: None,
        });
    }

    let obj = key_or_options.as_object().ok_or_else(|| {
        Exception::throw_type(
            ctx,
            "The first argument must be of type string or an instance of Buffer, TypedArray, DataView, or KeyObject, or an object with a \"key\" property.",
        )
    })?;

    let key_val: Value = obj
        .get("key")
        .map_err(|_| Exception::throw_type(ctx, "The \"options.key\" property is required."))?;
    if key_val.is_undefined() || key_val.is_null() {
        return Err(Exception::throw_type(
            ctx,
            "The \"options.key\" property is required.",
        ));
    }
    let key = key_bytes_from_value(ctx, &key_val)?;

    let padding = if let Ok(p) = obj.get::<_, Value>("padding") {
        if p.is_undefined() || p.is_null() {
            RSA_PKCS1_OAEP_PADDING
        } else if let Some(n) = p.as_number() {
            n as i32
        } else {
            return Err(Exception::throw_type(
                ctx,
                "The \"options.padding\" property must be of type number.",
            ));
        }
    } else {
        RSA_PKCS1_OAEP_PADDING
    };

    if padding != RSA_PKCS1_OAEP_PADDING {
        return Err(Exception::throw_message(
            ctx,
            &format!(
                "Unsupported padding mode: {}. Only RSA_PKCS1_OAEP_PADDING ({}) is supported.",
                padding, RSA_PKCS1_OAEP_PADDING
            ),
        ));
    }

    let hash = if let Ok(h) = obj.get::<_, Value>("oaepHash") {
        if h.is_undefined() || h.is_null() {
            HashAlgorithm::Sha1
        } else if let Some(s) = h.as_string() {
            let name = s.to_string()?;
            let algo = HashAlgorithm::try_from(name.as_str()).map_err(|err| {
                Exception::throw_message(ctx, &format!("Unsupported OAEP hash: {}", err))
            })?;
            if matches!(algo, HashAlgorithm::Md5) {
                return Err(Exception::throw_message(
                    ctx,
                    "MD5 is not supported for RSA-OAEP",
                ));
            }
            algo
        } else {
            return Err(Exception::throw_type(
                ctx,
                "The \"options.oaepHash\" property must be of type string.",
            ));
        }
    } else {
        HashAlgorithm::Sha1
    };

    let label = if let Ok(l) = obj.get::<_, Value>("oaepLabel") {
        if l.is_undefined() || l.is_null() {
            None
        } else {
            Some(binary_like_to_bytes(ctx, &l, "options.oaepLabel")?)
        }
    } else {
        None
    };

    Ok(ParsedPublicEncryptOptions {
        key,
        padding,
        hash,
        label,
    })
}

fn strip_ascii_whitespace(input: &[u8]) -> Vec<u8> {
    input
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect()
}

fn decode_pem_body(body: &[u8]) -> std::result::Result<Vec<u8>, RsaKeyError> {
    let cleaned = strip_ascii_whitespace(body);
    if cleaned.is_empty() {
        return Err(RsaKeyError::InvalidKey);
    }
    bytes_from_b64(cleaned).map_err(|_| RsaKeyError::InvalidKey)
}

fn extract_pem_der(pem: &str) -> std::result::Result<(bool, Vec<u8>), RsaKeyError> {
    // Returns (is_spki, der)
    const SPKI_BEGIN: &str = "-----BEGIN PUBLIC KEY-----";
    const SPKI_END: &str = "-----END PUBLIC KEY-----";
    const PKCS1_BEGIN: &str = "-----BEGIN RSA PUBLIC KEY-----";
    const PKCS1_END: &str = "-----END RSA PUBLIC KEY-----";

    if let Some(start) = pem.find(SPKI_BEGIN) {
        let after = start + SPKI_BEGIN.len();
        let end = pem[after..]
            .find(SPKI_END)
            .map(|i| after + i)
            .ok_or(RsaKeyError::InvalidKey)?;
        let body = &pem.as_bytes()[after..end];
        return Ok((true, decode_pem_body(body)?));
    }

    if let Some(start) = pem.find(PKCS1_BEGIN) {
        let after = start + PKCS1_BEGIN.len();
        let end = pem[after..]
            .find(PKCS1_END)
            .map(|i| after + i)
            .ok_or(RsaKeyError::InvalidKey)?;
        let body = &pem.as_bytes()[after..end];
        return Ok((false, decode_pem_body(body)?));
    }

    Err(RsaKeyError::InvalidKey)
}

/// Normalize a public key to PKCS#1 DER for `rsa_oaep_encrypt`.
///
/// Pure Rust error so unit tests can call this without a JS context.
pub fn normalize_rsa_public_key(key_bytes: &[u8]) -> std::result::Result<Vec<u8>, RsaKeyError> {
    // Try PEM first if it looks like text.
    let looks_like_pem = key_bytes.windows(5).any(|w| w == b"-----");
    if looks_like_pem {
        // Trailing MySQL NUL and other trailing bytes after the footer are ignored
        // because we only decode between header and footer.
        let pem = String::from_utf8_lossy(key_bytes);
        let (is_spki, der) = extract_pem_der(&pem)?;
        return import_public_der(&der, Some(is_spki));
    }

    // Bare DER: try SPKI then PKCS#1.
    import_public_der(key_bytes, None)
}

fn import_public_der(
    der: &[u8],
    prefer_spki: Option<bool>,
) -> std::result::Result<Vec<u8>, RsaKeyError> {
    match prefer_spki {
        Some(true) => CRYPTO_PROVIDER
            .import_rsa_public_key_spki(der)
            .map(|r| r.key_data)
            .map_err(map_provider_import_error),
        Some(false) => CRYPTO_PROVIDER
            .import_rsa_public_key_pkcs1(der)
            .map(|r| r.key_data)
            .map_err(map_provider_import_error),
        None => {
            if let Ok(r) = CRYPTO_PROVIDER.import_rsa_public_key_spki(der) {
                return Ok(r.key_data);
            }
            CRYPTO_PROVIDER
                .import_rsa_public_key_pkcs1(der)
                .map(|r| r.key_data)
                .map_err(map_provider_import_error)
        },
    }
}

fn map_provider_import_error(err: CryptoError) -> RsaKeyError {
    match err {
        CryptoError::UnsupportedAlgorithm => RsaKeyError::UnsupportedAlgorithm,
        _ => RsaKeyError::InvalidKey,
    }
}

fn map_rsa_key_error(ctx: &Ctx<'_>, err: RsaKeyError) -> rquickjs::Error {
    Exception::throw_message(ctx, &err.to_string())
}

/// Node-compatible `crypto.publicEncrypt(key, buffer)`.
pub fn public_encrypt<'js>(
    ctx: Ctx<'js>,
    key_or_options: Value<'js>,
    data: Value<'js>,
) -> Result<Value<'js>> {
    let options = parse_public_encrypt_options(&ctx, key_or_options)?;
    let _padding = options.padding; // validated to OAEP only
    let normalized =
        normalize_rsa_public_key(&options.key).map_err(|err| map_rsa_key_error(&ctx, err))?;
    let plaintext = binary_like_to_bytes(&ctx, &data, "buffer")?;

    let ciphertext = CRYPTO_PROVIDER
        .rsa_oaep_encrypt(
            &normalized,
            &plaintext,
            options.hash,
            options.label.as_deref(),
        )
        .map_err(|err| match err {
            CryptoError::UnsupportedAlgorithm => Exception::throw_message(
                &ctx,
                "RSA-OAEP is not supported by the active crypto provider",
            ),
            other => {
                Exception::throw_message(&ctx, &format!("RSA public encryption failed: {}", other))
            },
        })?;

    Buffer(ciphertext).into_js(&ctx)
}

/// Build the `crypto.constants` object with the supported padding constant.
pub fn create_constants_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let constants = Object::new(ctx.clone())?;
    constants.set("RSA_PKCS1_PADDING", RSA_PKCS1_PADDING)?;
    constants.set("RSA_NO_PADDING", RSA_NO_PADDING)?;
    constants.set("RSA_PKCS1_OAEP_PADDING", RSA_PKCS1_OAEP_PADDING)?;
    constants.set("RSA_PKCS1_PSS_PADDING", RSA_PKCS1_PSS_PADDING)?;
    constants.set("RSA_PSS_SALTLEN_DIGEST", RSA_PSS_SALTLEN_DIGEST)?;
    constants.set("RSA_PSS_SALTLEN_MAX_SIGN", RSA_PSS_SALTLEN_MAX_SIGN)?;
    constants.set("RSA_PSS_SALTLEN_AUTO", RSA_PSS_SALTLEN_AUTO)?;
    constants.set("POINT_CONVERSION_COMPRESSED", POINT_CONVERSION_COMPRESSED)?;
    constants.set(
        "POINT_CONVERSION_UNCOMPRESSED",
        POINT_CONVERSION_UNCOMPRESSED,
    )?;
    constants.set("POINT_CONVERSION_HYBRID", POINT_CONVERSION_HYBRID)?;
    Ok(constants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::CryptoProvider;
    use crate::CRYPTO_PROVIDER;
    use lexe_encoding::bytes_to_b64_string;

    fn to_pem(label: &str, der: &[u8]) -> String {
        let b64 = bytes_to_b64_string(der);
        let mut lines = vec![format!("-----BEGIN {}-----", label)];
        for chunk in b64.as_bytes().chunks(64) {
            lines.push(String::from_utf8_lossy(chunk).into_owned());
        }
        lines.push(format!("-----END {}-----", label));
        lines.join("\n")
    }

    fn generate_pair() -> (Vec<u8>, Vec<u8>) {
        CRYPTO_PROVIDER
            .generate_rsa_key(2048, &[1, 0, 1])
            .expect("generate rsa")
    }

    #[test]
    fn oaep_spki_pem_sha1_roundtrip() {
        let (private_key, public_pkcs1) = generate_pair();
        let spki = CRYPTO_PROVIDER
            .export_rsa_public_key_spki(&public_pkcs1)
            .expect("export spki");
        let pem = to_pem("PUBLIC KEY", &spki);
        // Call the production normalizer, not a test copy.
        let normalized = normalize_rsa_public_key(pem.as_bytes()).unwrap();
        let plaintext = b"mysql-auth-secret";
        let ct = CRYPTO_PROVIDER
            .rsa_oaep_encrypt(&normalized, plaintext, HashAlgorithm::Sha1, None)
            .expect("encrypt");
        assert_eq!(ct.len(), 256);
        let pt = CRYPTO_PROVIDER
            .rsa_oaep_decrypt(&private_key, &ct, HashAlgorithm::Sha1, None)
            .expect("decrypt");
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn oaep_pkcs1_pem_and_der() {
        let (private_key, public_pkcs1) = generate_pair();
        let pem = to_pem("RSA PUBLIC KEY", &public_pkcs1);
        let normalized_pem = normalize_rsa_public_key(pem.as_bytes()).unwrap();
        let normalized_der = normalize_rsa_public_key(&public_pkcs1).unwrap();
        let plaintext = b"hello-pkcs1";
        for key in [&normalized_pem, &normalized_der] {
            let ct = CRYPTO_PROVIDER
                .rsa_oaep_encrypt(key, plaintext, HashAlgorithm::Sha1, None)
                .unwrap();
            let pt = CRYPTO_PROVIDER
                .rsa_oaep_decrypt(&private_key, &ct, HashAlgorithm::Sha1, None)
                .unwrap();
            assert_eq!(pt, plaintext);
        }
    }

    #[test]
    fn oaep_spki_der_and_trailing_nul() {
        let (private_key, public_pkcs1) = generate_pair();
        let spki = CRYPTO_PROVIDER
            .export_rsa_public_key_spki(&public_pkcs1)
            .unwrap();
        let mut pem = to_pem("PUBLIC KEY", &spki).into_bytes();
        pem.push(0); // MySQL trailing NUL
        let normalized = normalize_rsa_public_key(&pem).unwrap();
        let ct = CRYPTO_PROVIDER
            .rsa_oaep_encrypt(&normalized, b"nul", HashAlgorithm::Sha1, None)
            .unwrap();
        let pt = CRYPTO_PROVIDER
            .rsa_oaep_decrypt(&private_key, &ct, HashAlgorithm::Sha1, None)
            .unwrap();
        assert_eq!(pt, b"nul");

        let normalized_der = normalize_rsa_public_key(&spki).unwrap();
        let ct2 = CRYPTO_PROVIDER
            .rsa_oaep_encrypt(&normalized_der, b"der", HashAlgorithm::Sha1, None)
            .unwrap();
        assert_eq!(
            CRYPTO_PROVIDER
                .rsa_oaep_decrypt(&private_key, &ct2, HashAlgorithm::Sha1, None)
                .unwrap(),
            b"der"
        );
    }

    #[test]
    fn oaep_with_label() {
        let (private_key, public_pkcs1) = generate_pair();
        let label = b"mysql-label";
        let ct = CRYPTO_PROVIDER
            .rsa_oaep_encrypt(&public_pkcs1, b"data", HashAlgorithm::Sha1, Some(label))
            .unwrap();
        let pt = CRYPTO_PROVIDER
            .rsa_oaep_decrypt(&private_key, &ct, HashAlgorithm::Sha1, Some(label))
            .unwrap();
        assert_eq!(pt, b"data");
    }

    #[test]
    fn invalid_keys() {
        assert_eq!(
            normalize_rsa_public_key(b"not-a-key"),
            Err(RsaKeyError::InvalidKey)
        );
        assert_eq!(
            normalize_rsa_public_key(b"-----BEGIN PUBLIC KEY-----\nbad\n-----END PUBLIC KEY-----"),
            Err(RsaKeyError::InvalidKey)
        );
        assert_eq!(
            normalize_rsa_public_key(
                b"-----BEGIN PUBLIC KEY-----\nAAAA\n-----END RSA PUBLIC KEY-----"
            ),
            Err(RsaKeyError::InvalidKey)
        );
    }

    #[test]
    fn max_plaintext_boundary_sha1_2048() {
        // max = k - 2*hLen - 2 = 256 - 2*20 - 2 = 214
        let (_private_key, public_pkcs1) = generate_pair();
        let max = vec![0x41u8; 214];
        assert!(CRYPTO_PROVIDER
            .rsa_oaep_encrypt(&public_pkcs1, &max, HashAlgorithm::Sha1, None)
            .is_ok());
        let too_long = vec![0x41u8; 215];
        assert!(CRYPTO_PROVIDER
            .rsa_oaep_encrypt(&public_pkcs1, &too_long, HashAlgorithm::Sha1, None)
            .is_err());
    }
}
