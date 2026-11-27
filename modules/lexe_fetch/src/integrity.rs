// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_encoding::bytes_from_b64;
use rquickjs::{Ctx, Exception, Result};
use sha2::{Digest, Sha256, Sha384, Sha512};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HashAlg {
    Sha256 = 1,
    Sha384 = 2,
    Sha512 = 3,
}

struct ParsedHash {
    alg: HashAlg,
    digest_token: String,
}

fn is_valid_sri_base64_chars(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
}

fn decode_sri_digest(token: &str) -> Option<Vec<u8>> {
    let mut normalized = String::with_capacity(token.len() + 2);
    for ch in token.chars() {
        match ch {
            '-' => normalized.push('+'),
            '_' => normalized.push('/'),
            _ => normalized.push(ch),
        }
    }
    match normalized.len() % 4 {
        2 => normalized.push_str("=="),
        3 => normalized.push('='),
        1 => return None,
        _ => {},
    }
    bytes_from_b64(normalized.as_bytes()).ok()
}

fn parse_integrity_metadata(metadata: &str) -> Vec<ParsedHash> {
    let mut out = Vec::new();
    for raw_token in metadata.split_ascii_whitespace() {
        let expr = raw_token.split('?').next().unwrap_or("");
        let Some((alg_str, digest)) = expr.split_once('-') else {
            continue;
        };
        if !is_valid_sri_base64_chars(digest) {
            continue;
        }
        let alg = match alg_str {
            "sha256" => HashAlg::Sha256,
            "sha384" => HashAlg::Sha384,
            "sha512" => HashAlg::Sha512,
            _ => continue,
        };
        out.push(ParsedHash {
            alg,
            digest_token: digest.to_string(),
        });
    }
    out
}

/// Verify response body bytes against W3C Subresource Integrity metadata.
/// Unknown algorithms are ignored; when one or more supported (`sha256`,
/// `sha384`, `sha512`) expressions exist, only the strongest algorithm tier
/// is evaluated and at least one digest must match.
pub(crate) fn verify_integrity(ctx: &Ctx<'_>, metadata: &str, body: &[u8]) -> Result<()> {
    let parsed = parse_integrity_metadata(metadata);
    let Some(strongest) = parsed.iter().map(|entry| entry.alg).max() else {
        return Ok(());
    };

    let expected_digest: Vec<u8> = match strongest {
        HashAlg::Sha256 => Sha256::digest(body).to_vec(),
        HashAlg::Sha384 => Sha384::digest(body).to_vec(),
        HashAlg::Sha512 => Sha512::digest(body).to_vec(),
    };

    let matched = parsed
        .iter()
        .filter(|entry| entry.alg == strongest)
        .filter_map(|entry| decode_sri_digest(&entry.digest_token))
        .any(|candidate| candidate == expected_digest);

    if matched {
        return Ok(());
    }

    Err(Exception::throw_type(
        ctx,
        "Failed to fetch: Subresource Integrity check failed",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_sync_with;

    #[tokio::test]
    async fn verifies_matching_and_strongest_tier_sri() {
        test_sync_with(|ctx| {
            // echo -n "hello" | openssl dgst -sha256 -binary | base64
            let sha256_hello = "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
            // printf 'hello' | openssl dgst -sha512 -binary | base64
            let sha512_hello = "sha512-m3HSJL1i83hdltRq0+o9czGb+8KJDKra4t/3JRlnPKcjI8PZm6XBHXx6zG4UuMXaDEZjR1wuXDre9G9zvN7AQw==";

            assert!(verify_integrity(&ctx, sha256_hello, b"hello").is_ok());
            assert!(verify_integrity(&ctx, "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", b"hello").is_err());
            // Strongest tier wins per SRI spec: valid sha256 + wrong sha512 must fail.
            assert!(verify_integrity(
                &ctx,
                &format!("{sha256_hello} sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="),
                b"hello"
            )
            .is_err());
            // Weak sha256 + valid sha512 must pass.
            assert!(verify_integrity(
                &ctx,
                &format!("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= {sha512_hello}?foo=bar"),
                b"hello"
            )
            .is_ok());
            // Unknown algorithms are ignored.
            assert!(verify_integrity(&ctx, "md5-1B2M2Y8AsgTpgAmY7PhCfg==", b"hello").is_ok());
            Ok(())
        })
        .await;
    }
}
