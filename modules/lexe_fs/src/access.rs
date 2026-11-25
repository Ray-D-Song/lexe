// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#[cfg(unix)]
use std::ffi::CString;
use std::io;

use lexe_context::CtxExtension;
use rquickjs::{function::Rest, prelude::Opt, Ctx, Exception, FromJs, Function, Result, Value};
use tokio::task;

use crate::errors::{create_fs_error, defer_fs_callback, throw_fs_error};
use crate::{CONSTANT_F_OK, CONSTANT_R_OK, CONSTANT_W_OK, CONSTANT_X_OK};

const SYSCALL: &str = "access";
const MODE_MASK: u32 = CONSTANT_F_OK | CONSTANT_R_OK | CONSTANT_W_OK | CONSTANT_X_OK;

/// Shared access check used by callback, promise, and sync APIs.
///
/// On Unix this uses the OS `access(2)` syscall so checks respect the current
/// process credentials. On other platforms this falls back to metadata-based
/// existence / permission approximations.
pub async fn access_impl(path: &str, mode: u32) -> io::Result<()> {
    let path = path.to_owned();
    task::spawn_blocking(move || access_impl_sync(&path, mode))
        .await
        .map_err(io::Error::other)?
}

fn access_impl_sync(path: &str, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        let c_path = CString::new(path)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.to_string()))?;
        // SAFETY: path is a valid NUL-terminated C string; mode matches libc R/W/X/F_OK.
        let rc = unsafe { libc::access(c_path.as_ptr(), mode as i32) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(unix))]
    {
        // Windows / other: existence via metadata; writeability via readonly bit.
        let metadata = std::fs::metadata(path)?;
        if mode & CONSTANT_W_OK != 0 && metadata.permissions().readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Permission denied. File not writable",
            ));
        }
        let _ = (CONSTANT_R_OK, CONSTANT_X_OK);
        Ok(())
    }
}

fn parse_mode_value(ctx: &Ctx<'_>, value: Option<Value<'_>>) -> Result<u32> {
    let Some(value) = value else {
        return Ok(CONSTANT_F_OK);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(CONSTANT_F_OK);
    }

    let number = if let Some(n) = value.as_int() {
        n as f64
    } else if let Some(n) = value.as_number() {
        n
    } else {
        return Err(Exception::throw_type(
            ctx,
            "The \"mode\" argument must be of type number",
        ));
    };

    if !number.is_finite() || number.fract() != 0.0 || number < 0.0 {
        return Err(Exception::throw_range(
            ctx,
            "The value of \"mode\" is out of range. It must be an integer >= 0",
        ));
    }

    let mode = number as u32;
    if mode & !MODE_MASK != 0 {
        return Err(Exception::throw_range(
            ctx,
            "The value of \"mode\" is out of range. It must be a valid access mode mask",
        ));
    }

    Ok(mode)
}

pub async fn access(ctx: Ctx<'_>, path: String, mode: Opt<Value<'_>>) -> Result<()> {
    let mode = parse_mode_value(&ctx, mode.0)?;
    match access_impl(&path, mode).await {
        Ok(()) => Ok(()),
        Err(err) => Err(throw_fs_error(&ctx, err, SYSCALL, &path)),
    }
}

pub fn access_sync(ctx: Ctx<'_>, path: String, mode: Opt<Value<'_>>) -> Result<()> {
    let mode = parse_mode_value(&ctx, mode.0)?;
    match access_impl_sync(&path, mode) {
        Ok(()) => Ok(()),
        Err(err) => Err(throw_fs_error(&ctx, err, SYSCALL, &path)),
    }
}

/// Callback-style `fs.access(path[, mode], callback)`.
pub fn access_callback<'js>(ctx: Ctx<'js>, path: String, args: Rest<Value<'js>>) -> Result<()> {
    let mut args = args.0;
    let (mode, callback) = match args.len() {
        0 => {
            return Err(Exception::throw_type(
                &ctx,
                "The \"cb\" argument must be of type function",
            ));
        },
        1 => {
            let only = args.remove(0);
            if only.as_function().is_some() {
                (CONSTANT_F_OK, Function::from_js(&ctx, only)?)
            } else {
                return Err(Exception::throw_type(
                    &ctx,
                    "The \"cb\" argument must be of type function",
                ));
            }
        },
        _ => {
            let mode_or_cb = args.remove(0);
            if mode_or_cb.as_function().is_some() {
                (CONSTANT_F_OK, Function::from_js(&ctx, mode_or_cb)?)
            } else {
                let mode = parse_mode_value(&ctx, Some(mode_or_cb))?;
                let callback = Function::from_js(&ctx, args.remove(0)).map_err(|_| {
                    Exception::throw_type(&ctx, "The \"cb\" argument must be of type function")
                })?;
                (mode, callback)
            }
        },
    };

    ctx.clone().spawn_exit_simple(async move {
        match access_impl(&path, mode).await {
            Ok(()) => {
                defer_fs_callback(&ctx, callback, None, None)?;
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
