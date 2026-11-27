// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `pbkdf2`, `pbkdf2Sync`, `scrypt`, and `scryptSync`.

use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_utils::{error::ErrorExtensions, result::ResultExt};
use rquickjs::{function::Opt, Ctx, Exception, Function, IntoJs, Null, Result, Value};
use scrypt::{scrypt as scrypt_kdf, Params as ScryptParams};

use crate::{
    hash::HashAlgorithm,
    key_object::{throw_code_error, value_to_bytes_with_encoding},
    provider::CryptoProvider,
    CRYPTO_PROVIDER,
};

const DEFAULT_SCRYPT_N: u32 = 16_384;
const DEFAULT_SCRYPT_R: u32 = 8;
const DEFAULT_SCRYPT_P: u32 = 1;
const DEFAULT_SCRYPT_MAXMEM: u64 = 32 * 1024 * 1024;
const MAX_KEYLEN: i64 = i32::MAX as i64;

fn parse_non_neg_keylen(ctx: &Ctx<'_>, keylen: i64) -> Result<usize> {
    if !(0..=MAX_KEYLEN).contains(&keylen) {
        return throw_code_error(
            ctx,
            "RangeError",
            "ERR_OUT_OF_RANGE",
            "The value of \"keylen\" is out of range. It must be >= 0 and <= 2147483647",
        );
    }
    Ok(keylen as usize)
}

type ParsedPbkdf2Args = (Vec<u8>, Vec<u8>, u32, usize, HashAlgorithm);

fn parse_pbkdf2_args<'js>(
    ctx: &Ctx<'js>,
    password: Value<'js>,
    salt: Value<'js>,
    iterations: i64,
    keylen: i64,
    digest: Value<'js>,
) -> Result<ParsedPbkdf2Args> {
    let pw = value_to_bytes_with_encoding(ctx, &password, Some("utf8"), "password")?;
    let salt_bytes = value_to_bytes_with_encoding(ctx, &salt, Some("utf8"), "salt")?;
    if !(1..=MAX_KEYLEN).contains(&iterations) {
        return throw_code_error(
            ctx,
            "RangeError",
            "ERR_OUT_OF_RANGE",
            "The value of \"iterations\" is out of range. It must be >= 1 and <= 2147483647",
        );
    }
    let out_len = parse_non_neg_keylen(ctx, keylen)?;
    let digest_str = digest
        .as_string()
        .ok_or_else(|| {
            Exception::throw_type(ctx, "The \"digest\" argument must be of type string")
        })?
        .to_string()?;
    let hash_alg = HashAlgorithm::try_from(digest_str.as_str())
        .map_err(|_| Exception::throw_type(ctx, &format!("Invalid digest: {digest_str}")))?;
    if matches!(hash_alg, HashAlgorithm::Md5) {
        return Err(Exception::throw_type(
            ctx,
            "Digest 'md5' is not supported for PBKDF2",
        ));
    }
    Ok((pw, salt_bytes, iterations as u32, out_len, hash_alg))
}

pub fn pbkdf2_sync<'js>(
    ctx: Ctx<'js>,
    password: Value<'js>,
    salt: Value<'js>,
    iterations: i64,
    keylen: i64,
    digest: Value<'js>,
) -> Result<Buffer> {
    let (pw, salt_bytes, iters, out_len, hash_alg) =
        parse_pbkdf2_args(&ctx, password, salt, iterations, keylen, digest)?;
    if out_len == 0 {
        return Ok(Buffer(Vec::new()));
    }
    let derived = CRYPTO_PROVIDER
        .pbkdf2_derive_key(&pw, &salt_bytes, iters, out_len, hash_alg)
        .or_throw(&ctx)?;
    Ok(Buffer(derived))
}

pub fn pbkdf2<'js>(
    ctx: Ctx<'js>,
    password: Value<'js>,
    salt: Value<'js>,
    iterations: i64,
    keylen: i64,
    digest: Value<'js>,
    callback: Function<'js>,
) -> Result<()> {
    let (pw, salt_bytes, iters, out_len, hash_alg) =
        parse_pbkdf2_args(&ctx, password, salt, iterations, keylen, digest)?;
    ctx.clone().spawn_exit(async move {
        let res = tokio::task::spawn_blocking(move || {
            if out_len == 0 {
                Ok(Vec::new())
            } else {
                CRYPTO_PROVIDER
                    .pbkdf2_derive_key(&pw, &salt_bytes, iters, out_len, hash_alg)
                    .map_err(|e| format!("{e}"))
            }
        })
        .await
        .map_err(|e| format!("PBKDF2 task panicked: {e}"))
        .and_then(|r| r);

        match res {
            Ok(bytes) => {
                let _: () = callback.call((Null.into_js(&ctx), Buffer(bytes)))?;
            },
            Err(msg) => {
                let err = Exception::throw_message(&ctx, &msg).into_value(&ctx)?;
                let _: () = callback.call((err,))?;
            },
        }
        Ok::<_, rquickjs::Error>(())
    })?;
    Ok(())
}

struct ParsedScryptArgs {
    pw: Vec<u8>,
    salt: Vec<u8>,
    keylen: usize,
    log_n: u8,
    r: u32,
    p: u32,
}

fn read_opt_u64<'js>(
    ctx: &Ctx<'js>,
    obj: &rquickjs::Object<'js>,
    key: &str,
) -> Result<Option<u64>> {
    let val: Value = obj.get(key)?;
    if val.is_undefined() || val.is_null() {
        return Ok(None);
    }
    let Some(num) = val.as_number() else {
        return throw_code_error(
            ctx,
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!("The \"options.{key}\" property must be of type number"),
        );
    };
    if !num.is_finite() || num < 0.0 || num.fract() != 0.0 {
        return throw_code_error(
            ctx,
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!("The value of \"options.{key}\" is out of range"),
        );
    }
    Ok(Some(num as u64))
}

fn parse_scrypt_args<'js>(
    ctx: &Ctx<'js>,
    password: Value<'js>,
    salt: Value<'js>,
    keylen: i64,
    options: Option<Value<'js>>,
) -> Result<ParsedScryptArgs> {
    let pw = value_to_bytes_with_encoding(ctx, &password, Some("utf8"), "password")?;
    let salt_bytes = value_to_bytes_with_encoding(ctx, &salt, Some("utf8"), "salt")?;
    let out_len = parse_non_neg_keylen(ctx, keylen)?;

    let mut cost_n = DEFAULT_SCRYPT_N as u64;
    let mut r = DEFAULT_SCRYPT_R as u64;
    let mut p = DEFAULT_SCRYPT_P as u64;
    let mut maxmem = DEFAULT_SCRYPT_MAXMEM;

    if let Some(opt_val) = options {
        if !opt_val.is_undefined() && !opt_val.is_null() {
            let obj = opt_val.into_object().ok_or_else(|| {
                Exception::throw_type(ctx, "The \"options\" argument must be of type object")
            })?;
            let opt_cost = read_opt_u64(ctx, &obj, "cost")?;
            let opt_n = read_opt_u64(ctx, &obj, "N")?;
            if opt_cost.is_some() && opt_n.is_some() {
                return throw_code_error(
                    ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_VALUE",
                    "Cannot specify both options.cost and options.N",
                );
            }
            if let Some(v) = opt_cost.or(opt_n) {
                cost_n = v;
            }
            let opt_bs = read_opt_u64(ctx, &obj, "blockSize")?;
            let opt_r = read_opt_u64(ctx, &obj, "r")?;
            if opt_bs.is_some() && opt_r.is_some() {
                return throw_code_error(
                    ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_VALUE",
                    "Cannot specify both options.blockSize and options.r",
                );
            }
            if let Some(v) = opt_bs.or(opt_r) {
                r = v;
            }
            let opt_par = read_opt_u64(ctx, &obj, "parallelization")?;
            let opt_p = read_opt_u64(ctx, &obj, "p")?;
            if opt_par.is_some() && opt_p.is_some() {
                return throw_code_error(
                    ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_VALUE",
                    "Cannot specify both options.parallelization and options.p",
                );
            }
            if let Some(v) = opt_par.or(opt_p) {
                p = v;
            }
            if let Some(v) = read_opt_u64(ctx, &obj, "maxmem")? {
                maxmem = v;
            }
        }
    }

    if cost_n <= 1 || !cost_n.is_power_of_two() || r == 0 || p == 0 {
        return throw_code_error(
            ctx,
            "RangeError",
            "ERR_CRYPTO_INVALID_SCRYPT_PARAMS",
            "Invalid scrypt params",
        );
    }
    let log_n = cost_n.trailing_zeros() as u8;
    let r_u32 =
        u32::try_from(r).map_err(|_| Exception::throw_range(ctx, "Invalid scrypt blockSize"))?;
    let p_u32 = u32::try_from(p)
        .map_err(|_| Exception::throw_range(ctx, "Invalid scrypt parallelization"))?;

    let mem_needed = 128u128
        .saturating_mul(cost_n as u128)
        .saturating_mul(r as u128);
    if mem_needed > maxmem as u128 || ScryptParams::new(log_n, r_u32, p_u32).is_err() {
        return throw_code_error(
            ctx,
            "RangeError",
            "ERR_CRYPTO_INVALID_SCRYPT_PARAMS",
            "Memory limit exceeded",
        );
    }

    Ok(ParsedScryptArgs {
        pw,
        salt: salt_bytes,
        keylen: out_len,
        log_n,
        r: r_u32,
        p: p_u32,
    })
}

fn run_scrypt(args: &ParsedScryptArgs) -> std::result::Result<Vec<u8>, String> {
    if args.keylen == 0 {
        return Ok(Vec::new());
    }
    let params = ScryptParams::new(args.log_n, args.r, args.p)
        .map_err(|_| "Invalid scrypt params".to_string())?;
    let mut out = vec![0u8; args.keylen];
    scrypt_kdf(&args.pw, &args.salt, &params, &mut out)
        .map_err(|e| format!("scrypt failed: {e}"))?;
    Ok(out)
}

pub fn scrypt_sync<'js>(
    ctx: Ctx<'js>,
    password: Value<'js>,
    salt: Value<'js>,
    keylen: i64,
    options: Opt<Value<'js>>,
) -> Result<Buffer> {
    let args = parse_scrypt_args(&ctx, password, salt, keylen, options.0)?;
    let out = run_scrypt(&args).or_throw(&ctx)?;
    Ok(Buffer(out))
}

pub fn scrypt<'js>(
    ctx: Ctx<'js>,
    password: Value<'js>,
    salt: Value<'js>,
    keylen: i64,
    options_or_cb: Value<'js>,
    cb_opt: Opt<Function<'js>>,
) -> Result<()> {
    let (options, callback) = if let Some(cb) = cb_opt.0 {
        (Some(options_or_cb), cb)
    } else {
        let cb = options_or_cb.into_function().ok_or_else(|| {
            Exception::throw_type(&ctx, "The \"callback\" argument must be of type function")
        })?;
        (None, cb)
    };
    let args = parse_scrypt_args(&ctx, password, salt, keylen, options)?;
    ctx.clone().spawn_exit(async move {
        let res = tokio::task::spawn_blocking(move || run_scrypt(&args))
            .await
            .map_err(|e| format!("scrypt task panicked: {e}"))
            .and_then(|r| r);

        match res {
            Ok(bytes) => {
                let _: () = callback.call((Null.into_js(&ctx), Buffer(bytes)))?;
            },
            Err(msg) => {
                let err = Exception::throw_message(&ctx, &msg).into_value(&ctx)?;
                let _: () = callback.call((err,))?;
            },
        }
        Ok::<_, rquickjs::Error>(())
    })?;
    Ok(())
}
