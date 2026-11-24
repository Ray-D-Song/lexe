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
    deflate, deflate_raw, deflate_raw_sync, deflate_sync, gunzip, gunzip_sync, gzip, gzip_sync,
    inflate, inflate_raw, inflate_raw_sync, inflate_sync,
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
    "BrotliCompress",
    "BrotliDecompress",
    "ZstdCompress",
    "ZstdDecompress",
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

    function toLevel(options) {
      if (!options || options.level === undefined || options.level === null) {
        return 6;
      }
      const level = Number(options.level);
      if (level === -1 || Number.isNaN(level)) return 6;
      return Math.max(0, Math.min(11, level | 0));
    }

    function makeTransform(encoding, decompress) {
      return class extends Transform {
        constructor(options) {
          super(options || {});
          this._encoding = encoding;
          this._decompress = decompress;
          this._handle = decompress
            ? createDecoder(encoding)
            : createEncoder(encoding, toLevel(options));
          this._finished = false;
        }

        _transform(chunk, _enc, cb) {
          try {
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
            if (!this._finished) {
              this._finished = true;
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
          this._handle = null;
          cb(err);
        }

        // Node exposes these on the stream instance.
        flush(cb) {
          if (typeof cb === "function") cb();
        }

        reset() {
          this._handle = this._decompress
            ? createDecoder(this._encoding)
            : createEncoder(this._encoding, 6);
          this._finished = false;
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

            let constants = glue.get::<_, Function>("constants")?;
            default.set("constants", constants.call::<_, Object>(())?)?;

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
