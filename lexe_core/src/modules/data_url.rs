// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{
    loader::{ImportAttributes, Loader, Resolver},
    Ctx, Error, Module, Result,
};

fn is_data_url(specifier: &str) -> bool {
    specifier.len() >= 5 && specifier[..5].eq_ignore_ascii_case("data:")
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            if let (Some(hi), Some(lo)) = (hex_val(input[i + 1]), hex_val(input[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

pub fn parse_data_url_js(specifier: &str) -> std::result::Result<String, String> {
    if !is_data_url(specifier) {
        return Err("Not a data: URL".to_string());
    }
    let rest = &specifier[5..];
    let (meta, raw_body) = rest
        .split_once(',')
        .ok_or_else(|| "Invalid data: URL: missing ','".to_string())?;
    let body = raw_body.split('#').next().unwrap_or(raw_body);

    let mut is_base64 = false;
    let mut mime: Option<&str> = None;
    for (idx, part) in meta.split(';').enumerate() {
        let trimmed = part.trim();
        if idx == 0 {
            if !trimmed.is_empty() {
                mime = Some(trimmed);
            }
            continue;
        }
        if trimmed.eq_ignore_ascii_case("base64") {
            is_base64 = true;
        }
    }

    if let Some(m) = mime {
        let lower = m.to_ascii_lowercase();
        if !matches!(
            lower.as_str(),
            "text/javascript"
                | "application/javascript"
                | "text/ecmascript"
                | "application/ecmascript"
                | "text/jscript"
        ) {
            return Err(format!("Unsupported data: URL MIME type: {m}"));
        }
    }

    let bytes = if is_base64 {
        let cleaned = percent_decode(body.as_bytes());
        let ascii: Vec<u8> = cleaned
            .into_iter()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        lexe_encoding::bytes_from_b64(ascii)
            .map_err(|e| format!("Invalid base64 in data: URL: {e}"))?
    } else {
        percent_decode(body.as_bytes())
    };

    String::from_utf8(bytes).map_err(|_| "Invalid UTF-8 in data: URL".to_string())
}

#[derive(Debug, Default)]
pub struct DataUrlResolver;

impl Resolver for DataUrlResolver {
    fn resolve(
        &mut self,
        _ctx: &Ctx<'_>,
        _base: &str,
        name: &str,
        _attr: Option<ImportAttributes<'_>>,
    ) -> Result<String> {
        if is_data_url(name) {
            return Ok(name.to_string());
        }
        Err(Error::new_resolving("", name))
    }
}

#[derive(Debug, Default)]
pub struct DataUrlLoader;

impl Loader for DataUrlLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attr: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js>> {
        if !is_data_url(name) {
            return Err(Error::new_loading(name));
        }
        let source =
            parse_data_url_js(name).map_err(|msg| Error::new_loading_message(name, msg))?;
        let module = Module::declare(ctx.clone(), name, source)?;
        if let Ok(meta) = module.meta() {
            let _ = lexe_modules::module::resolver::populate_import_meta(ctx, &meta, name);
        }
        Ok(module)
    }
}
