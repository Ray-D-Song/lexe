// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::net::IpAddr;

use idna::domain_to_ascii;
use rquickjs::{Ctx, Result, Value};

use crate::certificate::CertObject;
use crate::error::{cert_altname_invalid, ERR_TLS_CERT_ALTNAME_FORMAT};

/// Node-compatible server identity check. Returns `Some(Error)` on failure.
pub fn check_server_identity<'js>(
    ctx: &Ctx<'js>,
    hostname: &str,
    cert: &CertObject,
) -> Result<Option<Value<'js>>> {
    let hostname = hostname.trim();
    if hostname.is_empty() {
        return Ok(None);
    }

    let mut dns_names = Vec::new();
    let mut ips = Vec::new();

    if let Some(alt_names) = &cert.subjectaltname {
        let split = if alt_names.contains('"') {
            split_escaped_alt_names(alt_names)
                .map_err(|_| rquickjs::Exception::throw_message(ctx, ERR_TLS_CERT_ALTNAME_FORMAT))?
        } else {
            alt_names.split(", ").map(str::to_string).collect()
        };
        for name in split {
            if let Some(dns) = name.strip_prefix("DNS:") {
                dns_names.push(dns.to_string());
            } else if let Some(ip) = name.strip_prefix("IP Address:") {
                ips.push(canonicalize_ip(ip));
            }
        }
    }

    let hostname_no_dot = unfqdn(hostname);
    let is_ip = parse_ip(hostname_no_dot).is_some();

    let valid = if is_ip {
        let canonical = canonicalize_ip(hostname_no_dot);
        ips.iter().any(|ip| ip == &canonical)
    } else {
        let hostname_ascii = domain_to_ascii(unfqdn(hostname)).unwrap_or_default();
        let host_parts = split_host(&hostname_ascii);
        let wildcard = |pattern: &str| check_host_parts(&host_parts, pattern, true);

        if !dns_names.is_empty() {
            dns_names.iter().any(|name| wildcard(name))
        } else if let Some(cn) = cert.subject.get("CN") {
            match cn {
                CnValue::Single(s) => wildcard(s),
                CnValue::Multiple(list) => list.iter().any(|s| wildcard(s)),
            }
        } else {
            false
        }
    };

    if valid {
        return Ok(None);
    }

    let reason = if is_ip {
        format!(
            "IP: {hostname_no_dot} is not in the cert's list: {}",
            ips.join(", ")
        )
    } else if !dns_names.is_empty() {
        format!(
            "Host: {hostname_no_dot}. is not in the cert's altnames: {}",
            cert.subjectaltname.as_deref().unwrap_or("")
        )
    } else if let Some(cn) = cert.subject.get("CN") {
        format!("Host: {hostname_no_dot}. is not cert's CN: {cn}")
    } else {
        "Cert does not contain a DNS name".to_string()
    };

    let cert_obj = cert.to_js_object(ctx)?;
    let err = cert_altname_invalid(ctx, &reason, hostname_no_dot, cert_obj)?;
    Ok(Some(err.into_value()))
}

#[derive(Debug, Clone)]
pub enum CnValue {
    Single(String),
    Multiple(Vec<String>),
}

impl std::fmt::Display for CnValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(s) => write!(f, "{s}"),
            Self::Multiple(list) => write!(f, "{list:?}"),
        }
    }
}

fn unfqdn(host: &str) -> &str {
    host.strip_suffix('.').unwrap_or(host)
}

fn to_lower_ascii(c: char) -> char {
    if ('A'..='Z').contains(&c) {
        char::from_u32(32 + c as u32).unwrap_or(c)
    } else {
        c
    }
}

fn split_host(host: &str) -> Vec<String> {
    unfqdn(host)
        .chars()
        .map(to_lower_ascii)
        .collect::<String>()
        .split('.')
        .map(str::to_string)
        .collect()
}

fn check_host_parts(host_parts: &[String], pattern: &str, wildcards: bool) -> bool {
    if pattern.is_empty() {
        return false;
    }

    let pattern_parts = split_host(pattern);
    if host_parts.len() != pattern_parts.len() {
        return false;
    }
    if pattern_parts.iter().any(|p| p.is_empty()) {
        return false;
    }
    if pattern_parts.iter().any(|p| !p.is_ascii()) {
        return false;
    }

    for i in (1..host_parts.len()).rev() {
        if host_parts[i] != pattern_parts[i] {
            return false;
        }
    }

    let host_subdomain = &host_parts[0];
    let pattern_subdomain = &pattern_parts[0];
    let parts: Vec<&str> = pattern_subdomain.split('*').collect();

    if parts.len() == 1 || pattern_subdomain.contains("xn--") {
        return host_subdomain == pattern_subdomain;
    }
    if !wildcards || parts.len() > 2 || pattern_parts.len() <= 2 {
        return false;
    }

    let prefix = parts[0];
    let suffix = parts[1];
    if prefix.len() + suffix.len() > host_subdomain.len() {
        return false;
    }
    host_subdomain.starts_with(prefix) && host_subdomain.ends_with(suffix)
}

fn parse_ip(host: &str) -> Option<IpAddr> {
    host.parse().ok()
}

pub fn canonicalize_ip(ip: &str) -> String {
    ip.parse::<IpAddr>()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| ip.to_string())
}

fn split_escaped_alt_names(alt_names: &str) -> std::result::Result<Vec<String>, &'static str> {
    let mut result = Vec::new();
    let mut current_token = String::new();
    let mut offset = 0usize;

    while offset < alt_names.len() {
        let rest = &alt_names[offset..];
        let next_sep = rest.find(", ");
        let next_quote = rest.find('"');

        if let Some(quote_pos) = next_quote {
            if next_sep.is_none() || quote_pos < next_sep.unwrap() {
                current_token.push_str(&rest[..quote_pos]);
                let after_quote = &rest[quote_pos..];
                let parsed = parse_json_string_literal(after_quote)?;
                current_token.push_str(&parsed.0);
                offset += quote_pos + parsed.1;
                continue;
            }
        }

        if let Some(sep_pos) = next_sep {
            current_token.push_str(&rest[..sep_pos]);
            result.push(current_token);
            current_token = String::new();
            offset += sep_pos + 2;
        } else {
            current_token.push_str(rest);
            offset = alt_names.len();
        }
    }
    result.push(current_token);
    Ok(result)
}

fn parse_json_string_literal(input: &str) -> std::result::Result<(String, usize), &'static str> {
    if !input.starts_with('"') {
        return Err(crate::error::ERR_TLS_CERT_ALTNAME_FORMAT);
    }
    let mut out = String::new();
    let mut chars = input[1..].char_indices();
    loop {
        let (i, c) = match chars.next() {
            Some(v) => v,
            None => return Err(crate::error::ERR_TLS_CERT_ALTNAME_FORMAT),
        };
        if c == '"' {
            return Ok((out, i + 2));
        }
        if c == '\\' {
            let (_, esc) = chars
                .next()
                .ok_or(crate::error::ERR_TLS_CERT_ALTNAME_FORMAT)?;
            match esc {
                '"' | '\\' | '/' => out.push(esc),
                'b' => out.push('\x08'),
                'f' => out.push('\x0c'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = input[i + 2..].chars().take(4).collect();
                    if hex.len() != 4 {
                        return Err(crate::error::ERR_TLS_CERT_ALTNAME_FORMAT);
                    }
                    let code = u32::from_str_radix(&hex, 16)
                        .map_err(|_| crate::error::ERR_TLS_CERT_ALTNAME_FORMAT)?;
                    out.push(
                        char::from_u32(code).ok_or(crate::error::ERR_TLS_CERT_ALTNAME_FORMAT)?,
                    );
                    for _ in 0..3 {
                        chars.next();
                    }
                },
                _ => return Err(crate::error::ERR_TLS_CERT_ALTNAME_FORMAT),
            }
        } else if c.is_control() {
            return Err(crate::error::ERR_TLS_CERT_ALTNAME_FORMAT);
        } else {
            out.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::CnValue;
    use std::collections::BTreeMap;

    fn make_cert(san: &str, cn: Option<&str>) -> CertObject {
        let mut subject = BTreeMap::new();
        if let Some(cn) = cn {
            subject.insert("CN".to_string(), CnValue::Single(cn.to_string()));
        }
        CertObject {
            subject,
            issuer: BTreeMap::new(),
            subjectaltname: Some(san.to_string()),
            valid_from: String::new(),
            valid_to: String::new(),
            serial_number: String::new(),
            fingerprint: String::new(),
            fingerprint256: String::new(),
            raw: Vec::new(),
            issuer_certificate: None,
        }
    }

    #[test]
    fn dns_san_match() {
        let cert = make_cert("DNS:www.example.com", None);
        let rt = rquickjs::Runtime::new().unwrap();
        let ctx = rquickjs::Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            assert!(check_server_identity(&ctx, "www.example.com", &cert)
                .unwrap()
                .is_none());
        });
    }

    #[test]
    fn wildcard_matches_subdomain() {
        let host_parts = split_host("www.example.com");
        assert!(check_host_parts(&host_parts, "*.example.com", true));
        assert!(!check_host_parts(&host_parts, "*.com", true));
    }

    #[test]
    fn xn_wildcard_not_expanded() {
        let host_parts = split_host("xn--example.com");
        assert!(!check_host_parts(&host_parts, "xn--*.com", true));
    }

    #[test]
    fn cn_array_identity_mismatch() {
        let mut subject = BTreeMap::new();
        subject.insert(
            "CN".to_string(),
            CnValue::Multiple(vec!["a.example.com".to_string(), "localhost".to_string()]),
        );
        let cert = CertObject {
            subject,
            issuer: BTreeMap::new(),
            subjectaltname: None,
            valid_from: String::new(),
            valid_to: String::new(),
            serial_number: String::new(),
            fingerprint: String::new(),
            fingerprint256: String::new(),
            raw: Vec::new(),
            issuer_certificate: None,
        };
        let rt = rquickjs::Runtime::new().unwrap();
        let ctx = rquickjs::Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            assert!(check_server_identity(&ctx, "localhost", &cert)
                .unwrap()
                .is_none());
            match check_server_identity(&ctx, "missing.example.com", &cert) {
                Ok(None) => panic!("expected identity mismatch"),
                Ok(Some(_)) => {},
                Err(_) => {},
            }
        });
    }

    #[test]
    fn canonicalize_ipv6_loopback() {
        assert_eq!(canonicalize_ip("::1"), "::1");
        assert_eq!(canonicalize_ip("0:0:0:0:0:0:0:1"), "::1");
    }
}
