// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::collections::HashMap;
use std::sync::Arc;

use crate::secure_context::SecureContext;

/// Per-server SNI hostname → secure context mapping.
#[derive(Debug, Default)]
pub struct SniRegistry {
    map: HashMap<String, Arc<SecureContext>>,
}

impl SniRegistry {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn insert(&mut self, hostname: impl Into<String>, context: Arc<SecureContext>) {
        self.map.insert(hostname.into().to_lowercase(), context);
    }

    pub fn resolve(&self, server_name: &str) -> Option<Arc<SecureContext>> {
        let name = server_name.to_lowercase();
        if let Some(ctx) = self.map.get(&name) {
            return Some(ctx.clone());
        }
        for (pattern, ctx) in &self.map {
            if pattern.starts_with("*.") {
                let suffix = &pattern[1..];
                if name.ends_with(suffix) {
                    let prefix = name.strip_suffix(suffix).unwrap_or("");
                    if !prefix.is_empty() && !prefix.contains('.') {
                        return Some(ctx.clone());
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::TlsVersion;

    fn empty_context() -> Arc<SecureContext> {
        Arc::new(SecureContext {
            ca: Vec::new(),
            cert_chain: Vec::new(),
            key: None,
            passphrase: None,
            min_version: TlsVersion::V1_2,
            max_version: TlsVersion::V1_3,
            alpn_protocols: None,
        })
    }

    #[test]
    fn sni_exact_match_is_case_insensitive() {
        let mut registry = SniRegistry::new();
        let ctx = empty_context();
        registry.insert("Example.COM", ctx);
        assert!(registry.resolve("example.com").is_some());
    }

    #[test]
    fn sni_wildcard_match() {
        let mut registry = SniRegistry::new();
        registry.insert("*.example.com", empty_context());
        assert!(registry.resolve("www.example.com").is_some());
        assert!(registry.resolve("foo.bar.example.com").is_none());
    }
}
