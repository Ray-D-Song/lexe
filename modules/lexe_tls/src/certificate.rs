// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::collections::BTreeMap;

use rquickjs::{Ctx, IntoJs, Object, Result};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_parser::prelude::*;

use crate::identity::CnValue;

type CertResult<T> = std::result::Result<T, String>;

/// Node-style certificate object (Rust representation).
#[derive(Debug, Clone)]
pub struct CertObject {
    pub subject: BTreeMap<String, CnValue>,
    pub issuer: BTreeMap<String, CnValue>,
    pub subjectaltname: Option<String>,
    pub valid_from: String,
    pub valid_to: String,
    pub serial_number: String,
    pub fingerprint: String,
    pub fingerprint256: String,
    pub raw: Vec<u8>,
    pub issuer_certificate: Option<Box<CertObject>>,
}

impl CertObject {
    pub fn to_js_object<'js>(&self, ctx: &Ctx<'js>) -> Result<Object<'js>> {
        cert_to_js(ctx, self, false)
    }

    pub fn to_js_object_detailed<'js>(&self, ctx: &Ctx<'js>) -> Result<Object<'js>> {
        cert_to_js(ctx, self, true)
    }
}

pub fn parse_cert_der(der: &[u8]) -> CertResult<CertObject> {
    let (_, cert) = X509Certificate::from_der(der).map_err(|e| e.to_string())?;
    Ok(cert_from_x509(&cert, der))
}

pub fn parse_cert_pem(pem: &[u8]) -> CertResult<CertObject> {
    let der = crate::pem::pem_block_to_der(pem)?;
    parse_cert_der(&der)
}

pub fn cert_object_from_js<'js>(_ctx: &Ctx<'js>, obj: &Object<'js>) -> CertResult<CertObject> {
    use rquickjs::Object as JsObject;

    let subject = if let Ok(subject_obj) = obj.get::<_, JsObject>("subject") {
        dn_map_from_js(&subject_obj)?
    } else {
        BTreeMap::new()
    };

    let issuer = if let Ok(issuer_obj) = obj.get::<_, JsObject>("issuer") {
        dn_map_from_js(&issuer_obj)?
    } else {
        BTreeMap::new()
    };

    let subjectaltname = obj
        .get::<_, Option<String>>("subjectaltname")
        .map_err(|e| e.to_string())?;

    Ok(CertObject {
        subject,
        issuer,
        subjectaltname,
        valid_from: obj
            .get::<_, Option<String>>("valid_from")
            .ok()
            .flatten()
            .unwrap_or_default(),
        valid_to: obj
            .get::<_, Option<String>>("valid_to")
            .ok()
            .flatten()
            .unwrap_or_default(),
        serial_number: obj
            .get::<_, Option<String>>("serialNumber")
            .ok()
            .flatten()
            .unwrap_or_default(),
        fingerprint: obj
            .get::<_, Option<String>>("fingerprint")
            .ok()
            .flatten()
            .unwrap_or_default(),
        fingerprint256: obj
            .get::<_, Option<String>>("fingerprint256")
            .ok()
            .flatten()
            .unwrap_or_default(),
        raw: Vec::new(),
        issuer_certificate: None,
    })
}

fn dn_map_from_js(obj: &rquickjs::Object<'_>) -> CertResult<BTreeMap<String, CnValue>> {
    let mut map = BTreeMap::new();
    for key in obj.keys::<String>() {
        let key = key.map_err(|e| e.to_string())?;
        if let Ok(value) = obj.get::<_, String>(&key) {
            map.insert(key, CnValue::Single(value));
        } else if let Ok(array) = obj.get::<_, rquickjs::Array>(&key) {
            let mut values = Vec::new();
            for i in 0..array.len() {
                if let Ok(value) = array.get::<String>(i) {
                    values.push(value);
                }
            }
            match values.len() {
                0 => {},
                1 => {
                    map.insert(key, CnValue::Single(values.remove(0)));
                },
                _ => {
                    map.insert(key, CnValue::Multiple(values));
                },
            }
        }
    }
    Ok(map)
}

pub fn parse_cert_chain_der(chain: &[Vec<u8>]) -> CertResult<Vec<CertObject>> {
    let mut certs = chain
        .iter()
        .map(|der| parse_cert_der(der))
        .collect::<CertResult<Vec<_>>>()?;
    if certs.is_empty() {
        return Ok(certs);
    }
    if certs.len() == 1 {
        let root = certs[0].clone();
        certs[0].issuer_certificate = Some(Box::new(root));
        return Ok(certs);
    }

    let n = certs.len();
    let root = certs[n - 1].clone();
    certs[n - 1].issuer_certificate = Some(Box::new(root));

    for i in (0..n - 1).rev() {
        let issuer = certs[i + 1].clone();
        certs[i].issuer_certificate = Some(Box::new(issuer));
    }

    Ok(certs)
}

fn cert_from_x509(cert: &X509Certificate<'_>, raw: &[u8]) -> CertObject {
    let subject = dn_to_map(&cert.subject());
    let issuer = dn_to_map(&cert.issuer());
    let subjectaltname = extract_subject_alt_name(cert);
    let valid_from = format_asn1_time(&cert.validity().not_before);
    let valid_to = format_asn1_time(&cert.validity().not_after);
    let serial_number = cert.serial.to_str_radix(16).to_uppercase();
    let fingerprint = fingerprint_hex::<Sha1>(raw);
    let fingerprint256 = fingerprint_hex::<Sha256>(raw);

    CertObject {
        subject,
        issuer,
        subjectaltname,
        valid_from,
        valid_to,
        serial_number,
        fingerprint,
        fingerprint256,
        raw: raw.to_vec(),
        issuer_certificate: None,
    }
}

fn dn_to_map(dn: &X509Name<'_>) -> BTreeMap<String, CnValue> {
    let mut map = BTreeMap::new();
    for rdn in dn.iter() {
        for attr in rdn.iter() {
            if let Ok(s) = attr.as_str() {
                let oid = attr.attr_type().to_id_string();
                let key = oid_to_short_name(&oid);
                map.entry(key.to_string())
                    .and_modify(|v: &mut CnValue| {
                        let existing = std::mem::replace(v, CnValue::Single(String::new()));
                        match existing {
                            CnValue::Single(a) => {
                                *v = CnValue::Multiple(vec![a, s.to_string()]);
                            },
                            CnValue::Multiple(mut list) => {
                                list.push(s.to_string());
                                *v = CnValue::Multiple(list);
                            },
                        }
                    })
                    .or_insert(CnValue::Single(s.to_string()));
            }
        }
    }
    map
}

fn oid_to_short_name(oid: &str) -> &str {
    match oid {
        "2.5.4.3" => "CN",
        "2.5.4.4" => "SN",
        "2.5.4.5" => "serialNumber",
        "2.5.4.6" => "C",
        "2.5.4.7" => "L",
        "2.5.4.8" => "ST",
        "2.5.4.9" => "STREET",
        "2.5.4.10" => "O",
        "2.5.4.11" => "OU",
        "2.5.4.12" => "T",
        "2.5.4.13" => "D",
        "2.5.4.42" => "G",
        "2.5.4.43" => "I",
        "1.2.840.113549.1.9.1" => "emailAddress",
        _ => oid,
    }
}

fn extract_subject_alt_name(cert: &X509Certificate<'_>) -> Option<String> {
    let ext = cert
        .extensions()
        .iter()
        .find(|e| e.oid == oid_registry::OID_X509_EXT_SUBJECT_ALT_NAME)?;
    if let ParsedExtension::SubjectAlternativeName(san) = ext.parsed_extension() {
        let mut parts = Vec::new();
        for name in &san.general_names {
            match name {
                GeneralName::DNSName(dns) => parts.push(format!("DNS:{dns}")),
                GeneralName::IPAddress(ip) => {
                    let ip_str = if ip.len() == 4 {
                        format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
                    } else if ip.len() == 16 {
                        let segments: Vec<String> = ip
                            .chunks(2)
                            .map(|c| format!("{:x}", u16::from_be_bytes([c[0], c[1]])))
                            .collect();
                        segments.join(":")
                    } else {
                        continue;
                    };
                    parts.push(format!("IP Address:{ip_str}"));
                },
                _ => {},
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    } else {
        None
    }
}

fn format_asn1_time(time: &ASN1Time) -> String {
    time.to_string()
}

fn fingerprint_hex<D: Digest + Default>(data: &[u8]) -> String {
    let mut hasher = D::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn cert_to_js<'js>(ctx: &Ctx<'js>, cert: &CertObject, detailed: bool) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("subject", dn_map_to_js(ctx, &cert.subject)?)?;
    obj.set("issuer", dn_map_to_js(ctx, &cert.issuer)?)?;
    if let Some(san) = &cert.subjectaltname {
        obj.set("subjectaltname", san.as_str())?;
    }
    obj.set("valid_from", cert.valid_from.as_str())?;
    obj.set("valid_to", cert.valid_to.as_str())?;
    obj.set("serialNumber", cert.serial_number.as_str())?;
    obj.set("fingerprint", cert.fingerprint.as_str())?;
    obj.set("fingerprint256", cert.fingerprint256.as_str())?;
    obj.set("raw", lexe_buffer::Buffer(cert.raw.clone()).into_js(ctx)?)?;

    if detailed {
        if let Some(issuer) = &cert.issuer_certificate {
            obj.set("issuerCertificate", issuer.to_js_object_detailed(ctx)?)?;
        }
    }

    Ok(obj)
}

fn dn_map_to_js<'js>(ctx: &Ctx<'js>, map: &BTreeMap<String, CnValue>) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    for (key, value) in map {
        match value {
            CnValue::Single(s) => {
                obj.set(key.as_str(), s.as_str())?;
            },
            CnValue::Multiple(list) => {
                let array = rquickjs::Array::new(ctx.clone())?;
                for (i, item) in list.iter().enumerate() {
                    array.set(i, item.as_str())?;
                }
                obj.set(key.as_str(), array)?;
            },
        }
    }
    Ok(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{check_server_identity, CnValue};
    use rquickjs::{Context, Runtime};

    #[test]
    fn fingerprint_format() {
        let fp = fingerprint_hex::<Sha256>(b"test");
        assert!(fp.contains(':'));
        assert_eq!(fp.len(), 32 * 3 - 1);
    }

    #[test]
    fn parse_cert_chain_links_all_issuer_levels() {
        let leaf = crate::pem::pem_block_to_der(fixtures::CHAIN_LEAF_CERT.as_bytes()).unwrap();
        let intermediate =
            crate::pem::pem_block_to_der(fixtures::INTERMEDIATE_CERT.as_bytes()).unwrap();
        let root = crate::pem::pem_block_to_der(fixtures::ROOT_CA.as_bytes()).unwrap();

        let chain = parse_cert_chain_der(&[leaf, intermediate, root]).unwrap();
        let leaf_fp = chain[0].fingerprint256.clone();
        let intermediate_fp = chain[1].fingerprint256.clone();
        let root_fp = chain[2].fingerprint256.clone();

        let intermediate_link = chain[0].issuer_certificate.as_ref().expect("leaf issuer");
        assert_eq!(intermediate_link.fingerprint256, intermediate_fp);

        let root_link = intermediate_link
            .issuer_certificate
            .as_ref()
            .expect("intermediate issuer");
        assert_eq!(root_link.fingerprint256, root_fp);

        let root_self = root_link
            .issuer_certificate
            .as_ref()
            .expect("root self issuer");
        assert_eq!(root_self.fingerprint256, root_fp);
        assert_eq!(leaf_fp, chain[0].fingerprint256);
    }

    #[test]
    fn cert_object_from_js_ignores_corrupt_raw() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            let cert = Object::new(ctx.clone()).unwrap();
            cert.set("subjectaltname", "DNS:localhost").unwrap();
            cert.set("raw", vec![0u8, 1, 2]).unwrap();
            let parsed = cert_object_from_js(&ctx, &cert).unwrap();
            assert_eq!(parsed.subjectaltname.as_deref(), Some("DNS:localhost"));
            assert!(parsed.raw.is_empty());
        });
    }

    #[test]
    fn exported_check_server_identity_uses_subject_fields() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            let cert = Object::new(ctx.clone()).unwrap();
            cert.set("subjectaltname", "DNS:localhost").unwrap();
            cert.set("raw", vec![0u8, 1, 2]).unwrap();
            let parsed = cert_object_from_js(&ctx, &cert).unwrap();
            assert!(check_server_identity(&ctx, "localhost", &parsed)
                .unwrap()
                .is_none());
        });
    }

    #[test]
    fn cert_object_from_js_reads_cn_arrays() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            let cert = Object::new(ctx.clone()).unwrap();
            let subject = Object::new(ctx.clone()).unwrap();
            let cn = rquickjs::Array::new(ctx.clone()).unwrap();
            cn.set(0, "a.example.com").unwrap();
            cn.set(1, "localhost").unwrap();
            subject.set("CN", cn).unwrap();
            cert.set("subject", subject).unwrap();

            let parsed = cert_object_from_js(&ctx, &cert).unwrap();
            match parsed.subject.get("CN").unwrap() {
                CnValue::Multiple(values) => {
                    assert!(values.contains(&"localhost".to_string()));
                },
                other => panic!("expected multiple CN values, got {other:?}"),
            }
            assert!(check_server_identity(&ctx, "localhost", &parsed).is_ok());
            assert!(check_server_identity(&ctx, "localhost", &parsed)
                .unwrap()
                .is_none());
        });
    }

    mod fixtures {
        pub use lexe_test_tls::fixtures::{CHAIN_LEAF_CERT, INTERMEDIATE_CERT, ROOT_CA};
    }
}
