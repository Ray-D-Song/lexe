// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_buffer::Buffer;
use lexe_compression::raw_zlib::{RawZlibDecoder, RawZlibEncoder, ZlibMode, ZlibOptions};
use lexe_context::CtxExtension;
use lexe_utils::{bytes::ObjectBytes, object::ObjectExt, result::ResultExt};
use rquickjs::{
    prelude::{Opt, Rest},
    Ctx, Error, Exception, Function, IntoJs, Null, Result, Value,
};

use super::{define_cb_function, define_sync_function};

fn throw_coded<'js>(ctx: &Ctx<'js>, err_name: &str, code: &str, msg: &str) -> Error {
    let ctor: Function<'js> = match ctx.eval(err_name) {
        Ok(f) => f,
        Err(err) => return err,
    };
    let Ok(val): Result<Value<'js>> = ctor.call((msg,)) else {
        return Exception::throw_message(ctx, msg);
    };
    if let Some(obj) = val.as_object() {
        let _ = obj.set("code", code);
    }
    match Exception::from_value(val) {
        Ok(exc) => exc.throw(),
        Err(err) => err,
    }
}

pub fn parse_zlib_options<'js>(
    ctx: &Ctx<'js>,
    options: Opt<Value<'js>>,
    is_encode: bool,
) -> Result<ZlibOptions> {
    let mut opts = ZlibOptions::default();
    let Some(val) = options.0 else {
        return Ok(opts);
    };
    if val.is_undefined() || val.is_null() {
        return Ok(opts);
    }

    if let Some(level) = val.get_optional::<_, i32>("level")? {
        if is_encode && !(-1..=9).contains(&level) {
            return Err(throw_coded(
                ctx,
                "RangeError",
                "ERR_OUT_OF_RANGE",
                "The value of \"options.level\" is out of range.",
            ));
        }
        opts.level = level;
    }
    if let Some(window_bits) = val.get_optional::<_, i32>("windowBits")? {
        if !(8..=15).contains(&window_bits) {
            return Err(throw_coded(
                ctx,
                "RangeError",
                "ERR_OUT_OF_RANGE",
                "The value of \"options.windowBits\" is out of range.",
            ));
        }
        opts.window_bits = window_bits;
    }
    if let Some(mem_level) = val.get_optional::<_, i32>("memLevel")? {
        if is_encode && !(1..=9).contains(&mem_level) {
            return Err(throw_coded(
                ctx,
                "RangeError",
                "ERR_OUT_OF_RANGE",
                "The value of \"options.memLevel\" is out of range.",
            ));
        }
        opts.mem_level = mem_level;
    }
    if let Some(strategy) = val.get_optional::<_, i32>("strategy")? {
        if is_encode && !(0..=4).contains(&strategy) {
            return Err(throw_coded(
                ctx,
                "RangeError",
                "ERR_OUT_OF_RANGE",
                "The value of \"options.strategy\" is out of range.",
            ));
        }
        opts.strategy = strategy;
    }
    if let Some(dict_val) = val.get_optional::<_, Value<'js>>("dictionary")? {
        if !dict_val.is_undefined() && !dict_val.is_null() {
            let dict = ObjectBytes::from(ctx, &dict_val)?;
            opts.dictionary = Some(dict.as_bytes(ctx)?.to_vec());
        }
    }

    Ok(opts)
}

enum ZlibCommand {
    Deflate,
    DeflateRaw,
    Gzip,
    Inflate,
    InflateRaw,
    Gunzip,
    Unzip,
}

fn zlib_converter<'js>(
    ctx: Ctx<'js>,
    bytes: ObjectBytes<'js>,
    options: Opt<Value<'js>>,
    command: ZlibCommand,
) -> Result<Value<'js>> {
    let src = bytes.as_bytes(&ctx)?;
    let is_encode = matches!(
        command,
        ZlibCommand::Deflate | ZlibCommand::DeflateRaw | ZlibCommand::Gzip
    );
    let opts = parse_zlib_options(&ctx, options, is_encode)?;

    let out = match command {
        ZlibCommand::Deflate => {
            let mut enc = RawZlibEncoder::new(ZlibMode::Deflate, opts)?;
            let mut dst = enc.compress_chunk(src)?;
            dst.extend(enc.finish()?);
            dst
        },
        ZlibCommand::DeflateRaw => {
            let mut enc = RawZlibEncoder::new(ZlibMode::DeflateRaw, opts)?;
            let mut dst = enc.compress_chunk(src)?;
            dst.extend(enc.finish()?);
            dst
        },
        ZlibCommand::Gzip => {
            let mut enc = RawZlibEncoder::new(ZlibMode::Gzip, opts)?;
            let mut dst = enc.compress_chunk(src)?;
            dst.extend(enc.finish()?);
            dst
        },
        ZlibCommand::Inflate => {
            let mut dec = RawZlibDecoder::new(ZlibMode::Inflate, opts)?;
            let mut dst = dec.decompress_chunk(src)?;
            dst.extend(dec.finish()?);
            dst
        },
        ZlibCommand::InflateRaw => {
            let mut dec = RawZlibDecoder::new(ZlibMode::InflateRaw, opts)?;
            let mut dst = dec.decompress_chunk(src)?;
            dst.extend(dec.finish()?);
            dst
        },
        ZlibCommand::Gunzip => {
            let mut dec = RawZlibDecoder::new(ZlibMode::Gunzip, opts)?;
            let mut dst = dec.decompress_chunk(src)?;
            dst.extend(dec.finish()?);
            dst
        },
        ZlibCommand::Unzip => {
            let mut dec = RawZlibDecoder::new(ZlibMode::Unzip, opts)?;
            let mut dst = dec.decompress_chunk(src)?;
            dst.extend(dec.finish()?);
            dst
        },
    };

    Buffer(out).into_js(&ctx)
}

pub(crate) fn crc32<'js>(
    ctx: Ctx<'js>,
    data: ObjectBytes<'js>,
    value: Opt<Value<'js>>,
) -> Result<f64> {
    let init = match value.0 {
        None => 0u32,
        Some(v) if v.is_undefined() => 0u32,
        Some(v) => {
            let n = if let Some(i) = v.as_int() {
                i as f64
            } else if let Some(f) = v.as_number() {
                f
            } else {
                return Err(throw_coded(
                    &ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_TYPE",
                    "The \"value\" argument must be of type number.",
                ));
            };
            if !n.is_finite() || n.fract() != 0.0 || n < 0.0 || n > u32::MAX as f64 {
                return Err(throw_coded(
                    &ctx,
                    "RangeError",
                    "ERR_OUT_OF_RANGE",
                    "The value of \"value\" is out of range. It must be >= 0 and <= 4294967295.",
                ));
            }
            n as u32
        },
    };

    let mut hasher = crc32fast::Hasher::new_with_initial(init);
    hasher.update(data.as_bytes(&ctx)?);
    Ok(f64::from(hasher.finalize()))
}

define_cb_function!(deflate, zlib_converter, ZlibCommand::Deflate);
define_sync_function!(deflate_sync, zlib_converter, ZlibCommand::Deflate);

define_cb_function!(deflate_raw, zlib_converter, ZlibCommand::DeflateRaw);
define_sync_function!(deflate_raw_sync, zlib_converter, ZlibCommand::DeflateRaw);

define_cb_function!(gzip, zlib_converter, ZlibCommand::Gzip);
define_sync_function!(gzip_sync, zlib_converter, ZlibCommand::Gzip);

define_cb_function!(inflate, zlib_converter, ZlibCommand::Inflate);
define_sync_function!(inflate_sync, zlib_converter, ZlibCommand::Inflate);

define_cb_function!(inflate_raw, zlib_converter, ZlibCommand::InflateRaw);
define_sync_function!(inflate_raw_sync, zlib_converter, ZlibCommand::InflateRaw);

define_cb_function!(gunzip, zlib_converter, ZlibCommand::Gunzip);
define_sync_function!(gunzip_sync, zlib_converter, ZlibCommand::Gunzip);

define_cb_function!(unzip, zlib_converter, ZlibCommand::Unzip);
define_sync_function!(unzip_sync, zlib_converter, ZlibCommand::Unzip);
