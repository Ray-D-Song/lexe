// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! The asynchronous fd family: `fs.read`, `fs.write`, `fs.fstat`,
//! `fs.ftruncate`, `fs.fchmod`, `fs.fsync`, `fs.fdatasync`, `fs.futimes`,
//! `fs.readv`, `fs.writev`.
//!
//! These operate on the descriptor table in [`crate::fd_sync`], so a descriptor
//! opened through either path is visible to both and they share one file offset
//! — what Node does, and why this module keeps no table of its own.
//!
//! Worth recording because the obvious reading of the docs is wrong: in Node
//! these live on `fs` (callback style) and on `FileHandle` (promise style).
//! **`fs.promises.read` and friends do not exist** — measured on Node 24, all
//! ten are `undefined`. There is nothing promise-shaped to add under
//! `fs.promises`; the `FileHandle` methods are the promise form.
//!
//! The bodies are synchronous: a descriptor read is a syscall and the table
//! holds a `std::fs::File`. The callback contract is what the async form adds.

use std::io::{Read, Seek, SeekFrom, Write};

use raster_runtime_buffer::ArrayBufferView;
use rquickjs::{prelude::Rest, Ctx, Exception, FromJs, Function, IntoJs, Result, Value};

use crate::errors::defer_fs_callback;
use crate::fd_sync::{parse_position_lenient, throw_einval, throw_invalid_arg_type};

/// Turn a completed operation's payload into the single value the callback gets.
trait IntoCbValue<'js> {
    fn into_cb_value(self, ctx: &Ctx<'js>) -> Result<Value<'js>>;
}

impl<'js> IntoCbValue<'js> for () {
    fn into_cb_value(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        Ok(Value::new_undefined(ctx.clone()))
    }
}

impl<'js> IntoCbValue<'js> for usize {
    fn into_cb_value(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        Ok(Value::new_int(ctx.clone(), self as i32))
    }
}

impl<'js> IntoCbValue<'js> for crate::stats::Stats {
    fn into_cb_value(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        self.into_js(ctx)
    }
}

fn into_cb_value<'js, T: IntoCbValue<'js>>(ctx: &Ctx<'js>, value: T) -> Result<Value<'js>> {
    value.into_cb_value(ctx)
}

/// Report the result through the project's existing deferred-callback helper.
///
/// `defer_fs_callback` is what `stat`/`access`/`realpath` use from inside
/// `spawn_exit_simple`, and those work — measured with an in-callback marker.
/// Hand-rolling `Args` here, or going through `Function::defer` with a tuple,
/// panics in `ArgsSlice::Stack` with `len 4 index 4`, so this reuses the proven
/// path instead.
fn report<'js>(ctx: &Ctx<'js>, callback: Function<'js>, result: Result<Value<'js>>) -> Result<()> {
    match result {
        Ok(value) => defer_fs_callback(ctx, callback, None, Some(value)),
        Err(err) => {
            // `catch` gives back the thrown value; the callback helper wants an
            // `Exception`, so rebuild one when it is an object (which a thrown
            // error always is here).
            let thrown = ctx.catch();
            let exception = Exception::from_value(thrown)
                .map_err(|_| throw_invalid_arg_type(ctx, "callback error"))?;
            let _ = err;
            defer_fs_callback(ctx, callback, Some(exception), None)
        },
    }
}

/// Split the trailing callback off an argument list.
fn take_callback<'js>(ctx: &Ctx<'js>, args: &mut Vec<Value<'js>>) -> Result<Function<'js>> {
    let last = args.pop().ok_or_else(|| {
        Exception::throw_type(ctx, "The \"cb\" argument must be of type function")
    })?;
    Function::from_js(ctx, last)
        .map_err(|_| Exception::throw_type(ctx, "The \"cb\" argument must be of type function"))
}

/// The `buffers` argument of `readv`/`writev`.
///
/// Node accepts an array of `ArrayBufferView` (Buffer, TypedArray, DataView),
/// mixed freely — measured: `[Buffer, Uint8Array]` works. Anything else throws
/// `ERR_INVALID_ARG_TYPE` mentioning `ArrayBufferView[]`. An empty array is an
/// error for `readv` (EINVAL) but a no-op returning 0 for `writev`.
fn collect_views<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<Vec<ArrayBufferView<'js>>> {
    let Some(array) = value.as_array() else {
        return Err(throw_invalid_arg_type(
            ctx,
            "The \"buffers\" argument must be an ArrayBufferView[]. Received an instance of Object",
        ));
    };
    let mut views = Vec::with_capacity(array.len());
    for item in array.iter::<Value>() {
        let item = item?;
        let view = <ArrayBufferView as FromJs>::from_js(ctx, item.clone()).map_err(|_| {
            throw_invalid_arg_type(
                ctx,
                "The \"buffers\" argument must be an ArrayBufferView[]. Received an instance of Array",
            )
        })?;
        views.push(view);
    }
    Ok(views)
}

/// `fs.readv(fd, buffers[, position], cb)`.
pub fn readv_callback<'js>(ctx: Ctx<'js>, fd: i32, mut args: Rest<Value<'js>>) -> Result<()> {
    let callback = take_callback(&ctx, &mut args.0)?;
    let mut rest = args.0;
    if rest.is_empty() {
        let err = throw_invalid_arg_type(
            &ctx,
            "The \"buffers\" argument must be an ArrayBufferView[].",
        );
        return report(
            &ctx,
            callback,
            Err(err).map(|_: ()| Value::new_undefined(ctx.clone())),
        );
    }
    let buffers = rest.remove(0);
    let views = match collect_views(&ctx, &buffers) {
        Ok(v) => v,
        Err(e) => {
            return report(
                &ctx,
                callback,
                Err(e).map(|_: ()| Value::new_undefined(ctx.clone())),
            )
        },
    };
    if views.is_empty() {
        // Node reports EINVAL for an empty iovec on read (not a 0-byte read),
        // with syscall "read" — measured on Node 24.
        let err = throw_einval(&ctx, "read");
        return report(
            &ctx,
            callback,
            Err(err).map(|_: ()| Value::new_undefined(ctx.clone())),
        );
    }
    let pos = parse_position_lenient(&ctx, &rest.first().cloned());

    let mut total = 0usize;
    for mut view in views {
        let len = view.len();
        let mut chunk = vec![0u8; len];
        // `read_at(Some(_))` is a pread: it restores the file offset afterwards,
        // so the shared cursor never advances. Every buffer therefore needs an
        // explicit position of its own, `pos + total`. With `position: null` the
        // reads simply follow the shared cursor, which each read advances.
        let this_pos = pos.map(|p| p + total as i64);
        let read = match crate::fd_sync::with_fd(&ctx, fd, "read", |entry| {
            read_at(&mut entry.file, &mut chunk, this_pos)
        }) {
            Ok(n) => n,
            Err(e) => {
                return report(
                    &ctx,
                    callback,
                    Err(e).map(|_: ()| Value::new_undefined(ctx.clone())),
                )
            },
        };
        let dst = match view.as_bytes_mut() {
            Some(d) => d,
            None => {
                let err = throw_invalid_arg_type(&ctx, "Buffer is detached");
                return report(
                    &ctx,
                    callback,
                    Err(err).map(|_: ()| Value::new_undefined(ctx.clone())),
                );
            },
        };
        dst[..read].copy_from_slice(&chunk[..read]);
        total += read;
        if read < len {
            break;
        }
    }
    report(
        &ctx,
        callback,
        Ok(Value::new_int(ctx.clone(), total as i32)),
    )
}

/// `fs.writev(fd, buffers[, position], cb)`. Reports the total bytes written.
pub fn writev_callback<'js>(ctx: Ctx<'js>, fd: i32, mut args: Rest<Value<'js>>) -> Result<()> {
    let callback = take_callback(&ctx, &mut args.0)?;
    let mut rest = args.0;
    if rest.is_empty() {
        let err = throw_invalid_arg_type(
            &ctx,
            "The \"buffers\" argument must be an ArrayBufferView[].",
        );
        return report(
            &ctx,
            callback,
            Err(err).map(|_: ()| Value::new_undefined(ctx.clone())),
        );
    }
    let buffers = rest.remove(0);
    let views = match collect_views(&ctx, &buffers) {
        Ok(v) => v,
        Err(e) => {
            return report(
                &ctx,
                callback,
                Err(e).map(|_: ()| Value::new_undefined(ctx.clone())),
            )
        },
    };
    // An empty iovec writes nothing; Node returns 0 rather than failing.
    let pos = parse_position_lenient(&ctx, &rest.first().cloned());

    let mut total = 0usize;
    for view in views {
        let bytes = match view.as_bytes() {
            Some(b) => b.to_vec(),
            None => {
                let err = throw_invalid_arg_type(&ctx, "Buffer is detached");
                return report(
                    &ctx,
                    callback,
                    Err(err).map(|_: ()| Value::new_undefined(ctx.clone())),
                );
            },
        };
        let written = match crate::fd_sync::with_fd(&ctx, fd, "write", |entry| {
            write_at(&mut entry.file, &bytes, pos.map(|p| p + total as i64))
        }) {
            Ok(n) => n,
            Err(e) => {
                return report(
                    &ctx,
                    callback,
                    Err(e).map(|_: ()| Value::new_undefined(ctx.clone())),
                )
            },
        };
        total += written;
        if written < bytes.len() {
            break;
        }
    }
    report(
        &ctx,
        callback,
        Ok(Value::new_int(ctx.clone(), total as i32)),
    )
}

/// `fs.fstat(fd, cb)`.
pub fn fstat_callback<'js>(ctx: Ctx<'js>, fd: i32, args: Rest<Value<'js>>) -> Result<()> {
    let mut rest = args.0;
    let callback = take_callback(&ctx, &mut rest)?;
    let result = crate::fd_sync::with_fd(&ctx, fd, "fstat", |entry| entry.file.metadata())
        .map(crate::stats::Stats::new);
    report(&ctx, callback, result.and_then(|v| into_cb_value(&ctx, v)))
}

/// `fs.ftruncate(fd[, len], cb)`.
pub fn ftruncate_callback<'js>(ctx: Ctx<'js>, fd: i32, mut args: Rest<Value<'js>>) -> Result<()> {
    let callback = take_callback(&ctx, &mut args.0)?;
    let len = args
        .0
        .first()
        .and_then(|v| v.get::<rquickjs::Coerced<u64>>().ok().map(|c| c.0))
        .unwrap_or(0);
    let result = crate::fd_sync::with_fd(&ctx, fd, "ftruncate", |entry| entry.file.set_len(len));
    report(&ctx, callback, result.and_then(|v| into_cb_value(&ctx, v)))
}

/// `fs.fchmod(fd, mode, cb)`.
pub fn fchmod_callback<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    mode: u32,
    args: Rest<Value<'js>>,
) -> Result<()> {
    let mut rest = args.0;
    let callback = take_callback(&ctx, &mut rest)?;
    #[cfg(unix)]
    let result = {
        use std::os::unix::fs::PermissionsExt;
        crate::fd_sync::with_fd(&ctx, fd, "fchmod", |entry| {
            entry
                .file
                .set_permissions(std::fs::Permissions::from_mode(mode))
        })
    };
    #[cfg(not(unix))]
    let result: Result<()> = {
        let _ = (fd, mode);
        Ok(())
    };
    report(&ctx, callback, result.and_then(|v| into_cb_value(&ctx, v)))
}

/// `fs.fsync(fd, cb)`.
pub fn fsync_callback<'js>(ctx: Ctx<'js>, fd: i32, args: Rest<Value<'js>>) -> Result<()> {
    let mut rest = args.0;
    let callback = take_callback(&ctx, &mut rest)?;
    let result = crate::fd_sync::with_fd(&ctx, fd, "fsync", |entry| entry.file.sync_all());
    report(&ctx, callback, result.and_then(|v| into_cb_value(&ctx, v)))
}

/// `fs.fdatasync(fd, cb)`.
pub fn fdatasync_callback<'js>(ctx: Ctx<'js>, fd: i32, args: Rest<Value<'js>>) -> Result<()> {
    let mut rest = args.0;
    let callback = take_callback(&ctx, &mut rest)?;
    let result = crate::fd_sync::with_fd(&ctx, fd, "fdatasync", |entry| entry.file.sync_data());
    report(&ctx, callback, result.and_then(|v| into_cb_value(&ctx, v)))
}

/// `fs.futimes(fd, atime, mtime, cb)`.
pub fn futimes_callback<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    atime: f64,
    mtime: f64,
    args: Rest<Value<'js>>,
) -> Result<()> {
    let mut rest = args.0;
    let callback = take_callback(&ctx, &mut rest)?;
    let atime = filetime_from_secs(atime);
    let mtime = filetime_from_secs(mtime);
    let result = crate::fd_sync::with_fd(&ctx, fd, "futimes", |entry| {
        // `std::fs::File::set_times` goes through a handle duplicated for the
        // fs table; on some kernels the update does not reach the inode that
        // way. `futimens` on the descriptor is what Node's `futimes` maps to,
        // so call it directly.
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let raw = entry.file.as_raw_fd();
            let times = [
                libc::timespec {
                    tv_sec: atime
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as libc::time_t)
                        .unwrap_or(0),
                    tv_nsec: atime
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_nanos() as i64)
                        .unwrap_or(0),
                },
                libc::timespec {
                    tv_sec: mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as libc::time_t)
                        .unwrap_or(0),
                    tv_nsec: mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_nanos() as i64)
                        .unwrap_or(0),
                },
            ];
            let rc = unsafe { libc::futimens(raw, times.as_ptr()) };
            if rc != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            entry.file.set_times(
                std::fs::FileTimes::new()
                    .set_accessed(atime)
                    .set_modified(mtime),
            )
        }
    });
    report(&ctx, callback, result.and_then(|v| into_cb_value(&ctx, v)))
}

fn filetime_from_secs(secs: f64) -> std::time::SystemTime {
    if !secs.is_finite() || secs <= 0.0 {
        return std::time::UNIX_EPOCH;
    }
    std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs)
}

/// Read at `position`, or from the shared offset when `None`.
fn read_at(
    file: &mut std::fs::File,
    chunk: &mut [u8],
    position: Option<i64>,
) -> std::io::Result<usize> {
    match position {
        Some(pos) => {
            let current = file.stream_position()?;
            file.seek(SeekFrom::Start(pos as u64))?;
            let n = file.read(chunk)?;
            file.seek(SeekFrom::Start(current))?;
            Ok(n)
        },
        None => file.read(chunk),
    }
}

/// Write at `position`, or at the shared offset when `None`.
fn write_at(
    file: &mut std::fs::File,
    bytes: &[u8],
    position: Option<i64>,
) -> std::io::Result<usize> {
    match position {
        Some(pos) => {
            let current = file.stream_position()?;
            file.seek(SeekFrom::Start(pos as u64))?;
            let n = file.write(bytes)?;
            file.seek(SeekFrom::Start(current))?;
            Ok(n)
        },
        None => file.write(bytes),
    }
}
