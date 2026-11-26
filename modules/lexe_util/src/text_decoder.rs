// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_encoding::{Encoder, IncrementalDecoder};
use lexe_utils::{bytes::ObjectBytes, object::ObjectExt, result::ResultExt};
use rquickjs::{
    atom::PredefinedAtom, function::Opt, prelude::Coerced, Ctx, FromJs, Object, Result, Value,
};

#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct TextDecoder {
    #[qjs(skip_trace)]
    encoder: Encoder,
    fatal: bool,
    ignore_bom: bool,
    #[qjs(skip_trace)]
    decoder: IncrementalDecoder,
    do_not_flush: bool,
}

#[rquickjs::methods]
impl<'js> TextDecoder {
    #[qjs(constructor)]
    pub fn new(
        ctx: Ctx<'js>,
        label: Opt<Value<'js>>,
        options: Opt<Option<Object<'js>>>,
    ) -> Result<Self> {
        let mut fatal = false;
        let mut ignore_bom = false;

        let label_str: Option<String> = match label.0 {
            None => None,
            Some(v) if v.is_undefined() => None,
            Some(v) => Some(Coerced::<String>::from_js(&ctx, v)?.0),
        };
        let encoder = Encoder::from_optional_str(label_str.as_deref()).or_throw_range(&ctx, "")?;

        if let Some(options) = options.0.flatten() {
            if let Some(opt) = options.get_optional("fatal")? {
                fatal = opt;
            }
            if let Some(opt) = options.get_optional("ignoreBOM")? {
                ignore_bom = opt;
            }
        }

        Ok(TextDecoder {
            encoder,
            fatal,
            ignore_bom,
            decoder: IncrementalDecoder::new(),
            do_not_flush: false,
        })
    }

    #[qjs(get)]
    fn encoding(&self) -> &str {
        self.encoder.as_label()
    }

    #[qjs(get)]
    fn fatal(&self) -> bool {
        self.fatal
    }

    #[qjs(get, rename = "ignoreBOM")]
    fn ignore_bom(&self) -> bool {
        self.ignore_bom
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(TextDecoder)
    }

    pub fn decode(
        &mut self,
        ctx: Ctx<'js>,
        input: Opt<Value<'js>>,
        options: Opt<Object<'js>>,
    ) -> Result<String> {
        let mut stream = false;
        if let Some(options) = options.0 {
            if let Some(value) = options.get_optional("stream")? {
                stream = value;
            }
        }

        let bytes = match input.0 {
            None => Vec::new(),
            Some(value) if value.is_undefined() => Vec::new(),
            Some(value) => ObjectBytes::from(&ctx, &value)?.as_bytes(&ctx)?.to_vec(),
        };

        let lossy = !self.fatal;

        if stream {
            self.do_not_flush = true;
            return self
                .decoder
                .decode_chunk(&self.encoder, &bytes, lossy, true, self.ignore_bom)
                .or_throw_type(&ctx, "");
        }

        if self.do_not_flush {
            self.do_not_flush = false;
            let mut output = self
                .decoder
                .decode_chunk(&self.encoder, &bytes, lossy, true, self.ignore_bom)
                .or_throw_type(&ctx, "")?;
            output.push_str(
                &self
                    .decoder
                    .flush(&self.encoder, lossy, self.ignore_bom)
                    .or_throw_type(&ctx, "")?,
            );
            return Ok(output);
        }

        self.decoder.reset();
        self.decoder
            .decode_chunk(&self.encoder, &bytes, lossy, false, self.ignore_bom)
            .or_throw_type(&ctx, "")
    }
}
