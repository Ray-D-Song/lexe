// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `fs.createReadStream` built on embedded `stream.Readable`.
//!
//! Path coercion reuses [`crate::realpath::path_from_value_create_read_stream`]
//! unless `options.fd` is supplied with `null`/`undefined` path. Supports
//! `fd` (numeric descriptor or `FileHandle`), `signal` (`AbortSignal`),
//! custom `fs` providers, `autoClose`, `emitClose`, `start`, and `end`.

use rquickjs::{function::Opt, Ctx, Function, Result, Value};

use crate::realpath::path_from_value_create_read_stream;

/// JS implementation that expects `path` (string or null when `options.fd` is set).
const CREATE_READ_STREAM_IMPL_JS: &str = r#"(function () {
  return function createReadStreamImpl(path, options) {
    let opts = {};
    if (typeof options === "string") {
      opts = { encoding: options };
    } else if (options != null) {
      if (typeof options !== "object") {
        throw new TypeError('The "options" argument must be of type object. Received type ' + typeof options);
      }
      opts = options;
    }

    const fdOpt = opts.fd == null ? null : opts.fd;
    let isFileHandle = false;
    if (fdOpt !== null) {
      if (typeof fdOpt === "object" && typeof fdOpt.read === "function") {
        isFileHandle = true;
      } else if (typeof fdOpt !== "number" || !Number.isInteger(fdOpt) || fdOpt < 0) {
        const err = new TypeError('The "options.fd" property must be of type number or FileHandle');
        err.code = "ERR_INVALID_ARG_TYPE";
        throw err;
      }
    }

    const flags = opts.flags == null ? "r" : opts.flags;
    const mode = opts.mode === undefined ? 0o666 : opts.mode;
    const encoding = opts.encoding == null ? null : opts.encoding;
    const autoClose = opts.autoClose === undefined ? true : Boolean(opts.autoClose);
    const emitClose = opts.emitClose === undefined ? true : Boolean(opts.emitClose);
    const highWaterMark = opts.highWaterMark == null ? 64 * 1024 : opts.highWaterMark;
    const start = opts.start;
    const end = opts.end;
    const customFs = opts.fs == null ? null : opts.fs;
    const signal = opts.signal == null ? null : opts.signal;

    if (typeof highWaterMark !== "number" || !Number.isInteger(highWaterMark) || highWaterMark <= 0) {
      throw new RangeError('The value of "options.highWaterMark" is out of range. It must be a positive integer.');
    }
    if (start !== undefined && start !== null) {
      if (typeof start !== "number" || !Number.isInteger(start) || start < 0) {
        throw new RangeError('The value of "options.start" is out of range. It must be a non-negative integer.');
      }
    }
    if (end !== undefined && end !== null && end !== Infinity) {
      if (typeof end !== "number" || !Number.isInteger(end) || end < 0) {
        throw new RangeError('The value of "options.end" is out of range. It must be a non-negative integer.');
      }
    }
    if (
      start !== undefined &&
      start !== null &&
      end !== undefined &&
      end !== null &&
      end < start
    ) {
      throw new RangeError('The value of "options.end" is out of range. It must be >= options.start.');
    }
    if (customFs !== null) {
      if (typeof customFs !== "object" || typeof customFs.read !== "function") {
        const err = new TypeError('The "options.fs.read" property must be of type function');
        err.code = "ERR_INVALID_ARG_TYPE";
        throw err;
      }
      if (fdOpt === null && typeof customFs.open !== "function") {
        const err = new TypeError('The "options.fs.open" property must be of type function');
        err.code = "ERR_INVALID_ARG_TYPE";
        throw err;
      }
      if (autoClose && typeof customFs.close !== "function") {
        const err = new TypeError('The "options.fs.close" property must be of type function');
        err.code = "ERR_INVALID_ARG_TYPE";
        throw err;
      }
    }

    const { Readable } = require("stream");
    const fs = require("fs");
    const { Buffer } = require("buffer");

    let fileHandle = isFileHandle ? fdOpt : null;
    let openPromise = null;
    let forceClose = false;
    let closed = false;
    let eof = false;
    let position = start === undefined || start === null ? null : start;
    let cursorPos = start === undefined || start === null ? 0 : start;
    const rangeEnd = end === undefined ? null : end;
    let abortCleanup = null;

    function makeAbortError(sig) {
      const err = new Error("The operation was aborted", { cause: sig.reason });
      err.name = "AbortError";
      err.code = "ABORT_ERR";
      return err;
    }

    function detachAbort() {
      if (abortCleanup) {
        abortCleanup();
        abortCleanup = null;
      }
    }

    function closeUnderlying(cb) {
      detachAbort();
      if (closed) {
        if (typeof cb === "function") cb(null);
        return;
      }
      closed = true;
      if (fileHandle !== null) {
        const fh = fileHandle;
        fileHandle = null;
        stream.fd = null;
        fh.close().then(
          () => { if (typeof cb === "function") cb(null); },
          (e) => { if (typeof cb === "function") cb(e); }
        );
        return;
      }
      if (stream.fd !== null) {
        const toClose = stream.fd;
        stream.fd = null;
        const closeFn = customFs && typeof customFs.close === "function" ? customFs.close : fs.close;
        closeFn(toClose, (e) => {
          if (typeof cb === "function") cb(e || null);
        });
        return;
      }
      if (typeof cb === "function") cb(null);
    }

    const readableOpts = {
      highWaterMark,
      autoDestroy: autoClose,
      emitClose,
      read(n) {
        if (eof || stream.destroyed) return;
        ensureOpen().then(
          () => {
            if (eof || stream.destroyed) return;
            let toRead = n || highWaterMark;
            if (rangeEnd !== null && Number.isFinite(rangeEnd)) {
              const remaining = rangeEnd - cursorPos + 1;
              if (remaining <= 0) {
                eof = true;
                stream.push(null);
                return;
              }
              toRead = Math.min(toRead, remaining);
            }
            const buf = Buffer.allocUnsafe(toRead);
            const onRead = (err, bytesRead) => {
              if (err) {
                stream.destroy(err);
                return;
              }
              if (stream.destroyed) return;
              if (bytesRead === 0) {
                eof = true;
                stream.push(null);
                return;
              }
              stream.bytesRead += bytesRead;
              cursorPos += bytesRead;
              if (position !== null) {
                position += bytesRead;
                stream.pos = position;
              }
              const hitEnd =
                rangeEnd !== null &&
                Number.isFinite(rangeEnd) &&
                cursorPos > rangeEnd;
              if (hitEnd) {
                eof = true;
              }
              stream.push(buf.subarray(0, bytesRead));
              if (hitEnd) {
                stream.push(null);
              }
            };
            if (fileHandle !== null) {
              const args = position === null ? [buf, 0, toRead] : [buf, 0, toRead, position];
              fileHandle.read(...args).then(
                (res) => onRead(null, res.bytesRead),
                (err) => onRead(err)
              );
            } else {
              const readFn = customFs ? customFs.read : fs.read;
              readFn(stream.fd, buf, 0, toRead, position, onRead);
            }
          },
          (err) => {
            if (!stream.destroyed) stream.destroy(err);
          }
        );
      },
      destroy(err, cb) {
        detachAbort();
        if (autoClose || forceClose) {
          closeUnderlying((closeErr) => cb(err || closeErr));
        } else {
          cb(err);
        }
      },
    };
    if (encoding !== null) {
      readableOpts.encoding = encoding;
    }

    const stream = new Readable(readableOpts);
    stream.path = path;
    stream.flags = flags;
    stream.mode = mode;
    stream.start = start;
    stream.end = end;
    stream.pos = position;
    stream.bytesRead = 0;
    stream.autoClose = autoClose;

    if (fdOpt !== null) {
      stream.fd = isFileHandle ? fdOpt.fd : fdOpt;
      stream.pending = false;
      openPromise = Promise.resolve();
      queueMicrotask(() => {
        if (!stream.destroyed) stream.emit("ready");
      });
    } else {
      stream.fd = null;
      stream.pending = true;
    }

    function ensureOpen() {
      if (openPromise !== null) return openPromise;
      openPromise = new Promise((resolve, reject) => {
        if (stream.destroyed) {
          resolve();
          return;
        }
        const openFn = customFs ? customFs.open : fs.open;
        openFn(path, flags, mode, (err, openedFd) => {
          if (err) {
            reject(err);
            return;
          }
          if (stream.destroyed) {
            const closeFn =
              customFs && typeof customFs.close === "function" ? customFs.close : fs.close;
            closeFn(openedFd, () => resolve());
            return;
          }
          stream.fd = openedFd;
          stream.pending = false;
          stream.emit("open", openedFd);
          stream.emit("ready");
          resolve();
        });
      });
      return openPromise;
    }

    if (fdOpt === null) {
      queueMicrotask(() => {
        if (!stream.destroyed && !(signal && signal.aborted)) {
          ensureOpen().catch((err) => {
            if (!stream.destroyed) stream.destroy(err);
          });
        }
      });
    }

    stream.close = function (cb) {
      forceClose = true;
      if (typeof cb === "function") {
        if (closed) {
          queueMicrotask(() => cb(null));
        } else {
          stream.once("close", () => cb(null));
        }
      }
      if (!stream.destroyed) {
        stream.destroy();
      } else if (!closed) {
        closeUnderlying(() => {
          if (emitClose) stream.emit("close");
        });
      }
    };

    if (signal !== null) {
      if (signal.aborted) {
        queueMicrotask(() => {
          if (!stream.destroyed) stream.destroy(makeAbortError(signal));
        });
      } else if (typeof signal.addEventListener === "function") {
        const onAbort = () => {
          if (!stream.destroyed) stream.destroy(makeAbortError(signal));
        };
        signal.addEventListener("abort", onAbort, { once: true });
        abortCleanup = () => {
          if (typeof signal.removeEventListener === "function") {
            signal.removeEventListener("abort", onAbort);
          }
        };
      }
    }

    const ReadStreamCtor = globalThis.__lexeFsReadStreamClass;
    if (typeof ReadStreamCtor === "function" && stream instanceof Readable) {
      Object.setPrototypeOf(stream, ReadStreamCtor.prototype);
    }
    return stream;
  };
})()"#;

fn create_impl_function<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    ctx.eval(CREATE_READ_STREAM_IMPL_JS)
}

pub(crate) fn has_fd_option(options: Option<&Value<'_>>) -> bool {
    options
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, Option<Value>>("fd").ok().flatten())
        .is_some_and(|v| !v.is_null() && !v.is_undefined())
}

/// `fs.createReadStream(path[, options])` — path coercion in Rust, stream body in JS.
pub fn create_read_stream<'js>(
    ctx: Ctx<'js>,
    path: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let coerced_path =
        if has_fd_option(options.0.as_ref()) && (path.is_null() || path.is_undefined()) {
            Value::new_null(ctx.clone())
        } else {
            rquickjs::IntoJs::into_js(path_from_value_create_read_stream(&ctx, path)?, &ctx)?
        };
    let impl_fn = create_impl_function(&ctx)?;
    match options.0 {
        Some(opts) => impl_fn.call((coerced_path, opts)),
        None => impl_fn.call((coerced_path,)),
    }
}

/// Build the `fs.createReadStream` export.
pub fn create_create_read_stream<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    Function::new(ctx.clone(), create_read_stream)
}
