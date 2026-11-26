// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `X509Certificate`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use const_oid::db::{rfc5912, rfc8410};
use der::{Decode, Encode};
use lexe_buffer::Buffer;
use lexe_utils::result::ResultExt;
use rquickjs::{
    atom::PredefinedAtom,
    class::Trace,
    function::{Constructor, Opt},
    Class, Ctx, Exception, JsLifetime, Object, Result, Value,
};
use x509_cert::{
    ext::pkix::{name::GeneralName, BasicConstraints, SubjectAltName},
    name::Name,
    time::Time,
    Certificate,
};

use crate::{
    classic::{verify_material_bytes, DsaEncoding},
    hash::HashAlgorithm,
    key_object::{
        encode_pem, parse_public_key_from_spki_der, parse_public_key_value,
        value_to_bytes_with_encoding, KeyObject,
    },
    provider::{CryptoProvider, SimpleDigest},
    CRYPTO_PROVIDER,
};

#[rquickjs::class]
#[derive(Trace, JsLifetime)]
pub struct X509Certificate {
    #[qjs(skip_trace)]
    cert: Certificate,
    raw_der: Vec<u8>,
}

fn format_colon_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(3));
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02X}");
    }
    out
}

fn format_x509_name(name: &Name) -> String {
    let mut parts = Vec::new();
    for rdn in name.iter_rdn() {
        let mut rdn_parts = Vec::new();
        for atv in rdn.iter() {
            rdn_parts.push(atv.to_string());
        }
        parts.push(rdn_parts.join("+"));
    }
    parts.join("\n")
}

fn month_abbr(m: u8) -> &'static str {
    match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "Jan",
    }
}

fn format_openssl_time(time: &Time) -> String {
    let dt = time.to_date_time();
    format!(
        "{} {:2} {:02}:{:02}:{:02} {:04} GMT",
        month_abbr(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minutes(),
        dt.seconds(),
        dt.year()
    )
}

fn time_to_js_date<'js>(ctx: &Ctx<'js>, time: &Time) -> Result<Value<'js>> {
    let ms = time.to_unix_duration().as_millis() as f64;
    let date_ctor: Constructor = ctx.globals().get("Date")?;
    date_ctor.construct((ms,))
}

fn format_san_entries(cert: &Certificate) -> Option<Vec<(String, String)>> {
    let Ok(Some((_, san))) = cert.tbs_certificate().get_extension::<SubjectAltName>() else {
        return None;
    };
    let mut entries = Vec::new();
    for name in &san.0 {
        match name {
            GeneralName::DnsName(s) => entries.push(("DNS".to_string(), s.as_str().to_string())),
            GeneralName::Rfc822Name(s) => {
                entries.push(("email".to_string(), s.as_str().to_string()))
            },
            GeneralName::UniformResourceIdentifier(s) => {
                entries.push(("URI".to_string(), s.as_str().to_string()))
            },
            GeneralName::IpAddress(oct) => {
                let b = oct.as_bytes();
                let ip = if b.len() == 4 {
                    IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3])).to_string()
                } else if b.len() == 16 {
                    let arr: [u8; 16] = b.try_into().unwrap();
                    IpAddr::V6(Ipv6Addr::from(arr)).to_string()
                } else {
                    format_colon_hex(b)
                };
                entries.push(("IP Address".to_string(), ip));
            },
            GeneralName::DirectoryName(dn) => {
                entries.push(("DirName".to_string(), dn.to_string()));
            },
            GeneralName::RegisteredId(oid) => {
                entries.push(("Registered ID".to_string(), oid.to_string()));
            },
            _ => {},
        }
    }
    Some(entries)
}

fn match_hostname(pattern: &str, host: &str, allow_wildcards: bool) -> bool {
    let p = pattern.to_ascii_lowercase();
    let h = host.to_ascii_lowercase();
    if p == h {
        return true;
    }
    if !allow_wildcards || !p.starts_with("*.") {
        return false;
    }
    let suffix = &p[1..]; // ".example.com"
    if !h.ends_with(suffix) {
        return false;
    }
    let prefix = &h[..h.len() - suffix.len()];
    !prefix.is_empty() && !prefix.contains('.')
}

#[rquickjs::methods]
impl X509Certificate {
    #[qjs(constructor)]
    pub fn new<'js>(ctx: Ctx<'js>, buffer: Value<'js>) -> Result<Self> {
        let bytes = value_to_bytes_with_encoding(&ctx, &buffer, Some("utf8"), "buffer")?;
        let cert = if bytes
            .windows(27)
            .any(|w| w == b"-----BEGIN CERTIFICATE-----")
        {
            let (label, der) = der::pem::decode_vec(&bytes)
                .map_err(|e| Exception::throw_message(&ctx, &format!("Invalid X.509 PEM: {e}")))?;
            if label != "CERTIFICATE" {
                return Err(Exception::throw_message(
                    &ctx,
                    &format!("Expected CERTIFICATE PEM, got {label}"),
                ));
            }
            Certificate::from_der(&der).map_err(|e| {
                Exception::throw_message(&ctx, &format!("Invalid X.509 certificate: {e}"))
            })?
        } else {
            Certificate::from_der(&bytes)
                .map_err(|e| Exception::throw_message(&ctx, &format!("Invalid X.509 DER: {e}")))?
        };
        let raw_der = cert.to_der().map_err(|e| {
            Exception::throw_message(&ctx, &format!("Failed to encode certificate DER: {e}"))
        })?;
        Ok(Self { cert, raw_der })
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "X509Certificate"
    }

    #[qjs(get)]
    pub fn raw(&self) -> Buffer {
        Buffer(self.raw_der.clone())
    }

    #[qjs(get)]
    pub fn subject(&self) -> String {
        format_x509_name(self.cert.tbs_certificate().subject())
    }

    #[qjs(get)]
    pub fn issuer(&self) -> String {
        format_x509_name(self.cert.tbs_certificate().issuer())
    }

    #[qjs(get, rename = "serialNumber")]
    pub fn serial_number(&self) -> String {
        let bytes = self.cert.tbs_certificate().serial_number().as_bytes();
        let trimmed = match bytes.iter().position(|&b| b != 0) {
            Some(idx) => &bytes[idx..],
            None => &[0u8],
        };
        let mut out = String::with_capacity(trimmed.len() * 2);
        for b in trimmed {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{b:02X}");
        }
        out
    }

    #[qjs(get, rename = "validFrom")]
    pub fn valid_from(&self) -> String {
        format_openssl_time(&self.cert.tbs_certificate().validity().not_before)
    }

    #[qjs(get, rename = "validTo")]
    pub fn valid_to(&self) -> String {
        format_openssl_time(&self.cert.tbs_certificate().validity().not_after)
    }

    #[qjs(get, rename = "validFromDate")]
    pub fn valid_from_date<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        time_to_js_date(&ctx, &self.cert.tbs_certificate().validity().not_before)
    }

    #[qjs(get, rename = "validToDate")]
    pub fn valid_to_date<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        time_to_js_date(&ctx, &self.cert.tbs_certificate().validity().not_after)
    }

    #[qjs(get)]
    pub fn fingerprint(&self) -> String {
        let mut h = CRYPTO_PROVIDER.digest(HashAlgorithm::Sha1);
        h.update(&self.raw_der);
        format_colon_hex(&h.finalize())
    }

    #[qjs(get)]
    pub fn fingerprint256(&self) -> String {
        let mut h = CRYPTO_PROVIDER.digest(HashAlgorithm::Sha256);
        h.update(&self.raw_der);
        format_colon_hex(&h.finalize())
    }

    #[qjs(get)]
    pub fn fingerprint512(&self) -> String {
        let mut h = CRYPTO_PROVIDER.digest(HashAlgorithm::Sha512);
        h.update(&self.raw_der);
        format_colon_hex(&h.finalize())
    }

    #[qjs(get, rename = "publicKey")]
    pub fn public_key<'js>(&self, ctx: Ctx<'js>) -> Result<Class<'js, KeyObject>> {
        let spki_der = self
            .cert
            .tbs_certificate()
            .subject_public_key_info()
            .to_der()
            .map_err(|e| {
                Exception::throw_message(&ctx, &format!("Invalid SPKI in certificate: {e}"))
            })?;
        let mat = parse_public_key_from_spki_der(&spki_der).or_throw(&ctx)?;
        Class::instance(ctx, KeyObject::from_material(mat))
    }

    #[qjs(get)]
    pub fn ca(&self) -> bool {
        matches!(
            self.cert
                .tbs_certificate()
                .get_extension::<BasicConstraints>(),
            Ok(Some((_, bc))) if bc.ca
        )
    }

    #[qjs(get, rename = "subjectAltName")]
    pub fn subject_alt_name<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match format_san_entries(&self.cert) {
            Some(entries) if !entries.is_empty() => {
                let s = entries
                    .into_iter()
                    .map(|(k, v)| format!("{k}:{v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                Ok(rquickjs::String::from_str(ctx, &s)?.into_value())
            },
            _ => Ok(Value::new_undefined(ctx)),
        }
    }

    #[qjs(rename = "checkHost")]
    pub fn check_host<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        options: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let mut allow_wildcards = true;
        let mut use_subject_cn = true;
        if let Some(opt_val) = options.0 {
            if let Some(obj) = opt_val.as_object() {
                if let Ok(Some(w)) = obj.get::<_, Option<bool>>("wildcards") {
                    allow_wildcards = w;
                }
                if let Ok(Some(s)) = obj.get::<_, Option<String>>("subject") {
                    use_subject_cn = s != "never";
                }
            }
        }

        if let Some(entries) = format_san_entries(&self.cert) {
            let mut has_dns_san = false;
            for (kind, val) in &entries {
                if kind == "DNS" {
                    has_dns_san = true;
                    if match_hostname(val, &name, allow_wildcards) {
                        return Ok(rquickjs::String::from_str(ctx, val)?.into_value());
                    }
                }
            }
            if has_dns_san {
                return Ok(Value::new_undefined(ctx));
            }
        }

        if use_subject_cn {
            if let Ok(Some(cn)) = self.cert.tbs_certificate().subject().common_name() {
                let cn_str = cn.value();
                if match_hostname(&cn_str, &name, allow_wildcards) {
                    return Ok(rquickjs::String::from_str(ctx, &cn_str)?.into_value());
                }
            }
        }
        Ok(Value::new_undefined(ctx))
    }

    pub fn verify<'js>(&self, ctx: Ctx<'js>, public_key: Value<'js>) -> Result<bool> {
        let ko = parse_public_key_value(&ctx, public_key)?;
        let tbs_der = self.cert.tbs_certificate().to_der().map_err(|e| {
            Exception::throw_message(&ctx, &format!("Failed to encode TBSCertificate: {e}"))
        })?;
        let sig_bytes = self.cert.signature().raw_bytes();
        let sig_oid = self.cert.signature_algorithm().oid;

        let hash_opt = if sig_oid == rfc5912::SHA_256_WITH_RSA_ENCRYPTION
            || sig_oid == rfc5912::ECDSA_WITH_SHA_256
        {
            Some(HashAlgorithm::Sha256)
        } else if sig_oid == rfc5912::SHA_384_WITH_RSA_ENCRYPTION
            || sig_oid == rfc5912::ECDSA_WITH_SHA_384
        {
            Some(HashAlgorithm::Sha384)
        } else if sig_oid == rfc5912::SHA_512_WITH_RSA_ENCRYPTION
            || sig_oid == rfc5912::ECDSA_WITH_SHA_512
        {
            Some(HashAlgorithm::Sha512)
        } else if sig_oid == rfc5912::SHA_1_WITH_RSA_ENCRYPTION {
            Some(HashAlgorithm::Sha1)
        } else if sig_oid == rfc8410::ID_ED_25519 {
            None
        } else {
            return Err(Exception::throw_message(
                &ctx,
                &format!("Unsupported certificate signature algorithm OID: {sig_oid}"),
            ));
        };

        verify_material_bytes(
            &ko.material,
            hash_opt,
            &tbs_der,
            sig_bytes,
            None,
            None,
            DsaEncoding::Der,
        )
        .or_throw(&ctx)
    }

    #[qjs(rename = "toString")]
    pub fn to_pem_string(&self, ctx: Ctx<'_>) -> Result<String> {
        encode_pem("CERTIFICATE", &self.raw_der).or_throw(&ctx)
    }

    #[qjs(rename = "toJSON")]
    pub fn to_json(&self, ctx: Ctx<'_>) -> Result<String> {
        self.to_pem_string(ctx)
    }
}

#[allow(dead_code)]
fn _check_obj(_: Object<'_>) {}
