// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hasher},
    sync::OnceLock,
};

use lexe_utils::{
    bytes::ObjectBytes,
    primordials::{BasePrimordials, Primordial},
};
use rquickjs::{Ctx, IntoJs, Result, TypedArray, Value};

use crate::errors::throw_invalid_arg_type;

const MAGIC: &[u8; 4] = b"LXVM";
const FORMAT_VERSION: u8 = 1;
const HEADER_LEN: usize = 4 + 1 + 1 + 8 + 8;

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum CacheKind {
    Script = 1,
    Function = 2,
    Module = 3,
}

fn cache_keys() -> &'static (RandomState, RandomState) {
    static KEYS: OnceLock<(RandomState, RandomState)> = OnceLock::new();
    KEYS.get_or_init(|| (RandomState::new(), RandomState::new()))
}

fn compute_mac(kind: CacheKind, meta_key: &[u8], payload: &[u8]) -> (u64, u64) {
    let (s1, s2) = cache_keys();
    let mut h1 = s1.build_hasher();
    h1.write_u8(FORMAT_VERSION);
    h1.write_u8(kind as u8);
    h1.write_usize(meta_key.len());
    h1.write(meta_key);
    h1.write_usize(payload.len());
    h1.write(payload);

    let mut h2 = s2.build_hasher();
    h2.write_u8(kind as u8);
    h2.write_u8(FORMAT_VERSION);
    h2.write_usize(payload.len());
    h2.write(payload);
    h2.write_usize(meta_key.len());
    h2.write(meta_key);

    (h1.finish(), h2.finish())
}

pub fn encode_cache(kind: CacheKind, meta_key: &[u8], bytecode: &[u8]) -> Vec<u8> {
    let (mac_lo, mac_hi) = compute_mac(kind, meta_key, bytecode);
    let mut out = Vec::with_capacity(HEADER_LEN + bytecode.len());
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.push(kind as u8);
    out.extend_from_slice(&mac_lo.to_le_bytes());
    out.extend_from_slice(&mac_hi.to_le_bytes());
    out.extend_from_slice(bytecode);
    out
}

pub fn decode_cache<'a>(kind: CacheKind, meta_key: &[u8], data: &'a [u8]) -> Option<&'a [u8]> {
    if data.len() < HEADER_LEN {
        return None;
    }
    if &data[0..4] != MAGIC || data[4] != FORMAT_VERSION || data[5] != kind as u8 {
        return None;
    }
    let expected_lo = u64::from_le_bytes(data[6..14].try_into().ok()?);
    let expected_hi = u64::from_le_bytes(data[14..22].try_into().ok()?);
    let payload = &data[HEADER_LEN..];
    let (actual_lo, actual_hi) = compute_mac(kind, meta_key, payload);
    if actual_lo == expected_lo && actual_hi == expected_hi {
        Some(payload)
    } else {
        None
    }
}

pub fn extract_cached_data_bytes<'js>(
    ctx: &Ctx<'js>,
    arg_name: &str,
    val: &Value<'js>,
) -> Result<Vec<u8>> {
    BasePrimordials::init(ctx)?;
    if let Some(obj) = val.as_object() {
        if let Some(bytes) = ObjectBytes::from_array_buffer_view(obj)? {
            return Ok(bytes
                .as_bytes(ctx)
                .map(|slice| slice.to_vec())
                .unwrap_or_default());
        }
    }
    Err(throw_invalid_arg_type(
        ctx,
        arg_name,
        "an instance of Buffer, TypedArray, or DataView",
        val,
    ))
}

pub fn to_buffer<'js>(ctx: &Ctx<'js>, bytes: Vec<u8>) -> Result<Value<'js>> {
    if ctx.globals().contains_key("Buffer").unwrap_or(false) {
        if let Ok(buf) = lexe_buffer::Buffer(bytes.clone()).into_js(ctx) {
            return Ok(buf);
        }
    }
    TypedArray::<u8>::new(ctx.clone(), bytes)?.into_js(ctx)
}
