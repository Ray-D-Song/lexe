// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::io::Read;

use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_utils::{bytes::ObjectBytes, object::ObjectExt, result::ResultExt};
use rquickjs::{
    prelude::{Opt, Rest},
    Ctx, Error, Exception, Function, IntoJs, Null, Result, Value,
};

use super::{define_cb_function, define_sync_function};

enum ZlibCommand {
    Deflate,
    DeflateRaw,
    Gzip,
    Inflate,
    InflateRaw,
    Gunzip,
}

fn zlib_converter<'js>(
    ctx: Ctx<'js>,
    bytes: ObjectBytes<'js>,
    options: Opt<Value<'js>>,
    command: ZlibCommand,
) -> Result<Value<'js>> {
    let src = bytes.as_bytes(&ctx)?;

    let mut level = lexe_compression::zlib::Compression::default();
    if let Some(options) = options.0 {
        if let Some(opt) = options.get_optional("level")? {
            level = lexe_compression::zlib::Compression::new(opt);
        }
    }

    let mut dst: Vec<u8> = Vec::with_capacity(src.len());

    let _ = match command {
        ZlibCommand::Deflate => {
            lexe_compression::zlib::encoder(src, level).read_to_end(&mut dst)?
        },
        ZlibCommand::DeflateRaw => {
            lexe_compression::deflate::encoder(src, level).read_to_end(&mut dst)?
        },
        ZlibCommand::Gzip => lexe_compression::gz::encoder(src, level).read_to_end(&mut dst)?,
        ZlibCommand::Inflate => lexe_compression::zlib::decoder(src).read_to_end(&mut dst)?,
        ZlibCommand::InflateRaw => lexe_compression::deflate::decoder(src).read_to_end(&mut dst)?,
        ZlibCommand::Gunzip => lexe_compression::gz::decoder(src).read_to_end(&mut dst)?,
    };

    Buffer(dst).into_js(&ctx)
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
