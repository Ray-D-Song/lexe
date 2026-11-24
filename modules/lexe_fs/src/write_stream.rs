// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `fs.createWriteStream` built on embedded `stream.Writable`.
//!
//! Path coercion reuses [`crate::realpath::path_from_value`] (string / Buffer /
//! Uint8Array with strict UTF-8 / file URL), matching `createReadStream`.
//!
//! Unlike Node's `WriteStream` this is not a distinct class: `instanceof
//! fs.WriteStream` is therefore unsupported, and `fd`, `autoClose: false`, and a
//! custom `fs` provider are rejected the same way `createReadStream` rejects
//! them. The handle is always closed on `finish`/`error`/`destroy`.

use rquickjs::{function::Opt, Ctx, Function, Result, Value};

use crate::realpath::path_from_value_create_read_stream;

/// JS body. Expects a pre-coerced string path.
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

    if (opts.fd != null) {
      throw new Error("fs.createWriteStream: fd is not supported");
    }
    if (opts.signal != null) {
      throw new Error("fs.createWriteStream: AbortSignal is not supported");
    }
    if (opts.fs != null) {
      throw new Error("fs.createWriteStream: custom fs provider is not supported");
    }
    if (opts.autoClose === false) {
      throw new Error(
        "fs.createWriteStream: autoClose:false is not supported (no WriteStream.close()/fd surface)"
      );
    }

    const flags = opts.flags == null ? "w" : opts.flags;
    const encoding = opts.encoding == null ? null : opts.encoding;
    const mode = opts.mode === undefined ? 0o666 : opts.mode;
    let highWaterMark = opts.highWaterMark == null ? 16 * 1024 : opts.highWaterMark;
    const start = opts.start;

    if (typeof highWaterMark !== "number" || !Number.isInteger(highWaterMark) || highWaterMark <= 0) {
      throw new RangeError('The value of "options.highWaterMark" is out of range. It must be a positive integer.');
    }
    if (start !== undefined && start !== null) {
      if (typeof start !== "number" || !Number.isInteger(start) || start < 0) {
        throw new RangeError('The value of "options.start" is out of range. It must be a non-negative integer.');
      }
    }

    const { Writable } = require("stream");
    const fsp = require("fs/promises");
    const { Buffer } = require("buffer");

    // Open lazily on the first write so a stream that is created and dropped
    // without writing does not leave an empty file behind (Node does open
    // eagerly, but lazy opening avoids leaking a handle when nothing is written).
    let handlePromise = null;
    let position = start === undefined || start === null ? null : start;
    let closed = false;

    function openOnce() {
      if (handlePromise === null) {
        handlePromise = fsp.open(path, flags, mode);
      }
      return handlePromise;
    }

    async function closeOnce() {
      if (closed) return;
      closed = true;
      if (handlePromise === null) return;
      try {
        const fh = await handlePromise;
        await fh.close();
      } catch (_) {
        // Already closed / never opened: nothing left to release.
      }
    }

    function toBuffer(chunk, enc) {
      if (Buffer.isBuffer(chunk)) return chunk;
      if (typeof chunk === "string") {
        return Buffer.from(chunk, enc || encoding || "utf8");
      }
      if (chunk instanceof Uint8Array) return Buffer.from(chunk);
      throw new TypeError('The "chunk" argument must be of type string or Buffer');
    }

    const stream = new Writable({
      highWaterMark,
      decodeStrings: false,
      write(chunk, enc, cb) {
        // Strings were already preserved by `decodeStrings: false`, so this can
        // honour `encoding` from options on a per-chunk basis.
        let buf;
        try {
          buf = toBuffer(chunk, enc);
        } catch (err) {
          cb(err);
          return;
        }
        stream.bytesWritten += buf.length;
        openOnce().then(
          (fh) => {
            const writeArgs = position === null ? [buf] : [buf, 0, buf.length, position];
            fh.write(...writeArgs).then(
              (result) => {
                if (position !== null) {
                  position += result.bytesWritten;
                }
                cb();
              },
              (err) => cb(err)
            );
          },
          (err) => cb(err)
        );
      },
      final(cb) {
        closeOnce().then(() => cb(), cb);
      },
      destroy(err, cb) {
        closeOnce().then(() => cb(err), () => cb(err));
      },
    });

    stream.path = path;
    stream.bytesWritten = 0;

    stream.close = function (cb) {
      closeOnce().then(
        () => {
          if (typeof cb === "function") cb();
          stream.emit("close");
        },
        (err) => {
          if (typeof cb === "function") cb(err);
          else stream.emit("error", err);
        }
      );
    };

    // `fs.createWriteStream` returns a `WriteStream` in Node, and callers
    // branch on `instanceof fs.WriteStream`.
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
    let path = path_from_value_create_read_stream(&ctx, path)?;
    let impl_fn = create_impl_function(&ctx)?;
    match options.0 {
        Some(opts) => impl_fn.call((path, opts)),
        None => impl_fn.call((path,)),
    }
}

/// Build the `fs.createWriteStream` export.
pub fn create_create_write_stream<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    Function::new(ctx.clone(), create_write_stream)
}
