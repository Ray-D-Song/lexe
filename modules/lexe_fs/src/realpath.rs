// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::io;
use std::path::PathBuf;

use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_encoding::Encoder;
use lexe_url::{file_path_from_url, url_class::URL};
use lexe_utils::{bytes::ObjectBytes, object::ObjectExt, result::ResultExt};
use rquickjs::{
    function::{Opt, Rest},
    Class, Ctx, Exception, FromJs, Function, IntoJs, Result, Value,
};
use tokio::fs as tokio_fs;

use crate::errors::{create_fs_error, defer_fs_callback, throw_fs_error};

const SYSCALL: &str = "realpath";

#[derive(Clone, Default)]
enum ParsedEncoding {
    #[default]
    Utf8,
    Buffer,
    Other(Encoder),
}

#[derive(Clone, Default)]
struct RealpathOptions {
    encoding: ParsedEncoding,
}

impl<'js> FromJs<'js> for RealpathOptions {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let ty_name = value.type_name();
        let obj = value
            .as_object()
            .ok_or(rquickjs::Error::new_from_js(ty_name, "Object"))?;

        let encoding = obj.get_optional::<_, String>("encoding")?;
        Ok(Self {
            encoding: validate_encoding(ctx, encoding)?,
        })
    }
}

fn validate_encoding(ctx: &Ctx<'_>, encoding: Option<String>) -> Result<ParsedEncoding> {
    match encoding.as_deref() {
        None | Some("") => Ok(ParsedEncoding::Utf8),
        Some("buffer") => Ok(ParsedEncoding::Buffer),
        Some(label) => {
            let encoder = Encoder::from_str(label).or_throw(ctx)?;
            if matches!(encoder, Encoder::Utf8) {
                Ok(ParsedEncoding::Utf8)
            } else {
                Ok(ParsedEncoding::Other(encoder))
            }
        },
    }
}

fn parse_options<'js>(ctx: &Ctx<'js>, value: Option<Value<'js>>) -> Result<RealpathOptions> {
    match value {
        None => Ok(RealpathOptions::default()),
        Some(value) if value.is_undefined() || value.is_null() => Ok(RealpathOptions::default()),
        Some(value) => {
            if let Some(encoding) = value.as_string() {
                return Ok(RealpathOptions {
                    encoding: validate_encoding(ctx, Some(encoding.to_string()?))?,
                });
            }
            RealpathOptions::from_js(ctx, value)
        },
    }
}

/// Coerce a path-like value to a filesystem string (Node-style broad accept list).
///
/// Used by `realpath` / `realpathSync`. Accepts:
/// - `string`
/// - any `ArrayBuffer` / TypedArray / DataView byte view via [`ObjectBytes`]
///   with **strict** UTF-8 (invalid sequences throw; no U+FFFD replacement)
/// - `URL` with `file:` protocol via [`file_path_from_url`]
///
/// For `createReadStream`, use [`path_from_value_create_read_stream`] which
/// additionally rejects non-Uint8Array views (Node CRS parity).
pub(crate) fn path_from_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    if let Ok(url) = Class::<URL>::from_value(&value) {
        return file_path_from_url(ctx, &url.borrow().inner());
    }

    if let Some(obj) = value.as_object() {
        if let Some(bytes) = ObjectBytes::from_array_buffer(obj)? {
            let bytes = bytes.as_bytes(ctx)?;
            return String::from_utf8(bytes.to_vec()).map_err(|_| {
                Exception::throw_type(
                    ctx,
                    "The argument 'path' must be a string, Uint8Array, or URL without invalid UTF-8",
                )
            });
        }
    }

    if let Some(string) = value.as_string() {
        return string.to_string();
    }

    Err(Exception::throw_type(
        ctx,
        "The \"path\" argument must be of type string or an instance of Buffer or URL",
    ))
}

/// Strict path coercion for `fs.createReadStream` only.
///
/// Same UTF-8 / URL rules as [`path_from_value`], but only `string`, `URL`, and
/// `Buffer`/`Uint8Array` (`ObjectBytes::U8Array`) are accepted. Other TypedArray /
/// DataView / bare ArrayBuffer values throw a type error (Node CRS parity).
///
/// Detached buffers are mapped to a catchable JS TypeError via
/// [`ObjectBytes::as_bytes`] — never panic on `TypedArray::as_ref()`.
pub(crate) fn path_from_value_create_read_stream<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> Result<String> {
    if let Ok(url) = Class::<URL>::from_value(&value) {
        return file_path_from_url(ctx, &url.borrow().inner());
    }

    if let Some(obj) = value.as_object() {
        if let Some(bytes) = ObjectBytes::from_array_buffer_view(obj)? {
            match bytes {
                ObjectBytes::U8Array(typed_array) => {
                    // Never use TypedArray::as_ref() — it panics when detached.
                    // as_bytes() returns None for detached buffers; map to TypeError.
                    let raw = typed_array
                        .as_bytes()
                        .ok_or_else(|| Exception::throw_type(ctx, "ArrayBuffer is detached"))?;
                    return String::from_utf8(raw.to_vec()).map_err(|_| {
                        Exception::throw_type(
                            ctx,
                            "The argument 'path' must be a string, Uint8Array, or URL without invalid UTF-8",
                        )
                    });
                },
                // DataView, Int8Array, Uint16Array, Uint8ClampedArray, etc.
                _ => {
                    return Err(Exception::throw_type(
                        ctx,
                        "The \"path\" argument must be of type string or an instance of Buffer or URL",
                    ));
                },
            }
        }

        // Bare ArrayBuffer is not a view and is rejected for createReadStream.
        if ObjectBytes::from_array_buffer(obj)?.is_some() {
            return Err(Exception::throw_type(
                ctx,
                "The \"path\" argument must be of type string or an instance of Buffer or URL",
            ));
        }
    }

    if let Some(string) = value.as_string() {
        return string.to_string();
    }

    Err(Exception::throw_type(
        ctx,
        "The \"path\" argument must be of type string or an instance of Buffer or URL",
    ))
}

/// Strip Windows extended-length / UNC prefixes to match Node-style paths.
fn normalize_canonical_path(path: PathBuf) -> String {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
        s.into_owned()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().into_owned()
    }
}

fn encode_realpath_result<'js>(
    ctx: &Ctx<'js>,
    path: String,
    options: &RealpathOptions,
) -> Result<Value<'js>> {
    match &options.encoding {
        ParsedEncoding::Utf8 => path.into_js(ctx),
        ParsedEncoding::Buffer => Buffer(path.into_bytes()).into_js(ctx),
        ParsedEncoding::Other(encoder) => encoder
            .encode_to_string(path.as_bytes(), true)
            .or_throw(ctx)?
            .into_js(ctx),
    }
}

fn do_canonicalize_sync(path: &str) -> io::Result<String> {
    std::fs::canonicalize(path).map(normalize_canonical_path)
}

async fn do_canonicalize_async(path: &str) -> io::Result<String> {
    tokio_fs::canonicalize(path)
        .await
        .map(normalize_canonical_path)
}

/// `fs.realpathSync(path[, options])`
pub fn realpath_sync<'js>(
    ctx: Ctx<'js>,
    path: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let options = parse_options(&ctx, options.0)?;
    let path = path_from_value(&ctx, path)?;
    match do_canonicalize_sync(&path) {
        Ok(resolved) => encode_realpath_result(&ctx, resolved, &options),
        Err(err) => Err(throw_fs_error(&ctx, err, SYSCALL, &path)),
    }
}

/// `fs.promises.realpath(path[, options])` / `fs/promises.realpath`
pub async fn realpath_promises<'js>(
    ctx: Ctx<'js>,
    path: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let options = parse_options(&ctx, options.0)?;
    let path = path_from_value(&ctx, path)?;
    match do_canonicalize_async(&path).await {
        Ok(resolved) => encode_realpath_result(&ctx, resolved, &options),
        Err(err) => Err(throw_fs_error(&ctx, err, SYSCALL, &path)),
    }
}

/// Callback-style `fs.realpath(path[, options], callback)`.
pub fn realpath<'js>(ctx: Ctx<'js>, path: Value<'js>, args: Rest<Value<'js>>) -> Result<()> {
    let mut args = args.0;
    let (options_value, callback) = match args.len() {
        0 => {
            return Err(Exception::throw_type(
                &ctx,
                "The \"cb\" argument must be of type function",
            ));
        },
        1 => {
            let only = args.remove(0);
            if only.as_function().is_some() {
                (None, Function::from_js(&ctx, only)?)
            } else {
                return Err(Exception::throw_type(
                    &ctx,
                    "The \"cb\" argument must be of type function",
                ));
            }
        },
        _ => {
            let options_or_cb = args.remove(0);
            if options_or_cb.as_function().is_some() {
                // Treat as callback even if extra trailing args exist.
                (None, Function::from_js(&ctx, options_or_cb)?)
            } else {
                let callback = Function::from_js(&ctx, args.remove(0)).map_err(|_| {
                    Exception::throw_type(&ctx, "The \"cb\" argument must be of type function")
                })?;
                (Some(options_or_cb), callback)
            }
        },
    };

    // Validate options before spawning I/O so illegal encodings throw synchronously.
    let options = parse_options(&ctx, options_value)?;
    let path = path_from_value(&ctx, path)?;

    ctx.clone().spawn_exit_simple(async move {
        match do_canonicalize_async(&path).await {
            Ok(resolved) => {
                let result = encode_realpath_result(&ctx, resolved, &options)?;
                defer_fs_callback(&ctx, callback, None, Some(result))?;
            },
            Err(err) => {
                let exception = create_fs_error(&ctx, err, SYSCALL, &path)?;
                defer_fs_callback(&ctx, callback, Some(exception), None)?;
            },
        }
        Ok(())
    });

    Ok(())
}
