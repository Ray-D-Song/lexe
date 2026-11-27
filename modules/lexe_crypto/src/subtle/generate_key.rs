// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{object::Property, Array, Class, Ctx, Exception, Object, Result, Value};

use crate::{provider::CryptoProvider, CRYPTO_PROVIDER};

use crate::{hash::HashAlgorithm, subtle::CryptoKey};

use super::{
    algorithm_not_supported_error,
    crypto_key::KeyKind,
    key_algorithm::{KeyAlgorithm, KeyAlgorithmMode, KeyAlgorithmWithUsages},
};

pub async fn subtle_generate_key<'js>(
    ctx: Ctx<'js>,
    algorithm: Value<'js>,
    extractable: bool,
    key_usages: Array<'js>,
) -> Result<Value<'js>> {
    let KeyAlgorithmWithUsages {
        name,
        algorithm: key_algorithm,
        private_usages,
        public_usages,
    } = KeyAlgorithm::from_js(&ctx, KeyAlgorithmMode::Generate, algorithm, key_usages)?;

    let (private_key, public_or_secret_key) = generate_key(&ctx, &key_algorithm)?;

    if matches!(
        key_algorithm,
        KeyAlgorithm::Aes { .. } | KeyAlgorithm::Hmac { .. } | KeyAlgorithm::ChaCha20Poly1305
    ) {
        return Ok(Class::instance(
            ctx,
            CryptoKey::new(
                KeyKind::Secret,
                name,
                extractable,
                key_algorithm,
                public_usages,
                public_or_secret_key,
            ),
        )?
        .into_value());
    }

    let private_key = Class::instance(
        ctx.clone(),
        CryptoKey::new(
            KeyKind::Private,
            name.clone(),
            extractable,
            key_algorithm.clone(),
            private_usages,
            private_key,
        ),
    )?;

    let public_key = Class::instance(
        ctx.clone(),
        CryptoKey::new(
            KeyKind::Public,
            name,
            extractable,
            key_algorithm,
            public_usages,
            public_or_secret_key,
        ),
    )?;

    let key_pair = Object::new(ctx.clone())?;
    key_pair.prop("privateKey", Property::from(private_key).enumerable())?;
    key_pair.prop("publicKey", Property::from(public_key).enumerable())?;
    Ok(key_pair.into_value())
}

fn generate_key(ctx: &Ctx<'_>, algorithm: &KeyAlgorithm) -> Result<(Vec<u8>, Vec<u8>)> {
    match algorithm {
        KeyAlgorithm::Aes { length } => {
            // Default to AES-256
            let key = CRYPTO_PROVIDER.generate_aes_key(*length).map_err(|e| {
                Exception::throw_message(ctx, &format!("AES key generation failed: {}", e))
            })?;
            Ok((vec![], key))
        },
        KeyAlgorithm::Hmac { hash, length } => {
            let key = CRYPTO_PROVIDER
                .generate_hmac_key(*hash, *length)
                .map_err(|e| {
                    Exception::throw_message(ctx, &format!("HMAC key generation failed: {}", e))
                })?;
            Ok((vec![], key))
        },
        KeyAlgorithm::Ec { curve, .. } => CRYPTO_PROVIDER.generate_ec_key(*curve).map_err(|e| {
            Exception::throw_message(ctx, &format!("EC key generation failed: {}", e))
        }),
        KeyAlgorithm::Ed25519 => CRYPTO_PROVIDER.generate_ed25519_key().map_err(|e| {
            Exception::throw_message(ctx, &format!("Ed25519 key generation failed: {}", e))
        }),
        KeyAlgorithm::Ed448 => {
            #[cfg(feature = "_rustcrypto")]
            {
                use der::Encode as _;
                use elliptic_curve::Generate as _;
                use pkcs8::PrivateKeyInfoRef;
                use spki::AlgorithmIdentifierRef;
                let mut rng = rand::rng();
                let sk = ed448_goldilocks::SigningKey::generate_from_rng(&mut rng);
                let vk = sk.verifying_key();
                let seed = sk.to_bytes();
                let inner = der::asn1::OctetStringRef::new(seed.as_slice())
                    .and_then(|o| o.to_der())
                    .map_err(|e| {
                        Exception::throw_message(ctx, &format!("Ed448 keygen failed: {e}"))
                    })?;
                let pk_info = PrivateKeyInfoRef::new(
                    AlgorithmIdentifierRef {
                        oid: const_oid::db::rfc8410::ID_ED_448,
                        parameters: None,
                    },
                    der::asn1::OctetStringRef::new(&inner).map_err(|e| {
                        Exception::throw_message(ctx, &format!("Ed448 keygen failed: {e}"))
                    })?,
                );
                let priv_der = pk_info.to_der().map_err(|e| {
                    Exception::throw_message(ctx, &format!("Ed448 keygen failed: {e}"))
                })?;
                Ok((priv_der, vk.to_bytes().to_vec()))
            }
            #[cfg(not(feature = "_rustcrypto"))]
            algorithm_not_supported_error(ctx)
        },
        KeyAlgorithm::X25519 => CRYPTO_PROVIDER.generate_x25519_key().map_err(|e| {
            Exception::throw_message(ctx, &format!("X25519 key generation failed: {}", e))
        }),
        KeyAlgorithm::X448 => {
            #[cfg(feature = "_rustcrypto")]
            {
                use der::Encode as _;
                use pkcs8::PrivateKeyInfoRef;
                use rand::RngExt as _;
                use spki::AlgorithmIdentifierRef;
                let mut raw = [0u8; 56];
                rand::rng().fill(raw.as_mut_slice());
                let sk = x448::StaticSecret::from(raw);
                let pk = x448::PublicKey::from(&sk);
                let inner = der::asn1::OctetStringRef::new(sk.as_bytes())
                    .and_then(|o| o.to_der())
                    .map_err(|e| {
                        Exception::throw_message(ctx, &format!("X448 keygen failed: {e}"))
                    })?;
                let pk_info = PrivateKeyInfoRef::new(
                    AlgorithmIdentifierRef {
                        oid: const_oid::db::rfc8410::ID_X_448,
                        parameters: None,
                    },
                    der::asn1::OctetStringRef::new(&inner).map_err(|e| {
                        Exception::throw_message(ctx, &format!("X448 keygen failed: {e}"))
                    })?,
                );
                let priv_der = pk_info.to_der().map_err(|e| {
                    Exception::throw_message(ctx, &format!("X448 keygen failed: {e}"))
                })?;
                Ok((priv_der, pk.as_bytes().to_vec()))
            }
            #[cfg(not(feature = "_rustcrypto"))]
            algorithm_not_supported_error(ctx)
        },
        KeyAlgorithm::ChaCha20Poly1305 => Ok((vec![], crate::random_byte_array(32))),
        KeyAlgorithm::Rsa {
            modulus_length,
            public_exponent,
            ..
        } => CRYPTO_PROVIDER
            .generate_rsa_key(*modulus_length, public_exponent.as_ref())
            .map_err(|e| {
                Exception::throw_message(ctx, &format!("RSA key generation failed: {}", e))
            }),
        _ => algorithm_not_supported_error(ctx),
    }
}

#[allow(dead_code)]
fn generate_symmetric_key(_ctx: &Ctx<'_>, length: usize) -> Result<Vec<u8>> {
    Ok(crate::random_byte_array(length))
}

#[allow(dead_code)]
pub fn get_hash_length(ctx: &Ctx, hash: &HashAlgorithm, length: u16) -> Result<usize> {
    if length == 0 {
        return Ok(hash.block_len());
    }

    if !length.is_multiple_of(8) || (length / 8) as usize > 128 {
        return Err(Exception::throw_message(ctx, "Invalid HMAC key length"));
    }

    Ok((length / 8) as usize)
}
