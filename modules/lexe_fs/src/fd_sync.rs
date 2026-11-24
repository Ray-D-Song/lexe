// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! The synchronous fd family: `openSync`, `writeSync`, `readSync`, `closeSync`,
//! `fstatSync`.
//!
//! Node hands out real OS file descriptors, and every fd-based API — sync or
//! async — operates on that same descriptor, so they share one file offset.
//! Measured against Node 24 with a single descriptor:
//!
//! ```text
//! readSync(fd, null) -> "012"
//! fs.read(fd, null)  -> "345"
//! readSync(fd, null) -> "678"
//! ```
//!
//! `fsp.open()`'s `handle.fd` behaves the same. So the descriptor number is the
//! single source of truth, and this module keys a table by it. Each entry holds
//! a `try_clone` of the original file: on Unix that is a `dup`, which shares the
//! file offset, so a sync read moves the cursor the async side sees.
//!
//! An explicit `position` does not move the shared offset:
//!
//! ```text
//! readSync(fd, pos=5) -> "56"
//! readSync(fd, null)  -> "01"
//! ```

use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use lexe_buffer::ArrayBufferView;
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{function::Opt, Ctx, Exception, Result, Value};

use crate::errors::throw_fs_error;

/// One open descriptor, as the sync API sees it.
pub(crate) struct FdEntry {
    /// A clone of the caller's file, sharing the offset with the async side.
    pub(crate) file: std::fs::File,
    pub(crate) path: PathBuf,
}

/// Descriptor number → entry, keyed by the real OS fd like Node.
///
/// Process-wide rather than thread-local: `FileHandle`s are built on the
/// runtime's task, which can move between worker threads at an `.await`, and the
/// sync API has to see descriptors opened through either path.
static FDS: std::sync::Mutex<Option<HashMap<i32, FdEntry>>> = std::sync::Mutex::new(None);

/// Register a descriptor opened elsewhere (e.g. `fs.promises.open`) so the sync
/// family can operate on it. `file` must be a clone sharing the offset.
pub fn register(fd: i32, file: std::fs::File, path: PathBuf) {
    let mut guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(fd, FdEntry { file, path });
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

fn throw_invalid_arg_value<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    throw_with_code(ctx, "ERR_INVALID_ARG_VALUE", "TypeError", message)
}

/// Resolve a `position` argument the way Node does.
///
/// `null` and `undefined` mean "use the shared offset", and so does any negative
/// number - measured against Node 24: `readSync(fd, pos=-1)` reads from the
/// current offset and *advances* it, exactly like `null`. A non-integer number
/// is `ERR_OUT_OF_RANGE`; any other type is `ERR_INVALID_ARG_TYPE`.
fn parse_position<'js>(ctx: &Ctx<'js>, value: &Option<Value<'js>>) -> Result<Option<i64>> {
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

/// `writeSync`'s `position`, which Node does *not* validate.
///
/// Measured on Node 24: a string, an object and `1.5` are all accepted and
/// behave like "use the current offset" (`writeSync` walks the file forward for
/// each). Only `readSync` rejects those with `ERR_INVALID_ARG_TYPE`, so the two
/// get different parsers rather than one shared stricter one.
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
    flags: Opt<String>,
    mode: Opt<u32>,
) -> Result<i32> {
    use std::fs::OpenOptions;

    let flag = flags.0.unwrap_or_else(|| "r".to_string());
    let mut options = OpenOptions::new();
    match flag.as_str() {
        "r" => {
            options.read(true);
        },
        "r+" => {
            options.read(true).write(true);
        },
        "w" => {
            options.write(true).create(true).truncate(true);
        },
        "wx" => {
            options.write(true).create_new(true);
        },
        "w+" => {
            options.read(true).write(true).create(true).truncate(true);
        },
        "wx+" => {
            options.read(true).write(true).create_new(true);
        },
        "a" => {
            options.append(true).create(true);
        },
        "ax" => {
            options.append(true).create_new(true);
        },
        "a+" => {
            options.read(true).append(true).create(true);
        },
        "ax+" => {
            options.read(true).append(true).create_new(true);
        },
        _ => {
            return Err(throw_invalid_arg_value(
                &ctx,
                &format!("The argument 'flags' is invalid. Received '{flag}'"),
            ));
        },
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode.0.unwrap_or(0o666));
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
    }

    let path_buf = PathBuf::from(&path);
    let file = options
        .open(&path_buf)
        .map_err(|err| throw_fs_error(&ctx, err, "open", &path))?;

    // Keep one owned descriptor for the table and hand the caller its number.
    // `try_clone` on Unix is `dup`, so the two share the file offset.
    let shared = file
        .try_clone()
        .map_err(|err| throw_fs_error(&ctx, err, "open", &path))?;

    #[cfg(unix)]
    let fd = {
        use std::os::fd::{AsRawFd, IntoRawFd};
        let raw = file.as_raw_fd();
        register(raw, shared, path_buf);
        // The caller owns this descriptor now; the table keeps the `dup`.
        // Leaking the original keeps the number valid for the process lifetime,
        // which is what Node's contract implies: only `closeSync` releases it.
        let _ = file.into_raw_fd();
        raw
    };
    #[cfg(windows)]
    let fd = {
        use std::os::windows::io::{AsRawHandle, IntoRawHandle};
        let raw = file.as_raw_handle() as usize as i32;
        register(raw, shared, path_buf);
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
    let mut guard = FDS.lock().unwrap_or_else(|e| e.into_inner());
    let table = guard.get_or_insert_with(HashMap::new);
    let removed = table.remove(&fd);
    if removed.is_some() {
        Ok(())
    } else {
        Err(throw_ebadf(&ctx, fd, "close"))
    }
}

/// `fs.writeSync(fd, string[, position[, encoding]])` /
/// `fs.writeSync(fd, buffer[, offset[, length[, position]]])`. Returns bytes
/// written.
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
            Some(value) => value.as_string().map(|s| s.to_string()).transpose()?,
            None => None,
        };
        let text = s.to_string()?;
        let bytes: Vec<u8> = match encoding.as_deref().unwrap_or("utf8") {
            "latin1" | "binary" => text.chars().map(|c| c as u8).collect(),
            _ => text.into_bytes(),
        };
        return with_fd(&ctx, fd, "write", |entry| {
            write_at(&mut entry.file, &bytes, pos)
        })
        .map(|n| n as i64);
    }

    let bytes = ObjectBytes::from(&ctx, &data)?;
    let bytes = bytes.as_bytes(&ctx)?.to_vec();
    let offset = match &arg2.0 {
        Some(value) => value.get::<rquickjs::Coerced<i32>>()?.0.max(0) as usize,
        None => 0,
    };
    let length = match &arg3.0 {
        Some(value) => value.get::<rquickjs::Coerced<i32>>()?.0.max(0) as usize,
        None => bytes.len().saturating_sub(offset),
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
    let pos = parse_position_lenient(&ctx, &position.0);
    with_fd(&ctx, fd, "write", |entry| {
        write_at(&mut entry.file, &slice, pos)
    })
    .map(|n| n as i64)
}

/// `fs.readSync(fd, buffer[, offset[, length[, position]]])`. Returns bytes read,
/// `0` at end of file.
pub fn read_sync<'js>(
    ctx: Ctx<'js>,
    fd: i32,
    mut buffer: ArrayBufferView<'js>,
    offset: Opt<u32>,
    length: Opt<u32>,
    position: Opt<Value<'js>>,
) -> Result<i64> {
    let total = buffer.len();
    let offset = offset.0.unwrap_or(0) as usize;
    if offset > total {
        return Err(throw_out_of_range(
            &ctx,
            &format!(
                "The value of \"offset\" is out of range. It must be <= {total}. Received {offset}"
            ),
        ));
    }
    let length = length.0.map(|l| l as usize).unwrap_or(total - offset);
    if offset + length > total {
        return Err(throw_out_of_range(
            &ctx,
            &format!(
                "The value of \"length\" is out of range. It must be <= {}. Received {length}",
                total - offset
            ),
        ));
    }

    let pos = parse_position(&ctx, &position.0)?;
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

/// `fs.fstatSync(fd)`.
pub fn fstat_sync<'js>(ctx: Ctx<'js>, fd: i32) -> Result<crate::stats::Stats> {
    let metadata = with_fd(&ctx, fd, "fstat", |entry| entry.file.metadata())?;
    Ok(crate::stats::Stats::new(metadata))
}

/// Write `bytes` at `position`, or at the shared offset when `None`.
fn write_at(
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
