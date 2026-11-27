// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::result::ResultExt;
use rquickjs::{Array, ArrayBuffer, Class, Ctx, Exception, Result, Value};

use crate::{provider::CryptoProvider, subtle::CryptoKey, CRYPTO_PROVIDER};

#[cfg(feature = "_rustcrypto")]
use super::key_algorithm::Argon2Variant;
use super::{
    algorithm_mismatch_error, algorithm_not_supported_error,
    crypto_key::KeyKind,
    derive_algorithm::DeriveAlgorithm,
    key_algorithm::{
        EcAlgorithm, KeyAlgorithm, KeyAlgorithmMode, KeyAlgorithmWithUsages, KeyDerivation,
    },
};

fn parse_derive_bits_length(ctx: &Ctx<'_>, length: Value<'_>) -> Result<Option<u32>> {
    if length.is_null() || length.is_undefined() {
        return Ok(None);
    }
    let Some(n) = length.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            "length must be null or an unsigned integer",
        ));
    };
    if !n.is_finite() || n < 0.0 || n > u32::MAX as f64 {
        return Err(Exception::throw_message(ctx, "Invalid deriveBits length"));
    }
    Ok(Some(n as u32))
}

fn truncate_shared_secret(
    ctx: &Ctx<'_>,
    mut secret: Vec<u8>,
    length: Option<u32>,
) -> Result<Vec<u8>> {
    let Some(bits) = length else {
        return Ok(secret);
    };
    let max_bits = (secret.len() as u64) * 8;
    if (bits as u64) > max_bits {
        return Err(Exception::throw_message(
            ctx,
            "Requested length exceeds derived secret length",
        ));
    }
    if bits == 0 {
        return Ok(Vec::new());
    }
    let byte_len = (bits as usize).div_ceil(8);
    secret.truncate(byte_len);
    Ok(secret)
}

pub async fn subtle_derive_bits<'js>(
    ctx: Ctx<'js>,
    algorithm: DeriveAlgorithm,
    base_key: Class<'js, CryptoKey>,
    length: Value<'js>,
) -> Result<ArrayBuffer<'js>> {
    let base_key = base_key.borrow();
    base_key.check_validity("deriveBits").or_throw(&ctx)?;
    let bits_len = parse_derive_bits_length(&ctx, length)?;

    let bytes = derive_bits(&ctx, &algorithm, &base_key, bits_len)?;
    ArrayBuffer::new(ctx, bytes)
}

fn derive_bits(
    ctx: &Ctx<'_>,
    algorithm: &DeriveAlgorithm,
    base_key: &CryptoKey,
    length: Option<u32>,
) -> Result<Vec<u8>> {
    match algorithm {
        DeriveAlgorithm::Ecdh { curve, public_key } => {
            if let KeyAlgorithm::Ec {
                curve: base_key_curve,
                algorithm,
            } = &base_key.algorithm
            {
                if curve == base_key_curve
                    && base_key.kind == KeyKind::Private
                    && matches!(algorithm, EcAlgorithm::Ecdh)
                {
                    let handle = &base_key.handle;
                    let secret = CRYPTO_PROVIDER
                        .ecdh_derive_bits(*curve, handle, public_key)
                        .or_throw(ctx)?;
                    return truncate_shared_secret(ctx, secret, length);
                }
                return Err(Exception::throw_message(
                    ctx,
                    "ECDH curve must be same as baseKey",
                ));
            }
            algorithm_mismatch_error(ctx, "ECDH")
        },
        DeriveAlgorithm::X25519 { public_key } => {
            if !matches!(base_key.algorithm, KeyAlgorithm::X25519)
                || base_key.kind != KeyKind::Private
            {
                return algorithm_mismatch_error(ctx, "X25519");
            }

            let secret = CRYPTO_PROVIDER
                .x25519_derive_bits(&base_key.handle, public_key)
                .or_throw(ctx)?;
            truncate_shared_secret(ctx, secret, length)
        },
        DeriveAlgorithm::X448 { public_key } => {
            if !matches!(base_key.algorithm, KeyAlgorithm::X448)
                || base_key.kind != KeyKind::Private
            {
                return algorithm_mismatch_error(ctx, "X448");
            }
            #[cfg(feature = "_rustcrypto")]
            {
                use der::Decode as _;
                let pk_info =
                    pkcs8::PrivateKeyInfoRef::try_from(base_key.handle.as_ref()).or_throw(ctx)?;
                let raw = pk_info.private_key.as_bytes();
                let priv_slice = if raw.len() == 56 {
                    raw
                } else {
                    <&der::asn1::OctetStringRef>::from_der(raw)
                        .or_throw(ctx)?
                        .as_bytes()
                };
                let priv_arr: [u8; 56] = priv_slice
                    .try_into()
                    .map_err(|_| Exception::throw_type(ctx, "Invalid X448 private key"))?;
                let pub_key = x448::PublicKey::from_bytes(public_key.as_ref())
                    .ok_or_else(|| Exception::throw_type(ctx, "Invalid X448 public key"))?;
                let secret = x448::StaticSecret::from(priv_arr);
                let shared = secret.diffie_hellman(&pub_key);
                truncate_shared_secret(ctx, shared.as_bytes().to_vec(), length)
            }
            #[cfg(not(feature = "_rustcrypto"))]
            {
                let _ = public_key;
                algorithm_not_supported_error(ctx)
            }
        },
        DeriveAlgorithm::Derive(KeyDerivation::Hkdf { hash, salt, info }) => {
            if !matches!(base_key.algorithm, KeyAlgorithm::HkdfImport) {
                return algorithm_mismatch_error(ctx, "HKDF");
            }
            let Some(bits) = length else {
                return Err(Exception::throw_message(
                    ctx,
                    "HKDF requires explicit bit length",
                ));
            };
            if bits % 8 != 0 {
                return Err(Exception::throw_message(
                    ctx,
                    "HKDF length must be a multiple of 8",
                ));
            }
            let out_length = (bits / 8) as usize;
            if out_length == 0 {
                return Ok(Vec::new());
            }
            CRYPTO_PROVIDER
                .hkdf_derive_key(&base_key.handle, salt, info, out_length, *hash)
                .or_throw(ctx)
        },
        DeriveAlgorithm::Derive(KeyDerivation::Pbkdf2 {
            hash,
            salt,
            iterations,
        }) => {
            if !matches!(base_key.algorithm, KeyAlgorithm::Pbkdf2Import) {
                return algorithm_mismatch_error(ctx, "PBKDF2");
            }
            let Some(bits) = length else {
                return Err(Exception::throw_message(
                    ctx,
                    "PBKDF2 requires explicit bit length",
                ));
            };
            if bits % 8 != 0 {
                return Err(Exception::throw_message(
                    ctx,
                    "PBKDF2 length must be a multiple of 8",
                ));
            }
            let out_length = (bits / 8) as usize;
            if out_length == 0 {
                return Ok(Vec::new());
            }
            CRYPTO_PROVIDER
                .pbkdf2_derive_key(&base_key.handle, salt, *iterations, out_length, *hash)
                .or_throw(ctx)
        },
        DeriveAlgorithm::Derive(KeyDerivation::Argon2 {
            variant,
            nonce,
            parallelization,
            memory,
            passes,
            version,
            secret_value,
            associated_data,
        }) => {
            if !matches!(&base_key.algorithm, KeyAlgorithm::Argon2Import(v) if v == variant) {
                return algorithm_mismatch_error(ctx, &base_key.name);
            }
            let Some(bits) = length else {
                return Err(Exception::throw_message(
                    ctx,
                    "Argon2 requires explicit bit length",
                ));
            };
            if bits < 32 || bits % 8 != 0 {
                return Err(Exception::throw_message(
                    ctx,
                    "Argon2 length must be a multiple of 8 and at least 32 bits",
                ));
            }
            #[cfg(feature = "_rustcrypto")]
            {
                let out_len = (bits / 8) as usize;
                let alg = match variant {
                    Argon2Variant::Argon2d => argon2::Algorithm::Argon2d,
                    Argon2Variant::Argon2i => argon2::Algorithm::Argon2i,
                    Argon2Variant::Argon2id => argon2::Algorithm::Argon2id,
                };
                let ver = match version {
                    0x10 => argon2::Version::V0x10,
                    0x13 => argon2::Version::V0x13,
                    _ => return Err(Exception::throw_message(ctx, "Unsupported Argon2 version")),
                };
                let mut builder = argon2::ParamsBuilder::new();
                builder.m_cost(*memory);
                builder.t_cost(*passes);
                builder.p_cost(*parallelization);
                builder.output_len(out_len);
                if let Some(ad) = associated_data.as_deref() {
                    builder.data(argon2::AssociatedData::new(ad).or_throw(ctx)?);
                }
                let params = builder.build().or_throw(ctx)?;
                let hasher = match secret_value.as_deref() {
                    Some(secret) => {
                        argon2::Argon2::new_with_secret(secret, alg, ver, params).or_throw(ctx)?
                    },
                    None => argon2::Argon2::new(alg, ver, params),
                };
                let mut out = vec![0u8; out_len];
                hasher
                    .hash_password_into(&base_key.handle, nonce, &mut out)
                    .or_throw(ctx)?;
                Ok(out)
            }
            #[cfg(not(feature = "_rustcrypto"))]
            {
                let _ = (
                    nonce,
                    parallelization,
                    memory,
                    passes,
                    version,
                    secret_value,
                    associated_data,
                );
                algorithm_not_supported_error(ctx)
            }
        },
    }
}

pub async fn subtle_derive_key<'js>(
    ctx: Ctx<'js>,
    algorithm: DeriveAlgorithm,
    base_key: Class<'js, CryptoKey>,
    derived_key_algorithm: Value<'js>,
    extractable: bool,
    key_usages: Array<'js>,
) -> Result<Class<'js, CryptoKey>> {
    let base_key = &base_key.borrow();
    base_key.check_validity("deriveKey").or_throw(&ctx)?;

    let KeyAlgorithmWithUsages {
        algorithm: derived_key_algorithm,
        name,
        public_usages,
        ..
    } = KeyAlgorithm::from_js(
        &ctx,
        KeyAlgorithmMode::Derive,
        derived_key_algorithm,
        key_usages,
    )?;

    let length = match &derived_key_algorithm {
        KeyAlgorithm::Aes { length } => *length as u32,
        KeyAlgorithm::Hmac { length, .. } => *length as u32,
        KeyAlgorithm::ChaCha20Poly1305 => 256,
        KeyAlgorithm::Derive { .. } => 0,
        _ => {
            return algorithm_not_supported_error(&ctx);
        },
    };

    let bytes = derive_bits(&ctx, &algorithm, base_key, Some(length))?;

    let key = CryptoKey::new(
        KeyKind::Secret,
        name,
        extractable,
        derived_key_algorithm,
        public_usages,
        bytes,
    );

    Class::instance(ctx, key)
}
