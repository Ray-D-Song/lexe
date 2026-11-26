// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `createCipheriv` and `createDecipheriv` (`aes-{128,192,256}-{gcm,cbc,ctr}`).

use aes::{
    cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit, KeyIvInit, StreamCipher},
    Aes128, Aes192, Aes256,
};
use ctr::Ctr128BE;
use lexe_utils::{bytes::ObjectBytes, object::ObjectExt};
use rquickjs::{
    atom::PredefinedAtom, class::Trace, function::Opt, prelude::This, Class, Ctx, Exception,
    FromJs, JsLifetime, Result, Value,
};

use crate::key_object::{
    encode_bytes_to_output, optional_encoding, value_to_bytes_with_encoding, KeyMaterial, KeyObject,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum CipherMode {
    Gcm,
    Cbc,
    Ctr,
}

#[derive(Clone, Copy)]
struct CipherSpec {
    bits: usize,
    mode: CipherMode,
}

fn parse_cipher_spec(alg: &str) -> std::result::Result<CipherSpec, String> {
    let lower = alg.trim().to_ascii_lowercase();
    let (bits, mode) = match lower.as_str() {
        "aes-128-gcm" => (128, CipherMode::Gcm),
        "aes-192-gcm" => (192, CipherMode::Gcm),
        "aes-256-gcm" => (256, CipherMode::Gcm),
        "aes-128-cbc" => (128, CipherMode::Cbc),
        "aes-192-cbc" => (192, CipherMode::Cbc),
        "aes-256-cbc" => (256, CipherMode::Cbc),
        "aes-128-ctr" => (128, CipherMode::Ctr),
        "aes-192-ctr" => (192, CipherMode::Ctr),
        "aes-256-ctr" => (256, CipherMode::Ctr),
        _ => return Err(format!("Unknown cipher: {alg}")),
    };
    Ok(CipherSpec { bits, mode })
}

enum AesBlockCipher {
    Aes128(Aes128),
    Aes192(Aes192),
    Aes256(Aes256),
}

impl AesBlockCipher {
    fn new(key: &[u8]) -> std::result::Result<Self, String> {
        match key.len() {
            16 => Ok(Self::Aes128(
                Aes128::new_from_slice(key).map_err(|_| "Invalid key length".to_string())?,
            )),
            24 => Ok(Self::Aes192(
                Aes192::new_from_slice(key).map_err(|_| "Invalid key length".to_string())?,
            )),
            32 => Ok(Self::Aes256(
                Aes256::new_from_slice(key).map_err(|_| "Invalid key length".to_string())?,
            )),
            _ => Err("Invalid key length".to_string()),
        }
    }

    fn encrypt_block(&self, block: &mut [u8; 16]) {
        let mut arr = aes::cipher::Array(*block);
        match self {
            Self::Aes128(c) => c.encrypt_block(&mut arr),
            Self::Aes192(c) => c.encrypt_block(&mut arr),
            Self::Aes256(c) => c.encrypt_block(&mut arr),
        }
        *block = arr.into();
    }

    fn decrypt_block(&self, block: &mut [u8; 16]) {
        let mut arr = aes::cipher::Array(*block);
        match self {
            Self::Aes128(c) => c.decrypt_block(&mut arr),
            Self::Aes192(c) => c.decrypt_block(&mut arr),
            Self::Aes256(c) => c.decrypt_block(&mut arr),
        }
        *block = arr.into();
    }
}

enum AesCtrStream {
    Aes128(Ctr128BE<Aes128>),
    Aes192(Ctr128BE<Aes192>),
    Aes256(Ctr128BE<Aes256>),
}

impl AesCtrStream {
    fn new(key: &[u8], iv16: &[u8; 16]) -> std::result::Result<Self, String> {
        match key.len() {
            16 => Ok(Self::Aes128(
                Ctr128BE::new_from_slices(key, iv16)
                    .map_err(|_| "Invalid key or IV length".to_string())?,
            )),
            24 => Ok(Self::Aes192(
                Ctr128BE::new_from_slices(key, iv16)
                    .map_err(|_| "Invalid key or IV length".to_string())?,
            )),
            32 => Ok(Self::Aes256(
                Ctr128BE::new_from_slices(key, iv16)
                    .map_err(|_| "Invalid key or IV length".to_string())?,
            )),
            _ => Err("Invalid key length".to_string()),
        }
    }

    fn apply(&mut self, data: &mut [u8]) -> std::result::Result<(), String> {
        match self {
            Self::Aes128(c) => c.try_apply_keystream(data),
            Self::Aes192(c) => c.try_apply_keystream(data),
            Self::Aes256(c) => c.try_apply_keystream(data),
        }
        .map_err(|_| "CTR keystream exhausted".to_string())
    }
}

// GCM 32-bit counter stream (inc32 increments only the low 32 bits of the 128-bit counter block).
enum AesGcmCtr {
    Aes128(ctr::Ctr32BE<Aes128>),
    Aes192(ctr::Ctr32BE<Aes192>),
    Aes256(ctr::Ctr32BE<Aes256>),
}

impl AesGcmCtr {
    fn new(key: &[u8], counter_block: &[u8; 16]) -> std::result::Result<Self, String> {
        match key.len() {
            16 => Ok(Self::Aes128(
                ctr::Ctr32BE::new_from_slices(key, counter_block)
                    .map_err(|_| "Invalid key length".to_string())?,
            )),
            24 => Ok(Self::Aes192(
                ctr::Ctr32BE::new_from_slices(key, counter_block)
                    .map_err(|_| "Invalid key length".to_string())?,
            )),
            32 => Ok(Self::Aes256(
                ctr::Ctr32BE::new_from_slices(key, counter_block)
                    .map_err(|_| "Invalid key length".to_string())?,
            )),
            _ => Err("Invalid key length".to_string()),
        }
    }

    fn apply(&mut self, data: &mut [u8]) -> std::result::Result<(), String> {
        match self {
            Self::Aes128(c) => c.try_apply_keystream(data),
            Self::Aes192(c) => c.try_apply_keystream(data),
            Self::Aes256(c) => c.try_apply_keystream(data),
        }
        .map_err(|_| "GCM keystream exhausted".to_string())
    }
}

fn gf_mul_be(x: [u8; 16], y: [u8; 16]) -> [u8; 16] {
    let x = u128::from_be_bytes(x);
    let mut v = u128::from_be_bytes(y);
    let mut z = 0u128;
    const R: u128 = 0xe100_0000_0000_0000_0000_0000_0000_0000;
    for i in (0..128).rev() {
        if (x >> i) & 1 != 0 {
            z ^= v;
        }
        let lsb = v & 1;
        v >>= 1;
        if lsb != 0 {
            v ^= R;
        }
    }
    z.to_be_bytes()
}

struct GHashAcc {
    h: [u8; 16],
    acc: [u8; 16],
    buf: [u8; 16],
    buf_len: usize,
    aad_len: u64,
    data_len: u64,
    in_data: bool,
}

impl GHashAcc {
    fn new(h: [u8; 16]) -> Self {
        Self {
            h,
            acc: [0u8; 16],
            buf: [0u8; 16],
            buf_len: 0,
            aad_len: 0,
            data_len: 0,
            in_data: false,
        }
    }

    fn ghash_block(&mut self, block: &[u8; 16]) {
        for (a, &b) in self.acc.iter_mut().zip(block.iter()) {
            *a ^= b;
        }
        self.acc = gf_mul_be(self.acc, self.h);
    }

    fn pad_zero_and_flush(&mut self) {
        if self.buf_len > 0 {
            self.buf[self.buf_len..].fill(0);
            let block = self.buf;
            self.ghash_block(&block);
            self.buf_len = 0;
        }
    }

    fn feed_bytes(&mut self, mut input: &[u8]) {
        if self.buf_len > 0 {
            let take = input.len().min(16 - self.buf_len);
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&input[..take]);
            self.buf_len += take;
            input = &input[take..];
            if self.buf_len == 16 {
                let block = self.buf;
                self.ghash_block(&block);
                self.buf_len = 0;
            }
        }
        while input.len() >= 16 {
            let block: [u8; 16] = input[..16].try_into().unwrap();
            self.ghash_block(&block);
            input = &input[16..];
        }
        if !input.is_empty() {
            self.buf[..input.len()].copy_from_slice(input);
            self.buf_len = input.len();
        }
    }

    fn update_aad(&mut self, aad: &[u8]) -> std::result::Result<(), String> {
        if self.in_data {
            return Err("Invalid state: cannot setAAD after update".to_string());
        }
        self.aad_len = self
            .aad_len
            .checked_add(aad.len() as u64)
            .ok_or_else(|| "AAD length overflow".to_string())?;
        self.feed_bytes(aad);
        Ok(())
    }

    fn update_ciphertext(&mut self, ct: &[u8]) {
        if !self.in_data {
            self.pad_zero_and_flush();
            self.in_data = true;
        }
        self.data_len += ct.len() as u64;
        self.feed_bytes(ct);
    }

    fn finalize_tag(mut self, tag_mask: &[u8; 16]) -> [u8; 16] {
        self.pad_zero_and_flush();
        let mut len_block = [0u8; 16];
        len_block[..8].copy_from_slice(&(self.aad_len * 8).to_be_bytes());
        len_block[8..].copy_from_slice(&(self.data_len * 8).to_be_bytes());
        self.ghash_block(&len_block);
        let mut tag = self.acc;
        for (t, &m) in tag.iter_mut().zip(tag_mask.iter()) {
            *t ^= m;
        }
        tag
    }
}

fn inc32(block: &mut [u8; 16]) {
    let ctr = u32::from_be_bytes(block[12..16].try_into().unwrap()).wrapping_add(1);
    block[12..16].copy_from_slice(&ctr.to_be_bytes());
}

struct GcmState {
    ctr: AesGcmCtr,
    ghash: Option<GHashAcc>,
    tag_mask: [u8; 16],
    tag_len: usize,
    computed_tag: Option<Vec<u8>>,
    expected_tag: Option<Vec<u8>>,
}

impl GcmState {
    fn new(key: &[u8], iv: &[u8], tag_len: usize) -> std::result::Result<Self, String> {
        if iv.is_empty() {
            return Err("Invalid IV length".to_string());
        }
        let aes = AesBlockCipher::new(key)?;
        let mut h = [0u8; 16];
        aes.encrypt_block(&mut h);

        let j0 = if iv.len() == 12 {
            let mut b = [0u8; 16];
            b[..12].copy_from_slice(iv);
            b[15] = 1;
            b
        } else {
            let mut g = GHashAcc::new(h);
            g.update_ciphertext(iv);
            g.finalize_tag(&[0u8; 16])
        };

        let mut tag_mask = j0;
        aes.encrypt_block(&mut tag_mask);

        let mut icb = j0;
        inc32(&mut icb);
        let ctr = AesGcmCtr::new(key, &icb)?;

        Ok(Self {
            ctr,
            ghash: Some(GHashAcc::new(h)),
            tag_mask,
            tag_len,
            computed_tag: None,
            expected_tag: None,
        })
    }
}

enum CipherEngine {
    Gcm(GcmState),
    Cbc {
        aes: AesBlockCipher,
        prev: [u8; 16],
        pending: Vec<u8>,
        auto_padding: bool,
    },
    Ctr(AesCtrStream),
}

impl CipherEngine {
    fn new(
        spec: CipherSpec,
        key: &[u8],
        iv: &[u8],
        auth_tag_len: Option<usize>,
    ) -> std::result::Result<Self, String> {
        if key.len() * 8 != spec.bits {
            return Err("Invalid key length".to_string());
        }
        match spec.mode {
            CipherMode::Gcm => {
                let tag_len = auth_tag_len.unwrap_or(16);
                if !(4..=16).contains(&tag_len) {
                    return Err(format!("Invalid authentication tag length: {tag_len}"));
                }
                Ok(Self::Gcm(GcmState::new(key, iv, tag_len)?))
            },
            CipherMode::Cbc => {
                let prev: [u8; 16] = iv
                    .try_into()
                    .map_err(|_| "Invalid initialization vector".to_string())?;
                Ok(Self::Cbc {
                    aes: AesBlockCipher::new(key)?,
                    prev,
                    pending: Vec::with_capacity(16),
                    auto_padding: true,
                })
            },
            CipherMode::Ctr => {
                let iv16: [u8; 16] = iv
                    .try_into()
                    .map_err(|_| "Invalid initialization vector".to_string())?;
                Ok(Self::Ctr(AesCtrStream::new(key, &iv16)?))
            },
        }
    }

    fn encrypt_update(&mut self, data: &[u8]) -> std::result::Result<Vec<u8>, String> {
        match self {
            Self::Gcm(g) => {
                let mut out = data.to_vec();
                g.ctr.apply(&mut out)?;
                if let Some(ref mut ghash) = g.ghash {
                    ghash.update_ciphertext(&out);
                }
                Ok(out)
            },
            Self::Ctr(c) => {
                let mut out = data.to_vec();
                c.apply(&mut out)?;
                Ok(out)
            },
            Self::Cbc {
                aes, prev, pending, ..
            } => {
                pending.extend_from_slice(data);
                let full_blocks = pending.len() / 16;
                let mut out = Vec::with_capacity(full_blocks * 16);
                for chunk in pending[..full_blocks * 16].chunks_exact(16) {
                    let mut blk: [u8; 16] = chunk.try_into().unwrap();
                    for (b, &p) in blk.iter_mut().zip(prev.iter()) {
                        *b ^= p;
                    }
                    aes.encrypt_block(&mut blk);
                    *prev = blk;
                    out.extend_from_slice(&blk);
                }
                pending.drain(..full_blocks * 16);
                Ok(out)
            },
        }
    }

    fn encrypt_final(&mut self) -> std::result::Result<Vec<u8>, String> {
        match self {
            Self::Gcm(g) => {
                let ghash = g
                    .ghash
                    .take()
                    .ok_or_else(|| "Cipher already finalized".to_string())?;
                let full_tag = ghash.finalize_tag(&g.tag_mask);
                g.computed_tag = Some(full_tag[..g.tag_len].to_vec());
                Ok(Vec::new())
            },
            Self::Ctr(_) => Ok(Vec::new()),
            Self::Cbc {
                aes,
                prev,
                pending,
                auto_padding,
            } => {
                if !*auto_padding {
                    if !pending.is_empty() {
                        return Err("wrong final block length".to_string());
                    }
                    return Ok(Vec::new());
                }
                let pad_len = 16 - pending.len();
                let mut blk = [pad_len as u8; 16];
                blk[..pending.len()].copy_from_slice(pending);
                pending.clear();
                for (b, &p) in blk.iter_mut().zip(prev.iter()) {
                    *b ^= p;
                }
                aes.encrypt_block(&mut blk);
                *prev = blk;
                Ok(blk.to_vec())
            },
        }
    }

    fn decrypt_update(&mut self, data: &[u8]) -> std::result::Result<Vec<u8>, String> {
        match self {
            Self::Gcm(g) => {
                if let Some(ref mut ghash) = g.ghash {
                    ghash.update_ciphertext(data);
                }
                let mut out = data.to_vec();
                g.ctr.apply(&mut out)?;
                Ok(out)
            },
            Self::Ctr(c) => {
                let mut out = data.to_vec();
                c.apply(&mut out)?;
                Ok(out)
            },
            Self::Cbc {
                aes,
                prev,
                pending,
                auto_padding,
            } => {
                pending.extend_from_slice(data);
                let hold_back = if *auto_padding && pending.len() >= 16 {
                    16
                } else {
                    0
                };
                let process_bytes = ((pending.len().saturating_sub(hold_back)) / 16) * 16;
                let mut out = Vec::with_capacity(process_bytes);
                for chunk in pending[..process_bytes].chunks_exact(16) {
                    let ct: [u8; 16] = chunk.try_into().unwrap();
                    let mut pt = ct;
                    aes.decrypt_block(&mut pt);
                    for (p, &pv) in pt.iter_mut().zip(prev.iter()) {
                        *p ^= pv;
                    }
                    *prev = ct;
                    out.extend_from_slice(&pt);
                }
                pending.drain(..process_bytes);
                Ok(out)
            },
        }
    }

    fn decrypt_final(&mut self) -> std::result::Result<Vec<u8>, String> {
        match self {
            Self::Gcm(g) => {
                let expected = g.expected_tag.as_ref().ok_or_else(|| {
                    "Unsupported state or unable to authenticate data".to_string()
                })?;
                let ghash = g
                    .ghash
                    .take()
                    .ok_or_else(|| "Decipher already finalized".to_string())?;
                let full_tag = ghash.finalize_tag(&g.tag_mask);
                if expected.len() != g.tag_len {
                    return Err("Unsupported state or unable to authenticate data".to_string());
                }
                let mut diff = 0u8;
                for (&a, &b) in expected.iter().zip(full_tag[..expected.len()].iter()) {
                    diff |= a ^ b;
                }
                if diff != 0 {
                    return Err("Unsupported state or unable to authenticate data".to_string());
                }
                Ok(Vec::new())
            },
            Self::Ctr(_) => Ok(Vec::new()),
            Self::Cbc {
                aes,
                prev,
                pending,
                auto_padding,
            } => {
                if !*auto_padding {
                    if !pending.is_empty() {
                        return Err("wrong final block length".to_string());
                    }
                    return Ok(Vec::new());
                }
                if pending.len() != 16 {
                    return Err("wrong final block length".to_string());
                }
                let ct: [u8; 16] = pending.as_slice().try_into().unwrap();
                pending.clear();
                let mut pt = ct;
                aes.decrypt_block(&mut pt);
                for (p, &pv) in pt.iter_mut().zip(prev.iter()) {
                    *p ^= pv;
                }
                *prev = ct;
                let pad_len = pt[15] as usize;
                if pad_len == 0
                    || pad_len > 16
                    || !pt[16 - pad_len..].iter().all(|&b| b as usize == pad_len)
                {
                    return Err("bad decrypt".to_string());
                }
                Ok(pt[..16 - pad_len].to_vec())
            },
        }
    }
}

#[rquickjs::class]
#[derive(Trace, JsLifetime)]
pub struct Cipheriv {
    #[qjs(skip_trace)]
    engine: CipherEngine,
    finalized: bool,
}

#[rquickjs::methods]
impl Cipheriv {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'_>) -> Result<Self> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Cipheriv"
    }

    pub fn update<'js>(
        &mut self,
        ctx: Ctx<'js>,
        data: Value<'js>,
        input_encoding: Opt<Value<'js>>,
        output_encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        if self.finalized {
            return Err(Exception::throw_message(&ctx, "Cipher already finalized"));
        }
        let in_enc = optional_encoding(&ctx, input_encoding.0)?;
        let out_enc = optional_encoding(&ctx, output_encoding.0)?;
        let bytes = value_to_bytes_with_encoding(&ctx, &data, in_enc.as_deref(), "data")?;
        let out = self
            .engine
            .encrypt_update(&bytes)
            .map_err(|e| Exception::throw_message(&ctx, &e))?;
        encode_bytes_to_output(&ctx, out, out_enc.as_deref())
    }

    #[qjs(rename = "final")]
    pub fn r#final<'js>(
        &mut self,
        ctx: Ctx<'js>,
        output_encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        if self.finalized {
            return Err(Exception::throw_message(&ctx, "Cipher already finalized"));
        }
        self.finalized = true;
        let out_enc = optional_encoding(&ctx, output_encoding.0)?;
        let out = self
            .engine
            .encrypt_final()
            .map_err(|e| Exception::throw_message(&ctx, &e))?;
        encode_bytes_to_output(&ctx, out, out_enc.as_deref())
    }

    #[qjs(rename = "setAAD")]
    pub fn set_aad<'js>(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        buffer: ObjectBytes<'js>,
        _options: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let aad = buffer.as_bytes(&ctx)?;
        {
            let mut inner = this.0.borrow_mut();
            if inner.finalized {
                return Err(Exception::throw_message(&ctx, "Cipher already finalized"));
            }
            match &mut inner.engine {
                CipherEngine::Gcm(g) => {
                    if let Some(ref mut ghash) = g.ghash {
                        ghash
                            .update_aad(aad)
                            .map_err(|e| Exception::throw_message(&ctx, &e))?;
                    } else {
                        return Err(Exception::throw_message(&ctx, "Cipher already finalized"));
                    }
                },
                _ => {
                    return Err(Exception::throw_message(
                        &ctx,
                        "setAAD is only supported on authenticated ciphers",
                    ));
                },
            }
        }
        Ok(this.0)
    }

    #[qjs(rename = "getAuthTag")]
    pub fn get_auth_tag<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match &self.engine {
            CipherEngine::Gcm(g) => match &g.computed_tag {
                Some(tag) => encode_bytes_to_output(&ctx, tag.clone(), None),
                None => Err(Exception::throw_message(
                    &ctx,
                    "Invalid state: call final() before getAuthTag()",
                )),
            },
            _ => Err(Exception::throw_message(
                &ctx,
                "getAuthTag is only supported on authenticated ciphers",
            )),
        }
    }

    #[qjs(rename = "setAutoPadding")]
    pub fn set_auto_padding<'js>(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        auto_padding: Opt<bool>,
    ) -> Result<Class<'js, Self>> {
        {
            let mut inner = this.0.borrow_mut();
            if inner.finalized {
                return Err(Exception::throw_message(&ctx, "Cipher already finalized"));
            }
            if let CipherEngine::Cbc {
                auto_padding: ap, ..
            } = &mut inner.engine
            {
                *ap = auto_padding.0.unwrap_or(true);
            }
        }
        Ok(this.0)
    }
}

#[rquickjs::class]
#[derive(Trace, JsLifetime)]
pub struct Decipheriv {
    #[qjs(skip_trace)]
    engine: CipherEngine,
    finalized: bool,
}

#[rquickjs::methods]
impl Decipheriv {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'_>) -> Result<Self> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Decipheriv"
    }

    pub fn update<'js>(
        &mut self,
        ctx: Ctx<'js>,
        data: Value<'js>,
        input_encoding: Opt<Value<'js>>,
        output_encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        if self.finalized {
            return Err(Exception::throw_message(&ctx, "Decipher already finalized"));
        }
        let in_enc = optional_encoding(&ctx, input_encoding.0)?;
        let out_enc = optional_encoding(&ctx, output_encoding.0)?;
        let bytes = value_to_bytes_with_encoding(&ctx, &data, in_enc.as_deref(), "data")?;
        let out = self
            .engine
            .decrypt_update(&bytes)
            .map_err(|e| Exception::throw_message(&ctx, &e))?;
        encode_bytes_to_output(&ctx, out, out_enc.as_deref())
    }

    #[qjs(rename = "final")]
    pub fn r#final<'js>(
        &mut self,
        ctx: Ctx<'js>,
        output_encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        if self.finalized {
            return Err(Exception::throw_message(&ctx, "Decipher already finalized"));
        }
        self.finalized = true;
        let out_enc = optional_encoding(&ctx, output_encoding.0)?;
        let out = self
            .engine
            .decrypt_final()
            .map_err(|e| Exception::throw_message(&ctx, &e))?;
        encode_bytes_to_output(&ctx, out, out_enc.as_deref())
    }

    #[qjs(rename = "setAAD")]
    pub fn set_aad<'js>(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        buffer: ObjectBytes<'js>,
        _options: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let aad = buffer.as_bytes(&ctx)?;
        {
            let mut inner = this.0.borrow_mut();
            if inner.finalized {
                return Err(Exception::throw_message(&ctx, "Decipher already finalized"));
            }
            match &mut inner.engine {
                CipherEngine::Gcm(g) => {
                    if let Some(ref mut ghash) = g.ghash {
                        ghash
                            .update_aad(aad)
                            .map_err(|e| Exception::throw_message(&ctx, &e))?;
                    } else {
                        return Err(Exception::throw_message(&ctx, "Decipher already finalized"));
                    }
                },
                _ => {
                    return Err(Exception::throw_message(
                        &ctx,
                        "setAAD is only supported on authenticated ciphers",
                    ));
                },
            }
        }
        Ok(this.0)
    }

    #[qjs(rename = "setAuthTag")]
    pub fn set_auth_tag<'js>(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        buffer: ObjectBytes<'js>,
    ) -> Result<Class<'js, Self>> {
        let tag = buffer.as_bytes(&ctx)?.to_vec();
        {
            let mut inner = this.0.borrow_mut();
            if inner.finalized {
                return Err(Exception::throw_message(&ctx, "Decipher already finalized"));
            }
            match &mut inner.engine {
                CipherEngine::Gcm(g) => {
                    if !(4..=16).contains(&tag.len()) {
                        return Err(Exception::throw_message(
                            &ctx,
                            "Invalid authentication tag length",
                        ));
                    }
                    g.expected_tag = Some(tag);
                },
                _ => {
                    return Err(Exception::throw_message(
                        &ctx,
                        "setAuthTag is only supported on authenticated ciphers",
                    ));
                },
            }
        }
        Ok(this.0)
    }

    #[qjs(rename = "setAutoPadding")]
    pub fn set_auto_padding<'js>(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        auto_padding: Opt<bool>,
    ) -> Result<Class<'js, Self>> {
        {
            let mut inner = this.0.borrow_mut();
            if inner.finalized {
                return Err(Exception::throw_message(&ctx, "Decipher already finalized"));
            }
            if let CipherEngine::Cbc {
                auto_padding: ap, ..
            } = &mut inner.engine
            {
                *ap = auto_padding.0.unwrap_or(true);
            }
        }
        Ok(this.0)
    }
}

fn resolve_key_and_iv<'js>(
    ctx: &Ctx<'js>,
    algorithm: String,
    key: Value<'js>,
    iv: Value<'js>,
    options: Option<Value<'js>>,
) -> Result<CipherEngine> {
    let spec = parse_cipher_spec(&algorithm).map_err(|e| Exception::throw_message(ctx, &e))?;
    let key_bytes = if let Ok(ko) = Class::<KeyObject>::from_js(ctx, key.clone()) {
        match &ko.borrow().material {
            KeyMaterial::Secret(b) => b.clone(),
            _ => {
                return Err(Exception::throw_type(
                    ctx,
                    "Only secret KeyObject values are valid for symmetric ciphers",
                ))
            },
        }
    } else {
        value_to_bytes_with_encoding(ctx, &key, Some("utf8"), "key")?
    };

    let iv_bytes = if iv.is_null() {
        Vec::new()
    } else {
        value_to_bytes_with_encoding(ctx, &iv, Some("utf8"), "iv")?
    };

    let mut auth_tag_len: Option<usize> = None;
    if let Some(opt_val) = options {
        if !opt_val.is_undefined() && !opt_val.is_null() {
            let obj = opt_val.into_object().ok_or_else(|| {
                Exception::throw_type(ctx, "The \"options\" argument must be of type object")
            })?;
            if let Some(len) = obj.get_optional::<_, i32>("authTagLength")? {
                if len < 0 {
                    return Err(Exception::throw_range(ctx, "Invalid authTagLength"));
                }
                auth_tag_len = Some(len as usize);
            }
        }
    }

    CipherEngine::new(spec, &key_bytes, &iv_bytes, auth_tag_len)
        .map_err(|e| Exception::throw_message(ctx, &e))
}

pub fn create_cipheriv<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    key: Value<'js>,
    iv: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Class<'js, Cipheriv>> {
    let engine = resolve_key_and_iv(&ctx, algorithm, key, iv, options.0)?;
    Class::instance(
        ctx,
        Cipheriv {
            engine,
            finalized: false,
        },
    )
}

pub fn create_decipheriv<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    key: Value<'js>,
    iv: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Class<'js, Decipheriv>> {
    let engine = resolve_key_and_iv(&ctx, algorithm, key, iv, options.0)?;
    Class::instance(
        ctx,
        Decipheriv {
            engine,
            finalized: false,
        },
    )
}
