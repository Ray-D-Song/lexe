// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
#[allow(unused_imports)]
use std::{
    fs::Metadata,
    time::{Duration, SystemTime},
};

use raster_runtime_context::CtxExtension;
use raster_runtime_utils::result::ResultExt;
use rquickjs::{
    function::Rest, prelude::Opt, Ctx, Exception, FromJs, Function, IntoJs, Result, Value,
};
use tokio::fs;

use crate::errors::{create_fs_error, defer_fs_callback, throw_fs_error};

// The Stats implementation is very much based on Unix. The Windows implementation
// tries its best to mimic the implementation of libuv since it is the standard.
// See: https://github.com/libuv/libuv/blob/90648ea3e55125a5a819b32106da6462da310da6/src/win/fs.c
//
// By comparison, the Deno implementation is very basic and doesn't even try much.
// See: https://github.com/denoland/deno/blob/c9da27e147d0681724dd647593abbaa46417feb7/ext/io/fs.rs#L114-L182
//
// This implementation doesn't handle files created before UNIX_EPOCH.

#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct Stats {
    #[qjs(skip_trace)]
    metadata: Metadata,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Stats {
    #[qjs(skip)]
    pub fn new(metadata: Metadata) -> Self {
        Self { metadata }
    }

    #[qjs(get, enumerable)]
    pub fn dev(&self) -> u64 {
        #[cfg(unix)]
        {
            self.metadata.dev()
        }
        #[cfg(not(unix))]
        {
            // Unstable feature, see https://github.com/rust-lang/rust/issues/63010
            0
        }
    }

    #[qjs(get, enumerable)]
    pub fn ino(&self) -> u64 {
        #[cfg(unix)]
        {
            self.metadata.ino()
        }
        #[cfg(not(unix))]
        {
            // Unstable feature, see https://github.com/rust-lang/rust/issues/63010
            0
        }
    }

    #[qjs(get, enumerable)]
    pub fn mode(&self) -> u32 {
        #[cfg(unix)]
        {
            self.metadata.mode()
        }
        #[cfg(not(unix))]
        {
            0o666
        }
    }

    #[qjs(get, enumerable)]
    pub fn nlink(&self) -> u64 {
        #[cfg(unix)]
        {
            self.metadata.nlink()
        }
        #[cfg(not(unix))]
        {
            // Unstable feature, see https://github.com/rust-lang/rust/issues/63010
            1
        }
    }

    #[qjs(get, enumerable)]
    pub fn uid(&self) -> u32 {
        #[cfg(unix)]
        {
            self.metadata.uid()
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    #[qjs(get, enumerable)]
    pub fn gid(&self) -> u32 {
        #[cfg(unix)]
        {
            self.metadata.gid()
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    #[qjs(get, enumerable)]
    pub fn rdev(&self) -> u64 {
        #[cfg(unix)]
        {
            self.metadata.rdev()
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    #[qjs(get, enumerable)]
    pub fn size(&self) -> u64 {
        #[cfg(unix)]
        {
            self.metadata.size()
        }
        #[cfg(windows)]
        {
            if self.metadata.is_dir() {
                0
            } else {
                self.metadata.file_size()
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            0
        }
    }

    #[qjs(get, enumerable)]
    pub fn blksize(&self) -> u64 {
        #[cfg(unix)]
        {
            self.metadata.blksize()
        }
        #[cfg(not(unix))]
        {
            4096
        }
    }

    #[qjs(get, enumerable)]
    pub fn blocks(&self) -> u64 {
        #[cfg(unix)]
        {
            self.metadata.blocks()
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    #[qjs(get, enumerable)]
    pub fn atime_ms(&self, ctx: Ctx<'_>) -> Result<f64> {
        #[cfg(unix)]
        {
            _ = ctx;
            Ok(ms_from_parts(
                self.metadata.atime(),
                self.metadata.atime_nsec(),
            ))
        }
        #[cfg(not(unix))]
        {
            self.metadata.accessed().map(to_msec_f64).or_throw(&ctx)
        }
    }

    #[qjs(get, enumerable)]
    pub fn mtime_ms(&self, ctx: Ctx<'_>) -> Result<f64> {
        #[cfg(unix)]
        {
            _ = ctx;
            Ok(ms_from_parts(
                self.metadata.mtime(),
                self.metadata.mtime_nsec(),
            ))
        }
        #[cfg(not(unix))]
        {
            self.metadata.modified().map(to_msec_f64).or_throw(&ctx)
        }
    }

    #[qjs(get, enumerable)]
    pub fn ctime_ms(&self, ctx: Ctx<'_>) -> Result<f64> {
        #[cfg(unix)]
        {
            _ = ctx;
            Ok(ms_from_parts(
                self.metadata.ctime(),
                self.metadata.ctime_nsec(),
            ))
        }
        #[cfg(not(unix))]
        {
            self.metadata.modified().map(to_msec_f64).or_throw(&ctx)
        }
    }

    /// Milliseconds since the epoch, with sub-millisecond precision, matching
    /// Node. When the filesystem cannot report a creation time, Node returns
    /// epoch zero rather than falling back to `ctime` — measured on `/proc` and
    /// `/sys` files, where `birthtimeMs` is `0` and `birthtime.getTime()` is `0`.
    #[qjs(get, enumerable)]
    pub fn birthtime_ms(&self, ctx: Ctx<'_>) -> Result<f64> {
        match self.metadata.created() {
            Ok(created) => Ok(to_msec_f64(created)),
            Err(_) => {
                let _ = &ctx;
                Ok(0.0)
            },
        }
    }

    #[qjs(get, enumerable)]
    pub fn atime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        self.metadata.accessed().or_throw(&ctx)
    }

    #[qjs(get, enumerable)]
    pub fn mtime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        self.metadata.modified().or_throw(&ctx)
    }

    #[qjs(get, enumerable)]
    pub fn ctime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        #[cfg(unix)]
        {
            _ = ctx;
            // `ctime_nsec` is the sub-second part only; the seconds come from
            // `ctime()`. Using the nanoseconds alone gave a time in 1970.
            Ok(SystemTime::UNIX_EPOCH
                + Duration::from_secs(self.metadata.ctime() as u64)
                + Duration::from_nanos(self.metadata.ctime_nsec() as u64))
        }
        #[cfg(not(unix))]
        {
            self.metadata.modified().or_throw(&ctx)
        }
    }

    /// The creation time, or the epoch when the filesystem does not report one
    /// — same fallback as [`Self::birthtime_ms`], and the same value Node gives.
    #[qjs(get, enumerable)]
    pub fn birthtime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        let _ = &ctx;
        Ok(self.metadata.created().unwrap_or(SystemTime::UNIX_EPOCH))
    }

    pub fn is_file(&self) -> bool {
        self.metadata.is_file()
    }

    /// @deprecated Use `is_directory` instead
    pub fn is_dir(&self) -> bool {
        self.metadata.is_dir()
    }

    pub fn is_directory(&self) -> bool {
        self.metadata.is_dir()
    }

    /// @deprecated Use `is_symbolic_link` instead
    pub fn is_symlink(&self) -> bool {
        self.metadata.is_symlink()
    }

    pub fn is_symbolic_link(&self) -> bool {
        self.metadata.is_symlink()
    }

    #[qjs(rename = "isFIFO")]
    pub fn is_fifo(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata.file_type().is_fifo()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_block_device(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata.file_type().is_block_device()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_character_device(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata.file_type().is_char_device()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_socket(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata.file_type().is_socket()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

fn reject_bigint_stats(ctx: &Ctx<'_>, options: Option<Value<'_>>) -> Result<()> {
    let Some(options) = options else {
        return Ok(());
    };
    if options.is_undefined() || options.is_null() {
        return Ok(());
    }
    let Some(obj) = options.as_object() else {
        return Ok(());
    };
    if let Ok(true) = obj.get::<_, bool>("bigint") {
        return Err(Exception::throw_type(ctx, "BigIntStats is not supported"));
    }
    Ok(())
}

async fn metadata_impl(path: &str, follow_symlinks: bool) -> std::io::Result<Metadata> {
    if follow_symlinks {
        fs::metadata(path).await
    } else {
        fs::symlink_metadata(path).await
    }
}

fn metadata_impl_sync(path: &str, follow_symlinks: bool) -> std::io::Result<Metadata> {
    if follow_symlinks {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    }
}

pub async fn stat_fn(ctx: Ctx<'_>, path: String, options: Opt<Value<'_>>) -> Result<Stats> {
    reject_bigint_stats(&ctx, options.0)?;
    match metadata_impl(&path, true).await {
        Ok(metadata) => Ok(Stats::new(metadata)),
        Err(err) => Err(throw_fs_error(&ctx, err, "stat", &path)),
    }
}

pub fn stat_fn_sync(ctx: Ctx<'_>, path: String, options: Opt<Value<'_>>) -> Result<Stats> {
    reject_bigint_stats(&ctx, options.0)?;
    match metadata_impl_sync(&path, true) {
        Ok(metadata) => Ok(Stats::new(metadata)),
        Err(err) => Err(throw_fs_error(&ctx, err, "stat", &path)),
    }
}

pub async fn lstat_fn(ctx: Ctx<'_>, path: String, options: Opt<Value<'_>>) -> Result<Stats> {
    reject_bigint_stats(&ctx, options.0)?;
    match metadata_impl(&path, false).await {
        Ok(metadata) => Ok(Stats::new(metadata)),
        Err(err) => Err(throw_fs_error(&ctx, err, "lstat", &path)),
    }
}

pub fn lstat_fn_sync(ctx: Ctx<'_>, path: String, options: Opt<Value<'_>>) -> Result<Stats> {
    reject_bigint_stats(&ctx, options.0)?;
    match metadata_impl_sync(&path, false) {
        Ok(metadata) => Ok(Stats::new(metadata)),
        Err(err) => Err(throw_fs_error(&ctx, err, "lstat", &path)),
    }
}

fn parse_stat_callback_args<'js>(
    ctx: &Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<(Option<Value<'js>>, Function<'js>)> {
    let mut args = args.0;
    match args.len() {
        0 => Err(Exception::throw_type(
            ctx,
            "The \"cb\" argument must be of type function",
        )),
        1 => {
            let only = args.remove(0);
            if only.as_function().is_some() {
                Ok((None, Function::from_js(ctx, only)?))
            } else {
                Err(Exception::throw_type(
                    ctx,
                    "The \"cb\" argument must be of type function",
                ))
            }
        },
        _ => {
            let options_or_cb = args.remove(0);
            if options_or_cb.as_function().is_some() {
                Ok((None, Function::from_js(ctx, options_or_cb)?))
            } else {
                let callback = Function::from_js(ctx, args.remove(0)).map_err(|_| {
                    Exception::throw_type(ctx, "The \"cb\" argument must be of type function")
                })?;
                Ok((Some(options_or_cb), callback))
            }
        },
    }
}

/// Callback-style `fs.stat(path[, options], callback)`.
pub fn stat_callback<'js>(ctx: Ctx<'js>, path: String, args: Rest<Value<'js>>) -> Result<()> {
    let (options, callback) = parse_stat_callback_args(&ctx, args)?;
    reject_bigint_stats(&ctx, options)?;

    ctx.clone().spawn_exit_simple(async move {
        match metadata_impl(&path, true).await {
            Ok(metadata) => {
                let stats = Stats::new(metadata).into_js(&ctx)?;
                defer_fs_callback(&ctx, callback, None, Some(stats))?;
            },
            Err(err) => {
                let exception = create_fs_error(&ctx, err, "stat", &path)?;
                defer_fs_callback(&ctx, callback, Some(exception), None)?;
            },
        }
        Ok(())
    });

    Ok(())
}

/// Callback-style `fs.lstat(path[, options], callback)`.
pub fn lstat_callback<'js>(ctx: Ctx<'js>, path: String, args: Rest<Value<'js>>) -> Result<()> {
    let (options, callback) = parse_stat_callback_args(&ctx, args)?;
    reject_bigint_stats(&ctx, options)?;

    ctx.clone().spawn_exit_simple(async move {
        match metadata_impl(&path, false).await {
            Ok(metadata) => {
                let stats = Stats::new(metadata).into_js(&ctx)?;
                defer_fs_callback(&ctx, callback, None, Some(stats))?;
            },
            Err(err) => {
                let exception = create_fs_error(&ctx, err, "lstat", &path)?;
                defer_fs_callback(&ctx, callback, Some(exception), None)?;
            },
        }
        Ok(())
    });

    Ok(())
}

/// Milliseconds since the epoch as a float, keeping sub-millisecond precision.
///
/// Node's `*Ms` fields are floats, not integers: setting a timestamp of
/// `1234567890.123456` gives `1234567890123.456`, while the matching `Date`'s
/// `getTime()` truncates to `1234567890123`. The two are deliberately not equal.
fn to_msec_f64(time: SystemTime) -> f64 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64() * 1000.0,
        Err(e) => -(e.duration().as_secs_f64() * 1000.0),
    }
}

/// Build epoch milliseconds from a `time_t` seconds part and a nanoseconds part.
///
/// `MetadataExt::*_nsec` gives only the sub-second remainder, so the seconds must
/// be added separately — dropping them put every timestamp in 1970.
#[cfg(unix)]
fn ms_from_parts(secs: i64, nsec: i64) -> f64 {
    secs as f64 * 1000.0 + nsec as f64 / 1_000_000.0
}
