// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Single Executable Application (`lsea1`) container encoder and decoder.
//!
//! Legacy executables without an `lsea1` header contain raw `lrt01` bytecode directly
//! and decode with `is_sea = false`, preserving full backward compatibility.

use std::collections::HashMap;

pub const SEA_MAGIC: &[u8; 5] = b"lsea1";
pub const SEA_FLAG_ARGV_MODE: u8 = 0x01;
const COMPRESSION_RAW: u8 = 0;
const COMPRESSION_ZSTD: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAsset {
    pub compressed: bool,
    pub orig_len: u32,
    pub data: Vec<u8>,
}

impl StoredAsset {
    pub fn encode_bytes(raw: &[u8]) -> Result<Self, String> {
        let orig_len = u32::try_from(raw.len())
            .map_err(|_| format!("SEA asset exceeds 4 GiB limit ({} bytes)", raw.len()))?;
        if !raw.is_empty() {
            if let Ok(compressed) = zstd::bulk::compress(raw, 3) {
                if compressed.len() < raw.len() {
                    return Ok(Self {
                        compressed: true,
                        orig_len,
                        data: compressed,
                    });
                }
            }
        }
        Ok(Self {
            compressed: false,
            orig_len,
            data: raw.to_vec(),
        })
    }

    pub fn decompress(&self) -> Result<Vec<u8>, String> {
        if !self.compressed {
            return Ok(self.data.clone());
        }
        zstd::bulk::decompress(&self.data, self.orig_len as usize)
            .map_err(|e| format!("Failed to decompress SEA asset: {e}"))
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddedPayload {
    pub bytecode: Vec<u8>,
    pub is_sea: bool,
    pub sea_argv_mode: bool,
    pub keys: Vec<String>,
    pub assets: HashMap<String, StoredAsset>,
}

pub fn encode_sea_payload(
    bytecode: &[u8],
    assets: &[(String, Vec<u8>)],
    sea_argv_mode: bool,
) -> Result<Vec<u8>, String> {
    let bc_len = u32::try_from(bytecode.len()).map_err(|_| {
        format!(
            "Compiled bytecode exceeds 4 GiB limit ({} bytes)",
            bytecode.len()
        )
    })?;
    let asset_count = u32::try_from(assets.len()).map_err(|_| "Too many SEA assets".to_string())?;

    let flags = if sea_argv_mode { SEA_FLAG_ARGV_MODE } else { 0 };
    let mut out = Vec::with_capacity(6 + 4 + bytecode.len() + 4);
    out.extend_from_slice(SEA_MAGIC);
    out.push(flags);
    out.extend_from_slice(&bc_len.to_le_bytes());
    out.extend_from_slice(bytecode);
    out.extend_from_slice(&asset_count.to_le_bytes());

    for (key, raw) in assets {
        let key_bytes = key.as_bytes();
        let key_len =
            u32::try_from(key_bytes.len()).map_err(|_| format!("SEA asset key too long: {key}"))?;
        let stored = StoredAsset::encode_bytes(raw)?;
        let stored_len = u32::try_from(stored.data.len())
            .map_err(|_| format!("SEA compressed asset too large: {key}"))?;

        out.extend_from_slice(&key_len.to_le_bytes());
        out.extend_from_slice(key_bytes);
        out.push(if stored.compressed {
            COMPRESSION_ZSTD
        } else {
            COMPRESSION_RAW
        });
        out.extend_from_slice(&stored.orig_len.to_le_bytes());
        out.extend_from_slice(&stored_len.to_le_bytes());
        out.extend_from_slice(&stored.data);
    }

    Ok(out)
}

fn read_bytes<'a>(raw: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], String> {
    let end = pos
        .checked_add(len)
        .ok_or_else(|| "SEA payload offset overflow".to_string())?;
    let slice = raw
        .get(*pos..end)
        .ok_or_else(|| "Truncated SEA payload".to_string())?;
    *pos = end;
    Ok(slice)
}

fn read_u8(raw: &[u8], pos: &mut usize) -> Result<u8, String> {
    Ok(read_bytes(raw, pos, 1)?[0])
}

fn read_u32_le(raw: &[u8], pos: &mut usize) -> Result<u32, String> {
    let b = read_bytes(raw, pos, 4)?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

pub fn decode_embedded_payload(raw: &[u8]) -> Result<EmbeddedPayload, String> {
    if !raw.starts_with(SEA_MAGIC) {
        return Ok(EmbeddedPayload {
            bytecode: raw.to_vec(),
            is_sea: false,
            sea_argv_mode: false,
            keys: Vec::new(),
            assets: HashMap::new(),
        });
    }

    let mut pos = SEA_MAGIC.len();
    let flags = read_u8(raw, &mut pos)?;
    let sea_argv_mode = (flags & SEA_FLAG_ARGV_MODE) != 0;

    let bc_len = read_u32_le(raw, &mut pos)? as usize;
    let bytecode = read_bytes(raw, &mut pos, bc_len)?.to_vec();

    let asset_count = read_u32_le(raw, &mut pos)? as usize;
    let mut keys = Vec::with_capacity(asset_count);
    let mut assets = HashMap::with_capacity(asset_count);

    for _ in 0..asset_count {
        let key_len = read_u32_le(raw, &mut pos)? as usize;
        let key_bytes = read_bytes(raw, &mut pos, key_len)?;
        let key = std::str::from_utf8(key_bytes)
            .map_err(|e| format!("Invalid UTF-8 SEA asset key: {e}"))?
            .to_string();

        let compression = read_u8(raw, &mut pos)?;
        let compressed = match compression {
            COMPRESSION_RAW => false,
            COMPRESSION_ZSTD => true,
            other => return Err(format!("Unsupported SEA asset compression flag: {other}")),
        };
        let orig_len = read_u32_le(raw, &mut pos)?;
        let stored_len = read_u32_le(raw, &mut pos)? as usize;
        let data = read_bytes(raw, &mut pos, stored_len)?.to_vec();

        if !assets.contains_key(&key) {
            keys.push(key.clone());
        }
        assets.insert(
            key,
            StoredAsset {
                compressed,
                orig_len,
                data,
            },
        );
    }

    Ok(EmbeddedPayload {
        bytecode,
        is_sea: true,
        sea_argv_mode,
        keys,
        assets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_sea_container_raw_and_compressed() {
        let bytecode = b"lrt01uFAKE_BYTECODE".to_vec();
        let compressible = b"hello world! ".repeat(50);
        let tiny = vec![1, 2, 3, 4];

        let encoded = encode_sea_payload(
            &bytecode,
            &[
                ("small.bin".to_string(), tiny.clone()),
                ("text.txt".to_string(), compressible.clone()),
            ],
            true,
        )
        .unwrap();

        let decoded = decode_embedded_payload(&encoded).unwrap();
        assert!(decoded.is_sea);
        assert!(decoded.sea_argv_mode);
        assert_eq!(decoded.bytecode, bytecode);
        assert_eq!(decoded.keys, vec!["small.bin", "text.txt"]);
        assert_eq!(
            decoded
                .assets
                .get("small.bin")
                .unwrap()
                .decompress()
                .unwrap(),
            tiny
        );
        assert!(decoded.assets.get("text.txt").unwrap().compressed);
        assert_eq!(
            decoded
                .assets
                .get("text.txt")
                .unwrap()
                .decompress()
                .unwrap(),
            compressible
        );
    }

    #[test]
    fn legacy_lrt01_payload_decodes_as_non_sea() {
        let legacy = b"lrt01uLEGACY_PAYLOAD".to_vec();
        let decoded = decode_embedded_payload(&legacy).unwrap();
        assert!(!decoded.is_sea);
        assert!(!decoded.sea_argv_mode);
        assert_eq!(decoded.bytecode, legacy);
        assert!(decoded.keys.is_empty());
        assert!(decoded.assets.is_empty());
    }
}
