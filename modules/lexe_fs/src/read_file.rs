// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::io::Read;

use either::Either;
use lexe_buffer::Buffer;
use lexe_utils::object::ObjectExt;
use rquickjs::{function::Opt, Ctx, Error, FromJs, IntoJs, Result, Value};
use tokio::fs;

use crate::errors::throw_fs_error;
use crate::fd_sync;
use crate::realpath::path_from_value;

pub(crate) enum FileTarget {
    Fd(i32),
    Path(String),
}

pub(crate) fn parse_file_target<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<FileTarget> {
    if let Some(fd) = value.as_int() {
        return Ok(FileTarget::Fd(fd));
    }
    if let Some(n) = value.as_number() {
        if n.is_finite() && n.fract() == 0.0 {
            return Ok(FileTarget::Fd(n as i32));
        }
    }
    path_from_value(ctx, value).map(FileTarget::Path)
}

pub async fn read_file<'js>(
    ctx: Ctx<'js>,
    path: Value<'js>,
    options: Opt<Option<Either<String, ReadFileOptions>>>,
) -> Result<Value<'js>> {
    let bytes = match parse_file_target(&ctx, path)? {
        FileTarget::Fd(fd) => fd_sync::with_fd(&ctx, fd, "read", |entry| {
            let mut buf = Vec::new();
            entry.file.read_to_end(&mut buf)?;
            Ok(buf)
        })?,
        FileTarget::Path(path) => {
            lexe_utils::permissions::ensure_fs_read(&ctx, &path)?;
            fs::read(&path)
                .await
                .map_err(|err| throw_fs_error(&ctx, err, "open", &path))?
        },
    };

    handle_read_file_bytes(&ctx, options, bytes)
}

pub fn read_file_sync<'js>(
    ctx: Ctx<'js>,
    path: Value<'js>,
    options: Opt<Option<Either<String, ReadFileOptions>>>,
) -> Result<Value<'js>> {
    let bytes = match parse_file_target(&ctx, path)? {
        FileTarget::Fd(fd) => fd_sync::with_fd(&ctx, fd, "read", |entry| {
            let mut buf = Vec::new();
            entry.file.read_to_end(&mut buf)?;
            Ok(buf)
        })?,
        FileTarget::Path(path) => {
            lexe_utils::permissions::ensure_fs_read(&ctx, &path)?;
            std::fs::read(&path).map_err(|err| throw_fs_error(&ctx, err, "open", &path))?
        },
    };

    handle_read_file_bytes(&ctx, options, bytes)
}

pub(crate) fn handle_read_file_bytes<'a>(
    ctx: &Ctx<'a>,
    options: Opt<Option<Either<String, ReadFileOptions>>>,
    bytes: Vec<u8>,
) -> Result<Value<'a>> {
    let buffer = Buffer(bytes);

    if let Some(options) = options.0.flatten() {
        let encoding = match options {
            Either::Left(encoding) => Some(encoding),
            Either::Right(options) => options.encoding,
        };

        if let Some(encoding) = encoding {
            return buffer
                .to_string(ctx, &encoding)
                .and_then(|s| s.into_js(ctx));
        }
    }

    buffer.into_js(ctx)
}

pub(crate) struct ReadFileOptions {
    pub encoding: Option<String>,
}

impl<'js> FromJs<'js> for ReadFileOptions {
    fn from_js(_ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let ty_name = value.type_name();
        let obj = value
            .as_object()
            .ok_or(Error::new_from_js(ty_name, "Object"))?;

        let encoding = obj.get_optional::<_, String>("encoding")?;

        Ok(Self { encoding })
    }
}
