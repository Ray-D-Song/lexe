// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! The asynchronous fd family: `fs.read`, `fs.write`, `fs.fstat`,
//! `fs.ftruncate`, `fs.fchmod`, `fs.fchown`, `fs.fsync`, `fs.fdatasync`,
//! `fs.futimes`, `fs.readv`, `fs.writev`.
//!
//! These operate on the descriptor table in [`crate::fd_sync`], so a descriptor
//! opened through either path is visible to both and they share one file offset.

use lexe_buffer::{ArrayBufferView, Buffer};
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{prelude::Rest, Ctx, Exception, FromJs, Function, IntoJs, Result, Value};

use crate::errors::{defer_fs_callback, defer_fs_callback_2};
use crate::fd_sync::{
    collect_views, encode_string_bytes, parse_position, parse_position_lenient, read_at,
    set_file_times_fd, throw_einval, throw_invalid_arg_type, throw_out_of_range,
    utimes_value_to_secs, write_at,
};
use crate::stats::{metadata_to_stats, parse_stat_options};

/// Turn a completed operation's payload into the single value the callback gets.
trait IntoCbValue<'js> {
    fn into_cb_value(self, ctx: &Ctx<'js>) -> Result<Value<'js>>;
}

impl<'js> IntoCbValue<'js> for () {
    fn into_cb_value(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        Ok(Value::new_undefined(ctx.clone()))
    }
}

fn into_cb_value<'js, T: IntoCbValue<'js>>(ctx: &Ctx<'js>, value: T) -> Result<Value<'js>> {
    value.into_cb_value(ctx)
}

fn extract_exception<'js>(ctx: &Ctx<'js>) -> Result<Exception<'js>> {
    let thrown = ctx.catch();
    Exception::from_value(thrown).map_err(|_| throw_invalid_arg_type(ctx, "callback error"))
}

/// Report a 2-arg `(err, value)` callback via `defer_fs_callback`.
fn report<'js>(ctx: &Ctx<'js>, callback: Function<'js>, result: Result<Value<'js>>) -> Result<()> {
    match result {
        Ok(value) => defer_fs_callback(ctx, callback, None, Some(value)),
        Err(_err) => {
            let exception = extract_exception(ctx)?;
            defer_fs_callback(ctx, callback, Some(exception), None)
        },
    }
}

/// Report a 3-arg `(err, v1, v2)` callback via `defer_fs_callback_2` (or `(err)` on Err).
fn report_2<'js>(
    ctx: &Ctx<'js>,
    callback: Function<'js>,
    result: Result<(Value<'js>, Value<'js>)>,
) -> Result<()> {
    match result {
        Ok((v1, v2)) => defer_fs_callback_2(ctx, callback, v1, v2),
        Err(_err) => {
            let exception = extract_exception(ctx)?;
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

/// `fs.read(fd, ...[, options], cb)` supporting all Node overloads.
pub fn read_callback<'js>(ctx: Ctx<'js>, fd: i32, mut args: Rest<Value<'js>>) -> Result<()> {
    let callback = take_callback(&ctx, &mut args.0)?;
    let res = read_callback_inner(&ctx, fd, args.0);
    report_2(&ctx, callback, res)
}

fn read_callback_inner<'js>(
    ctx: &Ctx<'js>,
    fd: i32,
    mut rest: Vec<Value<'js>>,
) -> Result<(Value<'js>, Value<'js>)> {
    let (mut buffer_val, mut buffer, offset, length, pos) = if rest.is_empty() {
        let buf = Buffer::alloc(16384).into_js(ctx)?;
        let view = ArrayBufferView::from_js(ctx, buf.clone())?;
        let total = view.len();
        (buf, view, 0usize, total, None)
    } else {
        let first = rest.remove(0);
        // `fs.read(fd, options, cb)` when `first` is an options object rather than ArrayBufferView
        if let Ok(view) = ArrayBufferView::from_js(ctx, first.clone()) {
            let total = view.len();
            let (off, len, pos) = if rest
                .first()
                .is_some_and(|v| !v.is_null() && !v.is_undefined() && v.is_object())
            {
                let obj = rest.remove(0).into_object().unwrap();
                let off = obj
                    .get::<_, Option<Value>>("offset")?
                    .filter(|v| !v.is_null() && !v.is_undefined())
                    .map(|v| u32::from_js(ctx, v).map(|n| n as usize))
                    .transpose()?
                    .unwrap_or(0);
                if off > total {
                    return Err(throw_out_of_range(
                        ctx,
                        &format!("The value of \"offset\" is out of range. It must be <= {total}. Received {off}"),
                    ));
                }
                let len = obj
                    .get::<_, Option<Value>>("length")?
                    .filter(|v| !v.is_null() && !v.is_undefined())
                    .map(|v| u32::from_js(ctx, v).map(|n| n as usize))
                    .transpose()?
                    .unwrap_or(total - off);
                let pos_val = obj.get::<_, Option<Value>>("position")?;
                let pos = parse_position(ctx, &pos_val)?;
                (off, len, pos)
            } else {
                let off = match rest.first() {
                    Some(v) if !v.is_null() && !v.is_undefined() => {
                        u32::from_js(ctx, v.clone())? as usize
                    },
                    _ => 0,
                };
                if off > total {
                    return Err(throw_out_of_range(
                        ctx,
                        &format!("The value of \"offset\" is out of range. It must be <= {total}. Received {off}"),
                    ));
                }
                let len = match rest.get(1) {
                    Some(v) if !v.is_null() && !v.is_undefined() => {
                        u32::from_js(ctx, v.clone())? as usize
                    },
                    _ => total - off,
                };
                let pos = parse_position(ctx, &rest.get(2).cloned())?;
                (off, len, pos)
            };
            (first, view, off, len, pos)
        } else if let Some(obj) = first.as_object() {
            let buf_val = obj
                .get::<_, Option<Value>>("buffer")?
                .filter(|v| !v.is_null() && !v.is_undefined())
                .unwrap_or(Buffer::alloc(16384).into_js(ctx)?);
            let view = ArrayBufferView::from_js(ctx, buf_val.clone()).map_err(|_| {
                throw_invalid_arg_type(
                    ctx,
                    "The \"buffer\" argument must be an instance of Buffer, TypedArray, or DataView.",
                )
            })?;
            let total = view.len();
            let off = obj
                .get::<_, Option<Value>>("offset")?
                .filter(|v| !v.is_null() && !v.is_undefined())
                .map(|v| u32::from_js(ctx, v).map(|n| n as usize))
                .transpose()?
                .unwrap_or(0);
            if off > total {
                return Err(throw_out_of_range(
                    ctx,
                    &format!("The value of \"offset\" is out of range. It must be <= {total}. Received {off}"),
                ));
            }
            let len = obj
                .get::<_, Option<Value>>("length")?
                .filter(|v| !v.is_null() && !v.is_undefined())
                .map(|v| u32::from_js(ctx, v).map(|n| n as usize))
                .transpose()?
                .unwrap_or(total - off);
            let pos_val = obj.get::<_, Option<Value>>("position")?;
            let pos = parse_position(ctx, &pos_val)?;
            (buf_val, view, off, len, pos)
        } else {
            return Err(throw_invalid_arg_type(
                ctx,
                "The \"buffer\" argument must be an instance of Buffer, TypedArray, or DataView.",
            ));
        }
    };

    let total = buffer.len();
    if offset + length > total {
        return Err(throw_out_of_range(
            ctx,
            &format!(
                "The value of \"length\" is out of range. It must be <= {}. Received {length}",
                total - offset
            ),
        ));
    }

    let mut chunk = vec![0u8; length];
    let read = crate::fd_sync::with_fd(ctx, fd, "read", |entry| {
        read_at(&mut entry.file, &mut chunk, pos)
    })?;

    let dst = buffer
        .as_bytes_mut()
        .ok_or_else(|| throw_invalid_arg_type(ctx, "Buffer is detached"))?;
    dst[offset..offset + read].copy_from_slice(&chunk[..read]);
    // Ensure `buffer_val` is returned unmodified as the 3rd callback arg
    let _ = &mut buffer_val;
    Ok((Value::new_int(ctx.clone(), read as i32), buffer_val))
}

/// `fs.write(fd, buffer[, offset[, length[, position]]], cb)` /
/// `fs.write(fd, buffer[, options], cb)` /
/// `fs.write(fd, string[, position[, encoding]], cb)`.
pub fn write_callback<'js>(ctx: Ctx<'js>, fd: i32, mut args: Rest<Value<'js>>) -> Result<()> {
    let callback = take_callback(&ctx, &mut args.0)?;
    let res = write_callback_inner(&ctx, fd, args.0);
    report_2(&ctx, callback, res)
}

fn write_callback_inner<'js>(
    ctx: &Ctx<'js>,
    fd: i32,
    mut rest: Vec<Value<'js>>,
) -> Result<(Value<'js>, Value<'js>)> {
    if rest.is_empty() {
        return Err(throw_invalid_arg_type(
            ctx,
            "The \"data\" argument must be of type string or an instance of Buffer, TypedArray, or DataView.",
        ));
    }
    let data = rest.remove(0);

    // String overload: fs.write(fd, string[, position[, encoding]], cb)
    if let Some(s) = data.as_string() {
        let pos = parse_position_lenient(ctx, &rest.first().cloned());
        let encoding = match rest.get(1) {
            Some(v) if !v.is_null() && !v.is_undefined() => {
                v.as_string().map(|s| s.to_string()).transpose()?
            },
            _ => None,
        };
        let text = s.to_string()?;
        let bytes = encode_string_bytes(ctx, text, encoding.as_deref())?;
        let written = crate::fd_sync::with_fd(ctx, fd, "write", |entry| {
            write_at(&mut entry.file, &bytes, pos)
        })?;
        return Ok((Value::new_int(ctx.clone(), written as i32), data));
    }

    let raw_bytes = ObjectBytes::from(ctx, &data)?;
    let bytes = raw_bytes.as_bytes(ctx)?.to_vec();

    let (offset, length, pos) = if rest
        .first()
        .is_some_and(|v| !v.is_null() && !v.is_undefined() && v.is_object())
    {
        let obj = rest.remove(0).into_object().unwrap();
        let off = obj
            .get::<_, Option<Value>>("offset")?
            .filter(|v| !v.is_null() && !v.is_undefined())
            .map(|v| {
                v.get::<rquickjs::Coerced<i32>>()
                    .map(|c| c.0.max(0) as usize)
            })
            .transpose()?
            .unwrap_or(0);
        let len = obj
            .get::<_, Option<Value>>("length")?
            .filter(|v| !v.is_null() && !v.is_undefined())
            .map(|v| {
                v.get::<rquickjs::Coerced<i32>>()
                    .map(|c| c.0.max(0) as usize)
            })
            .transpose()?
            .unwrap_or_else(|| bytes.len().saturating_sub(off));
        let pos_val = obj.get::<_, Option<Value>>("position")?;
        let p = parse_position_lenient(ctx, &pos_val);
        (off, len, p)
    } else {
        let off = match rest.first() {
            Some(v) if !v.is_null() && !v.is_undefined() => {
                v.get::<rquickjs::Coerced<i32>>()?.0.max(0) as usize
            },
            _ => 0,
        };
        let len = match rest.get(1) {
            Some(v) if !v.is_null() && !v.is_undefined() => {
                v.get::<rquickjs::Coerced<i32>>()?.0.max(0) as usize
            },
            _ => bytes.len().saturating_sub(off),
        };
        let p = parse_position_lenient(ctx, &rest.get(2).cloned());
        (off, len, p)
    };

    if offset > bytes.len() {
        return Err(throw_out_of_range(
            ctx,
            &format!(
                "The value of \"offset\" is out of range. It must be <= {}. Received {offset}",
                bytes.len()
            ),
        ));
    }
    if offset + length > bytes.len() {
        return Err(throw_out_of_range(
            ctx,
            &format!(
                "The value of \"length\" is out of range. It must be <= {}. Received {length}",
                bytes.len() - offset
            ),
        ));
    }
    let slice = bytes[offset..offset + length].to_vec();
    let written = crate::fd_sync::with_fd(ctx, fd, "write", |entry| {
        write_at(&mut entry.file, &slice, pos)
    })?;
    Ok((Value::new_int(ctx.clone(), written as i32), data))
}

/// `fs.readv(fd, buffers[, position], cb)`.
pub fn readv_callback<'js>(ctx: Ctx<'js>, fd: i32, mut args: Rest<Value<'js>>) -> Result<()> {
    let callback = take_callback(&ctx, &mut args.0)?;
    let res = readv_callback_inner(&ctx, fd, args.0);
    report_2(&ctx, callback, res)
}

fn readv_callback_inner<'js>(
    ctx: &Ctx<'js>,
    fd: i32,
    mut rest: Vec<Value<'js>>,
) -> Result<(Value<'js>, Value<'js>)> {
    if rest.is_empty() {
        return Err(throw_invalid_arg_type(
            ctx,
            "The \"buffers\" argument must be an ArrayBufferView[].",
        ));
    }
    let buffers = rest.remove(0);
    let views = collect_views(ctx, &buffers)?;
    if views.is_empty() {
        // Node reports EINVAL for an empty iovec on read (not a 0-byte read),
        // with syscall "read" — measured on Node 24.
        return Err(throw_einval(ctx, "read"));
    }
    let pos = parse_position(ctx, &rest.first().cloned())?;

    let mut total = 0usize;
    for mut view in views {
        let len = view.len();
        let mut chunk = vec![0u8; len];
        let this_pos = pos.map(|p| p + total as i64);
        let read = crate::fd_sync::with_fd(ctx, fd, "read", |entry| {
            read_at(&mut entry.file, &mut chunk, this_pos)
        })?;
        let dst = view
            .as_bytes_mut()
            .ok_or_else(|| throw_invalid_arg_type(ctx, "Buffer is detached"))?;
        dst[..read].copy_from_slice(&chunk[..read]);
        total += read;
        if read < len {
            break;
        }
    }
    Ok((Value::new_int(ctx.clone(), total as i32), buffers))
}

/// `fs.writev(fd, buffers[, position], cb)`. Reports the total bytes written and `buffers`.
pub fn writev_callback<'js>(ctx: Ctx<'js>, fd: i32, mut args: Rest<Value<'js>>) -> Result<()> {
    let callback = take_callback(&ctx, &mut args.0)?;
    let res = writev_callback_inner(&ctx, fd, args.0);
    report_2(&ctx, callback, res)
}

fn writev_callback_inner<'js>(
    ctx: &Ctx<'js>,
    fd: i32,
    mut rest: Vec<Value<'js>>,
) -> Result<(Value<'js>, Value<'js>)> {
    if rest.is_empty() {
        return Err(throw_invalid_arg_type(
            ctx,
            "The \"buffers\" argument must be an ArrayBufferView[].",
        ));
    }
    let buffers = rest.remove(0);
    let views = collect_views(ctx, &buffers)?;
    let pos = parse_position_lenient(ctx, &rest.first().cloned());

    let mut total = 0usize;
    for view in views {
        let bytes = view
            .as_bytes()
            .ok_or_else(|| throw_invalid_arg_type(ctx, "Buffer is detached"))?
            .to_vec();
        let written = crate::fd_sync::with_fd(ctx, fd, "write", |entry| {
            write_at(&mut entry.file, &bytes, pos.map(|p| p + total as i64))
        })?;
        total += written;
        if written < bytes.len() {
            break;
        }
    }
    Ok((Value::new_int(ctx.clone(), total as i32), buffers))
}

/// `fs.fstat(fd[, options], cb)`.
pub fn fstat_callback<'js>(ctx: Ctx<'js>, fd: i32, args: Rest<Value<'js>>) -> Result<()> {
    let mut rest = args.0;
    let callback = take_callback(&ctx, &mut rest)?;
    let opts = parse_stat_options(rest.into_iter().next());
    let result = crate::fd_sync::with_fd(&ctx, fd, "fstat", |entry| entry.file.metadata())
        .and_then(|metadata| metadata_to_stats(&ctx, metadata, opts.bigint));
    report(&ctx, callback, result)
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
    let result = crate::fd_sync::fchmod_sync(ctx.clone(), fd, mode);
    report(&ctx, callback, result.and_then(|v| into_cb_value(&ctx, v)))
}

/// `fs.fchown(fd, uid, gid, cb)`.
pub fn fchown_callback<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    uid: u32,
    gid: u32,
    args: Rest<Value<'js>>,
) -> Result<()> {
    let mut rest = args.0;
    let callback = take_callback(&ctx, &mut rest)?;
    let result = crate::fd_sync::fchown_sync(ctx.clone(), fd, uid, gid);
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
    atime: Value<'js>,
    mtime: Value<'js>,
    args: Rest<Value<'js>>,
) -> Result<()> {
    let mut rest = args.0;
    let callback = take_callback(&ctx, &mut rest)?;
    let result = (|| -> Result<()> {
        let atime_secs = utimes_value_to_secs(&ctx, atime)?;
        let mtime_secs = utimes_value_to_secs(&ctx, mtime)?;
        crate::fd_sync::with_fd(&ctx, fd, "futimes", |entry| {
            set_file_times_fd(&entry.file, atime_secs, mtime_secs)
        })
    })();
    report(&ctx, callback, result.and_then(|v| into_cb_value(&ctx, v)))
}
