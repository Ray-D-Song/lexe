// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `CompressionStream` and `DecompressionStream`.
//!
//! These are `TransformStream` subclasses over the zlib streaming factories
//! (`createGzip`, `createDeflate`, `createDeflateRaw` and their inflate
//! counterparts), so the compression itself lives in one place — this module
//! only bridges the Node-style Duplex the factories return onto the WHATWG
//! transform contract.
//!
//! Byte chunks go in and come out as `Uint8Array`; `format` is one of `gzip`,
//! `deflate` or `deflate-raw`.

use rquickjs::{Ctx, Result, Value};

/// Install the two constructors on the global object.
pub fn install_compression_streams(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    if globals.contains_key("CompressionStream")? {
        return Ok(());
    }

    let value: rquickjs::Object = ctx.eval(COMPRESSION_STREAM_JS)?;
    globals.set(
        "CompressionStream",
        value.get::<_, Value>("CompressionStream")?,
    )?;
    globals.set(
        "DecompressionStream",
        value.get::<_, Value>("DecompressionStream")?,
    )?;
    Ok(())
}

/// Export both constructors from the `stream/web` module.
pub fn export_compression_streams(default: &rquickjs::Object<'_>) -> Result<()> {
    let globals = default.ctx().globals();
    default.set(
        "CompressionStream",
        globals.get::<_, Value>("CompressionStream")?,
    )?;
    default.set(
        "DecompressionStream",
        globals.get::<_, Value>("DecompressionStream")?,
    )?;
    Ok(())
}

/// The transform subclass shared by both directions.
///
/// `kind` selects the zlib factory pair; `compress` picks the direction.
const COMPRESSION_STREAM_JS: &str = r#"(function () {
  "use strict";

  const FORMATS = new Set(["gzip", "deflate", "deflate-raw"]);

  const arrayBufferByteLength = Object.getOwnPropertyDescriptor(
    ArrayBuffer.prototype,
    "byteLength"
  )?.get;
  const sharedArrayBufferByteLength =
    typeof SharedArrayBuffer === "function"
      ? Object.getOwnPropertyDescriptor(SharedArrayBuffer.prototype, "byteLength")?.get
      : undefined;

  const isArrayBufferLike = (chunk) => {
    if (chunk instanceof ArrayBuffer) return true;
    if (typeof SharedArrayBuffer === "function" && chunk instanceof SharedArrayBuffer) {
      return true;
    }
    if (arrayBufferByteLength) {
      try {
        arrayBufferByteLength.call(chunk);
        return true;
      } catch {}
    }
    if (sharedArrayBufferByteLength) {
      try {
        sharedArrayBufferByteLength.call(chunk);
        return true;
      } catch {}
    }
    return false;
  };

  const factoryFor = (kind, format) => {
    const zlib = require("node:zlib");
    if (kind === "compress") {
      if (format === "gzip") return zlib.createGzip();
      if (format === "deflate") return zlib.createDeflate();
      return zlib.createDeflateRaw();
    }
    if (format === "gzip") return zlib.createGunzip();
    if (format === "deflate") return zlib.createInflate();
    return zlib.createInflateRaw();
  };

  // A chunk as bytes: Buffers, typed arrays and array buffers (including cross-realm) all arrive here.
  const toBytes = (chunk) => {
    if (chunk instanceof Uint8Array) return chunk;
    if (ArrayBuffer.isView(chunk)) {
      return new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength);
    }
    if (isArrayBufferLike(chunk)) {
      return new Uint8Array(chunk);
    }
    // A string or non-BufferSource value is not a valid chunk for these streams.
    throw new TypeError("The provided value is not of type '(ArrayBuffer or ArrayBufferView)'");
  };

  const makeClass = (name, kind) => {
    const TransformStreamRef = globalThis.TransformStream;

    const toStreamError = (error) => {
      if (kind === "decompress" && !(error instanceof TypeError)) {
        return new TypeError(error && error.message ? error.message : String(error));
      }
      return error;
    };

    const Cls = class {
      #transform;

      constructor(format) {
        if (arguments.length < 1) {
          throw new TypeError(
            "Failed to construct '" +
              name +
              "': 1 argument required, but only 0 present."
          );
        }
        const fmt = `${format}`;
        if (!FORMATS.has(fmt)) {
          // Node reports a TypeError for an unsupported format rather than
          // silently doing nothing.
          throw new TypeError(
            'The "format" argument must be one of "deflate", "deflate-raw", or "gzip". Received ' +
              JSON.stringify(fmt)
          );
        }

        const handle = factoryFor(kind, fmt);
        const state = {
          handle,
          controller: null,
          pendingError: null,
          lastError: null,
          failed: false,
          closed: false,
          totalIn: 0,
          drain: null,
          drainReject: null,
          flushDetach: null,
          flushSettle: null,
          detach: () => {},
        };

        const cleanup = (error) => {
          const h = state.handle;
          if (h) {
            if (state.drain) {
              h.off("drain", state.drain);
              state.drain = null;
            }
            state.detach();
          }
          if (state.flushDetach) {
            const detachFlush = state.flushDetach;
            state.flushDetach = null;
            detachFlush();
          }
          if (state.drainReject) {
            const rejectDrain = state.drainReject;
            state.drainReject = null;
            try {
              rejectDrain(error);
            } catch {}
          }
          if (state.flushSettle) {
            const settleFlush = state.flushSettle;
            state.flushSettle = null;
            try {
              settleFlush(error);
            } catch {}
          }
          if (error !== null && error !== undefined) {
            state.failed = true;
            state.lastError = error;
          }
          state.handle = null;
          state.controller = null;
          state.pendingError = null;
          if (h) {
            try {
              h.destroy();
            } catch {}
          }
        };

        const onData = (chunk) => {
          if (!state.controller || state.closed) return;
          try {
            // Copy: the zlib buffer may be reused between events.
            state.controller.enqueue(new Uint8Array(chunk));
          } catch (error) {
            // The readable side was cancelled; stop feeding it.
            state.pendingError = toStreamError(error);
          }
        };

        const onError = (error) => {
          // An inflate failure surfaces here. Store it so any pending
          // `transform`/`flush` reports it, and error the readable side so a
          // consumer sees it without waiting for a flush.
          const err = toStreamError(error);
          state.pendingError = err;
          // Drop listeners and destroy the handle immediately: a live zlib
          // handle keeps the stream reachable (`gc_obj_list` assertion at exit).
          const controller = state.controller;
          cleanup(err);
          if (controller) {
            try {
              controller.error(err);
            } catch {}
          }
        };

        handle.on("data", onData);
        handle.on("error", onError);
        state.detach = () => {
          handle.off("data", onData);
          handle.off("error", onError);
        };

        this.#transform = new TransformStreamRef({
          start: (controller) => {
            state.controller = controller;
          },
          transform: (chunk, controller) => {
            state.controller = controller;
            let bytes;
            try {
              bytes = toBytes(chunk);
            } catch (error) {
              state.pendingError = error;
              controller.error(error);
              throw error;
            }
            const currentHandle = state.handle;
            if (!currentHandle || state.pendingError || state.failed) {
              const error = toStreamError(
                state.pendingError ||
                  state.lastError ||
                  new TypeError("The stream has already failed")
              );
              cleanup(error);
              controller.error(error);
              throw error;
            }
            state.totalIn += bytes.byteLength;
            let more;
            try {
              more = currentHandle.write(bytes);
            } catch (error) {
              const err = toStreamError(error);
              cleanup(err);
              controller.error(err);
              throw err;
            }
            if (state.failed) {
              const err = state.lastError || new TypeError("The stream has already failed");
              controller.error(err);
              throw err;
            }
            if (state.pendingError) {
              const err = toStreamError(state.pendingError);
              cleanup(err);
              controller.error(err);
              throw err;
            }
            if (more === false) {
              const drainPromise = new Promise((resolve, reject) => {
                const onDrain = () => {
                  currentHandle.off("drain", onDrain);
                  state.drain = null;
                  state.drainReject = null;
                  resolve();
                };
                state.drain = onDrain;
                state.drainReject = reject;
                currentHandle.on("drain", onDrain);
              });
              // Error already reaches the stream via `controller.error`; avoid
              // spurious unhandled rejections if rejected during cleanup/cancel.
              drainPromise.catch(() => {});
              return drainPromise;
            }
            return undefined;
          },
          flush: () => {
            const currentHandle = state.handle;
            return new Promise((resolve, reject) => {
              if (!currentHandle || state.pendingError || state.failed) {
                const error = toStreamError(
                  state.pendingError ||
                    state.lastError ||
                    new TypeError("The stream has already failed")
                );
                cleanup(error);
                reject(error);
                return;
              }
              const onFlushError = (error) => {
                cleanup(toStreamError(error));
              };
              const onEnd = () => {
                let trailingError = null;
                if (kind === "decompress" && fmt === "deflate") {
                  const consumed = currentHandle.bytesWritten;
                  if (consumed < state.totalIn) {
                    trailingError = new TypeError("Junk found after compressed data");
                  }
                }
                cleanup(trailingError);
              };
              state.flushDetach = () => {
                currentHandle.off("end", onEnd);
                currentHandle.off("error", onFlushError);
              };
              state.flushSettle = (error) => {
                if (error !== null && error !== undefined) {
                  reject(error);
                } else {
                  resolve();
                }
              };
              currentHandle.once("end", onEnd);
              currentHandle.once("error", onFlushError);
              currentHandle.end();
            });
          },
          cancel: (reason) => {
            state.closed = true;
            const cancelError =
              reason !== undefined ? reason : new TypeError("Stream cancelled");
            cleanup(cancelError);
          },
        });
      }

      get readable() {
        return this.#transform.readable;
      }

      get writable() {
        return this.#transform.writable;
      }
    };

    Object.defineProperty(Cls, "name", {
      value: name,
      configurable: true,
    });
    Object.defineProperty(Cls.prototype, Symbol.toStringTag, {
      value: name,
      configurable: true,
    });
    return Cls;
  };

  const CompressionStream = makeClass("CompressionStream", "compress");
  const DecompressionStream = makeClass("DecompressionStream", "decompress");

  return { CompressionStream, DecompressionStream };
})()"#;
