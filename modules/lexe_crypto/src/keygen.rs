// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `generateKey`, `generateKeySync`, `generateKeyPair`, `generateKeyPairSync`.

use elliptic_curve::{pkcs8::EncodePrivateKey as _, Generate};
use lexe_context::CtxExtension;
use lexe_utils::{error::ErrorExtensions, object::ObjectExt, result::ResultExt};
use p256::SecretKey as P256Secret;
use p384::SecretKey as P384Secret;
use p521::SecretKey as P521Secret;
use rand::RngExt;
use rquickjs::{
    function::Opt, Class, Ctx, Exception, Function, IntoJs, Null, Object, Result, Value,
};
use rsa::{pkcs1::EncodeRsaPrivateKey, BoxedUint, RsaPrivateKey};

use crate::{
    key_object::{
        parse_ec_curve_name, throw_code_error, value_to_bytes_with_encoding, KeyMaterial, KeyObject,
    },
    subtle::EllipticCurve,
};

enum SymmetricKeyType {
    Hmac(usize),
    Aes(usize),
}

fn parse_symmetric_keygen_args<'js>(
    ctx: &Ctx<'js>,
    key_type: &str,
    options: Object<'js>,
) -> Result<SymmetricKeyType> {
    let length_val: Value = options
        .get("length")
        .map_err(|_| Exception::throw_type(ctx, "The \"options.length\" property is required"))?;
    let Some(bits) = length_val.as_number() else {
        return throw_code_error(
            ctx,
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            "The \"options.length\" property must be of type number",
        );
    };
    if !bits.is_finite() || bits.fract() != 0.0 || bits < 0.0 {
        return throw_code_error(
            ctx,
            "RangeError",
            "ERR_OUT_OF_RANGE",
            "The value of \"options.length\" is out of range",
        );
    }
    match key_type {
        "hmac" => {
            if bits < 8.0 || bits > i32::MAX as f64 {
                return throw_code_error(
                    ctx,
                    "RangeError",
                    "ERR_OUT_OF_RANGE",
                    "The value of \"options.length\" is out of range. It must be >= 8 && <= 2147483647",
                );
            }
            let len_bytes = (bits as usize).div_ceil(8);
            Ok(SymmetricKeyType::Hmac(len_bytes))
        },
        "aes" => match bits as u64 {
            128 | 192 | 256 => Ok(SymmetricKeyType::Aes((bits as usize) / 8)),
            _ => throw_code_error(
                ctx,
                "RangeError",
                "ERR_INVALID_ARG_VALUE",
                "AES key length must be 128, 192, or 256 bits",
            ),
        },
        other => Err(Exception::throw_type(
            ctx,
            &format!("Unsupported key type: {other}"),
        )),
    }
}

pub fn generate_key_sync<'js>(
    ctx: Ctx<'js>,
    key_type: String,
    options: Object<'js>,
) -> Result<Class<'js, KeyObject>> {
    let spec = parse_symmetric_keygen_args(&ctx, &key_type, options)?;
    let byte_len = match spec {
        SymmetricKeyType::Hmac(n) | SymmetricKeyType::Aes(n) => n,
    };
    let mut bytes = vec![0u8; byte_len];
    rand::rng().fill(bytes.as_mut_slice());
    Class::instance(ctx, KeyObject::from_material(KeyMaterial::Secret(bytes)))
}

pub fn generate_key<'js>(
    ctx: Ctx<'js>,
    key_type: String,
    options: Object<'js>,
    callback: Function<'js>,
) -> Result<()> {
    let spec = parse_symmetric_keygen_args(&ctx, &key_type, options)?;
    let byte_len = match spec {
        SymmetricKeyType::Hmac(n) | SymmetricKeyType::Aes(n) => n,
    };
    ctx.clone().spawn_exit(async move {
        let mut bytes = vec![0u8; byte_len];
        rand::rng().fill(bytes.as_mut_slice());
        let ko = Class::instance(
            ctx.clone(),
            KeyObject::from_material(KeyMaterial::Secret(bytes)),
        )?;
        let _: () = callback.call((Null.into_js(&ctx), ko))?;
        Ok::<_, rquickjs::Error>(())
    })?;
    Ok(())
}

#[derive(Clone, Default)]
struct KeyEncodingSpec {
    format: Option<String>,
    key_type: Option<String>,
    cipher: Option<String>,
    passphrase: Option<Vec<u8>>,
}

enum KeyPairSpec {
    Rsa {
        modulus_length: usize,
        public_exponent: u64,
        is_pss: bool,
    },
    Ec(EllipticCurve),
    Ed25519,
    Ed448,
    X25519,
    X448,
}

fn parse_encoding_spec<'js>(
    ctx: &Ctx<'js>,
    options: Option<&Object<'js>>,
    prop: &str,
) -> Result<Option<KeyEncodingSpec>> {
    let Some(opts) = options else {
        return Ok(None);
    };
    let val: Value = opts.get(prop)?;
    if val.is_undefined() || val.is_null() {
        return Ok(None);
    }
    let obj = val.into_object().ok_or_else(|| {
        Exception::throw_type(
            ctx,
            &format!("The \"options.{prop}\" property must be an object"),
        )
    })?;
    let format = obj.get_optional::<_, String>("format")?;
    let key_type = obj.get_optional::<_, String>("type")?;
    let cipher = obj.get_optional::<_, String>("cipher")?;
    let pass_val: Value = obj.get("passphrase")?;
    let passphrase = if !pass_val.is_undefined() && !pass_val.is_null() {
        Some(value_to_bytes_with_encoding(
            ctx,
            &pass_val,
            Some("utf8"),
            "passphrase",
        )?)
    } else {
        None
    };
    Ok(Some(KeyEncodingSpec {
        format,
        key_type,
        cipher,
        passphrase,
    }))
}

fn parse_keypair_args<'js>(
    ctx: &Ctx<'js>,
    key_type: &str,
    options: Option<Value<'js>>,
) -> Result<(
    KeyPairSpec,
    Option<KeyEncodingSpec>,
    Option<KeyEncodingSpec>,
)> {
    let opts_obj = match options {
        None => None,
        Some(v) if v.is_undefined() || v.is_null() => None,
        Some(v) => Some(v.into_object().ok_or_else(|| {
            Exception::throw_type(ctx, "The \"options\" argument must be of type object")
        })?),
    };

    let pub_enc = parse_encoding_spec(ctx, opts_obj.as_ref(), "publicKeyEncoding")?;
    let priv_enc = parse_encoding_spec(ctx, opts_obj.as_ref(), "privateKeyEncoding")?;

    let spec = match key_type {
        "rsa" | "rsa-pss" => {
            let obj = opts_obj.as_ref().ok_or_else(|| {
                Exception::throw_type(
                    ctx,
                    "RSA key generation requires an options object with modulusLength",
                )
            })?;
            let mod_len: i64 = obj.get("modulusLength").map_err(|_| {
                Exception::throw_type(ctx, "The \"options.modulusLength\" property is required")
            })?;
            if !(512..=16384).contains(&mod_len) {
                return throw_code_error(
                    ctx,
                    "RangeError",
                    "ERR_OUT_OF_RANGE",
                    "The value of \"options.modulusLength\" is out of range. It must be >= 512 && <= 16384",
                );
            }
            let pub_exp = match obj.get::<_, Value>("publicExponent") {
                Ok(v) if !v.is_undefined() && !v.is_null() => {
                    if let Some(n) = v.as_number() {
                        if !n.is_finite() || n < 3.0 || (n as u64).is_multiple_of(2) {
                            return Err(Exception::throw_type(ctx, "Invalid RSA publicExponent"));
                        }
                        n as u64
                    } else {
                        65537
                    }
                },
                _ => 65537,
            };
            KeyPairSpec::Rsa {
                modulus_length: mod_len as usize,
                public_exponent: pub_exp,
                is_pss: key_type == "rsa-pss",
            }
        },
        "ec" => {
            let obj = opts_obj.as_ref().ok_or_else(|| {
                Exception::throw_type(ctx, "EC key generation requires options.namedCurve")
            })?;
            let curve_name: String = obj.get("namedCurve").map_err(|_| {
                Exception::throw_type(ctx, "The \"options.namedCurve\" property is required")
            })?;
            let curve = parse_ec_curve_name(&curve_name).ok_or_else(|| {
                Exception::throw_type(ctx, &format!("Invalid EC curve name: {curve_name}"))
            })?;
            KeyPairSpec::Ec(curve)
        },
        "ed25519" => KeyPairSpec::Ed25519,
        "ed448" => KeyPairSpec::Ed448,
        "x25519" => KeyPairSpec::X25519,
        "x448" => KeyPairSpec::X448,
        other => {
            return Err(Exception::throw_type(
                ctx,
                &format!("Unsupported key type: {other}"),
            ))
        },
    };
    Ok((spec, pub_enc, priv_enc))
}

fn generate_keypair_materials(
    spec: &KeyPairSpec,
) -> std::result::Result<(KeyMaterial, KeyMaterial), String> {
    let mut rng = rand::rng();
    let priv_mat = match spec {
        KeyPairSpec::Rsa {
            modulus_length,
            public_exponent,
            is_pss,
        } => {
            let exp = BoxedUint::from(*public_exponent);
            let priv_key = RsaPrivateKey::new_with_exp(&mut rng, *modulus_length, exp)
                .map_err(|e| format!("RSA key generation failed: {e}"))?;
            let pkcs1_der = priv_key
                .to_pkcs1_der()
                .map_err(|e| format!("Failed to encode generated RSA key: {e}"))?
                .as_bytes()
                .to_vec();
            KeyMaterial::RsaPrivate {
                pkcs1_der,
                is_pss: *is_pss,
            }
        },
        KeyPairSpec::Ec(curve) => {
            let pkcs8_der = match curve {
                EllipticCurve::P256 => P256Secret::generate_from_rng(&mut rng)
                    .to_pkcs8_der()
                    .map_err(|e| format!("Failed to encode P-256 key: {e}"))?
                    .as_bytes()
                    .to_vec(),
                EllipticCurve::P384 => P384Secret::generate_from_rng(&mut rng)
                    .to_pkcs8_der()
                    .map_err(|e| format!("Failed to encode P-384 key: {e}"))?
                    .as_bytes()
                    .to_vec(),
                EllipticCurve::P521 => P521Secret::generate_from_rng(&mut rng)
                    .to_pkcs8_der()
                    .map_err(|e| format!("Failed to encode P-521 key: {e}"))?
                    .as_bytes()
                    .to_vec(),
            };
            KeyMaterial::EcPrivate {
                curve: *curve,
                pkcs8_der,
            }
        },
        KeyPairSpec::Ed25519 => {
            let sk = ed25519_dalek::SigningKey::generate(&mut rng);
            KeyMaterial::Ed25519Private(sk.to_bytes().to_vec())
        },
        KeyPairSpec::Ed448 => {
            let sk = ed448_goldilocks::SigningKey::generate_from_rng(&mut rng);
            KeyMaterial::Ed448Private(sk.to_bytes().to_vec())
        },
        KeyPairSpec::X25519 => {
            let sk = x25519_dalek::StaticSecret::random_from_rng(&mut rng);
            KeyMaterial::X25519Private(sk.to_bytes().to_vec())
        },
        KeyPairSpec::X448 => {
            let mut raw = [0u8; 56];
            rng.fill(raw.as_mut_slice());
            let sk = x448::StaticSecret::from(raw);
            KeyMaterial::X448Private(sk.as_bytes().to_vec())
        },
    };
    let pub_mat = KeyObject::from_material(priv_mat.clone()).derive_public_material()?;
    Ok((pub_mat, priv_mat))
}

fn format_keypair_value<'js>(
    ctx: &Ctx<'js>,
    mat: KeyMaterial,
    enc: Option<&KeyEncodingSpec>,
) -> Result<Value<'js>> {
    let ko = KeyObject::from_material(mat);
    match enc {
        None => Class::instance(ctx.clone(), ko)?.into_js(ctx),
        Some(spec) => ko.export_with_spec(
            ctx,
            spec.format.as_deref(),
            spec.key_type.as_deref(),
            spec.cipher.as_deref(),
            spec.passphrase.as_deref(),
        ),
    }
}

pub fn generate_key_pair_sync<'js>(
    ctx: Ctx<'js>,
    key_type: String,
    options: Opt<Value<'js>>,
) -> Result<Object<'js>> {
    let (spec, pub_enc, priv_enc) = parse_keypair_args(&ctx, &key_type, options.0)?;
    let (pub_mat, priv_mat) = generate_keypair_materials(&spec).or_throw(&ctx)?;
    let pub_val = format_keypair_value(&ctx, pub_mat, pub_enc.as_ref())?;
    let priv_val = format_keypair_value(&ctx, priv_mat, priv_enc.as_ref())?;
    let out = Object::new(ctx)?;
    out.set("publicKey", pub_val)?;
    out.set("privateKey", priv_val)?;
    Ok(out)
}

pub fn generate_key_pair<'js>(
    ctx: Ctx<'js>,
    key_type: String,
    options_or_cb: Value<'js>,
    cb_opt: Opt<Function<'js>>,
) -> Result<()> {
    let (options, callback) = if let Some(cb) = cb_opt.0 {
        (Some(options_or_cb), cb)
    } else {
        let cb = options_or_cb.into_function().ok_or_else(|| {
            Exception::throw_type(&ctx, "The \"callback\" argument must be of type function")
        })?;
        (None, cb)
    };
    let (spec, pub_enc, priv_enc) = parse_keypair_args(&ctx, &key_type, options)?;
    ctx.clone().spawn_exit(async move {
        let res = tokio::task::spawn_blocking(move || generate_keypair_materials(&spec))
            .await
            .map_err(|e| format!("Key generation panicked: {e}"))
            .and_then(|r| r);
        match res {
            Ok((pub_mat, priv_mat)) => {
                let pub_val = format_keypair_value(&ctx, pub_mat, pub_enc.as_ref())?;
                let priv_val = format_keypair_value(&ctx, priv_mat, priv_enc.as_ref())?;
                let _: () = callback.call((Null.into_js(&ctx), pub_val, priv_val))?;
            },
            Err(msg) => {
                let err = Exception::throw_message(&ctx, &msg).into_value(&ctx)?;
                let _: () = callback.call((err,))?;
            },
        }
        Ok::<_, rquickjs::Error>(())
    })?;
    Ok(())
}
