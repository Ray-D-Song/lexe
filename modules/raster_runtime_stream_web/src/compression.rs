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

  // A chunk as bytes: Buffers, typed arrays and array buffers all arrive here.
  const toBytes = (chunk) => {
    if (chunk instanceof Uint8Array) return chunk;
    if (chunk instanceof ArrayBuffer) return new Uint8Array(chunk);
    if (ArrayBuffer.isView(chunk)) {
      return new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength);
    }
    // A string is not a valid chunk for these streams; Node throws.
    throw new TypeError("This TransformStream is not a string transform");
  };

  const makeClass = (name, kind) => {
    const TransformStreamRef = globalThis.TransformStream;

    return class extends TransformStreamRef {
      #format;
      // A plain holder rather than instance fields: the `start` handler runs
      // before the derived class's private fields are initialised, so it cannot
      // touch them.
      #state = { handle: null, controller: null, pendingError: null };

      constructor(format) {
        const state = { handle: null, controller: null, pendingError: null, drain: null, drainReject: null };
        super({
          start: (controller) => {
            state.controller = controller;
          },
          transform: (chunk, controller) => {
            state.controller = controller;
            // Error the stream before throwing: if the transform rejects
            // without this, `flush` still waits on the zlib handle's `end`,
            // which never arrives for input we refused.
            let bytes;
            try {
              bytes = toBytes(chunk);
            } catch (error) {
              state.pendingError = error;
              controller.error(error);
              throw error;
            }
            const handle = state.handle;
            if (!handle || state.pendingError) {
              const error =
                state.pendingError || new Error("The stream has already failed");
              state.pendingError = null;
              controller.error(error);
              throw error;
            }
            // A decode failure is thrown *synchronously* by `write` (the handle
            // reports invalid data as an exception, not as an `error` event), so
            // it escapes this function without the handle ever emitting. Clean
            // up here, otherwise the live zlib handle keeps the transform
            // reachable and the runtime reports a leaked object at exit.
            let more;
            try {
              more = handle.write(bytes);
            } catch (error) {
              if (state.drain) {
                handle.off("drain", state.drain);
                state.drain = null;
              }
              if (state.drainReject) {
                const rejectDrain = state.drainReject;
                state.drainReject = null;
                try {
                  rejectDrain(error);
                } catch {}
              }
              state.detach();
              try {
                handle.destroy();
              } catch {}
              state.handle = null;
              state.controller = null;
              state.pendingError = null;
              controller.error(error);
              throw error;
            }
            if (more === false) {
              // The handle signals write readiness with `drain`. If it errors
              // instead, nothing will ever drain, so this promise must reject:
              // a waiter left pending forever keeps the transform (and the
              // handle it closes over) reachable, which the runtime reports as
              // a leaked object at exit.
              const drainPromise = new Promise((resolve, reject) => {
                const onDrain = () => {
                  handle.off("drain", onDrain);
                  state.drain = null;
                  state.drainReject = null;
                  resolve();
                };
                state.drain = onDrain;
                state.drainReject = reject;
                handle.on("drain", onDrain);
              });
              // The error already reaches JavaScript through the stream
              // (`controller.error`); this rejection exists only so the
              // transform stops waiting. Marking it handled keeps it from also
              // surfacing as an unhandled rejection.
              drainPromise.catch(() => {});
              return drainPromise;
            }
            return undefined;
          },
          flush: () => {
            const handle = state.handle;
            return new Promise((resolve, reject) => {
              if (!handle || state.pendingError) {
                const error =
                  state.pendingError || new Error("The stream has already failed");
                state.detach();
                try {
                  if (handle) handle.destroy();
                } catch {}
                state.handle = null;
                state.controller = null;
                state.pendingError = null;
                reject(error);
                return;
              }
              // Both listeners are removed on either outcome: a leftover
              // `error` handler keeps this transform reachable from the zlib
              // handle, which is a cycle the runtime cannot collect.
              const onError = (error) => {
                handle.off("end", onEnd);
                state.detach();
                state.handle = null;
                state.controller = null;
                state.pendingError = null;
                reject(error);
              };
              const onEnd = () => {
                // The zlib stream finished; the last `data` events already
                // enqueued their chunks.
                handle.off("error", onError);
                state.detach();
                resolve();
              };
              handle.once("end", onEnd);
              handle.once("error", onError);
              handle.end();
            });
          },
        });

        this.#state = state;

        if (typeof format !== "string" || !FORMATS.has(format)) {
          // Node reports a TypeError for an unsupported format rather than
          // silently doing nothing.
          throw new TypeError(
            'The "format" argument must be one of "deflate", "deflate-raw", or "gzip". Received ' +
              JSON.stringify(format)
          );
        }

        this.#format = format;
        const handle = factoryFor(kind, format);
        state.handle = handle;

        const onData = (chunk) => {
          if (!state.controller) return;
          try {
            // Copy: the zlib buffer is reused between events.
            state.controller.enqueue(new Uint8Array(chunk));
          } catch (error) {
            // The readable side was cancelled; stop feeding it.
            state.pendingError = error;
          }
        };
        const onError = (error) => {
          // An inflate failure surfaces here. Store it so the pending
          // `transform`/`flush` reports it, and error the readable side so a
          // consumer sees it without waiting for a flush.
          state.pendingError = error;
          // `controller.error()` rejects the stream, and a rejected transform's
          // `flush` never runs, so nothing else will release this handle. Drop
          // the listeners and destroy it here: a live zlib handle keeps the
          // transform reachable, which the runtime reports as a leaked object
          // (`gc_obj_list` assertion) when the worker exits.
          state.detach();
          try {
            handle.destroy();
          } catch {}
          const controller = state.controller;
          if (state.drain) {
            handle.off("drain", state.drain);
            state.drain = null;
          }
          if (state.drainReject) {
            const rejectDrain = state.drainReject;
            state.drainReject = null;
            try {
              rejectDrain(error);
            } catch {}
          }
          state.handle = null;
          state.controller = null;
          state.pendingError = null;
          if (controller) {
            try {
              controller.error(error);
            } catch {}
          }
        };
        handle.on("data", onData);
        handle.on("error", onError);
        // Kept on the state so `flush` can detach them.
        state.detach = () => {
          handle.off("data", onData);
          handle.off("error", onError);
        };
      }

      get format() {
        return this.#format;
      }

      get encoding() {
        return this.#format;
      }
    };
  };

  const CompressionStream = makeClass("CompressionStream", "compress");
  const DecompressionStream = makeClass("DecompressionStream", "decompress");

  return { CompressionStream, DecompressionStream };
})()"#;
