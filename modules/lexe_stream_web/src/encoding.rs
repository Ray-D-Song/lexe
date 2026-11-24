// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{Ctx, Object, Result, Value};

pub fn define_text_encoding_constructors(ctx: &Ctx<'_>) -> Result<()> {
    lexe_util::define_text_encoding_constructors(ctx)
}

pub fn install_encoding_streams(ctx: &Ctx<'_>) -> Result<()> {
    define_text_encoding_constructors(ctx)?;

    let globals = ctx.globals();
    if globals.contains_key("TextEncoderStream")? {
        return Ok(());
    }

    let module_value: Value = ctx.eval(
        r#"(function () {
  class TextEncoderStream {
    #pendingHighSurrogate = null;
    #handle;
    #transform;

    constructor() {
      this.#handle = new TextEncoder();
      this.#transform = new TransformStream({
        transform: (chunk, controller) => {
          chunk = String(chunk);
          let finalChunk = "";
          for (let i = 0; i < chunk.length; i++) {
            const item = chunk[i];
            const codeUnit = item.charCodeAt(0);
            if (this.#pendingHighSurrogate !== null) {
              const highSurrogate = this.#pendingHighSurrogate;
              this.#pendingHighSurrogate = null;
              if (codeUnit >= 0xdc00 && codeUnit <= 0xdfff) {
                finalChunk += highSurrogate + item;
                continue;
              }
              finalChunk += "\uFFFD";
            }
            if (codeUnit >= 0xd800 && codeUnit <= 0xdbff) {
              this.#pendingHighSurrogate = item;
              continue;
            }
            if (codeUnit >= 0xdc00 && codeUnit <= 0xdfff) {
              finalChunk += "\uFFFD";
              continue;
            }
            finalChunk += item;
          }
          if (finalChunk) {
            controller.enqueue(this.#handle.encode(finalChunk));
          }
        },
        flush: (controller) => {
          if (this.#pendingHighSurrogate !== null) {
            controller.enqueue(new Uint8Array([0xef, 0xbf, 0xbd]));
          }
        },
      });
    }

    get encoding() {
      return this.#handle.encoding;
    }

    get readable() {
      return this.#transform.readable;
    }

    get writable() {
      return this.#transform.writable;
    }
  }

  class TextDecoderStream {
    #handle;
    #transform;

    constructor(encoding = "utf-8", options = {}) {
      this.#handle = new TextDecoder(encoding, options);
      this.#transform = new TransformStream({
        transform: (chunk, controller) => {
          if (chunk === undefined) {
            throw new TypeError("chunk must not be undefined");
          }
          const value = this.#handle.decode(chunk, { stream: true });
          if (value) controller.enqueue(value);
        },
        flush: (controller) => {
          const value = this.#handle.decode();
          if (value) controller.enqueue(value);
          controller.terminate();
        },
      });
    }

    get encoding() {
      return this.#handle.encoding;
    }

    get fatal() {
      return this.#handle.fatal;
    }

    get ignoreBOM() {
      return this.#handle.ignoreBOM;
    }

    get readable() {
      return this.#transform.readable;
    }

    get writable() {
      return this.#transform.writable;
    }
  }

  return { TextEncoderStream, TextDecoderStream };
})()"#,
    )?;
    let module = module_value.as_object().expect("encoding streams module");
    let encoder_stream: Value = module.get("TextEncoderStream")?;
    let decoder_stream: Value = module.get("TextDecoderStream")?;

    globals.set("TextEncoderStream", encoder_stream)?;
    globals.set("TextDecoderStream", decoder_stream)?;

    Ok(())
}

pub fn export_encoding_streams(default: &Object<'_>) -> Result<()> {
    let globals = default.ctx().globals();
    let encoder_stream: Value = globals.get("TextEncoderStream")?;
    let decoder_stream: Value = globals.get("TextDecoderStream")?;
    default.set("TextEncoderStream", encoder_stream)?;
    default.set("TextDecoderStream", decoder_stream)?;
    Ok(())
}
