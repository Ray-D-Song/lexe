// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{io, result::Result as StdResult};

use once_cell::sync::Lazy;
use rquickjs::{
    loader::{ImportAttributes, Loader},
    Ctx, Error, Module, Object, Result,
};
use tracing::trace;
use zstd::{bulk::Decompressor, dict::DecoderDictionary};

use crate::bytecode::{
    BYTECODE_COMPRESSED, BYTECODE_FILE_EXT, BYTECODE_UNCOMPRESSED, BYTECODE_VERSION,
    SIGNATURE_LENGTH,
};

use super::{BYTECODE_CACHE, CJS_IMPORT_PREFIX, CJS_LOADER_PREFIX, COMPRESSION_DICT};

static DECOMPRESSOR_DICT: Lazy<Option<DecoderDictionary>> = Lazy::new(|| {
    if COMPRESSION_DICT.is_empty() {
        None
    } else {
        Some(DecoderDictionary::copy(COMPRESSION_DICT))
    }
});

#[derive(Debug, Default)]
pub struct EmbeddedLoader;

impl EmbeddedLoader {
    pub fn load_bytecode_module<'js>(ctx: Ctx<'js>, buf: &[u8]) -> Result<Module<'js>> {
        let bytes = Self::get_module_bytecode(buf)?;
        unsafe { Module::load(ctx, &bytes) }
    }

    #[inline]
    pub fn uncompressed_size(input: &[u8]) -> StdResult<(usize, &[u8]), io::Error> {
        let size = input.get(..4).ok_or(io::ErrorKind::InvalidInput)?;
        let size: &[u8; 4] = size.try_into().map_err(|_| io::ErrorKind::InvalidInput)?;
        let uncompressed_size = u32::from_le_bytes(*size) as usize;
        let rest = &input[4..];
        Ok((uncompressed_size, rest))
    }

    fn get_module_bytecode(input: &[u8]) -> Result<Vec<u8>> {
        let (_, compressed, input) = Self::get_bytecode_signature(input)?;

        if compressed {
            let (size, input) = Self::uncompressed_size(input)?;
            let mut buf = Vec::with_capacity(size);
            let mut decompressor = match DECOMPRESSOR_DICT.as_ref() {
                Some(dictionary) => Decompressor::with_prepared_dictionary(dictionary)?,
                None => Decompressor::new()?,
            };
            decompressor.decompress_to_buffer(input, &mut buf)?;
            return Ok(buf);
        }

        Ok(input.to_vec())
    }

    fn get_bytecode_signature(input: &[u8]) -> StdResult<(&[u8], bool, &[u8]), io::Error> {
        let raw_signature = input
            .get(..SIGNATURE_LENGTH)
            .ok_or(io::Error::new::<String>(
                io::ErrorKind::InvalidInput,
                "Invalid bytecode signature length".into(),
            ))?;

        let (last, signature) = raw_signature.split_last().unwrap();

        if signature != BYTECODE_VERSION.as_bytes() {
            return Err(io::Error::new::<String>(
                io::ErrorKind::InvalidInput,
                "Invalid bytecode version".into(),
            ));
        }

        let mut compressed = None;
        if *last == BYTECODE_COMPRESSED {
            compressed = Some(true)
        } else if *last == BYTECODE_UNCOMPRESSED {
            compressed = Some(false)
        }

        let rest = &input[SIGNATURE_LENGTH..];
        Ok((
            signature,
            compressed.ok_or(io::Error::new::<String>(
                io::ErrorKind::InvalidInput,
                "Invalid bytecode signature".into(),
            ))?,
            rest,
        ))
    }

    fn normalize_name(name: &str) -> (bool, bool, &str, &str) {
        if !name.starts_with("__") {
            // If name doesn’t start with "__", return defaults
            return (false, false, name, name);
        }

        if let Some(cjs_path) = name.strip_prefix(CJS_IMPORT_PREFIX) {
            // If it starts with CJS_IMPORT_PREFIX, mark as from_cjs_import
            return (true, false, name, cjs_path);
        }

        if let Some(cjs_path) = name.strip_prefix(CJS_LOADER_PREFIX) {
            // If it starts with CJS_LOADER_PREFIX, mark as is_cjs
            return (false, true, cjs_path, cjs_path);
        }

        // Default return if no prefixes match
        (false, false, name, name)
    }

    fn load_module<'js>(name: &str, ctx: &Ctx<'js>) -> Result<(Module<'js>, Option<String>)> {
        let ctx = ctx.clone();

        let (_, _, normalized_name, path) = Self::normalize_name(name);

        if let Some(bytes) = BYTECODE_CACHE.get(path) {
            trace!("Loading embedded module: {}\n", path);

            return Ok((Self::load_bytecode_module(ctx, bytes)?, Some(path.into())));
        }

        let bytes = std::fs::read(path)?;
        let bytes: &[u8] = &bytes;

        if normalized_name.ends_with(BYTECODE_FILE_EXT) {
            trace!("Loading binary module: {}\n", path);
            return Ok((Self::load_bytecode_module(ctx, bytes)?, Some(path.into())));
        }

        Err(Error::new_loading_message(path, "unable to load"))
    }
}

impl Loader for EmbeddedLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attr: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js>> {
        let (module, url) = Self::load_module(name, ctx)?;
        if let Some(url) = url {
            let meta: Object = module.meta()?;
            meta.prop("url", url.as_str())?;
            // Node 20.11+ / 24.x: import.meta.dirname / filename
            let file_path = url.strip_prefix("file://").unwrap_or(url.as_str());
            if let Some(parent) = std::path::Path::new(file_path).parent() {
                meta.prop("dirname", parent.to_string_lossy().as_ref())?;
            }
            meta.prop("filename", file_path)?;
        }

        Ok(module)
    }
}
