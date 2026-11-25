// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `fs.createWriteStream` built on embedded `stream.Writable`.
//!
//! Path coercion reuses [`crate::realpath::path_from_value_create_read_stream`]
//! unless `options.fd` is supplied with `null`/`undefined` path. Supports
//! `fd` (numeric descriptor or `FileHandle`), `signal` (`AbortSignal`),
//! custom `fs` providers, `autoClose`, `emitClose`, and `start`.

use rquickjs::{function::Opt, Ctx, Function, Result, Value};

use crate::read_stream::has_fd_option;
use crate::realpath::path_from_value_create_read_stream;

/// JS body. Expects pre-coerced path (string or null when `options.fd` is set).
const CREATE_WRITE_STREAM_IMPL_JS: &str = r#"(function () {
  return function createWriteStreamImpl(path, options) {
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
      if (typeof fdOpt === "object" && typeof fdOpt.write === "function") {
        isFileHandle = true;
      } else if (typeof fdOpt !== "number" || !Number.isInteger(fdOpt) || fdOpt < 0) {
        const err = new TypeError('The "options.fd" property must be of type number or FileHandle');
        err.code = "ERR_INVALID_ARG_TYPE";
        throw err;
      }
    }

    const flags = opts.flags == null ? "w" : opts.flags;
    const encoding = opts.encoding == null ? "utf8" : opts.encoding;
    const mode = opts.mode === undefined ? 0o666 : opts.mode;
    const autoClose = opts.autoClose === undefined ? true : Boolean(opts.autoClose);
    const emitClose = opts.emitClose === undefined ? true : Boolean(opts.emitClose);
    const highWaterMark = opts.highWaterMark == null ? 16 * 1024 : opts.highWaterMark;
    const start = opts.start;
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
    if (customFs !== null) {
      if (
        typeof customFs !== "object" ||
        (typeof customFs.write !== "function" && typeof customFs.writev !== "function")
      ) {
        const err = new TypeError('The "options.fs.write" property must be of type function');
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

    const { Writable } = require("stream");
    const fs = require("fs");
    const { Buffer } = require("buffer");

    let fileHandle = isFileHandle ? fdOpt : null;
    let openPromise = null;
    let position = start === undefined || start === null ? null : start;
    let closed = false;
    let forceClose = false;
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

    function toBuffer(chunk, enc) {
      if (Buffer.isBuffer(chunk)) return chunk;
      if (typeof chunk === "string") {
        const resolvedEnc = enc && enc !== "buffer" ? enc : encoding;
        return Buffer.from(chunk, resolvedEnc || "utf8");
      }
      if (chunk instanceof Uint8Array) return Buffer.from(chunk);
      throw new TypeError('The "chunk" argument must be of type string or Buffer');
    }

    const writableOpts = {
      highWaterMark,
      decodeStrings: false,
      autoDestroy: autoClose,
      emitClose,
      write(chunk, enc, cb) {
        let buf;
        try {
          buf = toBuffer(chunk, enc);
        } catch (err) {
          cb(err);
          return;
        }
        stream.bytesWritten += buf.length;
        ensureOpen().then(
          () => {
            if (stream.destroyed) {
              cb(new Error("WriteStream destroyed"));
              return;
            }
            if (fileHandle !== null) {
              const args = position === null ? [buf, 0, buf.length] : [buf, 0, buf.length, position];
              fileHandle.write(...args).then(
                (result) => {
                  if (position !== null) position += result.bytesWritten;
                  cb();
                },
                (err) => cb(err)
              );
            } else if (customFs && typeof customFs.write === "function") {
              customFs.write(stream.fd, buf, 0, buf.length, position, (err, bytesWritten) => {
                if (err) {
                  cb(err);
                  return;
                }
                if (position !== null) position += bytesWritten;
                cb();
              });
            } else if (customFs && typeof customFs.writev === "function") {
              customFs.writev(stream.fd, [buf], position, (err, bytesWritten) => {
                if (err) {
                  cb(err);
                  return;
                }
                if (position !== null) position += bytesWritten;
                cb();
              });
            } else {
              fs.write(stream.fd, buf, 0, buf.length, position, (err, bytesWritten) => {
                if (err) {
                  cb(err);
                  return;
                }
                if (position !== null) position += bytesWritten;
                cb();
              });
            }
          },
          (err) => cb(err)
        );
      },
      final(cb) {
        ensureOpen().then(() => cb(), cb);
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

    if (customFs && typeof customFs.writev === "function") {
      writableOpts.writev = function (chunks, cb) {
        const bufs = [];
        let total = 0;
        try {
          for (const entry of chunks) {
            const b = toBuffer(entry.chunk, entry.encoding);
            total += b.length;
            bufs.push(b);
          }
        } catch (err) {
          cb(err);
          return;
        }
        stream.bytesWritten += total;
        ensureOpen().then(
          () => {
            customFs.writev(stream.fd, bufs, position, (err, bytesWritten) => {
              if (err) {
                cb(err);
                return;
              }
              if (position !== null) position += bytesWritten;
              cb();
            });
          },
          (err) => cb(err)
        );
      };
    }

    const stream = new Writable(writableOpts);
    stream.path = path;
    stream.flags = flags;
    stream.mode = mode;
    stream.start = start;
    stream.pos = position;
    stream.bytesWritten = 0;
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

    const WriteStreamCtor = globalThis.__lexeFsWriteStreamClass;
    if (typeof WriteStreamCtor === "function") {
      Object.setPrototypeOf(stream, WriteStreamCtor.prototype);
    }
    return stream;
  };
})()"#;

fn create_impl_function<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    ctx.eval(CREATE_WRITE_STREAM_IMPL_JS)
}

/// `fs.createWriteStream(path[, options])` — path coercion in Rust, stream body in JS.
pub fn create_write_stream<'js>(
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

/// Build the `fs.createWriteStream` export.
pub fn create_create_write_stream<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    Function::new(ctx.clone(), create_write_stream)
}
