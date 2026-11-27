// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{prelude::Opt, Class, Ctx, Exception, Function, Object, Result, Value};

use crate::{server::Http2Server, stream::Http2Stream};

const WRAP_KEY: &str = "__lexeWrapRequestResponse";

const COMPAT_CLASSES_SCRIPT: &str = r#"(function (target, Http2StreamCtor) {
  let ReadableBase = null;
  let StreamBase = null;
  if (typeof require === "function") {
    try {
      const streamMod = require("node:stream");
      if (streamMod && typeof streamMod.Readable === "function") {
        ReadableBase = streamMod.Readable;
      }
      if (streamMod && typeof streamMod.Stream === "function") {
        StreamBase = streamMod.Stream;
      }
    } catch {}
  }

  const EventEmitterProto =
    Http2StreamCtor && Http2StreamCtor.prototype
      ? Object.getPrototypeOf(Http2StreamCtor.prototype)
      : null;

  function buildRawHeaders(headers) {
    const raw = [];
    if (!headers || typeof headers !== "object") return raw;
    for (const key of Object.keys(headers)) {
      const val = headers[key];
      if (Array.isArray(val)) {
        for (const item of val) raw.push(key, String(item));
      } else if (val !== undefined) {
        raw.push(key, String(val));
      }
    }
    return raw;
  }

  function Http2ServerRequest(stream, headers, options, rawHeaders) {
    if (ReadableBase) {
      ReadableBase.call(this, options);
    }
    this._stream = stream || null;
    this._headers = headers || (stream && stream.headers) || {};
    this._rawHeaders = Array.isArray(rawHeaders)
      ? rawHeaders
      : buildRawHeaders(this._headers);
    this._trailers = {};
    this._rawTrailers = [];
    this._aborted = false;
    this._complete = false;
    this._urlOverride = undefined;

    if (stream && typeof stream.on === "function") {
      stream.on("data", (chunk) => {
        if (typeof this.push === "function") this.push(chunk);
        else if (typeof this.emit === "function") this.emit("data", chunk);
      });
      stream.on("end", () => {
        this._complete = true;
        if (typeof this.push === "function") this.push(null);
        else if (typeof this.emit === "function") this.emit("end");
      });
      stream.on("aborted", () => {
        this._aborted = true;
        if (typeof this.emit === "function") this.emit("aborted");
      });
      stream.on("error", (err) => {
        if (typeof this.emit === "function") this.emit("error", err);
      });
      stream.on("close", () => {
        if (typeof this.emit === "function") this.emit("close");
      });
    }
  }

  if (ReadableBase) {
    Object.setPrototypeOf(Http2ServerRequest.prototype, ReadableBase.prototype);
    Object.setPrototypeOf(Http2ServerRequest, ReadableBase);
  } else if (EventEmitterProto) {
    Object.setPrototypeOf(Http2ServerRequest.prototype, EventEmitterProto);
  }

  Object.defineProperties(Http2ServerRequest.prototype, {
    aborted: {
      configurable: true,
      get() {
        return this._aborted;
      },
    },
    complete: {
      configurable: true,
      get() {
        return this._complete;
      },
    },
    stream: {
      configurable: true,
      get() {
        return this._stream;
      },
    },
    headers: {
      configurable: true,
      get() {
        return this._headers;
      },
    },
    rawHeaders: {
      configurable: true,
      get() {
        return this._rawHeaders;
      },
    },
    trailers: {
      configurable: true,
      get() {
        return this._trailers;
      },
    },
    rawTrailers: {
      configurable: true,
      get() {
        return this._rawTrailers;
      },
    },
    httpVersionMajor: {
      configurable: true,
      get() {
        return 2;
      },
    },
    httpVersionMinor: {
      configurable: true,
      get() {
        return 0;
      },
    },
    httpVersion: {
      configurable: true,
      get() {
        return "2.0";
      },
    },
    socket: {
      configurable: true,
      get() {
        return (this._stream && this._stream.session && this._stream.session.socket) || this._stream || null;
      },
    },
    connection: {
      configurable: true,
      get() {
        return this.socket;
      },
    },
    method: {
      configurable: true,
      get() {
        return (this._stream && this._stream.method) || this._headers[":method"] || "GET";
      },
    },
    authority: {
      configurable: true,
      get() {
        return (this._stream && this._stream.authority) || this._headers[":authority"] || "";
      },
    },
    scheme: {
      configurable: true,
      get() {
        return (this._stream && this._stream.scheme) || this._headers[":scheme"] || "http";
      },
    },
    url: {
      configurable: true,
      get() {
        if (this._urlOverride !== undefined) return this._urlOverride;
        return (this._stream && this._stream.url) || this._headers[":path"] || "/";
      },
      set(val) {
        this._urlOverride = val;
      },
    },
  });

  Http2ServerRequest.prototype._read = function _read() {
    if (this._stream && typeof this._stream.resume === "function") {
      this._stream.resume();
    }
  };

  Http2ServerRequest.prototype.setTimeout = function setTimeout(msecs, callback) {
    if (this._stream && typeof this._stream.setTimeout === "function") {
      this._stream.setTimeout(msecs, callback);
    }
    return this;
  };

  function Http2ServerResponse(stream, options) {
    if (StreamBase) {
      StreamBase.call(this, options);
    }
    this._stream = stream || null;
    this._headers = Object.create(null);
    this._headerNames = Object.create(null);
    this._trailers = null;
    this._statusCode = 200;
    this._statusMessage = "";
    this._sendDate = true;
    this._headersSent = false;
    this._writableEnded = false;
    this._writableFinished = false;
    this._writableCorked = 0;
    this._chunks = [];
  }

  if (StreamBase) {
    Object.setPrototypeOf(Http2ServerResponse.prototype, StreamBase.prototype);
    Object.setPrototypeOf(Http2ServerResponse, StreamBase);
  } else if (EventEmitterProto) {
    Object.setPrototypeOf(Http2ServerResponse.prototype, EventEmitterProto);
  }

  Object.defineProperties(Http2ServerResponse.prototype, {
    _header: {
      configurable: true,
      get() {
        return undefined;
      },
    },
    writableEnded: {
      configurable: true,
      get() {
        return this._writableEnded;
      },
    },
    finished: {
      configurable: true,
      get() {
        return this._writableEnded;
      },
    },
    writableFinished: {
      configurable: true,
      get() {
        return this._writableFinished;
      },
    },
    writableCorked: {
      configurable: true,
      get() {
        return this._writableCorked;
      },
    },
    writableHighWaterMark: {
      configurable: true,
      get() {
        return 16384;
      },
    },
    writableLength: {
      configurable: true,
      get() {
        let len = 0;
        for (const c of this._chunks) len += c.length || 0;
        return len;
      },
    },
    socket: {
      configurable: true,
      get() {
        return (this._stream && this._stream.session && this._stream.session.socket) || this._stream || null;
      },
    },
    connection: {
      configurable: true,
      get() {
        return this.socket;
      },
    },
    stream: {
      configurable: true,
      get() {
        return this._stream;
      },
    },
    headersSent: {
      configurable: true,
      get() {
        return this._headersSent;
      },
    },
    sendDate: {
      configurable: true,
      get() {
        return this._sendDate;
      },
      set(val) {
        this._sendDate = Boolean(val);
      },
    },
    statusCode: {
      configurable: true,
      get() {
        return this._statusCode;
      },
      set(val) {
        const code = Number(val);
        this._statusCode = code;
        if (this._stream) {
          this._stream.statusCode = code;
        }
      },
    },
    statusMessage: {
      configurable: true,
      get() {
        return this._statusMessage;
      },
      set(val) {
        this._statusMessage = val == null ? "" : String(val);
      },
    },
  });

  Http2ServerResponse.prototype.setHeader = function setHeader(name, value) {
    const key = String(name).toLowerCase();
    this._headerNames[key] = String(name);
    this._headers[key] = Array.isArray(value) ? value.map(String) : String(value);
    return this;
  };

  Http2ServerResponse.prototype.appendHeader = function appendHeader(name, value) {
    const key = String(name).toLowerCase();
    const prev = this._headers[key];
    if (prev === undefined) {
      return this.setHeader(name, value);
    }
    const next = Array.isArray(prev) ? prev.slice() : [prev];
    if (Array.isArray(value)) {
      for (const item of value) next.push(String(item));
    } else {
      next.push(String(value));
    }
    this._headers[key] = next;
    return this;
  };

  Http2ServerResponse.prototype.getHeader = function getHeader(name) {
    return this._headers[String(name).toLowerCase()];
  };

  Http2ServerResponse.prototype.hasHeader = function hasHeader(name) {
    return String(name).toLowerCase() in this._headers;
  };

  Http2ServerResponse.prototype.removeHeader = function removeHeader(name) {
    const key = String(name).toLowerCase();
    delete this._headers[key];
    delete this._headerNames[key];
  };

  Http2ServerResponse.prototype.getHeaderNames = function getHeaderNames() {
    return Object.keys(this._headers);
  };

  Http2ServerResponse.prototype.getHeaders = function getHeaders() {
    const out = Object.create(null);
    for (const key of Object.keys(this._headers)) {
      out[key] = this._headers[key];
    }
    return out;
  };

  Http2ServerResponse.prototype.flushHeaders = function flushHeaders() {
    if (this._headersSent) return;
    this._headersSent = true;
    if (this._stream && typeof this._stream.writeHead === "function") {
      this._stream.writeHead(this._statusCode, this._headers);
    }
  };

  Http2ServerResponse.prototype.writeHead = function writeHead(statusCode, statusMessage, headers) {
    let hdrs = headers;
    if (typeof statusMessage === "object" && statusMessage !== null) {
      hdrs = statusMessage;
    } else if (typeof statusMessage === "string") {
      this._statusMessage = statusMessage;
    }
    this.statusCode = statusCode;
    if (hdrs && typeof hdrs === "object") {
      if (Array.isArray(hdrs)) {
        for (const pair of hdrs) {
          if (Array.isArray(pair) && pair.length >= 2) {
            this.setHeader(pair[0], pair[1]);
          }
        }
      } else {
        for (const key of Object.keys(hdrs)) {
          this.setHeader(key, hdrs[key]);
        }
      }
    }
    this._headersSent = true;
    if (this._stream && typeof this._stream.writeHead === "function") {
      this._stream.writeHead(this._statusCode, this._headers);
    }
    return this;
  };

  Http2ServerResponse.prototype.cork = function cork() {
    this._writableCorked += 1;
  };

  Http2ServerResponse.prototype.uncork = function uncork() {
    if (this._writableCorked > 0) this._writableCorked -= 1;
  };

  Http2ServerResponse.prototype.setTrailer = function setTrailer(name, value) {
    if (!this._trailers) this._trailers = Object.create(null);
    this._trailers[String(name).toLowerCase()] = value;
  };

  Http2ServerResponse.prototype.addTrailers = function addTrailers(headers) {
    if (!headers || typeof headers !== "object") return;
    for (const key of Object.keys(headers)) {
      this.setTrailer(key, headers[key]);
    }
  };

  function toBufferChunk(chunk, encoding) {
    if (chunk === undefined || chunk === null) return null;
    if (typeof Buffer === "function") {
      if (Buffer.isBuffer(chunk)) return chunk;
      if (chunk instanceof Uint8Array) return Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength);
      return Buffer.from(String(chunk), typeof encoding === "string" ? encoding : "utf8");
    }
    return chunk;
  }

  Http2ServerResponse.prototype.write = function write(chunk, encoding, callback) {
    let enc = encoding;
    let cb = callback;
    if (typeof encoding === "function") {
      cb = encoding;
      enc = undefined;
    }
    const buf = toBufferChunk(chunk, enc);
    if (buf !== null) {
      this._chunks.push(buf);
    }
    if (!this._headersSent) {
      this.flushHeaders();
    }
    if (typeof cb === "function") {
      queueMicrotask(cb);
    }
    return true;
  };

  Http2ServerResponse.prototype.end = function end(chunk, encoding, callback) {
    let data = chunk;
    let enc = encoding;
    let cb = callback;
    if (typeof chunk === "function") {
      cb = chunk;
      data = undefined;
      enc = undefined;
    } else if (typeof encoding === "function") {
      cb = encoding;
      enc = undefined;
    }

    if (this._writableEnded) {
      if (typeof cb === "function") queueMicrotask(cb);
      return this;
    }

    const lastBuf = toBufferChunk(data, enc);
    if (lastBuf !== null) {
      this._chunks.push(lastBuf);
    }

    if (!this._headersSent) {
      this.flushHeaders();
    }

    let payload;
    if (this._chunks.length === 0) {
      payload = undefined;
    } else if (this._chunks.length === 1) {
      payload = this._chunks[0];
    } else if (typeof Buffer === "function") {
      payload = Buffer.concat(this._chunks);
    } else {
      payload = this._chunks.join("");
    }
    this._chunks.length = 0;
    this._writableEnded = true;
    this._writableFinished = true;

    if (this._stream && typeof this._stream.end === "function") {
      this._stream.end(payload);
    }

    if (typeof cb === "function") {
      queueMicrotask(cb);
    }
    if (typeof this.emit === "function") {
      this.emit("finish");
      this.emit("close");
    }
    return this;
  };

  Http2ServerResponse.prototype.destroy = function destroy(err) {
    this._writableEnded = true;
    if (this._stream && typeof this._stream.destroy === "function") {
      this._stream.destroy(err);
    }
    if (err && typeof this.emit === "function") {
      this.emit("error", err);
    }
    if (typeof this.emit === "function") {
      this.emit("close");
    }
    return this;
  };

  Http2ServerResponse.prototype.setTimeout = function setTimeout(msecs, callback) {
    if (this._stream && typeof this._stream.setTimeout === "function") {
      this._stream.setTimeout(msecs, callback);
    }
    return this;
  };

  Http2ServerResponse.prototype.createPushResponse = function createPushResponse(_headers, callback) {
    const err = new Error("HTTP/2 server push is not supported in lexe");
    err.code = "ERR_HTTP2_PUSH_DISABLED";
    if (typeof callback === "function") {
      queueMicrotask(() => callback(err));
      return;
    }
    throw err;
  };

  Http2ServerResponse.prototype.writeContinue = function writeContinue() {};
  Http2ServerResponse.prototype.writeEarlyHints = function writeEarlyHints() {};

  target.Http2ServerRequest = Http2ServerRequest;
  target.Http2ServerResponse = Http2ServerResponse;

  return {
    wrapRequestResponse(reqStream, resStream) {
      return [
        new Http2ServerRequest(reqStream, reqStream ? reqStream.headers : undefined),
        new Http2ServerResponse(resStream),
      ];
    },
  };
})"#;

pub fn init_compat_classes<'js>(ctx: &Ctx<'js>, default: &Object<'js>) -> Result<()> {
    let factory: Function = ctx.eval(COMPAT_CLASSES_SCRIPT)?;
    let stream_ctor: Value = default.get("Http2Stream")?;
    let helper: Object = factory.call((default.clone(), stream_ctor))?;
    let wrap_fn: Function = helper.get("wrapRequestResponse")?;
    if let Some(proto) = Class::<Http2Server>::prototype(ctx)? {
        let define_prop: Function = ctx.eval(
            "(function (proto, key, fn) { Object.defineProperty(proto, key, { value: fn, configurable: true }); })",
        )?;
        let _: Value = define_prop.call((proto, WRAP_KEY, wrap_fn))?;
    }
    Ok(())
}

pub fn wrap_request_response<'js>(
    ctx: &Ctx<'js>,
    req_stream: Class<'js, Http2Stream<'js>>,
    res_stream: Class<'js, Http2Stream<'js>>,
) -> Result<(Value<'js>, Value<'js>)> {
    let proto = Class::<Http2Server>::prototype(ctx)?
        .ok_or_else(|| Exception::throw_type(ctx, "Http2Server prototype missing"))?;
    let wrap_fn: Function = proto.get(WRAP_KEY)?;
    let pair: rquickjs::Array = wrap_fn.call((req_stream, res_stream))?;
    Ok((pair.get(0)?, pair.get(1)?))
}

pub fn perform_server_handshake<'js>(
    ctx: Ctx<'js>,
    socket: Value<'js>,
    _options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    if !socket.is_object() || socket.is_null() {
        let err = Exception::from_message(
            ctx.clone(),
            "The \"socket\" argument must be of type object",
        )?;
        let _ = err.as_object().set("code", "ERR_INVALID_ARG_TYPE");
        return Err(ctx.throw(err.into()));
    }
    let err = Exception::from_message(
        ctx.clone(),
        "http2.performServerHandshake is not implemented in lexe yet",
    )?;
    let _ = err.as_object().set("code", "ERR_HTTP2_ERROR");
    Err(ctx.throw(err.into()))
}
