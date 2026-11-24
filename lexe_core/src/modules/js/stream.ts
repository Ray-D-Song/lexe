// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
import { EventEmitter as EE } from "node:events";
// @ts-ignore
import { Readable } from "readable-stream/lib/_stream_readable.js";
// @ts-ignore
import { Writable } from "readable-stream/lib/_stream_writable.js";
// @ts-ignore
import { Duplex } from "readable-stream/lib/_stream_duplex.js";
// @ts-ignore
import { Transform } from "readable-stream/lib/_stream_transform.js";
// @ts-ignore
import { PassThrough } from "readable-stream/lib/_stream_passthrough.js";
// @ts-ignore
import { finished } from "readable-stream/lib/internal/streams/end-of-stream.js";
// @ts-ignore
import { pipeline } from "readable-stream/lib/internal/streams/pipeline.js";

import { inherits } from "node:util";

inherits(Stream, EE);
Stream.Readable = Readable;
Stream.Writable = Writable;
Stream.Duplex = Duplex;
Stream.Transform = Transform;
Stream.PassThrough = PassThrough;
Stream.finished = finished;
Stream.pipeline = pipeline;
Stream.Stream = Stream;

// Web stream adapters — bridge bundled Node.js streams to native web streams
const enc = new TextEncoder();

Readable.toWeb = function (readable: any) {
  const iterator = readable[Symbol.asyncIterator]();
  return new ReadableStream({
    async pull(controller: any) {
      const { value, done } = await iterator.next();
      if (done) {
        controller.close();
      } else {
        controller.enqueue(
          typeof value === "string" ? enc.encode(value) : value
        );
      }
    },
    cancel(reason: any) {
      readable.destroy(reason);
    },
  });
};

Readable.fromWeb = function (readableStream: any, options?: any) {
  const reader = readableStream.getReader();
  return new Readable({
    ...options,
    async read() {
      const { value, done } = await reader.read();
      this.push(done ? null : value);
    },
  });
};

Writable.toWeb = function (writable: any) {
  return new WritableStream({
    write(chunk: any) {
      return new Promise<void>((resolve) => {
        if (!writable.write(chunk)) {
          writable.once("drain", resolve);
        } else {
          resolve();
        }
      });
    },
    close() {
      return new Promise<void>((resolve) => {
        writable.end(resolve);
      });
    },
    abort(reason: any) {
      writable.destroy(reason);
    },
  });
};

Writable.fromWeb = function (writableStream: any, options?: any) {
  const writer = writableStream.getWriter();
  return new Writable({
    ...options,
    async write(chunk: any, _encoding: any, callback: any) {
      try {
        await writer.write(chunk);
        callback();
      } catch (e) {
        callback(e);
      }
    },
    final(callback: any) {
      writer.close().then(callback, callback);
    },
  });
};

Duplex.toWeb = function (duplex: any) {
  return {
    readable: Readable.toWeb(duplex),
    writable: Writable.toWeb(duplex),
  };
};

Duplex.fromWeb = function (pair: any, options?: any) {
  const reader = pair.readable.getReader();
  const writer = pair.writable.getWriter();
  return new Duplex({
    ...options,
    async read() {
      const { value, done } = await reader.read();
      this.push(done ? null : value);
    },
    async write(chunk: any, _encoding: any, callback: any) {
      try {
        await writer.write(chunk);
        callback();
      } catch (e) {
        callback(e);
      }
    },
    final(callback: any) {
      writer.close().then(callback, callback);
    },
  });
};

function Stream(this: any) {
  EE.call(this);
}

Stream.prototype.pipe = function (dest: any, options: any) {
  var source = this;

  function ondata(chunk: any) {
    if (dest.writable) {
      if (false === dest.write(chunk) && source.pause) {
        source.pause();
      }
    }
  }

  source.on("data", ondata);

  function ondrain() {
    if (source.readable && source.resume) {
      source.resume();
    }
  }

  dest.on("drain", ondrain);

  // If the 'end' option is not supplied, dest.end() will be called when
  // source gets the 'end' or 'close' events.  Only dest.end() once.
  if (!dest._isStdio && (!options || options.end !== false)) {
    source.on("end", onend);
    source.on("close", onclose);
  }

  var didOnEnd = false;
  function onend() {
    if (didOnEnd) return;
    didOnEnd = true;

    dest.end();
  }

  function onclose() {
    if (didOnEnd) return;
    didOnEnd = true;

    if (typeof dest.destroy === "function") dest.destroy();
  }

  // don't leave dangling pipes when there are errors.
  function onerror(er: any) {
    cleanup();
    if (source.listenerCount("error") === 0) {
      throw er; // Unhandled stream error in pipe.
    }
  }

  source.on("error", onerror);
  dest.on("error", onerror);

  // remove all the event listeners that were added.
  function cleanup() {
    source.removeListener("data", ondata);
    dest.removeListener("drain", ondrain);

    source.removeListener("end", onend);
    source.removeListener("close", onclose);

    source.removeListener("error", onerror);
    dest.removeListener("error", onerror);

    source.removeListener("end", cleanup);
    source.removeListener("close", cleanup);

    dest.removeListener("close", cleanup);
  }

  source.on("end", cleanup);
  source.on("close", cleanup);

  dest.on("close", cleanup);

  dest.emit("pipe", source);

  // Allow for unix-like usage: A.pipe(B).pipe(C)
  return dest;
};

// ---------------------------------------------------------------------------
// Stream state predicates and composition helpers.
//
// Semantics follow `refs/node/lib/internal/streams/utils.js`: each predicate
// returns `null` (not `false`) when the argument is not a Node stream, because
// callers use that to distinguish "not applicable" from "known false".
// ---------------------------------------------------------------------------

function isNodeStream(obj: any): boolean {
  return !!(
    obj &&
    (obj._readableState ||
      obj._writableState ||
      (typeof obj.write === "function" && typeof obj.on === "function") ||
      (typeof obj.pipe === "function" && typeof obj.on === "function"))
  );
}

function isReadableNodeStream(obj: any, strict = false): boolean {
  return !!(
    obj &&
    typeof obj.pipe === "function" &&
    typeof obj.on === "function" &&
    (!strict ||
      (typeof obj.pause === "function" && typeof obj.resume === "function")) &&
    (!obj._writableState || obj._readableState?.readable !== false) &&
    (!obj._writableState || obj._readableState)
  );
}

function isWritableNodeStream(obj: any): boolean {
  return !!(
    obj &&
    typeof obj.write === "function" &&
    typeof obj.on === "function" &&
    (!obj._readableState || obj._writableState?.writable !== false)
  );
}

function isReadableStream(obj: any): boolean {
  return !!(
    obj &&
    !isNodeStream(obj) &&
    typeof obj.pipeThrough === "function" &&
    typeof obj.getReader === "function" &&
    typeof obj.cancel === "function"
  );
}

function isWritableStream(obj: any): boolean {
  return !!(
    obj &&
    !isNodeStream(obj) &&
    typeof obj.getWriter === "function" &&
    typeof obj.abort === "function"
  );
}

function isDestroyed(stream: any): boolean | null {
  if (!isNodeStream(stream)) return null;
  const wState = stream._writableState;
  const rState = stream._readableState;
  const state = wState || rState;
  return !!(stream.destroyed || state?.destroyed);
}

function isDisturbed(stream: any): boolean {
  return !!(
    stream &&
    (stream.readableDidRead ||
      stream.readableAborted ||
      stream._readableState?.dataEmitted ||
      stream._readableState?.emittedReadable ||
      stream._readableState?.endEmitted ||
      stream._writableState?.ended)
  );
}

function isErrored(stream: any): boolean {
  return !!(
    stream &&
    (stream.errored ||
      stream._readableState?.errored ||
      stream._writableState?.errored)
  );
}

function isReadable(stream: any): boolean | null {
  if (stream && typeof stream.readable === "boolean") {
    if (!isReadableNodeStream(stream)) return stream.readable;
  }
  if (typeof stream?.readable !== "boolean") return null;
  if (isDestroyed(stream)) return false;
  return isReadableNodeStream(stream) && stream.readable && !isReadableFinished(stream);
}

function isWritable(stream: any): boolean | null {
  if (typeof stream?.writable !== "boolean") return null;
  if (isDestroyed(stream)) return false;
  return (
    isWritableNodeStream(stream) && stream.writable && !isWritableEnded(stream)
  );
}

function isReadableFinished(stream: any, strict?: boolean): boolean | null {
  if (!isReadableNodeStream(stream)) return null;
  const rState = stream._readableState;
  if (rState?.errored) return false;
  if (typeof rState?.endEmitted !== "boolean") return null;
  return !!(
    rState.endEmitted ||
    (strict === false && rState.ended === true && rState.length === 0)
  );
}

function isWritableEnded(stream: any): boolean | null {
  if (!isWritableNodeStream(stream)) return null;
  if (stream.writableEnded === true) return true;
  const wState = stream._writableState;
  if (wState?.errored) return false;
  if (typeof wState?.ended !== "boolean") return null;
  return wState.ended;
}

function destroy(stream: any, error?: any): any {
  if (isNodeStream(stream)) {
    if (typeof stream.destroy === "function") {
      stream.destroy(error);
    }
    return stream;
  }
  if (isReadableStream(stream)) {
    stream.cancel(error);
    return stream;
  }
  return stream;
}

const kHighWaterMark = Symbol.for("lexe.stream.defaultHighWaterMark");
let defaultHighWaterMark: any = undefined;

function getDefaultHighWaterMark(objectMode: boolean = false) {
  if (defaultHighWaterMark === undefined) {
    return objectMode ? 16 : 16 * 1024;
  }
  return objectMode ? defaultHighWaterMark.object : defaultHighWaterMark.bytes;
}

function setDefaultHighWaterMark(objectMode: boolean, value: number) {
  if (typeof value !== "number" || value !== value || value < 0) {
    throw new TypeError(
      'The "value" argument must be of type number. Received ' + value
    );
  }
  if (defaultHighWaterMark === undefined) {
    defaultHighWaterMark = { object: 16, bytes: 16 * 1024 };
  }
  if (objectMode) {
    defaultHighWaterMark.object = value;
  } else {
    defaultHighWaterMark.bytes = value;
  }
  return value;
}

/** Abort every stream in `streams` when `signal` aborts. */
function addAbortSignal(signal: AbortSignal, stream: any): any {
  const streams =
    Array.isArray(stream) || isNodeStream(stream) || isReadableStream(stream)
      ? stream
      : [stream];
  const list = Array.isArray(streams) ? streams : [streams];

  const onAbort = () => {
    for (const s of list) {
      const err = new Error("The operation was aborted");
      err.name = "AbortError";
      (err as any).code = "ABORT_ERR";
      (err as any).cause = (signal as any).reason;
      destroy(s, err);
    }
  };

  if (signal.aborted) {
    onAbort();
  } else {
    signal.addEventListener("abort", onAbort, { once: true });
  }

  // Node attaches the signal so `pipeline` can track it.
  if (isNodeStream(stream)) {
    (stream as any)._abortSignal = signal;
  }
  return stream;
}

/**
 * Chain streams so the last one receives everything the first emits.
 *
 * Node distinguishes pipelines (async iterators / functions / web streams) from
 * plain Node streams; here both paths collapse to the same duplex wiring.
 */
function compose(...streams: any[]): any {
  if (streams.length === 0) {
    throw new TypeError("The 'streams' argument must be specified");
  }
  if (streams.length === 1) {
    return streams[0];
  }

  // A leading function is a pipeline source (Node allows `compose(fn, ...)`).
  let source = streams[0];
  let startIndex = 1;
  if (typeof source === "function" && !isNodeStream(source)) {
    const factory = source;
    const iterableSource = factory({ signal: undefined });
    source = Readable.from(iterableSource as any);
  }

  for (let i = startIndex; i < streams.length; i++) {
    const next = streams[i];
    if (next == null) continue;
    source = source.pipe(next);
  }
  return source;
}

/** Two Duplex objects wired to each other (Node's `duplexPair()`). */
function duplexPair(options?: any): [Duplex, Duplex] {
  let a: Duplex;
  let b: Duplex;

  const make = () =>
    new Duplex({
      ...options,
      write(chunk: any, _enc: string, cb: () => void) {
        const other = this === a ? b : a;
        if (!other.push(chunk)) {
          // Wait for the reader to drain before acking the writer.
          other.once("drain", cb);
        } else {
          cb();
        }
      },
      final(cb: () => void) {
        const other = this === a ? b : a;
        other.push(null);
        cb();
      },
      read() {},
    });

  a = make();
  b = make();
  return [a, b];
}

// Attach the helpers to the default export too: `stream.isReadable(s)` and
// `stream.compose(a, b)` are how most code reaches them.
Object.assign(Stream, {
  isReadable,
  isWritable,
  isDestroyed,
  isDisturbed,
  isErrored,
  isNodeStream,
  isReadableStream,
  isWritableStream,
  compose,
  duplexPair,
  addAbortSignal,
  getDefaultHighWaterMark,
  setDefaultHighWaterMark,
  destroy,
});

export default Stream;
export {
  Readable,
  Writable,
  Duplex,
  Transform,
  PassThrough,
  finished,
  pipeline,
  Stream,
  isReadable,
  isWritable,
  isDestroyed,
  isDisturbed,
  isErrored,
  isNodeStream,
  isReadableStream,
  isWritableStream,
  compose,
  duplexPair,
  addAbortSignal,
  getDefaultHighWaterMark,
  setDefaultHighWaterMark,
  destroy,
};
