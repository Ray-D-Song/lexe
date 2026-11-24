// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{Ctx, Exception, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TlsVersion {
    V1_2,
    V1_3,
}

pub const DEFAULT_MIN_VERSION: TlsVersion = TlsVersion::V1_2;
pub const DEFAULT_MAX_VERSION: TlsVersion = TlsVersion::V1_3;

impl TlsVersion {
    pub fn from_str(ctx: &Ctx<'_>, value: &str, which: &str) -> Result<Self> {
        match value {
            "TLSv1.2" => Ok(Self::V1_2),
            "TLSv1.3" => Ok(Self::V1_3),
            _ => Err(Exception::throw_type(
                ctx,
                &format!("{value} is not a valid {which} TLS version (TLSv1.2 or TLSv1.3 only)"),
            )),
        }
    }

    #[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
    pub fn to_rustls(&self) -> &'static rustls::SupportedProtocolVersion {
        match self {
            Self::V1_2 => &rustls::version::TLS12,
            Self::V1_3 => &rustls::version::TLS13,
        }
    }

    #[cfg(feature = "tls-openssl")]
    pub fn to_openssl(&self) -> openssl::ssl::SslVersion {
        match self {
            Self::V1_2 => openssl::ssl::SslVersion::TLS1_2,
            Self::V1_3 => openssl::ssl::SslVersion::TLS1_3,
        }
    }
}

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub fn rustls_protocol_versions(
    min: TlsVersion,
    max: TlsVersion,
) -> Vec<&'static rustls::SupportedProtocolVersion> {
    let mut versions = Vec::new();
    if min <= TlsVersion::V1_2 && max >= TlsVersion::V1_2 {
        versions.push(&rustls::version::TLS12);
    }
    if min <= TlsVersion::V1_3 && max >= TlsVersion::V1_3 {
        versions.push(&rustls::version::TLS13);
    }
    versions
}

/// Format a negotiated TLS protocol name for the Node.js API (`TLSv1.2`, `TLSv1.3`).
pub fn normalize_protocol_name(raw: &str) -> String {
    let normalized = raw.replace('_', ".");
    if normalized.starts_with("TLSv") || normalized.starts_with("TLSV") {
        return normalized;
    }
    match raw {
        "TLS1_2" | "TLS1.2" => "TLSv1.2".to_string(),
        "TLS1_3" | "TLS1.3" => "TLSv1.3".to_string(),
        _ => normalized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};

    #[test]
    fn parses_tls_versions() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            assert_eq!(
                TlsVersion::from_str(&ctx, "TLSv1.2", "minimum").unwrap(),
                TlsVersion::V1_2
            );
            assert_eq!(
                TlsVersion::from_str(&ctx, "TLSv1.3", "maximum").unwrap(),
                TlsVersion::V1_3
            );
            assert!(TlsVersion::from_str(&ctx, "TLSv1.1", "minimum").is_err());
        });
    }

    #[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
    #[test]
    fn rustls_version_range() {
        let versions = rustls_protocol_versions(TlsVersion::V1_2, TlsVersion::V1_3);
        assert_eq!(versions.len(), 2);
        let versions = rustls_protocol_versions(TlsVersion::V1_3, TlsVersion::V1_3);
        assert_eq!(versions.len(), 1);
    }

    #[test]
    fn normalize_protocol_name_maps_backend_values() {
        assert_eq!(normalize_protocol_name("TLSv1.3"), "TLSv1.3");
        assert_eq!(normalize_protocol_name("TLS1_3"), "TLSv1.3");
        assert_eq!(normalize_protocol_name("TLS1.3"), "TLSv1.3");
        assert_eq!(normalize_protocol_name("TLS1_2"), "TLSv1.2");
    }
}
