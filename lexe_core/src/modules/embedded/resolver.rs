// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::borrow::Cow;

use rquickjs::{
    loader::{ImportAttributes, Resolver},
    Ctx, Error, Result,
};
use tracing::trace;

use crate::modules::path;

use super::{BYTECODE_CACHE, CJS_IMPORT_PREFIX};

#[derive(Debug, Default)]
pub struct EmbeddedResolver;

#[allow(clippy::manual_strip)]
impl Resolver for EmbeddedResolver {
    fn resolve(
        &mut self,
        _ctx: &Ctx,
        base: &str,
        name: &str,
        _attr: Option<ImportAttributes>,
    ) -> Result<String> {
        let name = name.trim_start_matches(CJS_IMPORT_PREFIX);
        let name = name.trim_start_matches("node:").trim_end_matches("/");

        let base = base.trim_start_matches(CJS_IMPORT_PREFIX);

        trace!("Try resolve '{}' from '{}'", name, base);

        embedded_resolve(name, base).map(|name| name.into_owned())
    }
}

/// Specifiers that refer to filesystem paths or URLs must not be resolved as
/// embedded builtins. Without this guard, `path::normalize("./stream")` can
/// collapse to `"stream"` and incorrectly hit the bytecode cache.
fn is_path_or_url_specifier(x: &str) -> bool {
    x.starts_with("./")
        || x.starts_with("../")
        || x.starts_with('/')
        || x.starts_with("file:")
        || (cfg!(windows)
            && x.len() >= 3
            && x.as_bytes()[1] == b':'
            && (x.as_bytes()[2] == b'\\' || x.as_bytes()[2] == b'/'))
}

pub fn embedded_resolve<'a>(x: &'a str, y: &str) -> Result<Cow<'a, str>> {
    trace!("embedded_resolve(x, y):({}, {})", x, y);

    // Relative/absolute/`file:` requests are never embedded builtins.
    // Leave them for PackageResolver / filesystem resolution.
    if is_path_or_url_specifier(x) {
        trace!("+- Skipping embedded resolve for path/URL specifier: {}", x);
        return Err(Error::new_resolving(y.to_string(), x.to_string()));
    }

    // Bare module names only: exact bytecode cache hit.
    if BYTECODE_CACHE.contains_key(x) {
        trace!("+- Resolved by `BYTECODE_CACHE`: {}", x);
        return Ok(x.into());
    }

    // Bare names may still normalize (e.g. trailing slash cleanup) and hit cache.
    let x_normalized = path::normalize(x);
    if BYTECODE_CACHE.contains_key(&x_normalized) {
        trace!("+- Resolved by `BYTECODE_CACHE`: {}", x_normalized);
        return Ok(x_normalized.into());
    }

    // Node accepts a trailing separator on bare specifiers (`require("string_decoder/")`),
    // and the bundled commonjs helpers emit exactly that. `path::normalize` keeps the
    // separator, so retry after trimming it: without this, `node:stream` fails to load
    // as soon as a node_modules directory makes the package resolver take over.
    let trimmed = x.trim_end_matches(['/', '\\']);
    if trimmed.len() != x.len() && !trimmed.is_empty() {
        if BYTECODE_CACHE.contains_key(trimmed) {
            trace!(
                "+- Resolved by `BYTECODE_CACHE` (trailing sep): {}",
                trimmed
            );
            return Ok(trimmed.into());
        }
        let trimmed_normalized = path::normalize(trimmed);
        if BYTECODE_CACHE.contains_key(&trimmed_normalized) {
            trace!(
                "+- Resolved by `BYTECODE_CACHE` (trailing sep, normalized): {}",
                trimmed_normalized
            );
            return Ok(trimmed_normalized.into());
        }
    }

    Err(Error::new_resolving(y.to_string(), x.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_stream_resolves_when_present_in_cache_or_misses_cleanly() {
        // `stream` is a real embedded module in production builds; in unit
        // tests the cache content depends on the build. Either way, bare
        // names are allowed into the normalize fallback path.
        let result = embedded_resolve("stream", "file:///app/index.js");
        match result {
            Ok(name) => assert_eq!(name, "stream"),
            Err(err) => {
                let msg = format!("{err:?}");
                assert!(msg.contains("stream"), "unexpected resolve error: {msg}");
            },
        }
    }

    #[test]
    fn relative_stream_is_not_hijacked_by_embedded_resolver() {
        let err = embedded_resolve("./stream", "file:///app/lib/connection.js").unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("./stream") || msg.contains("resolving"),
            "relative ./stream must miss embedded resolve, got: {msg}"
        );
    }

    #[test]
    fn parent_relative_stream_is_not_hijacked() {
        assert!(embedded_resolve("../stream", "file:///app/lib/connection.js").is_err());
    }

    #[test]
    fn absolute_and_file_url_are_not_hijacked() {
        assert!(embedded_resolve("/tmp/stream", "file:///app/index.js").is_err());
        assert!(embedded_resolve("file:///tmp/stream", "file:///app/index.js").is_err());
    }
}
