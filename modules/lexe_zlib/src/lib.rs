// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    function::Func,
    module::{Declarations, Exports, ModuleDef},
    Ctx, Function, Object, Result, Value,
};

mod brotli;
mod stream;
mod zlib;
mod zstd;

use self::brotli::{br_comp, br_comp_sync, br_decomp, br_decomp_sync};
use self::stream::{create_decoder_js, create_encoder_js};
use self::zlib::{
    crc32, deflate, deflate_raw, deflate_raw_sync, deflate_sync, gunzip, gunzip_sync, gzip,
    gzip_sync, inflate, inflate_raw, inflate_raw_sync, inflate_sync, unzip, unzip_sync,
};
use self::zstd::{zstd_comp, zstd_comp_sync, zstd_decomp, zstd_decomp_sync};

#[macro_export]
macro_rules! define_sync_function {
    ($fn_name:ident, $converter:expr, $command:expr) => {
        pub(crate) fn $fn_name<'js>(
            ctx: Ctx<'js>,
            value: ObjectBytes<'js>,
            options: Opt<Value<'js>>,
        ) -> Result<Value<'js>> {
            $converter(ctx.clone(), value, options, $command)
        }
    };
}

#[macro_export]
macro_rules! define_cb_function {
    ($fn_name:ident, $converter:expr, $command:expr) => {
        pub(crate) fn $fn_name<'js>(
            ctx: Ctx<'js>,
            value: ObjectBytes<'js>,
            args: Rest<Value<'js>>,
        ) -> Result<()> {
            let mut args_iter = args.0.into_iter().rev();
            let cb: Function = args_iter
                .next()
                .and_then(|v| v.into_function())
                .or_throw_msg(&ctx, "Callback parameter is not a function")?;
            let options = match args_iter.next() {
                Some(v) => Opt(Some(v)),
                None => Opt(None),
            };

            ctx.clone().spawn_exit(async move {
                match $converter(ctx.clone(), value, options, $command) {
                    Ok(obj) => {
                        () = cb.call((Null.into_js(&ctx), obj))?;
                        Ok::<_, Error>(())
                    },
                    Err(err) => {
                        () = cb.call((Exception::from_message(ctx, &err.to_string()),))?;
                        Ok(())
                    },
                }
            })?;
            Ok(())
        }
    };
}
pub struct ZlibModule;

/// `zlib.create*()` factories Node exposes.
const STREAM_FACTORIES: &[&str] = &[
    "createGzip",
    "createGunzip",
    "createDeflate",
    "createInflate",
    "createDeflateRaw",
    "createInflateRaw",
    "createUnzip",
    "createBrotliCompress",
    "createBrotliDecompress",
    "createZstdCompress",
    "createZstdDecompress",
];

/// Transform subclasses Node also exports by name.
const STREAM_CLASSES: &[&str] = &[
    "Gzip",
    "Gunzip",
    "Deflate",
    "Inflate",
    "DeflateRaw",
    "InflateRaw",
    "Unzip",
    "BrotliCompress",
    "BrotliDecompress",
    "ZstdCompress",
    "ZstdDecompress",
];

/// Legacy top-level zlib constants mirrored from `zlib.constants`.
const LEGACY_CONSTANTS: &[&str] = &[
    "Z_NO_FLUSH",
    "Z_PARTIAL_FLUSH",
    "Z_SYNC_FLUSH",
    "Z_FULL_FLUSH",
    "Z_FINISH",
    "Z_BLOCK",
    "Z_TREES",
    "Z_OK",
    "Z_STREAM_END",
    "Z_NEED_DICT",
    "Z_ERRNO",
    "Z_STREAM_ERROR",
    "Z_DATA_ERROR",
    "Z_MEM_ERROR",
    "Z_BUF_ERROR",
    "Z_VERSION_ERROR",
    "Z_NO_COMPRESSION",
    "Z_BEST_SPEED",
    "Z_BEST_COMPRESSION",
    "Z_DEFAULT_COMPRESSION",
    "Z_DEFAULT_STRATEGY",
    "Z_FILTERED",
    "Z_HUFFMAN_ONLY",
    "Z_RLE",
    "Z_FIXED",
    "Z_MIN_WINDOWBITS",
    "Z_MAX_WINDOWBITS",
    "Z_DEFAULT_WINDOWBITS",
    "Z_MIN_CHUNK",
    "Z_MAX_CHUNK",
    "Z_DEFAULT_CHUNK",
    "Z_MIN_MEMLEVEL",
    "Z_MAX_MEMLEVEL",
    "Z_DEFAULT_MEMLEVEL",
    "Z_MIN_LEVEL",
    "Z_MAX_LEVEL",
    "Z_DEFAULT_LEVEL",
];

/// Builds the streaming half of `node:zlib` on top of `stream.Transform`.
///
/// `build(createEncoder)` receives the Rust encoder constructor
/// (`new (encoding, level) => { push(chunk), finish() }`) and returns the
/// factories, classes and `constants`.
const STREAM_GLUE_JS: &str = r#"(function () {
  "use strict";

  function constants() {
    // Node exposes zlib mode/flush/strategy constants; callers mostly pass them
    // straight back, so stable numeric values are enough.
    const c = {
      Z_NO_FLUSH: 0,
      Z_PARTIAL_FLUSH: 1,
      Z_SYNC_FLUSH: 2,
      Z_FULL_FLUSH: 3,
      Z_FINISH: 4,
      Z_BLOCK: 5,
      Z_TREES: 6,
      Z_OK: 0,
      Z_STREAM_END: 1,
      Z_NEED_DICT: 2,
      Z_ERRNO: -1,
      Z_STREAM_ERROR: -2,
      Z_DATA_ERROR: -3,
      Z_MEM_ERROR: -4,
      Z_BUF_ERROR: -5,
      Z_VERSION_ERROR: -6,
      Z_NO_COMPRESSION: 0,
      Z_BEST_SPEED: 1,
      Z_BEST_COMPRESSION: 9,
      Z_DEFAULT_COMPRESSION: -1,
      Z_DEFAULT_STRATEGY: 0,
      Z_FILTERED: 1,
      Z_HUFFMAN_ONLY: 2,
      Z_RLE: 3,
      Z_FIXED: 4,
      Z_MIN_WINDOWBITS: 8,
      Z_MAX_WINDOWBITS: 15,
      Z_DEFAULT_WINDOWBITS: 15,
      Z_MIN_CHUNK: 64,
      Z_MAX_CHUNK: Infinity,
      Z_DEFAULT_CHUNK: 16384,
      Z_MIN_MEMLEVEL: 1,
      Z_MAX_MEMLEVEL: 9,
      Z_DEFAULT_MEMLEVEL: 8,
      Z_MIN_LEVEL: -1,
      Z_MAX_LEVEL: 9,
      Z_DEFAULT_LEVEL: -1,
      BROTLI_OPERATION_PROCESS: 0,
      BROTLI_OPERATION_FLUSH: 1,
      BROTLI_OPERATION_FINISH: 2,
      BROTLI_OPERATION_EMIT_METADATA: 3,
      BROTLI_PARAM_MODE: 0,
      BROTLI_PARAM_QUALITY: 1,
      BROTLI_PARAM_SIZE_HINT: 5,
      BROTLI_MIN_QUALITY: 0,
      BROTLI_MAX_QUALITY: 11,
      BROTLI_DEFAULT_QUALITY: 11,
    };
    return c;
  }

  function build(createEncoder, createDecoder) {
    const { Transform } = require("node:stream");

    function makeTransform(encoding, decompress) {
      return class extends Transform {
        constructor(options) {
          super(options || {});
          this._encoding = encoding;
          this._decompress = decompress;
          this._options = options || {};
          this._handle = decompress
            ? createDecoder(encoding, this._options)
            : createEncoder(encoding, this._options);
          this._finished = false;
          this._closed = false;
          this._bytesWritten = 0;
        }

        get bytesWritten() {
          return this._handle ? this._handle.bytesWritten() : this._bytesWritten;
        }

        _transform(chunk, _enc, cb) {
          try {
            if (!this._handle) {
              throw new Error("zlib handle is closed");
            }
            const out = this._handle.push(chunk);
            if (out && out.length > 0) {
              this.push(out);
            }
            cb();
          } catch (err) {
            cb(err);
          }
        }

        _flush(cb) {
          try {
            if (!this._finished && this._handle) {
              this._finished = true;
              this._bytesWritten = this._handle.bytesWritten();
              const tail = this._handle.finish();
              if (tail && tail.length > 0) {
                this.push(tail);
              }
            }
            cb();
          } catch (err) {
            cb(err);
          }
        }

        _destroy(err, cb) {
          if (this._handle) {
            this._bytesWritten = this._handle.bytesWritten();
            this._handle.close();
            this._handle = null;
          }
          this._closed = true;
          cb(err);
        }

        flush(kind, cb) {
          if (typeof kind === "function") {
            cb = kind;
            kind = undefined;
          }
          if (kind === undefined) {
            kind = this._encoding === "br" ? 1 : 3;
          }
          if (!this._handle || !this._handle.alive()) {
            if (typeof cb === "function") queueMicrotask(cb);
            return;
          }
          try {
            const out = this._handle.flush(kind);
            if (out && out.length > 0) {
              this.push(out);
            }
            if (typeof cb === "function") queueMicrotask(cb);
          } catch (err) {
            if (typeof cb === "function") {
              queueMicrotask(() => cb(err));
            } else {
              this.destroy(err);
            }
          }
        }

        params(level, strategy, cb) {
          if (typeof level !== "number" || typeof strategy !== "number") {
            throw new TypeError("The \"level\" and \"strategy\" arguments must be numbers");
          }
          if (!this._handle || !this._handle.alive()) {
            if (typeof cb === "function") queueMicrotask(cb);
            return;
          }
          try {
            const out = this._handle.params(level, strategy);
            if (out && out.length > 0) {
              this.push(out);
            }
            if (typeof cb === "function") queueMicrotask(cb);
          } catch (err) {
            if (typeof cb === "function") {
              queueMicrotask(() => cb(err));
            } else {
              this.destroy(err);
            }
          }
        }

        reset() {
          if (this._closed || this.destroyed) return;
          if (this._handle && this._handle.alive()) {
            this._handle.reset();
          } else {
            const prev = this._bytesWritten;
            this._handle = this._decompress
              ? createDecoder(this._encoding, this._options)
              : createEncoder(this._encoding, this._options);
            this._bytesWritten = prev;
          }
          this._finished = false;
        }

        close(cb) {
          if (this._closed || this.destroyed) {
            if (typeof cb === "function") queueMicrotask(cb);
            return;
          }
          if (typeof cb === "function") {
            this.once("close", cb);
          }
          this.destroy();
        }
      };
    }

    const classes = {
      Gzip: makeTransform("gzip", false),
      Gunzip: makeTransform("gzip", true),
      Deflate: makeTransform("deflate", false),
      Inflate: makeTransform("deflate", true),
      DeflateRaw: makeTransform("deflate-raw", false),
      InflateRaw: makeTransform("deflate-raw", true),
      Unzip: makeTransform("unzip", true),
      BrotliCompress: makeTransform("br", false),
      BrotliDecompress: makeTransform("br", true),
      ZstdCompress: makeTransform("zstd", false),
      ZstdDecompress: makeTransform("zstd", true),
    };

    const factories = {
      createGzip: (options) => new classes.Gzip(options),
      createGunzip: (options) => new classes.Gunzip(options),
      createDeflate: (options) => new classes.Deflate(options),
      createInflate: (options) => new classes.Inflate(options),
      createDeflateRaw: (options) => new classes.DeflateRaw(options),
      createInflateRaw: (options) => new classes.InflateRaw(options),
      createUnzip: (options) => new classes.Unzip(options),
      createBrotliCompress: (options) => new classes.BrotliCompress(options),
      createBrotliDecompress: (options) => new classes.BrotliDecompress(options),
      createZstdCompress: (options) => new classes.ZstdCompress(options),
      createZstdDecompress: (options) => new classes.ZstdDecompress(options),
    };

    return Object.assign({}, classes, factories);
  }

  return { build, constants };
})()"#;

impl ModuleDef for ZlibModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("deflate")?;
        declare.declare("deflateSync")?;

        declare.declare("deflateRaw")?;
        declare.declare("deflateRawSync")?;

        declare.declare("gzip")?;
        declare.declare("gzipSync")?;

        declare.declare("inflate")?;
        declare.declare("inflateSync")?;

        declare.declare("inflateRaw")?;
        declare.declare("inflateRawSync")?;

        declare.declare("gunzip")?;
        declare.declare("gunzipSync")?;

        declare.declare("unzip")?;
        declare.declare("unzipSync")?;

        declare.declare("crc32")?;

        declare.declare("brotliCompress")?;
        declare.declare("brotliCompressSync")?;

        declare.declare("brotliDecompress")?;
        declare.declare("brotliDecompressSync")?;

        declare.declare("zstdCompress")?;
        declare.declare("zstdCompressSync")?;

        declare.declare("zstdDecompress")?;
        declare.declare("zstdDecompressSync")?;

        // Streaming factories (Node returns a Transform per call).
        for &name in STREAM_FACTORIES {
            declare.declare(name)?;
        }
        for &name in STREAM_CLASSES {
            declare.declare(name)?;
        }
        for &name in LEGACY_CONSTANTS {
            declare.declare(name)?;
        }
        declare.declare("constants")?;

        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            default.set("deflate", Func::from(deflate))?;
            default.set("deflateSync", Func::from(deflate_sync))?;

            default.set("deflateRaw", Func::from(deflate_raw))?;
            default.set("deflateRawSync", Func::from(deflate_raw_sync))?;

            default.set("gzip", Func::from(gzip))?;
            default.set("gzipSync", Func::from(gzip_sync))?;

            default.set("inflate", Func::from(inflate))?;
            default.set("inflateSync", Func::from(inflate_sync))?;

            default.set("inflateRaw", Func::from(inflate_raw))?;
            default.set("inflateRawSync", Func::from(inflate_raw_sync))?;

            default.set("gunzip", Func::from(gunzip))?;
            default.set("gunzipSync", Func::from(gunzip_sync))?;

            default.set("unzip", Func::from(unzip))?;
            default.set("unzipSync", Func::from(unzip_sync))?;

            default.set("crc32", Func::from(crc32))?;

            default.set("brotliCompress", Func::from(br_comp))?;
            default.set("brotliCompressSync", Func::from(br_comp_sync))?;

            default.set("brotliDecompress", Func::from(br_decomp))?;
            default.set("brotliDecompressSync", Func::from(br_decomp_sync))?;

            default.set("zstdCompress", Func::from(zstd_comp))?;
            default.set("zstdCompressSync", Func::from(zstd_comp_sync))?;

            default.set("zstdDecompress", Func::from(zstd_decomp))?;
            default.set("zstdDecompressSync", Func::from(zstd_decomp_sync))?;

            // `createGzip()` & friends return a Transform that emits one framed
            // stream across chunks; the JS glue owns the stream and calls the
            // Rust encoder for each chunk.
            let create_class = Function::new(ctx.clone(), create_encoder_js)?;
            let create_decoder = Function::new(ctx.clone(), create_decoder_js)?;
            let glue: Object = ctx.eval(STREAM_GLUE_JS)?;
            let build = glue.get::<_, Function>("build")?;
            let api: Object = build.call((create_class, create_decoder))?;
            for &name in STREAM_FACTORIES {
                let factory: Function = api.get(name)?;
                default.set(name, factory)?;
            }
            for &name in STREAM_CLASSES {
                let class: Value = api.get(name)?;
                default.set(name, class)?;
            }

            let constants_fn = glue.get::<_, Function>("constants")?;
            let constants_obj: Object = constants_fn.call(())?;
            for &name in LEGACY_CONSTANTS {
                let val: Value = constants_obj.get(name)?;
                default.set(name, val)?;
            }
            default.set("constants", constants_obj)?;

            Ok(())
        })
    }
}

impl From<ZlibModule> for ModuleInfo<ZlibModule> {
    fn from(val: ZlibModule) -> Self {
        ModuleInfo {
            name: "zlib",
            module: val,
        }
    }
}
