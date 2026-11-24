// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Streaming encoder handle used by the `zlib.create*()` stream factories.
//!
//! Node's `createGzip()` returns a Transform that emits a *single* framed
//! stream across every chunk written to it, so the compressor state has to live
//! between chunks. The JS side owns the Transform/backpressure behaviour and
//! calls into [`EncoderHandle`] for the actual compression steps.

use std::cell::RefCell;

use raster_runtime_buffer::Buffer;
use raster_runtime_compression::streaming::StreamingEncoder;
use raster_runtime_utils::bytes::ObjectBytes;
use rquickjs::{
    class::{Trace, Tracer},
    prelude::Opt,
    Class, Ctx, Exception, JsLifetime, Result,
};

#[rquickjs::class]
pub struct EncoderHandle {
    inner: RefCell<Option<EncoderHandleInner>>,
    /// Set when a `push` failed and the codec was released, so `finish` can tell
    /// "already finished normally" from "failed and cannot continue".
    failed: std::cell::Cell<bool>,
}

enum EncoderHandleInner {
    Encode(StreamingEncoder),
    Decode(raster_runtime_compression::streaming::StreamingDecoder),
}

unsafe impl<'js> JsLifetime<'js> for EncoderHandle {
    type Changed<'to> = EncoderHandle;
}

impl Trace<'_> for EncoderHandle {
    fn trace<'a>(&self, _tracer: Tracer<'a, '_>) {}
}

impl EncoderHandle {
    pub fn new(encoding: &str, level: u32) -> std::io::Result<Self> {
        Ok(Self {
            inner: RefCell::new(Some(EncoderHandleInner::Encode(StreamingEncoder::new(
                encoding, level,
            )?))),
            failed: std::cell::Cell::new(false),
        })
    }

    /// Decompressing variant of [`Self::new`] for `createGunzip()` and friends.
    pub fn new_decoder(encoding: &str) -> std::io::Result<Self> {
        Ok(Self {
            inner: RefCell::new(Some(EncoderHandleInner::Decode(
                raster_runtime_compression::streaming::StreamingDecoder::new(encoding)?,
            ))),
            failed: std::cell::Cell::new(false),
        })
    }
}

/// JS-facing constructor: `new (encoding, level) => EncoderHandle`.
pub fn create_encoder_js<'js>(
    ctx: Ctx<'js>,
    encoding: String,
    level: u32,
) -> Result<Class<'js, EncoderHandle>> {
    let handle = EncoderHandle::new(&encoding, level)
        .map_err(|e| rquickjs::Exception::throw_message(&ctx, &e.to_string()))?;
    Class::instance(ctx, handle)
}

/// JS-facing constructor for decompressing transforms: `new (encoding) => ...`.
pub fn create_decoder_js<'js>(
    ctx: Ctx<'js>,
    encoding: String,
) -> Result<Class<'js, EncoderHandle>> {
    let handle = EncoderHandle::new_decoder(&encoding)
        .map_err(|e| rquickjs::Exception::throw_message(&ctx, &e.to_string()))?;
    Class::instance(ctx, handle)
}

#[rquickjs::methods(rename_all = "camelCase")]
impl EncoderHandle {
    /// Compress (or decompress) one chunk and return the bytes produced so far.
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
            EncoderHandleInner::Encode(encoder) => encoder.compress_chunk(&bytes),
            EncoderHandleInner::Decode(decoder) => decoder.decompress_chunk(&bytes),
        };
        match out {
            Ok(out) => Buffer(out).into_js(&ctx),
            Err(err) => {
                // Release the codec before reporting the failure. Leaving it in
                // place would keep the decoder alive for the lifetime of the JS
                // handle, and the handle is reachable from the stream that
                // failed — a cross-language cycle the collector cannot break.
                drop(slot.take());
                self.failed.set(true);
                Err(Exception::throw_message(&ctx, &err.to_string()))
            },
        }
    }

    /// Flush the codec and return the trailing stream bytes.
    ///
    /// Returns an empty buffer when the handle already finished normally. After
    /// a `push` failure the codec has been released, so `finish` reports that
    /// rather than pretending a stream ended cleanly — a caller that ignores
    /// the earlier error must not read this as success.
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
        let out = match handle {
            EncoderHandleInner::Encode(encoder) => encoder.finish(),
            EncoderHandleInner::Decode(decoder) => decoder.finish(),
        }
        .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
        Buffer(out).into_js(&ctx)
    }

    /// Whether this handle is still usable (not finished/destroyed).
    pub fn alive(&self) -> bool {
        self.inner.borrow().is_some()
    }
}

use rquickjs::{IntoJs, Value};
