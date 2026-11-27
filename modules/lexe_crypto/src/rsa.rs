// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible RSA public/private encryption & decryption and `crypto.constants`.

#[cfg(feature = "_rustcrypto")]
use crate::key_object::{
    parse_private_key_value, parse_public_key_value, throw_code_error, KeyMaterial,
};
use lexe_buffer::Buffer;
#[cfg(any(not(feature = "_rustcrypto"), test))]
use lexe_encoding::bytes_from_b64;
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{Ctx, Exception, IntoJs, Object, Result, Value};
#[cfg(feature = "_rustcrypto")]
use rsa::{
    hazmat::{rsa_decrypt_and_check, rsa_encrypt},
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey},
    sha2::{Sha256, Sha384, Sha512},
    traits::PublicKeyParts,
    BoxedUint, Oaep, Pkcs1v15Encrypt, Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey,
};
#[cfg(feature = "_rustcrypto")]
use sha1::Sha1;

use crate::hash::HashAlgorithm;
#[cfg(any(not(feature = "_rustcrypto"), test))]
use crate::provider::{CryptoError, CryptoProvider};
#[cfg(any(not(feature = "_rustcrypto"), test))]
use crate::CRYPTO_PROVIDER;

/// Node `crypto.constants` RSA, DH, & EC point conversion constants.
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
pub const DH_CHECK_P_NOT_PRIME: i32 = 1;
pub const DH_CHECK_P_NOT_SAFE_PRIME: i32 = 2;
pub const DH_UNABLE_TO_CHECK_GENERATOR: i32 = 4;
pub const DH_NOT_SUITABLE_GENERATOR: i32 = 8;
pub const OPENSSL_VERSION_NUMBER: u32 = 805_306_624;

#[cfg(any(not(feature = "_rustcrypto"), test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RsaKeyError {
    InvalidKey,
    UnsupportedAlgorithm,
}

#[cfg(any(not(feature = "_rustcrypto"), test))]
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

#[cfg(not(feature = "_rustcrypto"))]
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

#[cfg(not(feature = "_rustcrypto"))]
fn key_bytes_from_value<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<Vec<u8>> {
    binary_like_to_bytes(ctx, value, "key")
}

fn parse_rsa_padding_and_oaep<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    default_padding: i32,
) -> Result<(i32, HashAlgorithm, Option<Vec<u8>>)> {
    let padding = if let Ok(p) = obj.get::<_, Value>("padding") {
        if p.is_undefined() || p.is_null() {
            default_padding
        } else if let Some(n) = p.as_number() {
            n as i32
        } else {
            return Err(Exception::throw_type(
                ctx,
                "The \"options.padding\" property must be of type number.",
            ));
        }
    } else {
        default_padding
    };

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

    Ok((padding, hash, label))
}

#[cfg(not(feature = "_rustcrypto"))]
fn parse_public_encrypt_options<'js>(
    ctx: &Ctx<'js>,
    key_or_options: Value<'js>,
) -> Result<ParsedPublicEncryptOptions> {
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
    let (padding, hash, label) = parse_rsa_padding_and_oaep(ctx, obj, RSA_PKCS1_OAEP_PADDING)?;
    if padding != RSA_PKCS1_OAEP_PADDING {
        return Err(Exception::throw_message(
            ctx,
            &format!(
                "Unsupported padding mode: {}. Only RSA_PKCS1_OAEP_PADDING ({}) is supported.",
                padding, RSA_PKCS1_OAEP_PADDING
            ),
        ));
    }

    Ok(ParsedPublicEncryptOptions {
        key,
        padding,
        hash,
        label,
    })
}

#[cfg(any(not(feature = "_rustcrypto"), test))]
fn strip_ascii_whitespace(input: &[u8]) -> Vec<u8> {
    input
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect()
}

#[cfg(any(not(feature = "_rustcrypto"), test))]
fn decode_pem_body(body: &[u8]) -> std::result::Result<Vec<u8>, RsaKeyError> {
    let cleaned = strip_ascii_whitespace(body);
    if cleaned.is_empty() {
        return Err(RsaKeyError::InvalidKey);
    }
    bytes_from_b64(cleaned).map_err(|_| RsaKeyError::InvalidKey)
}

#[cfg(any(not(feature = "_rustcrypto"), test))]
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
#[cfg(any(not(feature = "_rustcrypto"), test))]
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

#[cfg(any(not(feature = "_rustcrypto"), test))]
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

#[cfg(any(not(feature = "_rustcrypto"), test))]
fn map_provider_import_error(err: CryptoError) -> RsaKeyError {
    match err {
        CryptoError::UnsupportedAlgorithm => RsaKeyError::UnsupportedAlgorithm,
        _ => RsaKeyError::InvalidKey,
    }
}

#[cfg(not(feature = "_rustcrypto"))]
fn map_rsa_key_error(ctx: &Ctx<'_>, err: RsaKeyError) -> rquickjs::Error {
    Exception::throw_message(ctx, &err.to_string())
}

#[cfg(feature = "_rustcrypto")]
fn uint_to_padded_be(u: BoxedUint, padded_len: usize) -> std::result::Result<Vec<u8>, String> {
    let leading_zeros = u.leading_zeros() as usize / 8;
    let be = u.to_be_bytes();
    let trimmed = &be[leading_zeros..];
    if trimmed.len() > padded_len {
        return Err("RSA block exceeds modulus length".to_string());
    }
    let mut out = vec![0u8; padded_len];
    out[padded_len - trimmed.len()..].copy_from_slice(trimmed);
    Ok(out)
}

#[cfg(feature = "_rustcrypto")]
type ParsedRsaKeyAndOpts = (Vec<u8>, i32, HashAlgorithm, Option<Vec<u8>>);

#[cfg(feature = "_rustcrypto")]
fn parse_rsa_key_and_opts<'js>(
    ctx: &Ctx<'js>,
    key_or_options: Value<'js>,
    default_padding: i32,
    is_private: bool,
) -> Result<ParsedRsaKeyAndOpts> {
    let (padding, hash, label) = if !is_direct_key(&key_or_options) {
        if let Some(obj) = key_or_options.as_object() {
            if rquickjs::Class::<crate::key_object::KeyObject>::from_value(&key_or_options).is_ok()
            {
                (default_padding, HashAlgorithm::Sha1, None)
            } else {
                parse_rsa_padding_and_oaep(ctx, obj, default_padding)?
            }
        } else {
            (default_padding, HashAlgorithm::Sha1, None)
        }
    } else {
        (default_padding, HashAlgorithm::Sha1, None)
    };

    let ko = if is_private {
        parse_private_key_value(ctx, key_or_options)?
    } else {
        parse_public_key_value(ctx, key_or_options)?
    };
    let pkcs1_der = match ko.material {
        KeyMaterial::RsaPrivate { pkcs1_der, .. } | KeyMaterial::RsaPublic { pkcs1_der, .. } => {
            pkcs1_der
        },
        _ => return Err(Exception::throw_type(ctx, "Expected an RSA key")),
    };
    Ok((pkcs1_der, padding, hash, label))
}

#[cfg(feature = "_rustcrypto")]
fn rsa_oaep_encrypt_with(
    pub_key: &RsaPublicKey,
    hash: HashAlgorithm,
    label: Option<&[u8]>,
    plaintext: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    let mut rng = rand::rng();
    let label = label.filter(|l| !l.is_empty());
    let res = match hash {
        HashAlgorithm::Sha1 => match label {
            Some(l) => pub_key.encrypt(&mut rng, Oaep::<Sha1>::new_with_label::<_>(l), plaintext),
            None => pub_key.encrypt(&mut rng, Oaep::<Sha1>::new(), plaintext),
        },
        HashAlgorithm::Sha256 => match label {
            Some(l) => pub_key.encrypt(&mut rng, Oaep::<Sha256>::new_with_label::<_>(l), plaintext),
            None => pub_key.encrypt(&mut rng, Oaep::<Sha256>::new(), plaintext),
        },
        HashAlgorithm::Sha384 => match label {
            Some(l) => pub_key.encrypt(&mut rng, Oaep::<Sha384>::new_with_label::<_>(l), plaintext),
            None => pub_key.encrypt(&mut rng, Oaep::<Sha384>::new(), plaintext),
        },
        HashAlgorithm::Sha512 => match label {
            Some(l) => pub_key.encrypt(&mut rng, Oaep::<Sha512>::new_with_label::<_>(l), plaintext),
            None => pub_key.encrypt(&mut rng, Oaep::<Sha512>::new(), plaintext),
        },
        HashAlgorithm::Md5 => return Err("MD5 is not supported for RSA-OAEP".to_string()),
    };
    res.map_err(|e| format!("RSA public encryption failed: {e}"))
}

#[cfg(feature = "_rustcrypto")]
fn rsa_oaep_decrypt_with(
    priv_key: &RsaPrivateKey,
    hash: HashAlgorithm,
    label: Option<&[u8]>,
    ciphertext: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    let label = label.filter(|l| !l.is_empty());
    let res = match hash {
        HashAlgorithm::Sha1 => match label {
            Some(l) => priv_key.decrypt(Oaep::<Sha1>::new_with_label::<_>(l), ciphertext),
            None => priv_key.decrypt(Oaep::<Sha1>::new(), ciphertext),
        },
        HashAlgorithm::Sha256 => match label {
            Some(l) => priv_key.decrypt(Oaep::<Sha256>::new_with_label::<_>(l), ciphertext),
            None => priv_key.decrypt(Oaep::<Sha256>::new(), ciphertext),
        },
        HashAlgorithm::Sha384 => match label {
            Some(l) => priv_key.decrypt(Oaep::<Sha384>::new_with_label::<_>(l), ciphertext),
            None => priv_key.decrypt(Oaep::<Sha384>::new(), ciphertext),
        },
        HashAlgorithm::Sha512 => match label {
            Some(l) => priv_key.decrypt(Oaep::<Sha512>::new_with_label::<_>(l), ciphertext),
            None => priv_key.decrypt(Oaep::<Sha512>::new(), ciphertext),
        },
        HashAlgorithm::Md5 => return Err("MD5 is not supported for RSA-OAEP".to_string()),
    };
    res.map_err(|e| format!("RSA private decryption failed: {e}"))
}

/// Node-compatible `crypto.publicEncrypt(key, buffer)`.
pub fn public_encrypt<'js>(
    ctx: Ctx<'js>,
    key_or_options: Value<'js>,
    data: Value<'js>,
) -> Result<Value<'js>> {
    #[cfg(feature = "_rustcrypto")]
    {
        let (pkcs1_der, padding, hash, label) =
            parse_rsa_key_and_opts(&ctx, key_or_options, RSA_PKCS1_OAEP_PADDING, false)?;
        let plaintext = binary_like_to_bytes(&ctx, &data, "buffer")?;
        let pub_key = RsaPublicKey::from_pkcs1_der(&pkcs1_der)
            .map_err(|e| Exception::throw_message(&ctx, &format!("Invalid RSA public key: {e}")))?;
        let mut rng = rand::rng();
        let ciphertext = match padding {
            RSA_PKCS1_OAEP_PADDING => {
                rsa_oaep_encrypt_with(&pub_key, hash, label.as_deref(), &plaintext)
                    .map_err(|e| Exception::throw_message(&ctx, &e))?
            },
            RSA_PKCS1_PADDING => pub_key
                .encrypt(&mut rng, Pkcs1v15Encrypt, &plaintext)
                .map_err(|e| {
                    Exception::throw_message(&ctx, &format!("RSA public encryption failed: {e}"))
                })?,
            RSA_NO_PADDING => {
                if plaintext.len() != pub_key.size() {
                    return Err(Exception::throw_message(
                        &ctx,
                        "data length mismatch with key size for RSA_NO_PADDING",
                    ));
                }
                let m = BoxedUint::from_be_slice(&plaintext, pub_key.n_bits_precision())
                    .map_err(|_| Exception::throw_message(&ctx, "RSA input conversion failed"))?;
                if m >= pub_key.n().as_ref() {
                    return Err(Exception::throw_message(
                        &ctx,
                        "data too large for key modulus",
                    ));
                }
                let c = rsa_encrypt(&pub_key, &m).map_err(|e| {
                    Exception::throw_message(&ctx, &format!("RSA public encryption failed: {e}"))
                })?;
                uint_to_padded_be(c, pub_key.size())
                    .map_err(|e| Exception::throw_message(&ctx, &e))?
            },
            other => {
                return Err(Exception::throw_type(
                    &ctx,
                    &format!("Unsupported RSA publicEncrypt padding: {other}"),
                ));
            },
        };
        Buffer(ciphertext).into_js(&ctx)
    }

    #[cfg(not(feature = "_rustcrypto"))]
    {
        let options = parse_public_encrypt_options(&ctx, key_or_options)?;
        let _padding = options.padding;
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
                other => Exception::throw_message(
                    &ctx,
                    &format!("RSA public encryption failed: {}", other),
                ),
            })?;

        Buffer(ciphertext).into_js(&ctx)
    }
}

/// Node-compatible `crypto.privateDecrypt(key, buffer)`.
#[cfg(feature = "_rustcrypto")]
pub fn private_decrypt<'js>(
    ctx: Ctx<'js>,
    key_or_options: Value<'js>,
    data: Value<'js>,
) -> Result<Value<'js>> {
    let (pkcs1_der, padding, hash, label) =
        parse_rsa_key_and_opts(&ctx, key_or_options, RSA_PKCS1_OAEP_PADDING, true)?;
    if padding == RSA_PKCS1_PADDING {
        return throw_code_error(
            &ctx,
            "TypeError",
            "ERR_INVALID_ARG_VALUE",
            "RSA_PKCS1_PADDING is no longer supported for private decryption",
        );
    }
    let ciphertext = binary_like_to_bytes(&ctx, &data, "buffer")?;
    let priv_key = RsaPrivateKey::from_pkcs1_der(&pkcs1_der)
        .map_err(|e| Exception::throw_message(&ctx, &format!("Invalid RSA private key: {e}")))?;
    let mut rng = rand::rng();
    let plaintext = match padding {
        RSA_PKCS1_OAEP_PADDING => {
            rsa_oaep_decrypt_with(&priv_key, hash, label.as_deref(), &ciphertext)
                .map_err(|e| Exception::throw_message(&ctx, &e))?
        },
        RSA_NO_PADDING => {
            if ciphertext.len() != priv_key.size() {
                return Err(Exception::throw_message(
                    &ctx,
                    "data length mismatch with key size for RSA_NO_PADDING",
                ));
            }
            let c = BoxedUint::from_be_slice(&ciphertext, priv_key.n_bits_precision())
                .map_err(|_| Exception::throw_message(&ctx, "RSA input conversion failed"))?;
            let m = rsa_decrypt_and_check(&priv_key, Some(&mut rng), &c).map_err(|e| {
                Exception::throw_message(&ctx, &format!("RSA private decryption failed: {e}"))
            })?;
            uint_to_padded_be(m, priv_key.size()).map_err(|e| Exception::throw_message(&ctx, &e))?
        },
        other => {
            return Err(Exception::throw_type(
                &ctx,
                &format!("Unsupported RSA privateDecrypt padding: {other}"),
            ));
        },
    };
    Buffer(plaintext).into_js(&ctx)
}

/// Node-compatible `crypto.privateEncrypt(key, buffer)`.
#[cfg(feature = "_rustcrypto")]
pub fn private_encrypt<'js>(
    ctx: Ctx<'js>,
    key_or_options: Value<'js>,
    data: Value<'js>,
) -> Result<Value<'js>> {
    let (pkcs1_der, padding, _hash, _label) =
        parse_rsa_key_and_opts(&ctx, key_or_options, RSA_PKCS1_PADDING, true)?;
    let plaintext = binary_like_to_bytes(&ctx, &data, "buffer")?;
    let priv_key = RsaPrivateKey::from_pkcs1_der(&pkcs1_der)
        .map_err(|e| Exception::throw_message(&ctx, &format!("Invalid RSA private key: {e}")))?;
    let mut rng = rand::rng();
    let out = match padding {
        RSA_PKCS1_PADDING => priv_key
            .sign_with_rng(&mut rng, Pkcs1v15Sign::new_unprefixed(), &plaintext)
            .map_err(|e| {
                Exception::throw_message(&ctx, &format!("RSA private encryption failed: {e}"))
            })?,
        RSA_NO_PADDING => {
            if plaintext.len() != priv_key.size() {
                return Err(Exception::throw_message(
                    &ctx,
                    "data length mismatch with key size for RSA_NO_PADDING",
                ));
            }
            let m = BoxedUint::from_be_slice(&plaintext, priv_key.n_bits_precision())
                .map_err(|_| Exception::throw_message(&ctx, "RSA input conversion failed"))?;
            let c = rsa_decrypt_and_check(&priv_key, Some(&mut rng), &m).map_err(|e| {
                Exception::throw_message(&ctx, &format!("RSA private encryption failed: {e}"))
            })?;
            uint_to_padded_be(c, priv_key.size()).map_err(|e| Exception::throw_message(&ctx, &e))?
        },
        other => {
            return Err(Exception::throw_type(
                &ctx,
                &format!("Unsupported RSA privateEncrypt padding: {other}"),
            ));
        },
    };
    Buffer(out).into_js(&ctx)
}

/// Node-compatible `crypto.publicDecrypt(key, buffer)`.
#[cfg(feature = "_rustcrypto")]
pub fn public_decrypt<'js>(
    ctx: Ctx<'js>,
    key_or_options: Value<'js>,
    data: Value<'js>,
) -> Result<Value<'js>> {
    let (pkcs1_der, padding, _hash, _label) =
        parse_rsa_key_and_opts(&ctx, key_or_options, RSA_PKCS1_PADDING, false)?;
    let ciphertext = binary_like_to_bytes(&ctx, &data, "buffer")?;
    let pub_key = RsaPublicKey::from_pkcs1_der(&pkcs1_der)
        .map_err(|e| Exception::throw_message(&ctx, &format!("Invalid RSA public key: {e}")))?;
    let k = pub_key.size();
    if ciphertext.len() != k {
        return Err(Exception::throw_message(
            &ctx,
            "data length mismatch with key size",
        ));
    }
    let c = BoxedUint::from_be_slice(&ciphertext, pub_key.n_bits_precision())
        .map_err(|_| Exception::throw_message(&ctx, "RSA input conversion failed"))?;
    if c >= pub_key.n().as_ref() {
        return Err(Exception::throw_message(&ctx, "data greater than mod len"));
    }
    let m = rsa_encrypt(&pub_key, &c).map_err(|e| {
        Exception::throw_message(&ctx, &format!("RSA public decryption failed: {e}"))
    })?;
    let em = uint_to_padded_be(m, k).map_err(|e| Exception::throw_message(&ctx, &e))?;
    let out = match padding {
        RSA_PKCS1_PADDING => {
            if em.len() < 11 || em[0] != 0x00 || em[1] != 0x01 {
                return Err(Exception::throw_message(&ctx, "block type is not 01"));
            }
            let Some(sep_rel) = em[2..].iter().position(|&b| b != 0xff) else {
                return Err(Exception::throw_message(&ctx, "no zero byte in padding"));
            };
            if sep_rel < 8 || em[2 + sep_rel] != 0x00 {
                return Err(Exception::throw_message(
                    &ctx,
                    "bad pad byte count in PKCS#1 padding",
                ));
            }
            em[3 + sep_rel..].to_vec()
        },
        RSA_NO_PADDING => em,
        other => {
            return Err(Exception::throw_type(
                &ctx,
                &format!("Unsupported RSA publicDecrypt padding: {other}"),
            ));
        },
    };
    Buffer(out).into_js(&ctx)
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
    constants.set("DH_CHECK_P_NOT_PRIME", DH_CHECK_P_NOT_PRIME)?;
    constants.set("DH_CHECK_P_NOT_SAFE_PRIME", DH_CHECK_P_NOT_SAFE_PRIME)?;
    constants.set("DH_UNABLE_TO_CHECK_GENERATOR", DH_UNABLE_TO_CHECK_GENERATOR)?;
    constants.set("DH_NOT_SUITABLE_GENERATOR", DH_NOT_SUITABLE_GENERATOR)?;
    constants.set("OPENSSL_VERSION_NUMBER", OPENSSL_VERSION_NUMBER)?;
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
