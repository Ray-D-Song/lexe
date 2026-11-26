// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `KeyObject`, `createSecretKey`, `createPrivateKey`, and `createPublicKey`.

use const_oid::db::{rfc5912, rfc8410};
use der::{
    asn1::{BitStringRef, ObjectIdentifier, OctetStringRef, UintRef},
    Decode, Encode,
};
use lexe_buffer::Buffer;
use lexe_encoding::{
    bytes_from_b64, bytes_from_b64_url_safe, bytes_from_hex, bytes_to_b64_string,
    bytes_to_b64_url_safe_string, bytes_to_hex_string, Encoder,
};
use lexe_utils::{bytes::ObjectBytes, object::ObjectExt, result::ResultExt};
use pkcs8::PrivateKeyInfoRef;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, function::Opt, BigInt, Class, Ctx, Exception, FromJs,
    IntoJs, JsLifetime, Object, Result, Value,
};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey, EncodeRsaPublicKey},
    pkcs8::{DecodePrivateKey, EncodePrivateKey},
    RsaPrivateKey, RsaPublicKey,
};
use sec1::EcPrivateKey;
use spki::{AlgorithmIdentifierRef, SubjectPublicKeyInfo, SubjectPublicKeyInfoRef};

use crate::subtle::EllipticCurve;

const EC_PUBLIC_KEY_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const OID_PRIME256V1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const OID_SECP384R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");
const OID_SECP521R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.35");
const OID_RSA_PSS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.10");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyMaterial {
    Secret(Vec<u8>),
    RsaPrivate {
        pkcs1_der: Vec<u8>,
        is_pss: bool,
    },
    RsaPublic {
        pkcs1_der: Vec<u8>,
        is_pss: bool,
    },
    EcPrivate {
        curve: EllipticCurve,
        pkcs8_der: Vec<u8>,
    },
    EcPublic {
        curve: EllipticCurve,
        sec1: Vec<u8>,
    },
    Ed25519Private(Vec<u8>),
    Ed25519Public(Vec<u8>),
    X25519Private(Vec<u8>),
    X25519Public(Vec<u8>),
}

#[rquickjs::class]
#[derive(Clone, Trace, JsLifetime)]
pub struct KeyObject {
    #[qjs(skip_trace)]
    pub(crate) material: KeyMaterial,
}

impl KeyObject {
    pub(crate) fn from_material(material: KeyMaterial) -> Self {
        Self { material }
    }

    pub(crate) fn export_pkcs8_der(&self) -> std::result::Result<Vec<u8>, String> {
        match &self.material {
            KeyMaterial::RsaPrivate { pkcs1_der, .. } => {
                let key = RsaPrivateKey::from_pkcs1_der(pkcs1_der)
                    .map_err(|e| format!("Invalid RSA private key: {e}"))?;
                key.to_pkcs8_der()
                    .map(|doc| doc.as_bytes().to_vec())
                    .map_err(|e| format!("Failed to encode RSA private key: {e}"))
            },
            KeyMaterial::EcPrivate { pkcs8_der, .. } => Ok(pkcs8_der.clone()),
            KeyMaterial::Ed25519Private(seed32) => encode_okp_pkcs8(rfc8410::ID_ED_25519, seed32),
            KeyMaterial::X25519Private(raw32) => encode_okp_pkcs8(rfc8410::ID_X_25519, raw32),
            _ => Err("Key is not a private key".into()),
        }
    }

    pub(crate) fn export_spki_der(&self) -> std::result::Result<Vec<u8>, String> {
        match &self.material {
            KeyMaterial::RsaPublic { pkcs1_der, .. } => {
                let alg = spki::AlgorithmIdentifier::<der::asn1::Any> {
                    oid: rfc5912::RSA_ENCRYPTION,
                    parameters: Some(der::asn1::Null.into()),
                };
                let spki = SubjectPublicKeyInfo {
                    algorithm: alg,
                    subject_public_key: spki::der::asn1::BitString::from_bytes(pkcs1_der)
                        .map_err(|e| format!("Invalid RSA public key: {e}"))?,
                };
                spki.to_der()
                    .map_err(|e| format!("Failed to encode SPKI: {e}"))
            },
            KeyMaterial::EcPublic { curve, sec1 } => {
                let spki = SubjectPublicKeyInfo {
                    algorithm: spki::AlgorithmIdentifier::<ObjectIdentifier> {
                        oid: EC_PUBLIC_KEY_OID,
                        parameters: Some(curve_oid(*curve)),
                    },
                    subject_public_key: spki::der::asn1::BitString::from_bytes(sec1)
                        .map_err(|e| format!("Invalid EC public key: {e}"))?,
                };
                spki.to_der()
                    .map_err(|e| format!("Failed to encode SPKI: {e}"))
            },
            KeyMaterial::Ed25519Public(raw32) => encode_okp_spki(rfc8410::ID_ED_25519, raw32),
            KeyMaterial::X25519Public(raw32) => encode_okp_spki(rfc8410::ID_X_25519, raw32),
            _ => Err("Key is not a public key".into()),
        }
    }

    pub(crate) fn derive_public_material(&self) -> std::result::Result<KeyMaterial, String> {
        match &self.material {
            KeyMaterial::RsaPublic { .. }
            | KeyMaterial::EcPublic { .. }
            | KeyMaterial::Ed25519Public(_)
            | KeyMaterial::X25519Public(_) => Ok(self.material.clone()),
            KeyMaterial::RsaPrivate { pkcs1_der, is_pss } => {
                let priv_key = RsaPrivateKey::from_pkcs1_der(pkcs1_der)
                    .map_err(|e| format!("Invalid RSA private key: {e}"))?;
                let pub_der = priv_key
                    .to_public_key()
                    .to_pkcs1_der()
                    .map_err(|e| format!("Failed to extract RSA public key: {e}"))?;
                Ok(KeyMaterial::RsaPublic {
                    pkcs1_der: pub_der.as_bytes().to_vec(),
                    is_pss: *is_pss,
                })
            },
            KeyMaterial::EcPrivate { curve, pkcs8_der } => {
                let sec1 = ec_public_sec1_from_pkcs8(*curve, pkcs8_der)?;
                Ok(KeyMaterial::EcPublic {
                    curve: *curve,
                    sec1,
                })
            },
            KeyMaterial::Ed25519Private(seed32) => {
                let arr: [u8; 32] = seed32
                    .as_slice()
                    .try_into()
                    .map_err(|_| "Invalid Ed25519 private key length".to_string())?;
                let signing = ed25519_dalek::SigningKey::from_bytes(&arr);
                Ok(KeyMaterial::Ed25519Public(
                    signing.verifying_key().to_bytes().to_vec(),
                ))
            },
            KeyMaterial::X25519Private(raw32) => {
                let arr: [u8; 32] = raw32
                    .as_slice()
                    .try_into()
                    .map_err(|_| "Invalid X25519 private key length".to_string())?;
                let secret = x25519_dalek::StaticSecret::from(arr);
                let pub_key = x25519_dalek::PublicKey::from(&secret);
                Ok(KeyMaterial::X25519Public(pub_key.as_bytes().to_vec()))
            },
            KeyMaterial::Secret(_) => Err("Secret keys cannot derive a public key".into()),
        }
    }
}

#[rquickjs::methods]
impl KeyObject {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'_>) -> Result<Self> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "KeyObject"
    }

    #[qjs(get, rename = "type")]
    pub fn key_type(&self) -> &'static str {
        match &self.material {
            KeyMaterial::Secret(_) => "secret",
            KeyMaterial::RsaPrivate { .. }
            | KeyMaterial::EcPrivate { .. }
            | KeyMaterial::Ed25519Private(_)
            | KeyMaterial::X25519Private(_) => "private",
            KeyMaterial::RsaPublic { .. }
            | KeyMaterial::EcPublic { .. }
            | KeyMaterial::Ed25519Public(_)
            | KeyMaterial::X25519Public(_) => "public",
        }
    }

    #[qjs(get, rename = "symmetricKeySize")]
    pub fn symmetric_key_size<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match &self.material {
            KeyMaterial::Secret(bytes) => Ok(Value::new_number(ctx, bytes.len() as f64)),
            _ => Ok(Value::new_undefined(ctx)),
        }
    }

    #[qjs(get, rename = "asymmetricKeyType")]
    pub fn asymmetric_key_type<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let name = match &self.material {
            KeyMaterial::Secret(_) => return Ok(Value::new_undefined(ctx)),
            KeyMaterial::RsaPrivate { is_pss, .. } | KeyMaterial::RsaPublic { is_pss, .. } => {
                if *is_pss {
                    "rsa-pss"
                } else {
                    "rsa"
                }
            },
            KeyMaterial::EcPrivate { .. } | KeyMaterial::EcPublic { .. } => "ec",
            KeyMaterial::Ed25519Private(_) | KeyMaterial::Ed25519Public(_) => "ed25519",
            KeyMaterial::X25519Private(_) | KeyMaterial::X25519Public(_) => "x25519",
        };
        name.into_js(&ctx)
    }

    #[qjs(get, rename = "asymmetricKeyDetails")]
    pub fn asymmetric_key_details<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let obj = Object::new(ctx.clone())?;
        match &self.material {
            KeyMaterial::Secret(_) => return Ok(Value::new_undefined(ctx)),
            KeyMaterial::RsaPrivate { pkcs1_der, .. } => {
                let parsed = rsa::pkcs1::RsaPrivateKey::from_der(pkcs1_der).or_throw(&ctx)?;
                let mod_bits = (parsed.modulus.as_bytes().len() * 8) as u32;
                let exp_u64 = be_bytes_to_u64(parsed.public_exponent.as_bytes()).unwrap_or(65537);
                obj.set("modulusLength", mod_bits)?;
                obj.set("publicExponent", BigInt::from_u64(ctx.clone(), exp_u64)?)?;
            },
            KeyMaterial::RsaPublic { pkcs1_der, .. } => {
                let parsed = rsa::pkcs1::RsaPublicKey::from_der(pkcs1_der).or_throw(&ctx)?;
                let mod_bits = (parsed.modulus.as_bytes().len() * 8) as u32;
                let exp_u64 = be_bytes_to_u64(parsed.public_exponent.as_bytes()).unwrap_or(65537);
                obj.set("modulusLength", mod_bits)?;
                obj.set("publicExponent", BigInt::from_u64(ctx.clone(), exp_u64)?)?;
            },
            KeyMaterial::EcPrivate { curve, .. } | KeyMaterial::EcPublic { curve, .. } => {
                obj.set("namedCurve", openssl_curve_name(*curve))?;
            },
            KeyMaterial::Ed25519Private(_)
            | KeyMaterial::Ed25519Public(_)
            | KeyMaterial::X25519Private(_)
            | KeyMaterial::X25519Public(_) => {},
        }
        Ok(obj.into_value())
    }

    pub fn equals<'js>(&self, ctx: Ctx<'js>, other: Value<'js>) -> Result<bool> {
        let other_cls = Class::<KeyObject>::from_js(&ctx, other).map_err(|_| {
            Exception::throw_type(
                &ctx,
                "The \"otherKeyObject\" argument must be an instance of KeyObject",
            )
        })?;
        let other = other_cls.borrow();
        Ok(self.material == other.material)
    }

    pub fn export<'js>(&self, ctx: Ctx<'js>, options: Opt<Value<'js>>) -> Result<Value<'js>> {
        let mut format: Option<String> = None;
        let mut key_type: Option<String> = None;
        if let Some(opt_val) = options.0 {
            if !opt_val.is_undefined() && !opt_val.is_null() {
                let obj = opt_val.into_object().ok_or_else(|| {
                    Exception::throw_type(&ctx, "The \"options\" argument must be of type object")
                })?;
                format = obj.get_optional::<_, String>("format")?;
                key_type = obj.get_optional::<_, String>("type")?;
            }
        }

        match &self.material {
            KeyMaterial::Secret(bytes) => {
                if format.as_deref() == Some("jwk") {
                    let obj = Object::new(ctx.clone())?;
                    obj.set("kty", "oct")?;
                    obj.set("k", bytes_to_b64_url_safe_string(bytes))?;
                    return Ok(obj.into_value());
                }
                if let Some(fmt) = format.as_deref() {
                    if fmt != "buffer" {
                        return Err(Exception::throw_type(
                            &ctx,
                            &format!("Unsupported secret key export format: {fmt}"),
                        ));
                    }
                }
                Buffer(bytes.clone()).into_js(&ctx)
            },
            KeyMaterial::RsaPrivate { .. }
            | KeyMaterial::EcPrivate { .. }
            | KeyMaterial::Ed25519Private(_)
            | KeyMaterial::X25519Private(_) => {
                let fmt = format.as_deref().unwrap_or("pem");
                if fmt == "jwk" {
                    return self.export_asymmetric_jwk(&ctx);
                }
                let ty = key_type.as_deref().unwrap_or("pkcs8");
                let (pem_label, der_bytes) = match (ty, &self.material) {
                    ("pkcs8", _) => ("PRIVATE KEY", self.export_pkcs8_der().or_throw(&ctx)?),
                    ("pkcs1", KeyMaterial::RsaPrivate { pkcs1_der, .. }) => {
                        ("RSA PRIVATE KEY", pkcs1_der.clone())
                    },
                    ("sec1", KeyMaterial::EcPrivate { curve, pkcs8_der }) => (
                        "EC PRIVATE KEY",
                        ec_sec1_from_pkcs8(*curve, pkcs8_der).or_throw(&ctx)?,
                    ),
                    _ => {
                        return Err(Exception::throw_type(
                            &ctx,
                            &format!("Invalid private key export type '{ty}'"),
                        ));
                    },
                };
                format_exported_key(&ctx, fmt, pem_label, der_bytes)
            },
            KeyMaterial::RsaPublic { .. }
            | KeyMaterial::EcPublic { .. }
            | KeyMaterial::Ed25519Public(_)
            | KeyMaterial::X25519Public(_) => {
                let fmt = format.as_deref().unwrap_or("pem");
                if fmt == "jwk" {
                    return self.export_asymmetric_jwk(&ctx);
                }
                let ty = key_type.as_deref().unwrap_or("spki");
                let (pem_label, der_bytes) = match (ty, &self.material) {
                    ("spki", _) => ("PUBLIC KEY", self.export_spki_der().or_throw(&ctx)?),
                    ("pkcs1", KeyMaterial::RsaPublic { pkcs1_der, .. }) => {
                        ("RSA PUBLIC KEY", pkcs1_der.clone())
                    },
                    _ => {
                        return Err(Exception::throw_type(
                            &ctx,
                            &format!("Invalid public key export type '{ty}'"),
                        ));
                    },
                };
                format_exported_key(&ctx, fmt, pem_label, der_bytes)
            },
        }
    }
}

impl KeyObject {
    fn export_asymmetric_jwk<'js>(&self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        let obj = Object::new(ctx.clone())?;
        match &self.material {
            KeyMaterial::RsaPublic { pkcs1_der, .. } => {
                let k = rsa::pkcs1::RsaPublicKey::from_der(pkcs1_der).or_throw(ctx)?;
                obj.set("kty", "RSA")?;
                obj.set("n", bytes_to_b64_url_safe_string(k.modulus.as_bytes()))?;
                obj.set(
                    "e",
                    bytes_to_b64_url_safe_string(k.public_exponent.as_bytes()),
                )?;
            },
            KeyMaterial::RsaPrivate { pkcs1_der, .. } => {
                let k = rsa::pkcs1::RsaPrivateKey::from_der(pkcs1_der).or_throw(ctx)?;
                obj.set("kty", "RSA")?;
                obj.set("n", bytes_to_b64_url_safe_string(k.modulus.as_bytes()))?;
                obj.set(
                    "e",
                    bytes_to_b64_url_safe_string(k.public_exponent.as_bytes()),
                )?;
                obj.set(
                    "d",
                    bytes_to_b64_url_safe_string(k.private_exponent.as_bytes()),
                )?;
                obj.set("p", bytes_to_b64_url_safe_string(k.prime1.as_bytes()))?;
                obj.set("q", bytes_to_b64_url_safe_string(k.prime2.as_bytes()))?;
                obj.set("dp", bytes_to_b64_url_safe_string(k.exponent1.as_bytes()))?;
                obj.set("dq", bytes_to_b64_url_safe_string(k.exponent2.as_bytes()))?;
                obj.set("qi", bytes_to_b64_url_safe_string(k.coefficient.as_bytes()))?;
            },
            KeyMaterial::EcPublic { curve, sec1 } => {
                let (x, y) = ec_xy_from_sec1(*curve, sec1).or_throw(ctx)?;
                obj.set("kty", "EC")?;
                obj.set("crv", curve.as_str())?;
                obj.set("x", bytes_to_b64_url_safe_string(&x))?;
                obj.set("y", bytes_to_b64_url_safe_string(&y))?;
            },
            KeyMaterial::EcPrivate { curve, pkcs8_der } => {
                let (x, y, d) = ec_xyd_from_pkcs8(*curve, pkcs8_der).or_throw(ctx)?;
                obj.set("kty", "EC")?;
                obj.set("crv", curve.as_str())?;
                obj.set("x", bytes_to_b64_url_safe_string(&x))?;
                obj.set("y", bytes_to_b64_url_safe_string(&y))?;
                obj.set("d", bytes_to_b64_url_safe_string(&d))?;
            },
            KeyMaterial::Ed25519Public(raw32) => {
                obj.set("kty", "OKP")?;
                obj.set("crv", "Ed25519")?;
                obj.set("x", bytes_to_b64_url_safe_string(raw32))?;
            },
            KeyMaterial::Ed25519Private(seed32) => {
                let KeyMaterial::Ed25519Public(pub32) =
                    self.derive_public_material().or_throw(ctx)?
                else {
                    unreachable!();
                };
                obj.set("kty", "OKP")?;
                obj.set("crv", "Ed25519")?;
                obj.set("x", bytes_to_b64_url_safe_string(&pub32))?;
                obj.set("d", bytes_to_b64_url_safe_string(seed32))?;
            },
            KeyMaterial::X25519Public(raw32) => {
                obj.set("kty", "OKP")?;
                obj.set("crv", "X25519")?;
                obj.set("x", bytes_to_b64_url_safe_string(raw32))?;
            },
            KeyMaterial::X25519Private(raw32) => {
                let KeyMaterial::X25519Public(pub32) =
                    self.derive_public_material().or_throw(ctx)?
                else {
                    unreachable!();
                };
                obj.set("kty", "OKP")?;
                obj.set("crv", "X25519")?;
                obj.set("x", bytes_to_b64_url_safe_string(&pub32))?;
                obj.set("d", bytes_to_b64_url_safe_string(raw32))?;
            },
            KeyMaterial::Secret(_) => unreachable!(),
        }
        Ok(obj.into_value())
    }
}

pub fn create_secret_key<'js>(
    ctx: Ctx<'js>,
    key: Value<'js>,
    encoding: Opt<Value<'js>>,
) -> Result<Class<'js, KeyObject>> {
    let enc = optional_encoding(&ctx, encoding.0)?;
    let bytes = value_to_bytes_with_encoding(&ctx, &key, enc.as_deref(), "key")?;
    Class::instance(ctx, KeyObject::from_material(KeyMaterial::Secret(bytes)))
}

pub fn create_private_key<'js>(ctx: Ctx<'js>, key: Value<'js>) -> Result<Class<'js, KeyObject>> {
    let ko = parse_private_key_value(&ctx, key)?;
    Class::instance(ctx, ko)
}

pub fn create_public_key<'js>(ctx: Ctx<'js>, key: Value<'js>) -> Result<Class<'js, KeyObject>> {
    let ko = parse_public_key_value(&ctx, key)?;
    Class::instance(ctx, ko)
}

pub(crate) fn parse_private_key_value<'js>(ctx: &Ctx<'js>, key: Value<'js>) -> Result<KeyObject> {
    if let Ok(ko) = Class::<KeyObject>::from_js(ctx, key.clone()) {
        let borrowed = ko.borrow().clone();
        return match borrowed.material {
            KeyMaterial::RsaPrivate { .. }
            | KeyMaterial::EcPrivate { .. }
            | KeyMaterial::Ed25519Private(_)
            | KeyMaterial::X25519Private(_) => Ok(borrowed),
            _ => Err(Exception::throw_type(
                ctx,
                "Invalid key type: expected a private KeyObject",
            )),
        };
    }

    let spec = parse_key_input_spec(ctx, key, true)?;
    match spec {
        KeyInputSpec::Jwk(obj) => {
            let mat = parse_jwk_key(ctx, &obj, true)?;
            Ok(KeyObject::from_material(mat))
        },
        KeyInputSpec::Bytes { bytes, format, ty } => {
            let mat = match format.as_deref() {
                Some("der") => parse_private_der(&bytes, ty.as_deref()).or_throw(ctx)?,
                Some("pem") | None => {
                    parse_private_pem_or_der(&bytes, ty.as_deref()).or_throw(ctx)?
                },
                Some(other) => {
                    return Err(Exception::throw_type(
                        ctx,
                        &format!("Unsupported key format: {other}"),
                    ))
                },
            };
            Ok(KeyObject::from_material(mat))
        },
    }
}

pub(crate) fn parse_public_key_value<'js>(ctx: &Ctx<'js>, key: Value<'js>) -> Result<KeyObject> {
    if let Ok(ko) = Class::<KeyObject>::from_js(ctx, key.clone()) {
        let mat = ko.borrow().derive_public_material().or_throw(ctx)?;
        return Ok(KeyObject::from_material(mat));
    }

    let spec = parse_key_input_spec(ctx, key, false)?;
    match spec {
        KeyInputSpec::Jwk(obj) => {
            let mat = parse_jwk_key(ctx, &obj, false)?;
            let ko = KeyObject::from_material(mat);
            let derived = ko.derive_public_material().or_throw(ctx)?;
            Ok(KeyObject::from_material(derived))
        },
        KeyInputSpec::Bytes { bytes, format, ty } => {
            let mat = match format.as_deref() {
                Some("der") => parse_public_der(&bytes, ty.as_deref()).or_throw(ctx)?,
                Some("pem") | None => {
                    parse_public_pem_or_der(&bytes, ty.as_deref()).or_throw(ctx)?
                },
                Some(other) => {
                    return Err(Exception::throw_type(
                        ctx,
                        &format!("Unsupported key format: {other}"),
                    ))
                },
            };
            Ok(KeyObject::from_material(mat))
        },
    }
}

enum KeyInputSpec<'js> {
    Jwk(Object<'js>),
    Bytes {
        bytes: Vec<u8>,
        format: Option<String>,
        ty: Option<String>,
    },
}

fn parse_key_input_spec<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    _is_private: bool,
) -> Result<KeyInputSpec<'js>> {
    if let Some(s) = value.as_string() {
        return Ok(KeyInputSpec::Bytes {
            bytes: s.to_string()?.into_bytes(),
            format: Some("pem".to_string()),
            ty: None,
        });
    }
    if let Some(obj) = value.as_object() {
        if let Some(buf_bytes) = ObjectBytes::from_array_buffer(obj)? {
            return Ok(KeyInputSpec::Bytes {
                bytes: buf_bytes.as_bytes(ctx)?.to_vec(),
                format: None,
                ty: None,
            });
        }
        let format = obj.get_optional::<_, String>("format")?;
        let ty = obj.get_optional::<_, String>("type")?;
        let enc = obj.get_optional::<_, String>("encoding")?;
        let inner: Value = obj.get("key").map_err(|_| {
            Exception::throw_type(
                ctx,
                "The \"key\" property is required on key options object",
            )
        })?;
        if inner.is_undefined() || inner.is_null() {
            return Err(Exception::throw_type(
                ctx,
                "The \"key\" property is required on key options object",
            ));
        }
        if format.as_deref() == Some("jwk") {
            let jwk_obj = inner.into_object().ok_or_else(|| {
                Exception::throw_type(ctx, "The \"key\" property must be an object for jwk format")
            })?;
            return Ok(KeyInputSpec::Jwk(jwk_obj));
        }
        let bytes = value_to_bytes_with_encoding(ctx, &inner, enc.as_deref(), "options.key")?;
        return Ok(KeyInputSpec::Bytes { bytes, format, ty });
    }
    Err(Exception::throw_type(
        ctx,
        "The \"key\" argument must be of type string or an instance of Buffer, TypedArray, DataView, KeyObject, or an options object",
    ))
}

fn parse_private_pem_or_der(
    bytes: &[u8],
    ty: Option<&str>,
) -> std::result::Result<KeyMaterial, String> {
    if looks_like_pem(bytes) {
        let (label, der) =
            der::pem::decode_vec(bytes).map_err(|e| format!("Invalid PEM private key: {e}"))?;
        return match label {
            "PRIVATE KEY" => parse_pkcs8_private_der(&der),
            "RSA PRIVATE KEY" => parse_rsa_pkcs1_private_der(&der),
            "EC PRIVATE KEY" => parse_ec_sec1_private_der(&der),
            "ENCRYPTED PRIVATE KEY" => {
                Err("Encrypted PEM private keys are not supported".to_string())
            },
            other => Err(format!("Unsupported PEM private key label: {other}")),
        };
    }
    if let Some(ty) = ty {
        return parse_private_der(bytes, Some(ty));
    }
    parse_private_der(bytes, None)
}

fn parse_private_der(der: &[u8], ty: Option<&str>) -> std::result::Result<KeyMaterial, String> {
    match ty {
        Some("pkcs8") => parse_pkcs8_private_der(der),
        Some("pkcs1") => parse_rsa_pkcs1_private_der(der),
        Some("sec1") => parse_ec_sec1_private_der(der),
        Some(other) => Err(format!("Unsupported private DER key type: {other}")),
        None => parse_pkcs8_private_der(der)
            .or_else(|_| parse_rsa_pkcs1_private_der(der))
            .or_else(|_| parse_ec_sec1_private_der(der)),
    }
}

fn parse_public_pem_or_der(
    bytes: &[u8],
    ty: Option<&str>,
) -> std::result::Result<KeyMaterial, String> {
    if looks_like_pem(bytes) {
        let (label, der) =
            der::pem::decode_vec(bytes).map_err(|e| format!("Invalid PEM public key: {e}"))?;
        return match label {
            "PUBLIC KEY" => parse_public_key_from_spki_der(&der),
            "RSA PUBLIC KEY" => parse_rsa_pkcs1_public_der(&der),
            "CERTIFICATE" => {
                let cert = x509_cert::Certificate::from_der(&der)
                    .map_err(|e| format!("Invalid X.509 certificate: {e}"))?;
                let spki_der = cert
                    .tbs_certificate()
                    .subject_public_key_info()
                    .to_der()
                    .map_err(|e| format!("Invalid certificate SPKI: {e}"))?;
                parse_public_key_from_spki_der(&spki_der)
            },
            "PRIVATE KEY" | "RSA PRIVATE KEY" | "EC PRIVATE KEY" => {
                let priv_mat = match label {
                    "PRIVATE KEY" => parse_pkcs8_private_der(&der)?,
                    "RSA PRIVATE KEY" => parse_rsa_pkcs1_private_der(&der)?,
                    _ => parse_ec_sec1_private_der(&der)?,
                };
                KeyObject::from_material(priv_mat).derive_public_material()
            },
            other => Err(format!("Unsupported PEM public key label: {other}")),
        };
    }
    parse_public_der(bytes, ty)
}

fn parse_public_der(der: &[u8], ty: Option<&str>) -> std::result::Result<KeyMaterial, String> {
    match ty {
        Some("spki") => parse_public_key_from_spki_der(der),
        Some("pkcs1") => parse_rsa_pkcs1_public_der(der),
        Some(other) => Err(format!("Unsupported public DER key type: {other}")),
        None => parse_public_key_from_spki_der(der).or_else(|_| parse_rsa_pkcs1_public_der(der)),
    }
}

fn parse_rsa_pkcs1_private_der(der: &[u8]) -> std::result::Result<KeyMaterial, String> {
    RsaPrivateKey::from_pkcs1_der(der)
        .map_err(|e| format!("Invalid PKCS#1 RSA private key: {e}"))?;
    Ok(KeyMaterial::RsaPrivate {
        pkcs1_der: der.to_vec(),
        is_pss: false,
    })
}

fn parse_rsa_pkcs1_public_der(der: &[u8]) -> std::result::Result<KeyMaterial, String> {
    RsaPublicKey::from_pkcs1_der(der).map_err(|e| format!("Invalid PKCS#1 RSA public key: {e}"))?;
    Ok(KeyMaterial::RsaPublic {
        pkcs1_der: der.to_vec(),
        is_pss: false,
    })
}

fn parse_ec_sec1_private_der(der: &[u8]) -> std::result::Result<KeyMaterial, String> {
    let sec1 =
        EcPrivateKey::try_from(der).map_err(|e| format!("Invalid SEC1 EC private key: {e}"))?;
    let curve = ec_curve_from_sec1_params(sec1.parameters)
        .ok_or_else(|| "EC private key is missing supported named curve parameters".to_string())?;
    let pkcs8_der = match curve {
        EllipticCurve::P256 => p256::SecretKey::from_sec1_der(der)
            .map_err(|_| "Invalid P-256 SEC1 private key".to_string())?
            .to_pkcs8_der()
            .map_err(|_| "Failed to encode P-256 PKCS#8".to_string())?
            .as_bytes()
            .to_vec(),
        EllipticCurve::P384 => p384::SecretKey::from_sec1_der(der)
            .map_err(|_| "Invalid P-384 SEC1 private key".to_string())?
            .to_pkcs8_der()
            .map_err(|_| "Failed to encode P-384 PKCS#8".to_string())?
            .as_bytes()
            .to_vec(),
        EllipticCurve::P521 => p521::SecretKey::from_sec1_der(der)
            .map_err(|_| "Invalid P-521 SEC1 private key".to_string())?
            .to_pkcs8_der()
            .map_err(|_| "Failed to encode P-521 PKCS#8".to_string())?
            .as_bytes()
            .to_vec(),
    };
    Ok(KeyMaterial::EcPrivate { curve, pkcs8_der })
}

fn parse_pkcs8_private_der(der: &[u8]) -> std::result::Result<KeyMaterial, String> {
    let pk_info =
        PrivateKeyInfoRef::from_der(der).map_err(|e| format!("Invalid PKCS#8 private key: {e}"))?;
    let oid = pk_info.algorithm.oid;
    if oid == rfc5912::RSA_ENCRYPTION || oid == OID_RSA_PSS {
        let inner = pk_info.private_key.as_bytes();
        RsaPrivateKey::from_pkcs1_der(inner)
            .map_err(|e| format!("Invalid RSA PKCS#8 private key: {e}"))?;
        return Ok(KeyMaterial::RsaPrivate {
            pkcs1_der: inner.to_vec(),
            is_pss: oid == OID_RSA_PSS,
        });
    }
    if oid == EC_PUBLIC_KEY_OID {
        let curve = pk_info
            .algorithm
            .parameters
            .and_then(|any| ObjectIdentifier::try_from(any).ok())
            .and_then(curve_from_oid)
            .or_else(|| {
                EcPrivateKey::try_from(pk_info.private_key.as_bytes())
                    .ok()
                    .and_then(|ec| ec_curve_from_sec1_params(ec.parameters))
            })
            .ok_or_else(|| "Unsupported EC curve in PKCS#8 key".to_string())?;
        // Normalize via PKCS#8 first, or fall back to re-encoding inner SEC1 key with outer curve
        let normalized_der = if ec_public_sec1_from_pkcs8(curve, der).is_ok() {
            der.to_vec()
        } else {
            let inner = pk_info.private_key.as_bytes();
            match curve {
                EllipticCurve::P256 => p256::SecretKey::from_sec1_der(inner)
                    .map_err(|_| "Invalid P-256 PKCS#8 key".to_string())?
                    .to_pkcs8_der()
                    .map_err(|_| "Failed to encode P-256 PKCS#8".to_string())?
                    .as_bytes()
                    .to_vec(),
                EllipticCurve::P384 => p384::SecretKey::from_sec1_der(inner)
                    .map_err(|_| "Invalid P-384 PKCS#8 key".to_string())?
                    .to_pkcs8_der()
                    .map_err(|_| "Failed to encode P-384 PKCS#8".to_string())?
                    .as_bytes()
                    .to_vec(),
                EllipticCurve::P521 => p521::SecretKey::from_sec1_der(inner)
                    .map_err(|_| "Invalid P-521 PKCS#8 key".to_string())?
                    .to_pkcs8_der()
                    .map_err(|_| "Failed to encode P-521 PKCS#8".to_string())?
                    .as_bytes()
                    .to_vec(),
            }
        };
        return Ok(KeyMaterial::EcPrivate {
            curve,
            pkcs8_der: normalized_der,
        });
    }
    if oid == rfc8410::ID_ED_25519 {
        let seed32 = unwrap_okp_private_key(pk_info.private_key.as_bytes())?;
        return Ok(KeyMaterial::Ed25519Private(seed32));
    }
    if oid == rfc8410::ID_X_25519 {
        let raw32 = unwrap_okp_private_key(pk_info.private_key.as_bytes())?;
        return Ok(KeyMaterial::X25519Private(raw32));
    }
    Err(format!("Unsupported PKCS#8 key algorithm OID: {oid}"))
}

pub(crate) fn parse_public_key_from_spki_der(
    der: &[u8],
) -> std::result::Result<KeyMaterial, String> {
    let spki =
        SubjectPublicKeyInfoRef::try_from(der).map_err(|e| format!("Invalid SPKI key: {e}"))?;
    let oid = spki.algorithm.oid;
    let key_bytes = spki.subject_public_key.raw_bytes();
    if oid == rfc5912::RSA_ENCRYPTION || oid == OID_RSA_PSS {
        RsaPublicKey::from_pkcs1_der(key_bytes)
            .map_err(|e| format!("Invalid RSA SPKI key: {e}"))?;
        return Ok(KeyMaterial::RsaPublic {
            pkcs1_der: key_bytes.to_vec(),
            is_pss: oid == OID_RSA_PSS,
        });
    }
    if oid == EC_PUBLIC_KEY_OID {
        let curve = spki
            .algorithm
            .parameters
            .and_then(|any| ObjectIdentifier::try_from(any).ok())
            .and_then(curve_from_oid)
            .ok_or_else(|| "Unsupported EC curve in SPKI key".to_string())?;
        // Normalize compressed/hybrid/uncompressed SEC1 point to canonical uncompressed SEC1
        let sec1 = normalize_ec_point(curve, key_bytes)?;
        return Ok(KeyMaterial::EcPublic { curve, sec1 });
    }
    if oid == rfc8410::ID_ED_25519 {
        let arr: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| "Invalid Ed25519 public key length".to_string())?;
        ed25519_dalek::VerifyingKey::from_bytes(&arr)
            .map_err(|e| format!("Invalid Ed25519 public key: {e}"))?;
        return Ok(KeyMaterial::Ed25519Public(arr.to_vec()));
    }
    if oid == rfc8410::ID_X_25519 {
        if key_bytes.len() != 32 {
            return Err("Invalid X25519 public key length".to_string());
        }
        return Ok(KeyMaterial::X25519Public(key_bytes.to_vec()));
    }
    Err(format!("Unsupported SPKI key algorithm OID: {oid}"))
}

fn parse_jwk_key<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    require_private: bool,
) -> Result<KeyMaterial> {
    let kty: String = obj.get_required("kty", "key")?;
    match kty.as_str() {
        "RSA" => {
            let n = jwk_b64url_field(ctx, obj, "n")?;
            let e = jwk_b64url_field(ctx, obj, "e")?;
            if let Some(d_str) = obj.get_optional::<_, String>("d")? {
                let d = decode_b64url_str(ctx, &d_str)?;
                let p = jwk_b64url_field(ctx, obj, "p")?;
                let q = jwk_b64url_field(ctx, obj, "q")?;
                let dp = jwk_b64url_field(ctx, obj, "dp")?;
                let dq = jwk_b64url_field(ctx, obj, "dq")?;
                let qi = jwk_b64url_field(ctx, obj, "qi")?;
                let pkcs1 = rsa::pkcs1::RsaPrivateKey {
                    modulus: UintRef::new(&n).or_throw(ctx)?,
                    public_exponent: UintRef::new(&e).or_throw(ctx)?,
                    private_exponent: UintRef::new(&d).or_throw(ctx)?,
                    prime1: UintRef::new(&p).or_throw(ctx)?,
                    prime2: UintRef::new(&q).or_throw(ctx)?,
                    exponent1: UintRef::new(&dp).or_throw(ctx)?,
                    exponent2: UintRef::new(&dq).or_throw(ctx)?,
                    coefficient: UintRef::new(&qi).or_throw(ctx)?,
                    other_prime_infos: None,
                };
                let der = pkcs1.to_der().or_throw(ctx)?;
                RsaPrivateKey::from_pkcs1_der(&der).or_throw(ctx)?;
                Ok(KeyMaterial::RsaPrivate {
                    pkcs1_der: der,
                    is_pss: false,
                })
            } else {
                if require_private {
                    return Err(Exception::throw_type(
                        ctx,
                        "RSA JWK is missing private exponent \"d\"",
                    ));
                }
                let pkcs1 = rsa::pkcs1::RsaPublicKey {
                    modulus: UintRef::new(&n).or_throw(ctx)?,
                    public_exponent: UintRef::new(&e).or_throw(ctx)?,
                };
                let der = pkcs1.to_der().or_throw(ctx)?;
                RsaPublicKey::from_pkcs1_der(&der).or_throw(ctx)?;
                Ok(KeyMaterial::RsaPublic {
                    pkcs1_der: der,
                    is_pss: false,
                })
            }
        },
        "EC" => {
            let crv: String = obj.get_required("crv", "key")?;
            let curve = parse_ec_curve_name(&crv).ok_or_else(|| {
                Exception::throw_type(ctx, &format!("Unsupported EC JWK curve: {crv}"))
            })?;
            let coord_len = ec_coord_len(curve);
            if let Some(d_str) = obj.get_optional::<_, String>("d")? {
                let d = pad_left_bytes(decode_b64url_str(ctx, &d_str)?, coord_len);
                let pkcs8_der = match curve {
                    EllipticCurve::P256 => p256::SecretKey::from_slice(&d)
                        .map_err(|_| Exception::throw_type(ctx, "Invalid P-256 JWK private key"))?
                        .to_pkcs8_der()
                        .or_throw(ctx)?
                        .as_bytes()
                        .to_vec(),
                    EllipticCurve::P384 => p384::SecretKey::from_slice(&d)
                        .map_err(|_| Exception::throw_type(ctx, "Invalid P-384 JWK private key"))?
                        .to_pkcs8_der()
                        .or_throw(ctx)?
                        .as_bytes()
                        .to_vec(),
                    EllipticCurve::P521 => p521::SecretKey::from_slice(&d)
                        .map_err(|_| Exception::throw_type(ctx, "Invalid P-521 JWK private key"))?
                        .to_pkcs8_der()
                        .or_throw(ctx)?
                        .as_bytes()
                        .to_vec(),
                };
                Ok(KeyMaterial::EcPrivate { curve, pkcs8_der })
            } else {
                if require_private {
                    return Err(Exception::throw_type(
                        ctx,
                        "EC JWK is missing private coordinate \"d\"",
                    ));
                }
                let x = pad_left_bytes(jwk_b64url_field(ctx, obj, "x")?, coord_len);
                let y = pad_left_bytes(jwk_b64url_field(ctx, obj, "y")?, coord_len);
                let mut sec1 = Vec::with_capacity(1 + coord_len * 2);
                sec1.push(0x04);
                sec1.extend_from_slice(&x);
                sec1.extend_from_slice(&y);
                let sec1 = normalize_ec_point(curve, &sec1).or_throw(ctx)?;
                Ok(KeyMaterial::EcPublic { curve, sec1 })
            }
        },
        "OKP" => {
            let crv: String = obj.get_required("crv", "key")?;
            match (crv.as_str(), obj.get_optional::<_, String>("d")?) {
                ("Ed25519", Some(d_str)) => {
                    let d = decode_b64url_str(ctx, &d_str)?;
                    if d.len() != 32 {
                        return Err(Exception::throw_type(ctx, "Invalid Ed25519 JWK key length"));
                    }
                    Ok(KeyMaterial::Ed25519Private(d))
                },
                ("Ed25519", None) => {
                    if require_private {
                        return Err(Exception::throw_type(ctx, "Ed25519 JWK missing \"d\""));
                    }
                    let x = jwk_b64url_field(ctx, obj, "x")?;
                    let arr: [u8; 32] = x
                        .try_into()
                        .map_err(|_| Exception::throw_type(ctx, "Invalid Ed25519 JWK \"x\""))?;
                    ed25519_dalek::VerifyingKey::from_bytes(&arr)
                        .map_err(|_| Exception::throw_type(ctx, "Invalid Ed25519 public key"))?;
                    Ok(KeyMaterial::Ed25519Public(arr.to_vec()))
                },
                ("X25519", Some(d_str)) => {
                    let d = decode_b64url_str(ctx, &d_str)?;
                    if d.len() != 32 {
                        return Err(Exception::throw_type(ctx, "Invalid X25519 JWK key length"));
                    }
                    Ok(KeyMaterial::X25519Private(d))
                },
                ("X25519", None) => {
                    if require_private {
                        return Err(Exception::throw_type(ctx, "X25519 JWK missing \"d\""));
                    }
                    let x = jwk_b64url_field(ctx, obj, "x")?;
                    if x.len() != 32 {
                        return Err(Exception::throw_type(ctx, "Invalid X25519 JWK \"x\""));
                    }
                    Ok(KeyMaterial::X25519Public(x))
                },
                _ => Err(Exception::throw_type(
                    ctx,
                    &format!("Unsupported OKP JWK curve: {crv}"),
                )),
            }
        },
        other => Err(Exception::throw_type(
            ctx,
            &format!("Unsupported JWK key type: {other}"),
        )),
    }
}

pub(crate) fn optional_encoding<'js>(
    ctx: &Ctx<'js>,
    val: Option<Value<'js>>,
) -> Result<Option<String>> {
    let Some(v) = val else {
        return Ok(None);
    };
    if v.is_undefined() || v.is_null() {
        return Ok(None);
    }
    if let Some(s) = v.as_string() {
        return Ok(Some(s.to_string()?));
    }
    Err(Exception::throw_type(ctx, "encoding must be a string"))
}

pub(crate) fn value_to_bytes_with_encoding<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    encoding: Option<&str>,
    arg_name: &str,
) -> Result<Vec<u8>> {
    if let Some(s) = value.as_string() {
        let text = s.to_string()?;
        return decode_string_with_encoding(ctx, &text, encoding.unwrap_or("utf8"));
    }
    if let Some(obj) = value.as_object() {
        if let Some(bytes) = ObjectBytes::from_array_buffer(obj)? {
            return Ok(bytes.as_bytes(ctx)?.to_vec());
        }
    }
    Err(Exception::throw_type(
        ctx,
        &format!(
            "The \"{arg_name}\" argument must be of type string or an instance of Buffer, TypedArray, or DataView."
        ),
    ))
}

pub(crate) fn decode_string_with_encoding(
    ctx: &Ctx<'_>,
    text: &str,
    encoding: &str,
) -> Result<Vec<u8>> {
    let normalized = encoding.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "utf8" | "utf-8" => Ok(text.as_bytes().to_vec()),
        "ascii" => Ok(text.encode_utf16().map(|u| (u & 0x7f) as u8).collect()),
        "latin1" | "binary" => Ok(text.encode_utf16().map(|u| (u & 0xff) as u8).collect()),
        "hex" => bytes_from_hex(text.as_bytes()).or_throw(ctx),
        "base64" => bytes_from_b64(text.as_bytes()).or_throw(ctx),
        "base64url" => {
            let trimmed = text.trim_end_matches('=');
            bytes_from_b64_url_safe(trimmed.as_bytes()).or_throw(ctx)
        },
        "ucs2" | "ucs-2" | "utf16le" | "utf-16le" => {
            Ok(text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect())
        },
        _ => Encoder::from_str(encoding)
            .and_then(|enc| enc.decode_from_string(text.to_string()))
            .or_throw(ctx),
    }
}

pub(crate) fn encode_bytes_to_output<'js>(
    ctx: &Ctx<'js>,
    bytes: Vec<u8>,
    encoding: Option<&str>,
) -> Result<Value<'js>> {
    let Some(enc) = encoding else {
        return Buffer(bytes).into_js(ctx);
    };
    let normalized = enc.trim().to_ascii_lowercase();
    if normalized == "buffer" {
        return Buffer(bytes).into_js(ctx);
    }
    let out = match normalized.as_str() {
        "hex" => bytes_to_hex_string(&bytes),
        "base64" => bytes_to_b64_string(&bytes),
        "base64url" => bytes_to_b64_url_safe_string(&bytes),
        "latin1" | "binary" => bytes.iter().map(|&b| b as char).collect(),
        "utf8" | "utf-8" => String::from_utf8_lossy(&bytes).into_owned(),
        _ => Encoder::from_str(enc)
            .and_then(|e| e.encode_to_string(&bytes, true))
            .or_throw(ctx)?,
    };
    out.into_js(ctx)
}

pub(crate) fn encode_pem(label: &str, der: &[u8]) -> std::result::Result<String, String> {
    der::pem::encode_string(label, der::pem::LineEnding::LF, der)
        .map_err(|e| format!("Failed to encode PEM: {e}"))
}

pub(crate) fn parse_ec_curve_name(name: &str) -> Option<EllipticCurve> {
    match name {
        "P-256" | "prime256v1" | "secp256r1" => Some(EllipticCurve::P256),
        "P-384" | "secp384r1" => Some(EllipticCurve::P384),
        "P-521" | "secp521r1" => Some(EllipticCurve::P521),
        _ => None,
    }
}

pub(crate) fn openssl_curve_name(curve: EllipticCurve) -> &'static str {
    match curve {
        EllipticCurve::P256 => "prime256v1",
        EllipticCurve::P384 => "secp384r1",
        EllipticCurve::P521 => "secp521r1",
    }
}

pub(crate) fn ec_coord_len(curve: EllipticCurve) -> usize {
    match curve {
        EllipticCurve::P256 => 32,
        EllipticCurve::P384 => 48,
        EllipticCurve::P521 => 66,
    }
}

pub(crate) fn normalize_ec_point(
    curve: EllipticCurve,
    sec1: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    use elliptic_curve::sec1::ToSec1Point;
    let mut normalized;
    let bytes = if matches!(sec1.first(), Some(0x06 | 0x07)) {
        // Hybrid point format (`0x06`/`0x07 || x || y`) -> validate parity then decode as `0x04`
        let expected_odd = sec1[0] == 0x07;
        let expected_len = 1 + 2 * ec_coord_len(curve);
        if sec1.len() != expected_len {
            return Err("Invalid hybrid EC public key length".into());
        }
        let actual_odd = (sec1[sec1.len() - 1] & 1) != 0;
        if actual_odd != expected_odd {
            return Err("Invalid hybrid EC public key parity bit".into());
        }
        normalized = sec1.to_vec();
        normalized[0] = 0x04;
        &normalized
    } else {
        sec1
    };

    let out = match curve {
        EllipticCurve::P256 => p256::PublicKey::from_sec1_bytes(bytes)
            .map_err(|_| "Invalid P-256 public key".to_string())?
            .to_sec1_point(false)
            .as_bytes()
            .to_vec(),
        EllipticCurve::P384 => p384::PublicKey::from_sec1_bytes(bytes)
            .map_err(|_| "Invalid P-384 public key".to_string())?
            .to_sec1_point(false)
            .as_bytes()
            .to_vec(),
        EllipticCurve::P521 => p521::PublicKey::from_sec1_bytes(bytes)
            .map_err(|_| "Invalid P-521 public key".to_string())?
            .to_sec1_point(false)
            .as_bytes()
            .to_vec(),
    };
    Ok(out)
}

fn ec_curve_from_sec1_params(params: Option<sec1::EcParameters>) -> Option<EllipticCurve> {
    match params? {
        sec1::EcParameters::NamedCurve(oid) => curve_from_oid(oid),
    }
}

fn curve_from_oid(oid: ObjectIdentifier) -> Option<EllipticCurve> {
    if oid == OID_PRIME256V1 {
        Some(EllipticCurve::P256)
    } else if oid == OID_SECP384R1 {
        Some(EllipticCurve::P384)
    } else if oid == OID_SECP521R1 {
        Some(EllipticCurve::P521)
    } else {
        None
    }
}

fn curve_oid(curve: EllipticCurve) -> ObjectIdentifier {
    match curve {
        EllipticCurve::P256 => OID_PRIME256V1,
        EllipticCurve::P384 => OID_SECP384R1,
        EllipticCurve::P521 => OID_SECP521R1,
    }
}

fn ec_public_sec1_from_pkcs8(
    curve: EllipticCurve,
    pkcs8_der: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    use elliptic_curve::sec1::ToSec1Point;
    match curve {
        EllipticCurve::P256 => Ok(p256::SecretKey::from_pkcs8_der(pkcs8_der)
            .map_err(|_| "Invalid P-256 private key".to_string())?
            .public_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec()),
        EllipticCurve::P384 => Ok(p384::SecretKey::from_pkcs8_der(pkcs8_der)
            .map_err(|_| "Invalid P-384 private key".to_string())?
            .public_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec()),
        EllipticCurve::P521 => Ok(p521::SecretKey::from_pkcs8_der(pkcs8_der)
            .map_err(|_| "Invalid P-521 private key".to_string())?
            .public_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec()),
    }
}

fn ec_sec1_from_pkcs8(
    curve: EllipticCurve,
    pkcs8_der: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    let doc = match curve {
        EllipticCurve::P256 => p256::SecretKey::from_pkcs8_der(pkcs8_der)
            .map_err(|_| "Invalid P-256 private key".to_string())?
            .to_sec1_der(),
        EllipticCurve::P384 => p384::SecretKey::from_pkcs8_der(pkcs8_der)
            .map_err(|_| "Invalid P-384 private key".to_string())?
            .to_sec1_der(),
        EllipticCurve::P521 => p521::SecretKey::from_pkcs8_der(pkcs8_der)
            .map_err(|_| "Invalid P-521 private key".to_string())?
            .to_sec1_der(),
    }
    .map_err(|_| "Failed to encode EC SEC1 private key".to_string())?;
    Ok(doc.to_vec())
}

fn ec_xy_from_sec1(
    curve: EllipticCurve,
    sec1: &[u8],
) -> std::result::Result<(Vec<u8>, Vec<u8>), String> {
    let coord_len = ec_coord_len(curve);
    if sec1.len() != 1 + coord_len * 2 || sec1[0] != 0x04 {
        return Err("Invalid uncompressed EC point".to_string());
    }
    Ok((
        sec1[1..1 + coord_len].to_vec(),
        sec1[1 + coord_len..].to_vec(),
    ))
}

fn ec_xyd_from_pkcs8(
    curve: EllipticCurve,
    pkcs8_der: &[u8],
) -> std::result::Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    use elliptic_curve::sec1::ToSec1Point;
    match curve {
        EllipticCurve::P256 => {
            let sk = p256::SecretKey::from_pkcs8_der(pkcs8_der)
                .map_err(|_| "Invalid P-256 private key".to_string())?;
            let pt = sk.public_key().to_sec1_point(false);
            Ok((
                pt.x().unwrap().to_vec(),
                pt.y().unwrap().to_vec(),
                sk.to_bytes().to_vec(),
            ))
        },
        EllipticCurve::P384 => {
            let sk = p384::SecretKey::from_pkcs8_der(pkcs8_der)
                .map_err(|_| "Invalid P-384 private key".to_string())?;
            let pt = sk.public_key().to_sec1_point(false);
            Ok((
                pt.x().unwrap().to_vec(),
                pt.y().unwrap().to_vec(),
                sk.to_bytes().to_vec(),
            ))
        },
        EllipticCurve::P521 => {
            let sk = p521::SecretKey::from_pkcs8_der(pkcs8_der)
                .map_err(|_| "Invalid P-521 private key".to_string())?;
            let pt = sk.public_key().to_sec1_point(false);
            Ok((
                pt.x().unwrap().to_vec(),
                pt.y().unwrap().to_vec(),
                sk.to_bytes().to_vec(),
            ))
        },
    }
}

fn unwrap_okp_private_key(raw: &[u8]) -> std::result::Result<Vec<u8>, String> {
    if raw.len() == 32 {
        return Ok(raw.to_vec());
    }
    let octet = <&OctetStringRef>::from_der(raw)
        .map_err(|_| "Invalid OKP private key OCTET STRING".to_string())?;
    if octet.as_bytes().len() != 32 {
        return Err("OKP private key must be 32 bytes".to_string());
    }
    Ok(octet.as_bytes().to_vec())
}

fn encode_okp_pkcs8(oid: ObjectIdentifier, raw32: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let inner = OctetStringRef::new(raw32)
        .and_then(|o| o.to_der())
        .map_err(|e| format!("Failed to encode OKP key: {e}"))?;
    let info = PrivateKeyInfoRef {
        algorithm: AlgorithmIdentifierRef {
            oid,
            parameters: None,
        },
        private_key: OctetStringRef::new(&inner).map_err(|e| format!("{e}"))?,
        public_key: None,
    };
    info.to_der()
        .map_err(|e| format!("Failed to encode PKCS#8: {e}"))
}

fn encode_okp_spki(oid: ObjectIdentifier, raw32: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let spki = SubjectPublicKeyInfoRef {
        algorithm: AlgorithmIdentifierRef {
            oid,
            parameters: None,
        },
        subject_public_key: BitStringRef::from_bytes(raw32)
            .map_err(|e| format!("Invalid OKP public key: {e}"))?,
    };
    spki.to_der()
        .map_err(|e| format!("Failed to encode SPKI: {e}"))
}

fn format_exported_key<'js>(
    ctx: &Ctx<'js>,
    fmt: &str,
    pem_label: &str,
    der_bytes: Vec<u8>,
) -> Result<Value<'js>> {
    match fmt {
        "der" => Buffer(der_bytes).into_js(ctx),
        "pem" => encode_pem(pem_label, &der_bytes)
            .or_throw(ctx)?
            .into_js(ctx),
        other => Err(Exception::throw_type(
            ctx,
            &format!("Unsupported key export format: {other}"),
        )),
    }
}

fn looks_like_pem(bytes: &[u8]) -> bool {
    bytes.windows(11).any(|w| w == b"-----BEGIN ")
}

fn jwk_b64url_field<'js>(ctx: &Ctx<'js>, obj: &Object<'js>, field: &str) -> Result<Vec<u8>> {
    let s: String = obj.get_required(field, "key")?;
    decode_b64url_str(ctx, &s)
}

fn decode_b64url_str(ctx: &Ctx<'_>, s: &str) -> Result<Vec<u8>> {
    bytes_from_b64_url_safe(s.trim_end_matches('=').as_bytes()).or_throw(ctx)
}

fn pad_left_bytes(bytes: Vec<u8>, target_len: usize) -> Vec<u8> {
    if bytes.len() >= target_len {
        return bytes;
    }
    let mut padded = vec![0u8; target_len - bytes.len()];
    padded.extend_from_slice(&bytes);
    padded
}

fn be_bytes_to_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.len() > 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf[8 - bytes.len()..].copy_from_slice(bytes);
    Some(u64::from_be_bytes(buf))
}
