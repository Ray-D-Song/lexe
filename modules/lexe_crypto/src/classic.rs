// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Classic Node `node:crypto` primitives:
//! `timingSafeEqual`, `hkdf`, `hkdfSync`, `ECDH` / `createECDH`, `diffieHellman`, `sign`, `verify`.

use ed25519_dalek::{Signature as EdSignature, Signer, Verifier};
use elliptic_curve::{sec1::ToSec1Point, Generate};
use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_utils::{
    bytes::ObjectBytes, error::ErrorExtensions, object::ObjectExt, result::ResultExt,
};
use p256::{
    ecdsa::{Signature as P256Sig, SigningKey as P256SignKey, VerifyingKey as P256VerifyKey},
    SecretKey as P256Secret,
};
use p384::{
    ecdsa::{Signature as P384Sig, SigningKey as P384SignKey, VerifyingKey as P384VerifyKey},
    SecretKey as P384Secret,
};
use p521::{
    ecdsa::{Signature as P521Sig, SigningKey as P521SignKey, VerifyingKey as P521VerifyKey},
    SecretKey as P521Secret,
};
use pkcs8::DecodePrivateKey;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, function::Opt, prelude::This, ArrayBuffer, Class, Ctx,
    Exception, FromJs, Function, IntoJs, JsLifetime, Null, Object, Result, Value,
};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey},
    pss::Pss,
    sha2::{Digest as _, Sha256, Sha384, Sha512},
    signature::hazmat::{PrehashSigner, PrehashVerifier},
    traits::PublicKeyParts,
    Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey,
};
use sha1::Sha1;

use crate::{
    hash::HashAlgorithm,
    key_object::{
        ec_coord_len, encode_bytes_to_output, normalize_ec_point, optional_encoding,
        parse_ec_curve_name, parse_private_key_value, parse_public_key_value,
        value_to_bytes_with_encoding, KeyMaterial, KeyObject,
    },
    provider::{CryptoProvider, SimpleDigest},
    rsa::{RSA_PKCS1_PADDING, RSA_PKCS1_PSS_PADDING, RSA_PSS_SALTLEN_DIGEST},
    subtle::EllipticCurve,
    CRYPTO_PROVIDER,
};

fn throw_range_with_code<T>(ctx: &Ctx<'_>, code: &str, msg: &str) -> Result<T> {
    let ctor: Function = ctx.globals().get("RangeError")?;
    let err: Object = ctor.call((msg,))?;
    err.set("code", code)?;
    Err(ctx.throw(err.into_value()))
}

pub fn timing_safe_equal<'js>(ctx: Ctx<'js>, a: Value<'js>, b: Value<'js>) -> Result<bool> {
    let a_bytes = ObjectBytes::from_array_buffer_view(a.as_object().ok_or_else(|| {
        Exception::throw_type(&ctx, "Arguments must be Buffer, TypedArray, or DataView")
    })?)?
    .ok_or_else(|| {
        Exception::throw_type(
            &ctx,
            "The \"a\" argument must be an instance of Buffer, TypedArray, or DataView",
        )
    })?;
    let b_bytes = ObjectBytes::from_array_buffer_view(b.as_object().ok_or_else(|| {
        Exception::throw_type(&ctx, "Arguments must be Buffer, TypedArray, or DataView")
    })?)?
    .ok_or_else(|| {
        Exception::throw_type(
            &ctx,
            "The \"b\" argument must be an instance of Buffer, TypedArray, or DataView",
        )
    })?;

    let a_slice = a_bytes.as_bytes(&ctx)?;
    let b_slice = b_bytes.as_bytes(&ctx)?;
    if a_slice.len() != b_slice.len() {
        return throw_range_with_code(
            &ctx,
            "ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH",
            "Input buffers must have the same byte length",
        );
    }
    let mut diff = 0u8;
    for (&x, &y) in a_slice.iter().zip(b_slice.iter()) {
        diff |= x ^ y;
    }
    Ok(diff == 0)
}

fn hkdf_derive_impl<'js>(
    ctx: &Ctx<'js>,
    digest: String,
    ikm: Value<'js>,
    salt: Value<'js>,
    info: Value<'js>,
    keylen: i64,
) -> Result<Vec<u8>> {
    let hash_alg = HashAlgorithm::try_from(digest.as_str())
        .map_err(|e| Exception::throw_type(ctx, &format!("Invalid digest: {e}")))?;
    if matches!(hash_alg, HashAlgorithm::Md5) {
        return Err(Exception::throw_type(
            ctx,
            "Digest 'md5' is not supported for HKDF",
        ));
    }
    if keylen < 0 {
        return throw_range_with_code(
            ctx,
            "ERR_OUT_OF_RANGE",
            "The value of \"keylen\" is out of range. It must be >= 0",
        );
    }
    let keylen = keylen as usize;
    let max_len = 255 * hash_alg.digest_len();
    if keylen > max_len {
        return throw_range_with_code(
            ctx,
            "ERR_CRYPTO_INVALID_KEYLEN",
            &format!("keylen ({keylen}) exceeds maximum HKDF length ({max_len})"),
        );
    }

    let ikm_bytes = if let Ok(ko) = Class::<KeyObject>::from_js(ctx, ikm.clone()) {
        match &ko.borrow().material {
            KeyMaterial::Secret(b) => b.clone(),
            _ => {
                return Err(Exception::throw_type(
                    ctx,
                    "Only secret KeyObject is supported for HKDF ikm",
                ))
            },
        }
    } else {
        value_to_bytes_with_encoding(ctx, &ikm, Some("utf8"), "ikm")?
    };
    let salt_bytes = value_to_bytes_with_encoding(ctx, &salt, Some("utf8"), "salt")?;
    let info_bytes = value_to_bytes_with_encoding(ctx, &info, Some("utf8"), "info")?;

    if keylen == 0 {
        return Ok(Vec::new());
    }
    CRYPTO_PROVIDER
        .hkdf_derive_key(&ikm_bytes, &salt_bytes, &info_bytes, keylen, hash_alg)
        .or_throw(ctx)
}

pub fn hkdf_sync<'js>(
    ctx: Ctx<'js>,
    digest: String,
    ikm: Value<'js>,
    salt: Value<'js>,
    info: Value<'js>,
    keylen: i64,
) -> Result<ArrayBuffer<'js>> {
    let out = hkdf_derive_impl(&ctx, digest, ikm, salt, info, keylen)?;
    ArrayBuffer::new(ctx, out)
}

pub fn hkdf<'js>(
    ctx: Ctx<'js>,
    digest: String,
    ikm: Value<'js>,
    salt: Value<'js>,
    info: Value<'js>,
    keylen: i64,
    callback: Function<'js>,
) -> Result<()> {
    let derived = hkdf_derive_impl(&ctx, digest, ikm, salt, info, keylen);
    ctx.clone().spawn_exit(async move {
        match derived {
            Ok(bytes) => {
                let ab = ArrayBuffer::new(ctx.clone(), bytes)?;
                let _: () = callback.call((Null.into_js(&ctx), ab))?;
            },
            Err(err) => {
                let js_err = err.into_value(&ctx)?;
                let _: () = callback.call((js_err,))?;
            },
        }
        Ok::<_, rquickjs::Error>(())
    })?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EcPointFormat {
    Uncompressed,
    Compressed,
    Hybrid,
}

impl EcPointFormat {
    fn from_opt<'js>(ctx: &Ctx<'js>, val: Option<Value<'js>>) -> Result<Self> {
        let Some(v) = val else {
            return Ok(Self::Uncompressed);
        };
        if v.is_undefined() || v.is_null() {
            return Ok(Self::Uncompressed);
        }
        let s = v
            .as_string()
            .ok_or_else(|| Exception::throw_type(ctx, "format must be a string"))?
            .to_string()?;
        match s.as_str() {
            "uncompressed" => Ok(Self::Uncompressed),
            "compressed" => Ok(Self::Compressed),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(Exception::throw_type(
                ctx,
                &format!("Invalid ECDH point format: {other}"),
            )),
        }
    }
}

#[rquickjs::class]
#[derive(Trace, JsLifetime)]
pub struct ECDH {
    #[qjs(skip_trace)]
    curve: EllipticCurve,
    private_scalar: Option<Vec<u8>>,
    public_uncompressed: Option<Vec<u8>>,
}

impl ECDH {
    fn derive_pub_from_scalar(
        curve: EllipticCurve,
        scalar: &[u8],
    ) -> std::result::Result<(Vec<u8>, Vec<u8>), String> {
        let coord_len = ec_coord_len(curve);
        if scalar.len() > coord_len {
            return Err("Invalid ECDH private key length".into());
        }
        let mut padded = vec![0u8; coord_len - scalar.len()];
        padded.extend_from_slice(scalar);
        let pub_bytes = match curve {
            EllipticCurve::P256 => {
                let sk = P256Secret::from_slice(&padded)
                    .map_err(|_| "Invalid P-256 private key".to_string())?;
                sk.public_key().to_sec1_point(false).as_bytes().to_vec()
            },
            EllipticCurve::P384 => {
                let sk = P384Secret::from_slice(&padded)
                    .map_err(|_| "Invalid P-384 private key".to_string())?;
                sk.public_key().to_sec1_point(false).as_bytes().to_vec()
            },
            EllipticCurve::P521 => {
                let sk = P521Secret::from_slice(&padded)
                    .map_err(|_| "Invalid P-521 private key".to_string())?;
                sk.public_key().to_sec1_point(false).as_bytes().to_vec()
            },
        };
        Ok((padded, pub_bytes))
    }

    fn format_point(
        curve: EllipticCurve,
        uncompressed: &[u8],
        fmt: EcPointFormat,
    ) -> std::result::Result<Vec<u8>, String> {
        match fmt {
            EcPointFormat::Uncompressed => Ok(uncompressed.to_vec()),
            EcPointFormat::Compressed => match curve {
                EllipticCurve::P256 => Ok(p256::PublicKey::from_sec1_bytes(uncompressed)
                    .map_err(|_| "Invalid P-256 point".to_string())?
                    .to_sec1_point(true)
                    .as_bytes()
                    .to_vec()),
                EllipticCurve::P384 => Ok(p384::PublicKey::from_sec1_bytes(uncompressed)
                    .map_err(|_| "Invalid P-384 point".to_string())?
                    .to_sec1_point(true)
                    .as_bytes()
                    .to_vec()),
                EllipticCurve::P521 => Ok(p521::PublicKey::from_sec1_bytes(uncompressed)
                    .map_err(|_| "Invalid P-521 point".to_string())?
                    .to_sec1_point(true)
                    .as_bytes()
                    .to_vec()),
            },
            EcPointFormat::Hybrid => {
                let mut out = uncompressed.to_vec();
                let is_odd = (out[out.len() - 1] & 1) != 0;
                out[0] = if is_odd { 0x07 } else { 0x06 };
                Ok(out)
            },
        }
    }
}

#[rquickjs::methods]
impl ECDH {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'_>) -> Result<Self> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "ECDH"
    }

    #[qjs(rename = "generateKeys")]
    pub fn generate_keys<'js>(
        &mut self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
        format: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let enc = optional_encoding(&ctx, encoding.0)?;
        let fmt = EcPointFormat::from_opt(&ctx, format.0)?;
        let mut rng = rand::rng();
        let (priv_bytes, pub_uncompressed) = match self.curve {
            EllipticCurve::P256 => {
                let sk = P256Secret::try_generate_from_rng(&mut rng)
                    .map_err(|_| Exception::throw_message(&ctx, "Key generation failed"))?;
                (
                    sk.to_bytes().to_vec(),
                    sk.public_key().to_sec1_point(false).as_bytes().to_vec(),
                )
            },
            EllipticCurve::P384 => {
                let sk = P384Secret::try_generate_from_rng(&mut rng)
                    .map_err(|_| Exception::throw_message(&ctx, "Key generation failed"))?;
                (
                    sk.to_bytes().to_vec(),
                    sk.public_key().to_sec1_point(false).as_bytes().to_vec(),
                )
            },
            EllipticCurve::P521 => {
                let sk = P521Secret::try_generate_from_rng(&mut rng)
                    .map_err(|_| Exception::throw_message(&ctx, "Key generation failed"))?;
                (
                    sk.to_bytes().to_vec(),
                    sk.public_key().to_sec1_point(false).as_bytes().to_vec(),
                )
            },
        };
        let formatted = Self::format_point(self.curve, &pub_uncompressed, fmt).or_throw(&ctx)?;
        self.private_scalar = Some(priv_bytes);
        self.public_uncompressed = Some(pub_uncompressed);
        encode_bytes_to_output(&ctx, formatted, enc.as_deref())
    }

    #[qjs(rename = "getPublicKey")]
    pub fn get_public_key<'js>(
        &self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
        format: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let enc = optional_encoding(&ctx, encoding.0)?;
        let fmt = EcPointFormat::from_opt(&ctx, format.0)?;
        let Some(ref pub_uncompressed) = self.public_uncompressed else {
            return Err(Exception::throw_message(&ctx, "Failed to convert key"));
        };
        let formatted = Self::format_point(self.curve, pub_uncompressed, fmt).or_throw(&ctx)?;
        encode_bytes_to_output(&ctx, formatted, enc.as_deref())
    }

    #[qjs(rename = "getPrivateKey")]
    pub fn get_private_key<'js>(
        &self,
        ctx: Ctx<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let enc = optional_encoding(&ctx, encoding.0)?;
        let Some(ref priv_bytes) = self.private_scalar else {
            return Err(Exception::throw_message(&ctx, "Failed to convert key"));
        };
        encode_bytes_to_output(&ctx, priv_bytes.clone(), enc.as_deref())
    }

    #[qjs(rename = "setPrivateKey")]
    pub fn set_private_key<'js>(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        private_key: Value<'js>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let enc = optional_encoding(&ctx, encoding.0)?;
        let raw = value_to_bytes_with_encoding(&ctx, &private_key, enc.as_deref(), "privateKey")?;
        {
            let mut inner = this.0.borrow_mut();
            let (scalar, pub_bytes) =
                Self::derive_pub_from_scalar(inner.curve, &raw).or_throw(&ctx)?;
            inner.private_scalar = Some(scalar);
            inner.public_uncompressed = Some(pub_bytes);
        }
        Ok(this.0)
    }

    #[qjs(rename = "computeSecret")]
    pub fn compute_secret<'js>(
        &self,
        ctx: Ctx<'js>,
        other_public_key: Value<'js>,
        input_encoding: Opt<Value<'js>>,
        output_encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let in_enc = optional_encoding(&ctx, input_encoding.0)?;
        let out_enc = optional_encoding(&ctx, output_encoding.0)?;
        let Some(ref priv_bytes) = self.private_scalar else {
            return Err(Exception::throw_message(
                &ctx,
                "Both private and public keys must be set",
            ));
        };
        let peer_raw = value_to_bytes_with_encoding(
            &ctx,
            &other_public_key,
            in_enc.as_deref(),
            "otherPublicKey",
        )?;
        let peer_sec1 = normalize_ec_point(self.curve, &peer_raw).or_throw(&ctx)?;
        let shared = match self.curve {
            EllipticCurve::P256 => {
                let sk = P256Secret::from_slice(priv_bytes).or_throw(&ctx)?;
                let pk = p256::PublicKey::from_sec1_bytes(&peer_sec1).or_throw(&ctx)?;
                p256::elliptic_curve::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine())
                    .raw_secret_bytes()
                    .to_vec()
            },
            EllipticCurve::P384 => {
                let sk = P384Secret::from_slice(priv_bytes).or_throw(&ctx)?;
                let pk = p384::PublicKey::from_sec1_bytes(&peer_sec1).or_throw(&ctx)?;
                p384::elliptic_curve::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine())
                    .raw_secret_bytes()
                    .to_vec()
            },
            EllipticCurve::P521 => {
                let sk = P521Secret::from_slice(priv_bytes).or_throw(&ctx)?;
                let pk = p521::PublicKey::from_sec1_bytes(&peer_sec1).or_throw(&ctx)?;
                p521::elliptic_curve::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine())
                    .raw_secret_bytes()
                    .to_vec()
            },
        };
        encode_bytes_to_output(&ctx, shared, out_enc.as_deref())
    }

    #[qjs(static, rename = "convertKey")]
    pub fn convert_key<'js>(
        ctx: Ctx<'js>,
        key: Value<'js>,
        curve_name: String,
        input_encoding: Opt<Value<'js>>,
        output_encoding: Opt<Value<'js>>,
        format: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let curve = parse_ec_curve_name(&curve_name).ok_or_else(|| {
            Exception::throw_type(&ctx, &format!("Invalid EC curve: {curve_name}"))
        })?;
        let in_enc = optional_encoding(&ctx, input_encoding.0)?;
        let out_enc = optional_encoding(&ctx, output_encoding.0)?;
        let fmt = EcPointFormat::from_opt(&ctx, format.0)?;
        let raw = value_to_bytes_with_encoding(&ctx, &key, in_enc.as_deref(), "key")?;
        let uncompressed = normalize_ec_point(curve, &raw).or_throw(&ctx)?;
        let formatted = Self::format_point(curve, &uncompressed, fmt).or_throw(&ctx)?;
        encode_bytes_to_output(&ctx, formatted, out_enc.as_deref())
    }
}

pub fn create_ecdh<'js>(ctx: Ctx<'js>, curve_name: String) -> Result<Class<'js, ECDH>> {
    let curve = parse_ec_curve_name(&curve_name)
        .ok_or_else(|| Exception::throw_type(&ctx, &format!("Invalid EC curve: {curve_name}")))?;
    Class::instance(
        ctx,
        ECDH {
            curve,
            private_scalar: None,
            public_uncompressed: None,
        },
    )
}

pub fn diffie_hellman<'js>(ctx: Ctx<'js>, options: Object<'js>) -> Result<Buffer> {
    let priv_val: Value = options.get("privateKey").map_err(|_| {
        Exception::throw_type(&ctx, "The \"options.privateKey\" property is required")
    })?;
    let pub_val: Value = options.get("publicKey").map_err(|_| {
        Exception::throw_type(&ctx, "The \"options.publicKey\" property is required")
    })?;
    let priv_ko = parse_private_key_value(&ctx, priv_val)?;
    let pub_ko = parse_public_key_value(&ctx, pub_val)?;

    let shared = match (&priv_ko.material, &pub_ko.material) {
        (KeyMaterial::X25519Private(priv32), KeyMaterial::X25519Public(pub32)) => CRYPTO_PROVIDER
            .x25519_derive_bits(priv32, pub32)
            .or_throw(&ctx)?,
        (
            KeyMaterial::EcPrivate {
                curve: c1,
                pkcs8_der,
            },
            KeyMaterial::EcPublic { curve: c2, sec1 },
        ) => {
            if c1 != c2 {
                return Err(Exception::throw_type(
                    &ctx,
                    "Diffie-Hellman keys must use the same elliptic curve",
                ));
            }
            CRYPTO_PROVIDER
                .ecdh_derive_bits(*c1, pkcs8_der, sec1)
                .or_throw(&ctx)?
        },
        _ => {
            return Err(Exception::throw_type(
                &ctx,
                "Keys must be compatible EC or X25519 keys for diffieHellman",
            ))
        },
    };

    Ok(Buffer(shared))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DsaEncoding {
    Der,
    IeeeP1363,
}

struct SignVerifyKeyAndOpts {
    key: KeyObject,
    padding: Option<i32>,
    salt_length: Option<i32>,
    dsa_encoding: DsaEncoding,
}

fn parse_sign_verify_key<'js>(
    ctx: &Ctx<'js>,
    val: Value<'js>,
    is_private: bool,
) -> Result<SignVerifyKeyAndOpts> {
    if Class::<KeyObject>::from_js(ctx, val.clone()).is_ok()
        || val.as_string().is_some()
        || val
            .as_object()
            .and_then(|o| ObjectBytes::from_array_buffer(o).ok().flatten())
            .is_some()
    {
        let ko = if is_private {
            parse_private_key_value(ctx, val)?
        } else {
            parse_public_key_value(ctx, val)?
        };
        return Ok(SignVerifyKeyAndOpts {
            key: ko,
            padding: None,
            salt_length: None,
            dsa_encoding: DsaEncoding::Der,
        });
    }

    let obj = val
        .as_object()
        .ok_or_else(|| Exception::throw_type(ctx, "Invalid key argument"))?;
    let padding = obj.get_optional::<_, i32>("padding")?;
    let salt_length = obj.get_optional::<_, i32>("saltLength")?;
    let dsa_encoding = match obj.get_optional::<_, String>("dsaEncoding")?.as_deref() {
        None | Some("der") => DsaEncoding::Der,
        Some("ieee-p1363") => DsaEncoding::IeeeP1363,
        Some(other) => {
            return Err(Exception::throw_type(
                ctx,
                &format!("Invalid dsaEncoding: {other}"),
            ))
        },
    };

    let inner_key: Value = obj.get("key")?;
    let target = if Class::<KeyObject>::from_js(ctx, inner_key.clone()).is_ok() {
        inner_key
    } else {
        val.clone()
    };
    let ko = if is_private {
        parse_private_key_value(ctx, target)?
    } else {
        parse_public_key_value(ctx, target)?
    };
    Ok(SignVerifyKeyAndOpts {
        key: ko,
        padding,
        salt_length,
        dsa_encoding,
    })
}

fn parse_sign_digest_algorithm(ctx: &Ctx<'_>, alg: &Value<'_>) -> Result<Option<HashAlgorithm>> {
    if alg.is_null() || alg.is_undefined() {
        return Ok(None);
    }
    let raw = alg
        .as_string()
        .ok_or_else(|| {
            Exception::throw_type(ctx, "algorithm must be a string, null, or undefined")
        })?
        .to_string()?;
    let upper = raw.to_ascii_uppercase();
    let norm = upper
        .strip_prefix("RSA-")
        .or_else(|| upper.strip_prefix("ECDSA-WITH-"))
        .unwrap_or(upper.as_str());
    let norm = match norm {
        "SHA256WITHRSAENCRYPTION" => "SHA256",
        "SHA384WITHRSAENCRYPTION" => "SHA384",
        "SHA512WITHRSAENCRYPTION" => "SHA512",
        "SHA1WITHRSAENCRYPTION" => "SHA1",
        other => other,
    };
    let hash = HashAlgorithm::try_from(norm)
        .map_err(|_| Exception::throw_type(ctx, &format!("Unknown message digest: {raw}")))?;
    Ok(Some(hash))
}

fn digest_bytes(alg: HashAlgorithm, data: &[u8]) -> Vec<u8> {
    let mut h = CRYPTO_PROVIDER.digest(alg);
    h.update(data);
    h.finalize()
}

pub(crate) fn sign_material_bytes(
    material: &KeyMaterial,
    hash_opt: Option<HashAlgorithm>,
    data: &[u8],
    padding: Option<i32>,
    salt_length: Option<i32>,
    dsa_encoding: DsaEncoding,
) -> std::result::Result<Vec<u8>, String> {
    match material {
        KeyMaterial::Ed25519Private(seed32) => {
            if hash_opt.is_some() {
                return Err("Ed25519 sign requires algorithm to be null or undefined".into());
            }
            let arr: [u8; 32] = seed32
                .as_slice()
                .try_into()
                .map_err(|_| "Invalid Ed25519 private key".to_string())?;
            let sk = ed25519_dalek::SigningKey::from_bytes(&arr);
            let sig = sk
                .try_sign(data)
                .map_err(|_| "Ed25519 signing failed".to_string())?;
            Ok(sig.to_bytes().to_vec())
        },
        KeyMaterial::EcPrivate { curve, pkcs8_der } => {
            let hash =
                hash_opt.ok_or_else(|| "ECDSA sign requires a digest algorithm".to_string())?;
            let digest = digest_bytes(hash, data);
            match curve {
                EllipticCurve::P256 => {
                    let sk = P256Secret::from_pkcs8_der(pkcs8_der)
                        .map_err(|_| "Invalid P-256 private key".to_string())?;
                    let sig: P256Sig = P256SignKey::from(sk)
                        .sign_prehash(&digest)
                        .map_err(|_| "ECDSA signing failed".to_string())?;
                    Ok(match dsa_encoding {
                        DsaEncoding::IeeeP1363 => sig.to_bytes().to_vec(),
                        DsaEncoding::Der => sig.to_der().as_bytes().to_vec(),
                    })
                },
                EllipticCurve::P384 => {
                    let sk = P384Secret::from_pkcs8_der(pkcs8_der)
                        .map_err(|_| "Invalid P-384 private key".to_string())?;
                    let sig: P384Sig = P384SignKey::from(sk)
                        .sign_prehash(&digest)
                        .map_err(|_| "ECDSA signing failed".to_string())?;
                    Ok(match dsa_encoding {
                        DsaEncoding::IeeeP1363 => sig.to_bytes().to_vec(),
                        DsaEncoding::Der => sig.to_der().as_bytes().to_vec(),
                    })
                },
                EllipticCurve::P521 => {
                    let sk = P521Secret::from_pkcs8_der(pkcs8_der)
                        .map_err(|_| "Invalid P-521 private key".to_string())?;
                    let sig: P521Sig = P521SignKey::from(sk)
                        .sign_prehash(&digest)
                        .map_err(|_| "ECDSA signing failed".to_string())?;
                    Ok(match dsa_encoding {
                        DsaEncoding::IeeeP1363 => sig.to_bytes().to_vec(),
                        DsaEncoding::Der => sig.to_der().as_bytes().to_vec(),
                    })
                },
            }
        },
        KeyMaterial::RsaPrivate { pkcs1_der, is_pss } => {
            let hash =
                hash_opt.ok_or_else(|| "RSA sign requires a digest algorithm".to_string())?;
            let digest = digest_bytes(hash, data);
            let sk = RsaPrivateKey::from_pkcs1_der(pkcs1_der)
                .map_err(|_| "Invalid RSA private key".to_string())?;
            let mut rng = rand::rng();
            let default_pad = if *is_pss {
                RSA_PKCS1_PSS_PADDING
            } else {
                RSA_PKCS1_PADDING
            };
            let pad = padding.unwrap_or(default_pad);
            match pad {
                RSA_PKCS1_PADDING => match hash {
                    HashAlgorithm::Sha1 => {
                        sk.sign_with_rng(&mut rng, Pkcs1v15Sign::new::<Sha1>(), &digest)
                    },
                    HashAlgorithm::Sha256 => {
                        sk.sign_with_rng(&mut rng, Pkcs1v15Sign::new::<Sha256>(), &digest)
                    },
                    HashAlgorithm::Sha384 => {
                        sk.sign_with_rng(&mut rng, Pkcs1v15Sign::new::<Sha384>(), &digest)
                    },
                    HashAlgorithm::Sha512 => {
                        sk.sign_with_rng(&mut rng, Pkcs1v15Sign::new::<Sha512>(), &digest)
                    },
                    HashAlgorithm::Md5 => return Err("MD5 RSA signing is not supported".into()),
                }
                .map_err(|_| "RSA signing failed".to_string()),
                RSA_PKCS1_PSS_PADDING => {
                    let hlen = hash.digest_len();
                    let mod_bits = sk.n().bits();
                    let em_len = (mod_bits as usize - 1).div_ceil(8);
                    let slen = match salt_length {
                        Some(v) if v == RSA_PSS_SALTLEN_DIGEST => hlen,
                        Some(v) if v < 0 => em_len.saturating_sub(hlen + 2),
                        Some(v) => v as usize,
                        None => em_len.saturating_sub(hlen + 2),
                    };
                    match hash {
                        HashAlgorithm::Sha1 => {
                            sk.sign_with_rng(&mut rng, Pss::<Sha1>::new_with_salt(slen), &digest)
                        },
                        HashAlgorithm::Sha256 => {
                            sk.sign_with_rng(&mut rng, Pss::<Sha256>::new_with_salt(slen), &digest)
                        },
                        HashAlgorithm::Sha384 => {
                            sk.sign_with_rng(&mut rng, Pss::<Sha384>::new_with_salt(slen), &digest)
                        },
                        HashAlgorithm::Sha512 => {
                            sk.sign_with_rng(&mut rng, Pss::<Sha512>::new_with_salt(slen), &digest)
                        },
                        HashAlgorithm::Md5 => {
                            return Err("MD5 RSA-PSS signing is not supported".into())
                        },
                    }
                    .map_err(|_| "RSA-PSS signing failed".to_string())
                },
                other => Err(format!("Unsupported RSA padding mode: {other}")),
            }
        },
        _ => Err("Invalid key type for signing".into()),
    }
}

pub(crate) fn verify_material_bytes(
    material: &KeyMaterial,
    hash_opt: Option<HashAlgorithm>,
    data: &[u8],
    sig_bytes: &[u8],
    padding: Option<i32>,
    salt_length: Option<i32>,
    dsa_encoding: DsaEncoding,
) -> std::result::Result<bool, String> {
    match material {
        KeyMaterial::Ed25519Public(pub32) => {
            if hash_opt.is_some() {
                return Err("Ed25519 verify requires algorithm to be null or undefined".into());
            }
            let arr: [u8; 32] = pub32
                .as_slice()
                .try_into()
                .map_err(|_| "Invalid Ed25519 public key".to_string())?;
            let vk = ed25519_dalek::VerifyingKey::from_bytes(&arr)
                .map_err(|_| "Invalid Ed25519 public key".to_string())?;
            let Ok(sig_arr) = <[u8; 64]>::try_from(sig_bytes) else {
                return Ok(false);
            };
            let sig = EdSignature::from_bytes(&sig_arr);
            Ok(vk.verify(data, &sig).is_ok())
        },
        KeyMaterial::EcPublic { curve, sec1 } => {
            let hash =
                hash_opt.ok_or_else(|| "ECDSA verify requires a digest algorithm".to_string())?;
            let digest = digest_bytes(hash, data);
            match curve {
                EllipticCurve::P256 => {
                    let vk = P256VerifyKey::from_sec1_bytes(sec1)
                        .map_err(|_| "Invalid P-256 public key".to_string())?;
                    let sig = match dsa_encoding {
                        DsaEncoding::IeeeP1363 => P256Sig::from_slice(sig_bytes),
                        DsaEncoding::Der => P256Sig::from_der(sig_bytes),
                    };
                    let Ok(sig) = sig else {
                        return Ok(false);
                    };
                    Ok(vk.verify_prehash(&digest, &sig).is_ok())
                },
                EllipticCurve::P384 => {
                    let vk = P384VerifyKey::from_sec1_bytes(sec1)
                        .map_err(|_| "Invalid P-384 public key".to_string())?;
                    let sig = match dsa_encoding {
                        DsaEncoding::IeeeP1363 => P384Sig::from_slice(sig_bytes),
                        DsaEncoding::Der => P384Sig::from_der(sig_bytes),
                    };
                    let Ok(sig) = sig else {
                        return Ok(false);
                    };
                    Ok(vk.verify_prehash(&digest, &sig).is_ok())
                },
                EllipticCurve::P521 => {
                    let vk = P521VerifyKey::from_sec1_bytes(sec1)
                        .map_err(|_| "Invalid P-521 public key".to_string())?;
                    let sig = match dsa_encoding {
                        DsaEncoding::IeeeP1363 => P521Sig::from_slice(sig_bytes),
                        DsaEncoding::Der => P521Sig::from_der(sig_bytes),
                    };
                    let Ok(sig) = sig else {
                        return Ok(false);
                    };
                    Ok(vk.verify_prehash(&digest, &sig).is_ok())
                },
            }
        },
        KeyMaterial::RsaPublic { pkcs1_der, is_pss } => {
            let hash =
                hash_opt.ok_or_else(|| "RSA verify requires a digest algorithm".to_string())?;
            let digest = digest_bytes(hash, data);
            let pk = RsaPublicKey::from_pkcs1_der(pkcs1_der)
                .map_err(|_| "Invalid RSA public key".to_string())?;
            let default_pad = if *is_pss {
                RSA_PKCS1_PSS_PADDING
            } else {
                RSA_PKCS1_PADDING
            };
            let pad = padding.unwrap_or(default_pad);
            match pad {
                RSA_PKCS1_PADDING => Ok(match hash {
                    HashAlgorithm::Sha1 => pk
                        .verify(Pkcs1v15Sign::new::<Sha1>(), &digest, sig_bytes)
                        .is_ok(),
                    HashAlgorithm::Sha256 => pk
                        .verify(Pkcs1v15Sign::new::<Sha256>(), &digest, sig_bytes)
                        .is_ok(),
                    HashAlgorithm::Sha384 => pk
                        .verify(Pkcs1v15Sign::new::<Sha384>(), &digest, sig_bytes)
                        .is_ok(),
                    HashAlgorithm::Sha512 => pk
                        .verify(Pkcs1v15Sign::new::<Sha512>(), &digest, sig_bytes)
                        .is_ok(),
                    HashAlgorithm::Md5 => {
                        return Err("MD5 RSA verification is not supported".into())
                    },
                }),
                RSA_PKCS1_PSS_PADDING => {
                    let explicit_slen = match salt_length {
                        Some(v) if v == RSA_PSS_SALTLEN_DIGEST => Some(hash.digest_len()),
                        Some(v) if v >= 0 => Some(v as usize),
                        _ => None,
                    };
                    Ok(match hash {
                        HashAlgorithm::Sha1 => pk
                            .verify(
                                Pss {
                                    blinded: false,
                                    digest: Sha1::new(),
                                    salt_len: explicit_slen,
                                },
                                &digest,
                                sig_bytes,
                            )
                            .is_ok(),
                        HashAlgorithm::Sha256 => pk
                            .verify(
                                Pss {
                                    blinded: false,
                                    digest: Sha256::new(),
                                    salt_len: explicit_slen,
                                },
                                &digest,
                                sig_bytes,
                            )
                            .is_ok(),
                        HashAlgorithm::Sha384 => pk
                            .verify(
                                Pss {
                                    blinded: false,
                                    digest: Sha384::new(),
                                    salt_len: explicit_slen,
                                },
                                &digest,
                                sig_bytes,
                            )
                            .is_ok(),
                        HashAlgorithm::Sha512 => pk
                            .verify(
                                Pss {
                                    blinded: false,
                                    digest: Sha512::new(),
                                    salt_len: explicit_slen,
                                },
                                &digest,
                                sig_bytes,
                            )
                            .is_ok(),
                        HashAlgorithm::Md5 => {
                            return Err("MD5 RSA-PSS verification is not supported".into())
                        },
                    })
                },
                other => Err(format!("Unsupported RSA padding mode: {other}")),
            }
        },
        _ => Err("Invalid key type for verification".into()),
    }
}

pub fn sign<'js>(
    ctx: Ctx<'js>,
    algorithm: Value<'js>,
    data: Value<'js>,
    key: Value<'js>,
    callback: Opt<Function<'js>>,
) -> Result<Value<'js>> {
    let hash_opt = parse_sign_digest_algorithm(&ctx, &algorithm)?;
    let data_bytes = value_to_bytes_with_encoding(&ctx, &data, Some("utf8"), "data")?;
    let parsed = parse_sign_verify_key(&ctx, key, true)?;

    let res = sign_material_bytes(
        &parsed.key.material,
        hash_opt,
        &data_bytes,
        parsed.padding,
        parsed.salt_length,
        parsed.dsa_encoding,
    );

    if let Some(cb) = callback.0 {
        let cb_ctx = ctx.clone();
        ctx.clone().spawn_exit(async move {
            match res {
                Ok(sig) => {
                    let buf = Buffer(sig).into_js(&cb_ctx)?;
                    let _: () = cb.call((Null.into_js(&cb_ctx), buf))?;
                },
                Err(msg) => {
                    let err = Exception::throw_message(&cb_ctx, &msg).into_value(&cb_ctx)?;
                    let _: () = cb.call((err,))?;
                },
            }
            Ok::<_, rquickjs::Error>(())
        })?;
        return Ok(Value::new_undefined(ctx));
    }

    let sig = res.or_throw(&ctx)?;
    Buffer(sig).into_js(&ctx)
}

pub fn verify<'js>(
    ctx: Ctx<'js>,
    algorithm: Value<'js>,
    data: Value<'js>,
    key: Value<'js>,
    signature: Value<'js>,
    callback: Opt<Function<'js>>,
) -> Result<Value<'js>> {
    let hash_opt = parse_sign_digest_algorithm(&ctx, &algorithm)?;
    let data_bytes = value_to_bytes_with_encoding(&ctx, &data, Some("utf8"), "data")?;
    let parsed = parse_sign_verify_key(&ctx, key, false)?;
    let sig_bytes = value_to_bytes_with_encoding(&ctx, &signature, Some("utf8"), "signature")?;

    let res = verify_material_bytes(
        &parsed.key.material,
        hash_opt,
        &data_bytes,
        &sig_bytes,
        parsed.padding,
        parsed.salt_length,
        parsed.dsa_encoding,
    );

    if let Some(cb) = callback.0 {
        let cb_ctx = ctx.clone();
        ctx.clone().spawn_exit(async move {
            match res {
                Ok(valid) => {
                    let _: () = cb.call((Null.into_js(&cb_ctx), valid))?;
                },
                Err(msg) => {
                    let err = Exception::throw_message(&cb_ctx, &msg).into_value(&cb_ctx)?;
                    let _: () = cb.call((err,))?;
                },
            }
            Ok::<_, rquickjs::Error>(())
        })?;
        return Ok(Value::new_undefined(ctx));
    }

    let valid = res.or_throw(&ctx)?;
    Ok(Value::new_bool(ctx, valid))
}
