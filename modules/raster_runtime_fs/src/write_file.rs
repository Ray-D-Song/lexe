// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::errors::throw_fs_error;
use raster_runtime_utils::{bytes::ObjectBytes, object::ObjectExt};
use rquickjs::{function::Opt, Ctx, Error, Exception, FromJs, Result, Value};
use tokio::io::AsyncWriteExt;

/// Default mode for a newly created file, matching Node.
const DEFAULT_MODE: u32 = 0o666;

pub(crate) struct WriteFileOptions {
    pub mode: Option<u32>,
    pub flag: Option<String>,
    pub encoding: Option<String>,
}

impl<'js> FromJs<'js> for WriteFileOptions {
    fn from_js(_ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let ty_name = value.type_name();
        let obj = value
            .as_object()
            .ok_or(Error::new_from_js(ty_name, "Object"))?;

        let mode = obj.get_optional::<_, u32>("mode")?;
        let flag = obj.get_optional::<_, String>("flag")?;
        let encoding = obj.get_optional::<_, String>("encoding")?;

        Ok(Self {
            mode,
            flag,
            encoding,
        })
    }
}

/// Resolve the `options` argument of `writeFile`/`appendFile`: either an
/// encoding string or an options object. `undefined`/`null` mean "use defaults",
/// which is what a callback-style call passes when `options` is omitted.
fn resolve_options<'js>(
    ctx: &Ctx<'js>,
    options: Opt<Value<'js>>,
    default_flag: &str,
) -> Result<(Option<String>, Option<u32>, String)> {
    let Some(value) = options.0 else {
        return Ok((None, None, default_flag.to_string()));
    };
    if value.is_undefined() || value.is_null() {
        return Ok((None, None, default_flag.to_string()));
    }
    if let Some(encoding) = value.as_string() {
        return Ok((Some(encoding.to_string()?), None, default_flag.to_string()));
    }
    let opts = WriteFileOptions::from_js(ctx, value)?;
    let flag = opts.flag.unwrap_or_else(|| default_flag.to_string());
    Ok((opts.encoding, opts.mode, flag))
}

/// The flag combinations `writeFile` accepts, mapped the same way `open` does.
fn open_options(flag: &str) -> Option<tokio::fs::OpenOptions> {
    let mut o = tokio::fs::OpenOptions::new();
    match flag {
        "a" => {
            o.append(true).create(true);
            Some(o)
        },
        "ax" => {
            o.append(true).create_new(true);
            Some(o)
        },
        "a+" => {
            o.append(true).read(true).create(true);
            Some(o)
        },
        "r" => {
            o.read(true);
            Some(o)
        },
        "r+" => {
            o.read(true).write(true);
            Some(o)
        },
        "w" => {
            o.write(true).create(true).truncate(true);
            Some(o)
        },
        "wx" => {
            o.write(true).create_new(true);
            Some(o)
        },
        "w+" => {
            o.write(true).read(true).create(true).truncate(true);
            Some(o)
        },
        "wx+" => {
            o.write(true).read(true).create_new(true);
            Some(o)
        },
        _ => None,
    }
}

fn sync_open_options(flag: &str) -> Option<std::fs::OpenOptions> {
    let mut o = std::fs::OpenOptions::new();
    match flag {
        "a" => {
            o.append(true).create(true);
            Some(o)
        },
        "ax" => {
            o.append(true).create_new(true);
            Some(o)
        },
        "a+" => {
            o.append(true).read(true).create(true);
            Some(o)
        },
        "r" => {
            o.read(true);
            Some(o)
        },
        "r+" => {
            o.read(true).write(true);
            Some(o)
        },
        "w" => {
            o.write(true).create(true).truncate(true);
            Some(o)
        },
        "wx" => {
            o.write(true).create_new(true);
            Some(o)
        },
        "w+" => {
            o.write(true).read(true).create(true).truncate(true);
            Some(o)
        },
        "wx+" => {
            o.write(true).read(true).create_new(true);
            Some(o)
        },
        _ => None,
    }
}

/// Build a `TypeError` carrying `code`, the way Node reports argument errors.
fn throw_type_error_with_code<'js>(ctx: &Ctx<'js>, code: &str, message: &str) -> rquickjs::Error {
    let Ok(type_error) = ctx.eval::<rquickjs::Function, _>("TypeError") else {
        return Exception::throw_message(ctx, message);
    };
    let Ok(instance): std::result::Result<rquickjs::Value, _> = type_error.call((message,)) else {
        return Exception::throw_message(ctx, message);
    };
    if let Some(obj) = instance.as_object() {
        let _ = obj.set("code", code);
    }
    match Exception::from_value(instance) {
        Ok(exception) => exception.throw(),
        Err(err) => err,
    }
}

fn invalid_flag<'js>(ctx: &Ctx<'js>, flag: &str) -> rquickjs::Error {
    // Node reports an invalid `flag` through ERR_INVALID_ARG_VALUE.
    throw_type_error_with_code(
        ctx,
        "ERR_INVALID_ARG_VALUE",
        &format!("The argument 'flags' is invalid. Received '{flag}'"),
    )
}

fn invalid_data_type<'js>(ctx: &Ctx<'js>, received: &str) -> rquickjs::Error {
    throw_type_error_with_code(
        ctx,
        "ERR_INVALID_ARG_TYPE",
        &[
            "The \"data\" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received ",
            received,
        ]
        .concat(),
    )
}

/// Bytes for `data`, honouring `encoding` when a string was passed.
///
/// Node accepts only a string, `Buffer`, `TypedArray` or `DataView` here and
/// throws `ERR_INVALID_ARG_TYPE` for anything else. `ObjectBytes` is deliberately
/// laxer than that (it coerces and treats `undefined` as empty), which is right
/// for the call sites it was written for but wrong for `writeFile`, so the type
/// is checked before handing over.
fn data_bytes<'js>(ctx: &Ctx<'js>, data: &Value<'js>, encoding: Option<&str>) -> Result<Vec<u8>> {
    if let Some(s) = data.as_string() {
        let s = s.to_string()?;
        return match encoding.unwrap_or("utf8") {
            "utf8" | "utf-8" => Ok(s.into_bytes()),
            "latin1" | "binary" => Ok(s.chars().map(|c| c as u8).collect()),
            "utf16le" | "ucs2" | "ucs-2" => {
                Ok(s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect())
            },
            "ascii" => Ok(s.chars().map(|c| c as u8 & 0x7f).collect()),
            "base64" => Ok(base64_decode(&s)),
            "hex" => Ok(hex_decode(&s)),
            other => Err(Exception::throw_message(
                ctx,
                &["Unknown encoding: ", other].concat(),
            )),
        };
    }

    if !is_byte_source(ctx, data) {
        return Err(invalid_data_type(ctx, &describe_type(ctx, data)));
    }

    let bytes = ObjectBytes::from(ctx, data)?;
    Ok(bytes.as_bytes(ctx)?.to_vec())
}

/// Whether `value` is one of the byte containers Node accepts for `data`.
fn is_byte_source<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> bool {
    if value.is_string() {
        return true;
    }
    let Some(obj) = value.as_object() else {
        return false;
    };
    ObjectBytes::from_array_buffer(obj)
        .map(|found| found.is_some())
        .unwrap_or(false)
        && ObjectBytes::from(ctx, value).is_ok()
}

/// Node's wording for the rejected `data` type.
fn describe_type<'js>(_ctx: &Ctx<'js>, value: &Value<'js>) -> String {
    if value.is_null() {
        return "null".to_string();
    }
    if value.is_undefined() {
        return "undefined".to_string();
    }
    if let Some(n) = value.as_number() {
        let text = if n.fract() == 0.0 {
            format!("{}", n as i64)
        } else {
            format!("{n}")
        };
        return format!("type number ({text})");
    }
    if value.is_bool() {
        return "type boolean".to_string();
    }
    if value.is_function() {
        return "function".to_string();
    }
    if let Some(obj) = value.as_object() {
        if let Some(arr) = obj.as_array() {
            return format!("an instance of Array (length {})", arr.len());
        }
        return "an instance of Object".to_string();
    }
    "an unsupported type".to_string()
}

fn base64_decode(input: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in input.bytes() {
        let v = lookup[c as usize];
        if v == 255 {
            continue;
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn hex_decode(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 2);
    let bytes = input.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16);
        let lo = (bytes[i + 1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
            _ => break,
        }
        i += 2;
    }
    out
}

pub async fn write_file<'js>(
    ctx: Ctx<'js>,
    path: String,
    data: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<()> {
    write_file_flagged(ctx, path, data, options, "w").await
}

async fn write_file_flagged<'js>(
    ctx: Ctx<'js>,
    path: String,
    data: Value<'js>,
    options: Opt<Value<'js>>,
    default_flag: &str,
) -> Result<()> {
    let (encoding, mode, flag) = resolve_options(&ctx, options, default_flag)?;
    let bytes = data_bytes(&ctx, &data, encoding.as_deref())?;

    let mut options = open_options(&flag).ok_or_else(|| invalid_flag(&ctx, &flag))?;

    #[cfg(unix)]
    options.mode(mode.unwrap_or(DEFAULT_MODE));

    let mut file = options
        .open(&path)
        .await
        .map_err(|err| throw_fs_error(&ctx, err, "open", &path))?;

    file.write_all(&bytes)
        .await
        .map_err(|err| throw_fs_error(&ctx, err, "write", &path))?;
    file.flush()
        .await
        .map_err(|err| throw_fs_error(&ctx, err, "write", &path))?;

    Ok(())
}

pub fn write_file_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    data: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<()> {
    write_file_sync_flagged(ctx, path, data, options, "w")
}

fn write_file_sync_flagged<'js>(
    ctx: Ctx<'js>,
    path: String,
    data: Value<'js>,
    options: Opt<Value<'js>>,
    default_flag: &str,
) -> Result<()> {
    let (encoding, mode, flag) = resolve_options(&ctx, options, default_flag)?;
    let bytes = data_bytes(&ctx, &data, encoding.as_deref())?;

    let mut options = sync_open_options(&flag).ok_or_else(|| invalid_flag(&ctx, &flag))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode.unwrap_or(DEFAULT_MODE));
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
    }

    use std::io::Write;
    let mut file = options
        .open(&path)
        .map_err(|err| throw_fs_error(&ctx, err, "open", &path))?;
    file.write_all(&bytes)
        .map_err(|err| throw_fs_error(&ctx, err, "write", &path))?;
    file.flush()
        .map_err(|err| throw_fs_error(&ctx, err, "write", &path))?;

    Ok(())
}

/// `appendFile` is `writeFile` with `flag: 'a'` as the default. Keeping it that
/// way mirrors Node, where `appendFile` is a thin wrapper over `writeFile`.
pub async fn append_file<'js>(
    ctx: Ctx<'js>,
    path: String,
    data: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<()> {
    write_file_flagged(ctx, path, data, options, "a").await
}

pub fn append_file_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    data: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<()> {
    write_file_sync_flagged(ctx, path, data, options, "a")
}
