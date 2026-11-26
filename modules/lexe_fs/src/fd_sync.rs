// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! The synchronous fd family: `openSync`, `writeSync`, `readSync`, `closeSync`,
//! `fstatSync`, `fchmodSync`, `fchownSync`, `fdatasyncSync`, `fsyncSync`,
//! `ftruncateSync`, `futimesSync`, `readvSync`, `writevSync`.
//!
//! Node hands out real OS file descriptors, and every fd-based API — sync or
//! async — operates on that same descriptor, so they share one file offset.

use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use lexe_buffer::ArrayBufferView;
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{function::Opt, Ctx, Exception, FromJs, Function, Result, Value};

use crate::errors::throw_fs_error;
use crate::stats::{metadata_to_stats, parse_stat_options};

/// One open descriptor, as the sync API sees it.
pub(crate) struct FdEntry {
    /// A clone of the caller's file, sharing the offset with the async side.
    pub(crate) file: std::fs::File,
    pub(crate) path: PathBuf,
    /// True when opened by `openSync` (original raw fd was forgotten via `into_raw_fd`).
    pub(crate) owns_raw_fd: bool,
}

/// Descriptor number → entry, keyed by the real OS fd like Node.
static FDS: std::sync::Mutex<Option<HashMap<i32, FdEntry>>> = std::sync::Mutex::new(None);

/// Register a descriptor opened elsewhere (e.g. `fs.promises.open`) so the sync
/// family can operate on it. `file` must be a clone sharing the offset.
pub fn register(fd: i32, file: std::fs::File, path: PathBuf) {
    let mut guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_with(HashMap::new).insert(
        fd,
        FdEntry {
            file,
            path,
            owns_raw_fd: false,
        },
    );
}

/// Remove a descriptor registered by `FileHandle` when the handle closes.
pub fn unregister(fd: i32) -> bool {
    let mut guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_mut().is_some_and(|m| m.remove(&fd).is_some())
}

/// True when `fd` is open in this table.
pub fn is_open(fd: i32) -> bool {
    let guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().is_some_and(|m| m.contains_key(&fd))
}

/// Numeric `code` on an exception, the way Node reports these.
fn throw_with_code<'js>(ctx: &Ctx<'js>, code: &str, name: &str, message: &str) -> rquickjs::Error {
    let constructor: rquickjs::Function = match ctx.eval(name) {
        Ok(f) => f,
        Err(err) => return err,
    };
    let Ok(value): std::result::Result<Value, _> = constructor.call((message,)) else {
        return Exception::throw_message(ctx, message);
    };
    if let Some(obj) = value.as_object() {
        let _ = obj.set("code", code);
    }
    match Exception::from_value(value) {
        Ok(exception) => exception.throw(),
        Err(err) => err,
    }
}

/// `EBADF` carrying the failing syscall, matching Node's shape.
pub(crate) fn throw_ebadf<'js>(ctx: &Ctx<'js>, fd: i32, syscall: &str) -> rquickjs::Error {
    let message = format!("EBADF: bad file descriptor, {syscall}");
    match Exception::from_message(ctx.clone(), &message) {
        Ok(exception) => {
            let obj = exception.as_object();
            let _ = obj.set("code", "EBADF");
            let _ = obj.set("errno", -9_i32);
            let _ = obj.set("syscall", syscall);
            let _ = obj.set("fd", fd);
            exception.throw()
        },
        Err(err) => err,
    }
}

/// `EINVAL` with the failing syscall, matching Node's wording
/// (`EINVAL: invalid argument, read`).
pub(crate) fn throw_einval<'js>(ctx: &Ctx<'js>, syscall: &str) -> rquickjs::Error {
    let message = format!("EINVAL: invalid argument, {syscall}");
    match Exception::from_message(ctx.clone(), &message) {
        Ok(exception) => {
            let obj = exception.as_object();
            let _ = obj.set("code", "EINVAL");
            let _ = obj.set("errno", -22_i32);
            let _ = obj.set("syscall", syscall);
            exception.throw()
        },
        Err(err) => err,
    }
}

pub(crate) fn throw_out_of_range<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    throw_with_code(ctx, "ERR_OUT_OF_RANGE", "RangeError", message)
}

pub(crate) fn throw_invalid_arg_type<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    throw_with_code(ctx, "ERR_INVALID_ARG_TYPE", "TypeError", message)
}

pub(crate) fn throw_invalid_arg_value<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    throw_with_code(ctx, "ERR_INVALID_ARG_VALUE", "TypeError", message)
}

/// Resolve a `position` argument the way Node does.
///
/// `null` and `undefined` mean "use the shared offset", and so does any negative
/// integer — measured against Node 24: `readSync(fd, pos=-1)` reads from the
/// current offset and advances it, exactly like `null`. A non-integer number
/// is `ERR_OUT_OF_RANGE`; any other type is `ERR_INVALID_ARG_TYPE`.
pub(crate) fn parse_position<'js>(
    ctx: &Ctx<'js>,
    value: &Option<Value<'js>>,
) -> Result<Option<i64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    if let Some(n) = value.as_number() {
        if !n.is_finite() || n.fract() != 0.0 {
            return Err(throw_out_of_range(
                ctx,
                &format!("The value of \"position\" is out of range. It must be an integer. Received {n}"),
            ));
        }
        let pos = n as i64;
        return Ok(if pos < 0 { None } else { Some(pos) });
    }
    Err(throw_invalid_arg_type(
        ctx,
        &format!(
            "The \"position\" argument must be of type bigint or integer. Received {}",
            describe_position_type(value)
        ),
    ))
}

/// `writeSync`'s `position`, which Node does *not* validate strictly.
pub(crate) fn parse_position_lenient<'js>(
    _ctx: &Ctx<'js>,
    value: &Option<Value<'js>>,
) -> Option<i64> {
    let value = value.as_ref()?;
    if value.is_null() || value.is_undefined() {
        return None;
    }
    let n = value.as_number()?;
    if !n.is_finite() || n.fract() != 0.0 || n < 0.0 {
        return None;
    }
    Some(n as i64)
}

/// Node's wording for the rejected `position` type.
fn describe_position_type<'js>(value: &Value<'js>) -> String {
    if let Some(s) = value.as_string() {
        return format!("type string ('{}')", s.to_string().unwrap_or_default());
    }
    if value.is_bool() {
        return format!("type boolean ({})", value.as_bool().unwrap_or(false));
    }
    if value.as_object().is_some() {
        return "an instance of Object".to_string();
    }
    "an unsupported type".to_string()
}

/// Convert JS `number | Date | string` timestamp to seconds since UNIX epoch.
pub(crate) fn utimes_value_to_secs<'js>(ctx: &Ctx<'js>, t: Value<'js>) -> Result<f64> {
    if let Some(n) = t.as_number() {
        return Ok(n);
    }
    if let Some(obj) = t.as_object() {
        if let Ok(get_time) = obj.get::<_, Function>("getTime") {
            if let Ok(ms) = get_time.call::<_, f64>((rquickjs::prelude::This(obj.clone()),)) {
                return Ok(ms / 1000.0);
            }
        }
    }
    if let Some(s) = t.as_string() {
        if let Ok(text) = s.to_string() {
            if let Ok(secs) = text.trim().parse::<f64>() {
                return Ok(secs);
            }
            if let Ok(parse_fn) = ctx.eval::<Function, _>("Date.parse") {
                if let Ok(ms) = parse_fn.call::<_, f64>((text,)) {
                    if ms.is_finite() {
                        return Ok(ms / 1000.0);
                    }
                }
            }
        }
    }
    Err(Exception::throw_type(
        ctx,
        "time must be a number, string, or Date",
    ))
}

fn filetime_from_secs(secs: f64) -> std::time::SystemTime {
    if !secs.is_finite() || secs <= 0.0 {
        return std::time::UNIX_EPOCH;
    }
    std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs)
}

/// Set access and modification timestamps on an open raw Unix fd.
#[cfg(unix)]
pub(crate) fn set_file_times_raw_fd(
    raw: libc::c_int,
    atime_secs: f64,
    mtime_secs: f64,
) -> std::io::Result<()> {
    let atime = filetime_from_secs(atime_secs);
    let mtime = filetime_from_secs(mtime_secs);
    let to_ts = |st: std::time::SystemTime| match st.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => libc::timespec {
            tv_sec: d.as_secs() as libc::time_t,
            tv_nsec: d.subsec_nanos() as _,
        },
        Err(_) => libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
    };
    let times = [to_ts(atime), to_ts(mtime)];
    let rc = unsafe { libc::futimens(raw, times.as_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Set access and modification timestamps on an open descriptor.
pub(crate) fn set_file_times_fd(
    file: &std::fs::File,
    atime_secs: f64,
    mtime_secs: f64,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        set_file_times_raw_fd(file.as_raw_fd(), atime_secs, mtime_secs)
    }
    #[cfg(not(unix))]
    {
        let atime = filetime_from_secs(atime_secs);
        let mtime = filetime_from_secs(mtime_secs);
        file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(atime)
                .set_modified(mtime),
        )
    }
}

/// Collect an array of `ArrayBufferView` items (used by `readv`/`writev`).
pub(crate) fn collect_views<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
) -> Result<Vec<ArrayBufferView<'js>>> {
    let Some(array) = value.as_array() else {
        return Err(throw_invalid_arg_type(
            ctx,
            "The \"buffers\" argument must be an ArrayBufferView[]. Received an instance of Object",
        ));
    };
    let mut views = Vec::with_capacity(array.len());
    for item in array.iter::<Value>() {
        let item = item?;
        let view = <ArrayBufferView as FromJs>::from_js(ctx, item).map_err(|_| {
            throw_invalid_arg_type(
                ctx,
                "The \"buffers\" argument must be an ArrayBufferView[]. Received an instance of Array",
            )
        })?;
        views.push(view);
    }
    Ok(views)
}

/// Run `f` against `fd`, or throw `EBADF`.
pub(crate) fn with_fd<T>(
    ctx: &Ctx<'_>,
    fd: i32,
    syscall: &str,
    f: impl FnOnce(&mut FdEntry) -> std::io::Result<T>,
) -> Result<T> {
    let mut guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
    let table = guard.get_or_insert_with(HashMap::new);
    let Some(entry) = table.get_mut(&fd) else {
        return Err(throw_ebadf(ctx, fd, syscall));
    };
    let path = entry.path.to_string_lossy().into_owned();
    f(entry).map_err(|err| throw_fs_error(ctx, err, syscall, &path))
}

/// `fs.openSync(path[, flags[, mode]])` → descriptor number.
pub fn open_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    flags: Opt<Value<'js>>,
    mode: Opt<Value<'js>>,
) -> Result<i32> {
    let (options, _crt) = crate::open::build_std_open_options(&ctx, &path, flags, mode)?;

    let path_buf = PathBuf::from(&path);
    let file = options
        .open(&path_buf)
        .map_err(|err| throw_fs_error(&ctx, err, "open", &path))?;

    let shared = file
        .try_clone()
        .map_err(|err| throw_fs_error(&ctx, err, "open", &path))?;

    #[cfg(unix)]
    let fd = {
        use std::os::fd::{AsRawFd, IntoRawFd};
        let raw = file.as_raw_fd();
        let mut guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
        guard.get_or_insert_with(HashMap::new).insert(
            raw,
            FdEntry {
                file: shared,
                path: path_buf,
                owns_raw_fd: true,
            },
        );
        let _ = file.into_raw_fd();
        raw
    };
    #[cfg(windows)]
    let fd = {
        use std::os::windows::io::{AsRawHandle, IntoRawHandle};
        let raw = file.as_raw_handle() as usize as i32;
        let mut guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
        guard.get_or_insert_with(HashMap::new).insert(
            raw,
            FdEntry {
                file: shared,
                path: path_buf,
                owns_raw_fd: false,
            },
        );
        let _ = file.into_raw_handle();
        raw
    };
    #[cfg(not(any(unix, windows)))]
    let fd = {
        register(0, shared, path_buf);
        std::mem::forget(file);
        0
    };

    Ok(fd)
}

/// `fs.closeSync(fd)`.
pub fn close_sync(ctx: Ctx<'_>, fd: i32) -> Result<()> {
    let removed = {
        let mut guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
        let table = guard.get_or_insert_with(HashMap::new);
        table.remove(&fd)
    };
    if let Some(entry) = removed {
        let owns_raw_fd = entry.owns_raw_fd;
        drop(entry);
        #[cfg(unix)]
        if owns_raw_fd {
            unsafe {
                libc::close(fd);
            }
        }
        #[cfg(not(unix))]
        let _ = owns_raw_fd;
        Ok(())
    } else {
        Err(throw_ebadf(&ctx, fd, "close"))
    }
}

/// `fs.writeSync(fd, string[, position[, encoding]])` /
/// `fs.writeSync(fd, buffer[, offset[, length[, position]]])` /
/// `fs.writeSync(fd, buffer, options)`. Returns bytes written.
pub fn write_sync<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    data: Value<'js>,
    arg2: Opt<Value<'js>>,
    arg3: Opt<Value<'js>>,
    position: Opt<Value<'js>>,
) -> Result<i64> {
    // String form: writeSync(fd, string[, position[, encoding]])
    if let Some(s) = data.as_string() {
        let pos = parse_position_lenient(&ctx, &arg2.0);
        let encoding = match &arg3.0 {
            Some(value) if !value.is_null() && !value.is_undefined() => {
                value.as_string().map(|s| s.to_string()).transpose()?
            },
            _ => None,
        };
        let text = s.to_string()?;
        let bytes = encode_string_bytes(&ctx, text, encoding.as_deref())?;
        return with_fd(&ctx, fd, "write", |entry| {
            write_at(&mut entry.file, &bytes, pos)
        })
        .map(|n| n as i64);
    }

    let bytes = ObjectBytes::from(&ctx, &data)?;
    let bytes = bytes.as_bytes(&ctx)?.to_vec();

    // Options object overload: writeSync(fd, buffer, { offset, length, position })
    let (offset, length, pos) = if let Some(obj) = arg2
        .0
        .as_ref()
        .filter(|v| !v.is_null() && !v.is_undefined())
        .and_then(|v| v.as_object())
    {
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
        let p = parse_position_lenient(&ctx, &pos_val);
        (off, len, p)
    } else {
        let off = match &arg2.0 {
            Some(value) if !value.is_null() && !value.is_undefined() => {
                value.get::<rquickjs::Coerced<i32>>()?.0.max(0) as usize
            },
            _ => 0,
        };
        let len = match &arg3.0 {
            Some(value) if !value.is_null() && !value.is_undefined() => {
                value.get::<rquickjs::Coerced<i32>>()?.0.max(0) as usize
            },
            _ => bytes.len().saturating_sub(off),
        };
        let p = parse_position_lenient(&ctx, &position.0);
        (off, len, p)
    };

    if offset > bytes.len() {
        return Err(throw_out_of_range(
            &ctx,
            &format!(
                "The value of \"offset\" is out of range. It must be <= {}. Received {offset}",
                bytes.len()
            ),
        ));
    }
    if offset + length > bytes.len() {
        return Err(throw_out_of_range(
            &ctx,
            &format!(
                "The value of \"length\" is out of range. It must be <= {}. Received {length}",
                bytes.len() - offset
            ),
        ));
    }
    let slice = bytes[offset..offset + length].to_vec();
    with_fd(&ctx, fd, "write", |entry| {
        write_at(&mut entry.file, &slice, pos)
    })
    .map(|n| n as i64)
}

pub(crate) fn encode_string_bytes(
    ctx: &Ctx<'_>,
    text: String,
    encoding: Option<&str>,
) -> Result<Vec<u8>> {
    match encoding.unwrap_or("utf8") {
        "utf8" | "utf-8" => Ok(text.into_bytes()),
        "latin1" | "binary" => Ok(text.chars().map(|c| c as u8).collect()),
        other => {
            use lexe_encoding::Encoder;
            use lexe_utils::result::ResultExt;
            Encoder::from_str(other)
                .and_then(|enc| enc.decode_from_string(text))
                .or_throw(ctx)
        },
    }
}

/// `fs.readSync(fd, buffer[, offset[, length[, position]]])` /
/// `fs.readSync(fd, buffer[, options])`. Returns bytes read, `0` at EOF.
pub fn read_sync<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    mut buffer: ArrayBufferView<'js>,
    arg2: Opt<Value<'js>>,
    arg3: Opt<Value<'js>>,
    arg4: Opt<Value<'js>>,
) -> Result<i64> {
    let total = buffer.len();

    let (offset, length, pos) = if let Some(obj) = arg2
        .0
        .as_ref()
        .filter(|v| !v.is_null() && !v.is_undefined())
        .and_then(|v| v.as_object())
    {
        let off = obj
            .get::<_, Option<Value>>("offset")?
            .filter(|v| !v.is_null() && !v.is_undefined())
            .map(|v| u32::from_js(&ctx, v).map(|n| n as usize))
            .transpose()?
            .unwrap_or(0);
        if off > total {
            return Err(throw_out_of_range(
                &ctx,
                &format!(
                    "The value of \"offset\" is out of range. It must be <= {total}. Received {off}"
                ),
            ));
        }
        let len = obj
            .get::<_, Option<Value>>("length")?
            .filter(|v| !v.is_null() && !v.is_undefined())
            .map(|v| u32::from_js(&ctx, v).map(|n| n as usize))
            .transpose()?
            .unwrap_or(total - off);
        let pos_val = obj.get::<_, Option<Value>>("position")?;
        let p = parse_position(&ctx, &pos_val)?;
        (off, len, p)
    } else {
        let off = match &arg2.0 {
            Some(v) if !v.is_null() && !v.is_undefined() => u32::from_js(&ctx, v.clone())? as usize,
            _ => 0,
        };
        if off > total {
            return Err(throw_out_of_range(
                &ctx,
                &format!(
                    "The value of \"offset\" is out of range. It must be <= {total}. Received {off}"
                ),
            ));
        }
        let len = match &arg3.0 {
            Some(v) if !v.is_null() && !v.is_undefined() => u32::from_js(&ctx, v.clone())? as usize,
            _ => total - off,
        };
        let p = parse_position(&ctx, &arg4.0)?;
        (off, len, p)
    };

    if offset + length > total {
        return Err(throw_out_of_range(
            &ctx,
            &format!(
                "The value of \"length\" is out of range. It must be <= {}. Received {length}",
                total - offset
            ),
        ));
    }

    let mut chunk = vec![0u8; length];
    let read = with_fd(&ctx, fd, "read", |entry| {
        read_at(&mut entry.file, &mut chunk, pos)
    })?;

    let dst = buffer
        .as_bytes_mut()
        .ok_or_else(|| throw_invalid_arg_type(&ctx, "Buffer is detached"))?;
    dst[offset..offset + read].copy_from_slice(&chunk[..read]);
    Ok(read as i64)
}

/// `fs.fstatSync(fd[, options])`.
pub fn fstat_sync<'js>(ctx: Ctx<'js>, fd: i32, options: Opt<Value<'js>>) -> Result<Value<'js>> {
    let opts = parse_stat_options(options.0);
    let metadata = with_fd(&ctx, fd, "fstat", |entry| entry.file.metadata())?;
    metadata_to_stats(&ctx, metadata, opts.bigint)
}

/// `fs.fchmodSync(fd, mode)`.
pub fn fchmod_sync(ctx: Ctx<'_>, fd: i32, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        with_fd(&ctx, fd, "fchmod", |entry| {
            entry
                .file
                .set_permissions(std::fs::Permissions::from_mode(mode))
        })
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        with_fd(&ctx, fd, "fchmod", |_entry| Ok(()))
    }
}

/// `fs.fchownSync(fd, uid, gid)`.
pub fn fchown_sync(ctx: Ctx<'_>, fd: i32, uid: u32, gid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        with_fd(&ctx, fd, "fchown", |entry| {
            let rc = unsafe { libc::fchown(entry.file.as_raw_fd(), uid, gid) };
            if rc != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (uid, gid);
        with_fd(&ctx, fd, "fchown", |_entry| {
            Err(std::io::Error::from_raw_os_error(38))
        })
    }
}

/// `fs.fdatasyncSync(fd)`.
pub fn fdatasync_sync(ctx: Ctx<'_>, fd: i32) -> Result<()> {
    with_fd(&ctx, fd, "fdatasync", |entry| entry.file.sync_data())
}

/// `fs.fsyncSync(fd)`.
pub fn fsync_sync(ctx: Ctx<'_>, fd: i32) -> Result<()> {
    with_fd(&ctx, fd, "fsync", |entry| entry.file.sync_all())
}

/// `fs.ftruncateSync(fd[, len])`.
pub fn ftruncate_sync(ctx: Ctx<'_>, fd: i32, len: Opt<Value<'_>>) -> Result<()> {
    let target_len = match len.0 {
        None => 0u64,
        Some(v) if v.is_null() || v.is_undefined() => 0u64,
        Some(v) => {
            let n = v.get::<rquickjs::Coerced<i64>>()?.0;
            n.max(0) as u64
        },
    };
    with_fd(&ctx, fd, "ftruncate", |entry| {
        entry.file.set_len(target_len)
    })
}

/// `fs.futimesSync(fd, atime, mtime)`.
pub fn futimes_sync<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    atime: Value<'js>,
    mtime: Value<'js>,
) -> Result<()> {
    let atime_secs = utimes_value_to_secs(&ctx, atime)?;
    let mtime_secs = utimes_value_to_secs(&ctx, mtime)?;
    with_fd(&ctx, fd, "futimes", |entry| {
        set_file_times_fd(&entry.file, atime_secs, mtime_secs)
    })
}

/// `fs.readvSync(fd, buffers[, position])`.
pub fn readv_sync<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    buffers: Value<'js>,
    position: Opt<Value<'js>>,
) -> Result<usize> {
    let views = collect_views(&ctx, &buffers)?;
    if views.is_empty() {
        return Err(throw_einval(&ctx, "read"));
    }
    let pos = parse_position(&ctx, &position.0)?;
    let mut total = 0usize;
    for mut view in views {
        let len = view.len();
        let mut chunk = vec![0u8; len];
        let this_pos = pos.map(|p| p + total as i64);
        let read = with_fd(&ctx, fd, "read", |entry| {
            read_at(&mut entry.file, &mut chunk, this_pos)
        })?;
        let dst = view
            .as_bytes_mut()
            .ok_or_else(|| throw_invalid_arg_type(&ctx, "Buffer is detached"))?;
        dst[..read].copy_from_slice(&chunk[..read]);
        total += read;
        if read < len {
            break;
        }
    }
    Ok(total)
}

/// `fs.writevSync(fd, buffers[, position])`.
pub fn writev_sync<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    buffers: Value<'js>,
    position: Opt<Value<'js>>,
) -> Result<usize> {
    let views = collect_views(&ctx, &buffers)?;
    let pos = parse_position_lenient(&ctx, &position.0);
    let mut total = 0usize;
    for view in views {
        let bytes = view
            .as_bytes()
            .ok_or_else(|| throw_invalid_arg_type(&ctx, "Buffer is detached"))?
            .to_vec();
        let written = with_fd(&ctx, fd, "write", |entry| {
            write_at(&mut entry.file, &bytes, pos.map(|p| p + total as i64))
        })?;
        total += written;
        if written < bytes.len() {
            break;
        }
    }
    Ok(total)
}

/// Write `bytes` at `position`, or at the shared offset when `None`.
pub(crate) fn write_at(
    file: &mut std::fs::File,
    bytes: &[u8],
    position: Option<i64>,
) -> std::io::Result<usize> {
    match position {
        Some(pos) => {
            // An explicit position must not leave the shared offset moved, so
            // restore it afterwards.
            let current = file.stream_position()?;
            file.seek(SeekFrom::Start(pos as u64))?;
            let n = file.write(bytes)?;
            file.seek(SeekFrom::Start(current))?;
            Ok(n)
        },
        None => file.write(bytes),
    }
}

/// Read into `chunk` at `position`, or at the shared offset when `None`.
pub(crate) fn read_at(
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
