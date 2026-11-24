// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `fs.createReadStream` built on embedded `stream.Readable.from(asyncGenerator)`.
//!
//! Path coercion reuses [`crate::realpath::path_from_value`] (string / Buffer /
//! Uint8Array with strict UTF-8 / file URL). Does not implement a separate
//! ReadStream class, `fd`, AbortSignal, custom `fs` providers, or
//! `autoClose: false`. Open/read errors surface via the Readable `error` event.

use rquickjs::{function::Opt, Ctx, Function, Result, Value};

use crate::realpath::path_from_value_create_read_stream;

/// JS implementation that expects a pre-coerced string path.
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

    if (opts.fd != null) {
      throw new Error("fs.createReadStream: fd is not supported");
    }
    if (opts.signal != null) {
      throw new Error("fs.createReadStream: AbortSignal is not supported");
    }
    if (opts.fs != null) {
      throw new Error("fs.createReadStream: custom fs provider is not supported");
    }
    // Without a Node ReadStream class there is no close()/fd API to reclaim the
    // handle, so autoClose:false would permanently leak the open FileHandle.
    if (opts.autoClose === false) {
      throw new Error(
        "fs.createReadStream: autoClose:false is not supported (no ReadStream.close()/fd surface)"
      );
    }

    const flags = opts.flags == null ? "r" : opts.flags;
    const encoding = opts.encoding == null ? null : opts.encoding;
    let highWaterMark = opts.highWaterMark == null ? 64 * 1024 : opts.highWaterMark;
    const start = opts.start;
    const end = opts.end;

    if (typeof highWaterMark !== "number" || !Number.isInteger(highWaterMark) || highWaterMark <= 0) {
      throw new RangeError('The value of "options.highWaterMark" is out of range. It must be a positive integer.');
    }
    if (start !== undefined && start !== null) {
      if (typeof start !== "number" || !Number.isInteger(start) || start < 0) {
        throw new RangeError('The value of "options.start" is out of range. It must be a non-negative integer.');
      }
    }
    if (end !== undefined && end !== null) {
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

    const { Readable } = require("stream");
    const fsp = require("fs/promises");
    const { Buffer } = require("buffer");

    const hasRange = (start !== undefined && start !== null) || (end !== undefined && end !== null);
    const rangeStart = start == null ? 0 : start;
    const rangeEnd = end; // inclusive

    async function* generate() {
      const fh = await fsp.open(path, flags);
      try {
        let position = rangeStart;
        for (;;) {
          let toRead = highWaterMark;
          if (rangeEnd !== undefined && rangeEnd !== null) {
            const remaining = rangeEnd - position + 1;
            if (remaining <= 0) {
              break;
            }
            toRead = Math.min(toRead, remaining);
          }
          const buf = Buffer.alloc(toRead);
          const readArgs = hasRange
            ? [buf, 0, toRead, position]
            : [buf, 0, toRead];
          const { bytesRead } = await fh.read(...readArgs);
          if (bytesRead === 0) {
            break;
          }
          position += bytesRead;
          yield buf.subarray(0, bytesRead);
          if (rangeEnd !== undefined && rangeEnd !== null && position > rangeEnd) {
            break;
          }
        }
      } finally {
        await fh.close();
      }
    }

    const streamOpts = { highWaterMark };
    if (encoding != null) {
      streamOpts.encoding = encoding;
    }
    const stream = Readable.from(generate(), streamOpts);
    stream.path = path;
    stream.pending = false;
    // `fs.createReadStream` returns a `ReadStream` in Node, and callers branch
    // on `instanceof fs.ReadStream`. Re-parent the instance onto that prototype
    // so both identities hold without a second stream implementation.
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

/// `fs.createReadStream(path[, options])` — path coercion in Rust, stream body in JS.
pub fn create_read_stream<'js>(
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

/// Build the `fs.createReadStream` export.
pub fn create_create_read_stream<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    Function::new(ctx.clone(), create_read_stream)
}
