// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::io;

use rquickjs::{function::Args, Ctx, Exception, Function, Null, Result, Value};

pub fn io_error_code(err: &io::Error) -> &'static str {
    // Windows and Unix disagree on which `ErrorKind` an invalid descriptor
    // maps to, so the platform code wins when it is `EBADF`.
    #[cfg(unix)]
    if err.raw_os_error() == Some(9) {
        return "EBADF";
    }
    // ENOSPC has no dedicated `ErrorKind`; the raw code is the only reliable
    // signal, and it is what a full disk reports.
    #[cfg(unix)]
    if err.raw_os_error() == Some(28) {
        return "ENOSPC";
    }
    #[cfg(windows)]
    if err.raw_os_error() == Some(6) || err.raw_os_error() == Some(9) {
        return "EBADF";
    }
    match err.kind() {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::AlreadyExists => "EEXIST",
        io::ErrorKind::InvalidInput => "EINVAL",
        io::ErrorKind::TimedOut => "ETIMEDOUT",
        io::ErrorKind::Interrupted => "EINTR",
        io::ErrorKind::WouldBlock => "EAGAIN",
        io::ErrorKind::IsADirectory => "EISDIR",
        io::ErrorKind::NotADirectory => "ENOTDIR",
        io::ErrorKind::BrokenPipe => "EPIPE",
        io::ErrorKind::ConnectionRefused => "ECONNREFUSED",
        io::ErrorKind::ConnectionReset => "ECONNRESET",
        io::ErrorKind::ConnectionAborted => "ECONNABORTED",
        io::ErrorKind::NotConnected => "ENOTCONN",
        io::ErrorKind::AddrInUse => "EADDRINUSE",
        io::ErrorKind::AddrNotAvailable => "EADDRNOTAVAIL",
        io::ErrorKind::OutOfMemory => "ENOMEM",
        io::ErrorKind::WriteZero => "EIO",
        // `EBADF` has no stable `ErrorKind` on all platforms; the raw code is
        // checked below so a closed or write-only descriptor reports what Node
        // reports instead of `UNKNOWN`.
        io::ErrorKind::Unsupported => "EBADF",
        io::ErrorKind::UnexpectedEof => "EOF",
        _ => "UNKNOWN",
    }
}

pub fn create_fs_error<'js>(
    ctx: &Ctx<'js>,
    err: io::Error,
    syscall: &str,
    path: &str,
) -> Result<Exception<'js>> {
    let code = io_error_code(&err);
    let message = format!("{code}: {err}, {syscall} '{path}'");
    let exception = Exception::from_message(ctx.clone(), &message)?;
    exception.as_object().set("code", code)?;
    exception.as_object().set("path", path)?;
    exception.as_object().set("syscall", syscall)?;
    Ok(exception)
}

pub fn throw_fs_error(ctx: &Ctx<'_>, err: io::Error, syscall: &str, path: &str) -> rquickjs::Error {
    match create_fs_error(ctx, err, syscall, path) {
        Ok(exception) => exception.throw(),
        Err(error) => error,
    }
}

/// Schedule an error-first fs callback on the next job (not the current JS stack).
///
/// Success with a value: `callback(null, result)`.
/// Void success: `callback(null)`.
/// Failure: `callback(error)`.
pub fn defer_fs_callback<'js>(
    ctx: &Ctx<'js>,
    callback: Function<'js>,
    error: Option<Exception<'js>>,
    result: Option<Value<'js>>,
) -> Result<()> {
    match (error, result) {
        (Some(err), _) => {
            let mut args = Args::new(ctx.clone(), 1);
            args.push_arg(err.into_object())?;
            callback.defer_arg(args)?;
        },
        (None, Some(value)) => {
            let mut args = Args::new(ctx.clone(), 2);
            args.push_arg(Null)?;
            args.push_arg(value)?;
            callback.defer_arg(args)?;
        },
        (None, None) => {
            let mut args = Args::new(ctx.clone(), 1);
            args.push_arg(Null)?;
            callback.defer_arg(args)?;
        },
    }
    Ok(())
}
