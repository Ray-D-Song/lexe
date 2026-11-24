// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Streaming encoder and decoder handle used by `zlib.create*()` stream factories.

use std::cell::{Cell, RefCell};

use lexe_buffer::Buffer;
use lexe_compression::streaming::{StreamingDecoder, StreamingEncoder};
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{
    class::{Trace, Tracer},
    prelude::Opt,
    Class, Ctx, Exception, IntoJs, JsLifetime, Result, Value,
};

use super::zlib::parse_zlib_options;

#[rquickjs::class]
pub struct EncoderHandle {
    inner: RefCell<Option<EncoderHandleInner>>,
    failed: Cell<bool>,
    closed: Cell<bool>,
    last_bytes_written: Cell<u64>,
}

enum EncoderHandleInner {
    Encode(StreamingEncoder),
    Decode(StreamingDecoder),
}

unsafe impl<'js> JsLifetime<'js> for EncoderHandle {
    type Changed<'to> = EncoderHandle;
}

impl Trace<'_> for EncoderHandle {
    fn trace<'a>(&self, _tracer: Tracer<'a, '_>) {}
}

impl EncoderHandle {
    fn snapshot_bytes_written(&self, handle: &EncoderHandleInner) {
        let n = match handle {
            EncoderHandleInner::Encode(enc) => enc.bytes_written(),
            EncoderHandleInner::Decode(dec) => dec.bytes_written(),
        };
        self.last_bytes_written.set(n);
    }
}

pub fn create_encoder_js<'js>(
    ctx: Ctx<'js>,
    encoding: String,
    options: Opt<Value<'js>>,
) -> Result<Class<'js, EncoderHandle>> {
    let opts = parse_zlib_options(&ctx, options, true)?;
    let encoder = StreamingEncoder::new_with_options(&encoding, opts)
        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
    Class::instance(
        ctx,
        EncoderHandle {
            inner: RefCell::new(Some(EncoderHandleInner::Encode(encoder))),
            failed: Cell::new(false),
            closed: Cell::new(false),
            last_bytes_written: Cell::new(0),
        },
    )
}

pub fn create_decoder_js<'js>(
    ctx: Ctx<'js>,
    encoding: String,
    options: Opt<Value<'js>>,
) -> Result<Class<'js, EncoderHandle>> {
    let opts = parse_zlib_options(&ctx, options, false)?;
    let decoder = StreamingDecoder::new_with_options(&encoding, opts)
        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
    Class::instance(
        ctx,
        EncoderHandle {
            inner: RefCell::new(Some(EncoderHandleInner::Decode(decoder))),
            failed: Cell::new(false),
            closed: Cell::new(false),
            last_bytes_written: Cell::new(0),
        },
    )
}

#[rquickjs::methods(rename_all = "camelCase")]
impl EncoderHandle {
    pub fn push<'js>(&self, ctx: Ctx<'js>, chunk: ObjectBytes<'js>) -> Result<Value<'js>> {
        let bytes = chunk.as_bytes(&ctx)?.to_vec();
        let mut slot = self.inner.borrow_mut();
        let Some(handle) = slot.as_mut() else {
            return Err(Exception::throw_message(
                &ctx,
                if self.failed.get() {
                    "zlib handle failed and has been released"
                } else {
                    "zlib handle already finished"
                },
            ));
        };
        let out = match handle {
            EncoderHandleInner::Encode(enc) => enc.compress_chunk(&bytes),
            EncoderHandleInner::Decode(dec) => dec.decompress_chunk(&bytes),
        };
        self.snapshot_bytes_written(handle);
        match out {
            Ok(out) => Buffer(out).into_js(&ctx),
            Err(err) => {
                drop(slot.take());
                self.failed.set(true);
                Err(Exception::throw_message(&ctx, &err.to_string()))
            },
        }
    }

    pub fn flush<'js>(&self, ctx: Ctx<'js>, kind: Opt<i32>) -> Result<Value<'js>> {
        let kind = kind.0.unwrap_or(3);
        let mut slot = self.inner.borrow_mut();
        let Some(handle) = slot.as_mut() else {
            return Buffer(Vec::new()).into_js(&ctx);
        };
        let out = match handle {
            EncoderHandleInner::Encode(enc) => enc.flush_kind(kind),
            EncoderHandleInner::Decode(dec) => dec.flush_kind(kind),
        }
        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
        Buffer(out).into_js(&ctx)
    }

    pub fn params<'js>(&self, ctx: Ctx<'js>, level: i32, strategy: i32) -> Result<Value<'js>> {
        let mut slot = self.inner.borrow_mut();
        let Some(handle) = slot.as_mut() else {
            return Err(Exception::throw_message(&ctx, "zlib handle already finished"));
        };
        let out = match handle {
            EncoderHandleInner::Encode(enc) => enc.params(level, strategy),
            EncoderHandleInner::Decode(_) => {
                return Err(Exception::throw_message(
                    &ctx,
                    "params() is not supported on decompressor streams",
                ))
            },
        }
        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
        Buffer(out).into_js(&ctx)
    }

    pub fn reset(&self, ctx: Ctx<'_>) -> Result<()> {
        let mut slot = self.inner.borrow_mut();
        let Some(handle) = slot.as_mut() else {
            return Err(Exception::throw_message(&ctx, "zlib handle already finished"));
        };
        let res = match handle {
            EncoderHandleInner::Encode(enc) => enc.reset(),
            EncoderHandleInner::Decode(dec) => dec.reset(),
        };
        res.map_err(|e| Exception::throw_message(&ctx, &e.to_string()))
    }

    pub fn finish<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let taken = self.inner.borrow_mut().take();
        let Some(handle) = taken else {
            if self.failed.get() {
                return Err(Exception::throw_message(
                    &ctx,
                    "zlib handle failed and has been released",
                ));
            }
            return Buffer(Vec::new()).into_js(&ctx);
        };
        self.snapshot_bytes_written(&handle);
        let out = match handle {
            EncoderHandleInner::Encode(enc) => enc.finish(),
            EncoderHandleInner::Decode(dec) => dec.finish(),
        }
        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
        Buffer(out).into_js(&ctx)
    }

    pub fn close(&self) {
        self.closed.set(true);
        if let Some(handle) = self.inner.borrow_mut().take() {
            self.snapshot_bytes_written(&handle);
        }
    }

    #[qjs(rename = "bytesWritten")]
    pub fn bytes_written(&self) -> f64 {
        let slot = self.inner.borrow();
        let n = match slot.as_ref() {
            Some(handle) => match handle {
                EncoderHandleInner::Encode(enc) => enc.bytes_written(),
                EncoderHandleInner::Decode(dec) => dec.bytes_written(),
            },
            None => self.last_bytes_written.get(),
        };
        n as f64
    }

    pub fn alive(&self) -> bool {
        self.inner.borrow().is_some()
    }
}
