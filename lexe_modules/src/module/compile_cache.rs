// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! On-disk QuickJS bytecode compile cache backing `node:module`
//! `enableCompileCache`, `flushCompileCache`, `getCompileCacheDir`,
//! `NODE_COMPILE_CACHE`, and `NODE_DISABLE_COMPILE_CACHE`.

use std::{
    collections::HashMap,
    env,
    ffi::{c_void, CString},
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use rquickjs::{
    function::{Constructor, Opt},
    qjs, Ctx, Error, Function, Module, Object, Result, Value, WriteOptions,
};

const MAGIC: &[u8; 4] = b"LXCC";
const CACHE_VERSION: u32 = 1;
const KIND_CJS: u8 = 1;
const KIND_ESM: u8 = 2;

pub const STATUS_FAILED: i32 = 0;
pub const STATUS_ENABLED: i32 = 1;
pub const STATUS_ALREADY_ENABLED: i32 = 2;
pub const STATUS_DISABLED: i32 = 3;

#[derive(Default)]
struct CacheRuntime {
    disabled_by_env: bool,
    base_dir: Option<PathBuf>,
    versioned_dir: Option<PathBuf>,
    dirty: HashMap<String, Vec<u8>>,
    memory: HashMap<String, Vec<u8>>,
}

static COMPILE_CACHE: OnceLock<Mutex<CacheRuntime>> = OnceLock::new();

fn cache_lock() -> &'static Mutex<CacheRuntime> {
    COMPILE_CACHE.get_or_init(|| {
        let disabled = is_disabled_by_env();
        let mut rt = CacheRuntime {
            disabled_by_env: disabled,
            ..Default::default()
        };
        if !disabled {
            if let Ok(dir) = env::var("NODE_COMPILE_CACHE") {
                if !dir.is_empty() {
                    let _ = enable_in_runtime(&mut rt, Some(dir));
                }
            }
        }
        Mutex::new(rt)
    })
}

fn is_disabled_by_env() -> bool {
    env::var("NODE_DISABLE_COMPILE_CACHE")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

fn runtime_tag() -> String {
    format!(
        "lexe-{}-{}-{}",
        env!("CARGO_PKG_VERSION"),
        env::consts::ARCH,
        env::consts::OS
    )
}

fn enable_in_runtime(
    rt: &mut CacheRuntime,
    requested_dir: Option<String>,
) -> std::result::Result<(i32, Option<String>, Option<String>), String> {
    if rt.disabled_by_env || is_disabled_by_env() {
        rt.disabled_by_env = true;
        return Ok((
            STATUS_DISABLED,
            None,
            Some("Disabled by NODE_DISABLE_COMPILE_CACHE".to_string()),
        ));
    }

    if let Some(ref versioned) = rt.versioned_dir {
        return Ok((
            STATUS_ALREADY_ENABLED,
            Some(versioned.to_string_lossy().into_owned()),
            None,
        ));
    }

    let base_str = match requested_dir {
        Some(d) => {
            if d.is_empty() {
                return Err("Cache directory path must not be empty".to_string());
            }
            d
        },
        None => match env::var("NODE_COMPILE_CACHE") {
            Ok(d) if !d.is_empty() => d,
            _ => env::temp_dir()
                .join("node-compile-cache")
                .to_string_lossy()
                .into_owned(),
        },
    };

    let base_path = PathBuf::from(&base_str);
    let resolved_base = if base_path.is_absolute() {
        base_path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(base_path)
    };
    let versioned_path = resolved_base.join(runtime_tag());

    fs::create_dir_all(&versioned_path)
        .map_err(|e| format!("Failed to enable compile cache: {e}"))?;

    let base_out = resolved_base.to_string_lossy().into_owned();
    rt.base_dir = Some(resolved_base);
    rt.versioned_dir = Some(versioned_path);
    Ok((STATUS_ENABLED, Some(base_out), None))
}

fn throw_type_code(ctx: &Ctx<'_>, code: &str, message: &str) -> Error {
    if let Ok(ctor) = ctx.globals().get::<_, Constructor>("TypeError") {
        if let Ok(err) = ctor.construct::<_, Object>((message,)) {
            let _ = err.set("code", code);
            return ctx.throw(err.into_value());
        }
    }
    rquickjs::Exception::throw_type(ctx, message)
}

/// JS `module.enableCompileCache([cacheDir | options])`.
pub fn enable_compile_cache<'js>(ctx: Ctx<'js>, cache_dir: Opt<Value<'js>>) -> Result<Object<'js>> {
    let dir_arg = match cache_dir.0 {
        None => None,
        Some(v) if v.is_undefined() => None,
        Some(v) => {
            if let Some(s) = v.as_string() {
                Some(s.to_string()?)
            } else if let Some(obj) = v.as_object() {
                if v.is_array() {
                    return Err(throw_type_code(
                        &ctx,
                        "ERR_INVALID_ARG_TYPE",
                        "cacheDir should be a string",
                    ));
                }
                if let Ok(d) = obj.get::<_, Value>("directory") {
                    if d.is_undefined() {
                        None
                    } else if let Some(s) = d.as_string() {
                        Some(s.to_string()?)
                    } else {
                        return Err(throw_type_code(
                            &ctx,
                            "ERR_INVALID_ARG_TYPE",
                            "options.directory should be a string",
                        ));
                    }
                } else {
                    None
                }
            } else {
                return Err(throw_type_code(
                    &ctx,
                    "ERR_INVALID_ARG_TYPE",
                    "cacheDir should be a string",
                ));
            }
        },
    };

    let outcome = {
        let mut rt = cache_lock().lock().unwrap();
        enable_in_runtime(&mut rt, dir_arg)
    };

    let res = Object::new(ctx)?;
    match outcome {
        Ok((status, directory, message)) => {
            res.set("status", status)?;
            if let Some(dir) = directory {
                res.set("directory", dir)?;
            }
            if let Some(msg) = message {
                res.set("message", msg)?;
            }
        },
        Err(msg) => {
            res.set("status", STATUS_FAILED)?;
            res.set("message", msg)?;
        },
    }
    Ok(res)
}

/// JS `module.flushCompileCache([keepDeserializedCache])`.
pub fn flush_compile_cache(ctx: Ctx<'_>, keep: Opt<Value<'_>>) -> Result<()> {
    let keep_mem = match keep.0 {
        None => false,
        Some(v) if v.is_undefined() => false,
        Some(v) => match v.as_bool() {
            Some(b) => b,
            None => {
                return Err(throw_type_code(
                    &ctx,
                    "ERR_INVALID_ARG_TYPE",
                    "keepDeserializedCache should be a boolean",
                ));
            },
        },
    };

    flush_dirty_to_disk(keep_mem);
    Ok(())
}

/// JS `module.getCompileCacheDir()`.
pub fn get_compile_cache_dir(ctx: Ctx<'_>) -> Value<'_> {
    let rt = cache_lock().lock().unwrap();
    if rt.disabled_by_env {
        return Value::new_undefined(ctx);
    }
    match rt.versioned_dir {
        Some(ref dir) => rquickjs::String::from_str(ctx.clone(), &dir.to_string_lossy())
            .map(|s| s.into_value())
            .unwrap_or_else(|_| Value::new_undefined(ctx)),
        None => Value::new_undefined(ctx),
    }
}

pub fn flush_dirty_to_disk(keep_memory_cache: bool) {
    let (dir_opt, dirty_items) = {
        let mut rt = cache_lock().lock().unwrap();
        let dirty = std::mem::take(&mut rt.dirty);
        if !keep_memory_cache {
            rt.memory.clear();
        }
        (rt.versioned_dir.clone(), dirty)
    };

    let Some(dir) = dir_opt else {
        return;
    };
    if dirty_items.is_empty() {
        return;
    }
    let _ = fs::create_dir_all(&dir);
    for (key, blob) in dirty_items {
        let final_path = dir.join(format!("{key}.lxb"));
        let tmp_path = dir.join(format!("{key}.{}.tmp", std::process::id()));
        if fs::write(&tmp_path, &blob).is_ok() && fs::rename(&tmp_path, &final_path).is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
    }
}

pub fn is_enabled() -> bool {
    let rt = cache_lock().lock().unwrap();
    !rt.disabled_by_env && rt.versioned_dir.is_some()
}

/// Touch cache lock during module subsystem init so `NODE_COMPILE_CACHE` activates eagerly.
pub fn init_from_env() {
    let _ = cache_lock();
}

fn encode_entry(kind: u8, digest: &[u8; 32], code_len: usize, bytecode: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 4 + 1 + 32 + 8 + bytecode.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(digest);
    out.extend_from_slice(&(code_len as u64).to_le_bytes());
    out.extend_from_slice(bytecode);
    out
}

fn decode_entry<'a>(
    blob: &'a [u8],
    expected_kind: u8,
    expected_digest: &[u8; 32],
    expected_len: usize,
) -> Option<&'a [u8]> {
    if blob.len() < 49 {
        return None;
    }
    if &blob[0..4] != MAGIC {
        return None;
    }
    let ver = u32::from_le_bytes(blob[4..8].try_into().ok()?);
    if ver != CACHE_VERSION || blob[8] != expected_kind {
        return None;
    }
    if &blob[9..41] != expected_digest {
        return None;
    }
    let stored_len = u64::from_le_bytes(blob[41..49].try_into().ok()?) as usize;
    if stored_len != expected_len {
        return None;
    }
    Some(&blob[49..])
}

fn cache_key_and_digest(kind: u8, filename: &str, source_bytes: &[u8]) -> (String, [u8; 32]) {
    let mut hasher = Sha256::new();
    hasher.update(&[kind]);
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(b"\0");
    hasher.update(filename.as_bytes());
    hasher.update(b"\0");
    hasher.update(&(source_bytes.len() as u64).to_le_bytes());
    hasher.update(source_bytes);
    let digest = hasher.finalize();
    let hex = to_hex(&digest);
    (hex, digest)
}

fn read_cached_blob(key: &str) -> Option<(Vec<u8>, Option<PathBuf>)> {
    let dir_opt = {
        let rt = cache_lock().lock().unwrap();
        if let Some(mem) = rt.memory.get(key) {
            return Some((mem.clone(), None));
        }
        if let Some(dirty) = rt.dirty.get(key) {
            return Some((dirty.clone(), None));
        }
        rt.versioned_dir.clone()
    };
    let dir = dir_opt?;
    let path = dir.join(format!("{key}.lxb"));
    let data = fs::read(&path).ok()?;
    Some((data, Some(path)))
}

fn store_cached_blob(key: String, blob: Vec<u8>) {
    let mut rt = cache_lock().lock().unwrap();
    if rt.disabled_by_env || rt.versioned_dir.is_none() {
        return;
    }
    rt.memory.insert(key.clone(), blob.clone());
    rt.dirty.insert(key, blob);
}

fn eval_global_function_from_bytecode<'js>(ctx: &Ctx<'js>, bc: &[u8]) -> Option<Function<'js>> {
    let ctx_raw = ctx.as_raw().as_ptr();
    let obj = unsafe {
        qjs::JS_ReadObject(
            ctx_raw,
            bc.as_ptr(),
            bc.len() as qjs::size_t,
            qjs::JS_READ_OBJ_BYTECODE as std::ffi::c_int,
        )
    };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(obj) == qjs::JS_TAG_EXCEPTION } {
        unsafe {
            qjs::JS_FreeValue(ctx_raw, obj);
            let exc = qjs::JS_GetException(ctx_raw);
            qjs::JS_FreeValue(ctx_raw, exc);
        }
        return None;
    }
    let fn_val = unsafe { qjs::JS_EvalFunction(ctx_raw, obj) };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(fn_val) == qjs::JS_TAG_EXCEPTION } {
        unsafe {
            qjs::JS_FreeValue(ctx_raw, fn_val);
            let exc = qjs::JS_GetException(ctx_raw);
            qjs::JS_FreeValue(ctx_raw, exc);
        }
        return None;
    }
    let val = unsafe { Value::from_raw(ctx.clone(), fn_val) };
    val.into_function()
}

/// Compile or load cached QuickJS bytecode for a CommonJS module wrapper function.
pub fn compile_cjs_with_cache<'js>(
    ctx: &Ctx<'js>,
    wrapped_code: &str,
    filename: &str,
) -> Result<Function<'js>> {
    let enabled = is_enabled();
    let (key, digest) = if enabled {
        cache_key_and_digest(KIND_CJS, filename, wrapped_code.as_bytes())
    } else {
        (String::new(), [0u8; 32])
    };

    if enabled {
        if let Some((blob, disk_path)) = read_cached_blob(&key) {
            if let Some(bc) = decode_entry(&blob, KIND_CJS, &digest, wrapped_code.len()) {
                if let Some(func) = eval_global_function_from_bytecode(ctx, bc) {
                    return Ok(func);
                }
            }
            if let Some(path) = disk_path {
                let _ = fs::remove_file(path);
            }
        }
    }

    let c_code = CString::new(wrapped_code).map_err(|_| {
        rquickjs::Exception::throw_syntax(ctx, "NUL byte in CommonJS module source")
    })?;
    let c_file = CString::new(filename).map_err(|_| {
        rquickjs::Exception::throw_syntax(ctx, "NUL byte in CommonJS module filename")
    })?;
    let ctx_raw = ctx.as_raw().as_ptr();
    let mut eval_opts = qjs::JSEvalOptions {
        version: qjs::JS_EVAL_OPTIONS_VERSION as std::ffi::c_int,
        eval_flags: (qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as std::ffi::c_int,
        filename: c_file.as_ptr(),
        line_num: 1,
    };
    let compiled = unsafe {
        qjs::JS_Eval2(
            ctx_raw,
            c_code.as_ptr(),
            wrapped_code.len() as qjs::size_t,
            &mut eval_opts,
        )
    };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(compiled) == qjs::JS_TAG_EXCEPTION } {
        unsafe { qjs::JS_FreeValue(ctx_raw, compiled) };
        return Err(Error::Exception);
    }

    if enabled {
        let mut size: qjs::size_t = 0;
        let ptr = unsafe {
            qjs::JS_WriteObject(
                ctx_raw,
                &mut size,
                compiled,
                qjs::JS_WRITE_OBJ_BYTECODE as std::ffi::c_int,
            )
        };
        if !ptr.is_null() {
            let bc = unsafe { std::slice::from_raw_parts(ptr, size as usize) };
            let blob = encode_entry(KIND_CJS, &digest, wrapped_code.len(), bc);
            unsafe { qjs::js_free(ctx_raw, ptr as *mut c_void) };
            store_cached_blob(key, blob);
        }
    }

    let fn_val = unsafe { qjs::JS_EvalFunction(ctx_raw, compiled) };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(fn_val) == qjs::JS_TAG_EXCEPTION } {
        unsafe { qjs::JS_FreeValue(ctx_raw, fn_val) };
        return Err(Error::Exception);
    }
    let val = unsafe { Value::from_raw(ctx.clone(), fn_val) };
    val.into_function()
        .ok_or_else(|| rquickjs::Exception::throw_type(ctx, "Compiled CJS wrapper is not callable"))
}

/// Declare an ESM `Module` using the on-disk compile cache when enabled.
pub fn declare_esm_with_cache<'js>(
    ctx: &Ctx<'js>,
    module_name: &str,
    source: &[u8],
) -> Result<Module<'js>> {
    let enabled = is_enabled();
    let (key, digest) = if enabled {
        cache_key_and_digest(KIND_ESM, module_name, source)
    } else {
        (String::new(), [0u8; 32])
    };

    if enabled {
        if let Some((blob, disk_path)) = read_cached_blob(&key) {
            if let Some(bc) = decode_entry(&blob, KIND_ESM, &digest, source.len()) {
                if let Ok(module) = unsafe { Module::load(ctx.clone(), bc) } {
                    return Ok(module);
                }
            }
            if let Some(path) = disk_path {
                let _ = fs::remove_file(path);
            }
        }
    }

    let module = Module::declare(ctx.clone(), module_name, source)?;
    if enabled {
        if let Ok(bc) = module.write(WriteOptions::default()) {
            let blob = encode_entry(KIND_ESM, &digest, source.len(), &bc);
            store_cached_blob(key, blob);
        }
    }
    Ok(module)
}

fn to_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Minimal FIPS 180-4 SHA-256 so `lexe_modules` does not depend on optional `lexe_crypto`.
struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_bits: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total_bits: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_bits = self.total_bits.wrapping_add((data.len() as u64) * 8);
        while !data.is_empty() {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            self.buf[self.buf_len..].fill(0);
            let block = self.buf;
            self.compress(&block);
            self.buf_len = 0;
        }
        self.buf[self.buf_len..56].fill(0);
        self.buf[56..64].copy_from_slice(&self.total_bits.to_be_bytes());
        let block = self.buf;
        self.compress(&block);

        let mut out = [0u8; 32];
        for (i, w) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty_vector() {
        let h = Sha256::new().finalize();
        assert_eq!(
            to_hex(&h),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn roundtrips_entry_encoding() {
        let (_, dig) = cache_key_and_digest(KIND_CJS, "a.js", b"code");
        let blob = encode_entry(KIND_CJS, &dig, 4, b"bc123");
        assert_eq!(decode_entry(&blob, KIND_CJS, &dig, 4), Some(&b"bc123"[..]));
        assert_eq!(decode_entry(&blob, KIND_ESM, &dig, 4), None);
        assert_eq!(decode_entry(&blob, KIND_CJS, &dig, 5), None);
    }
}
