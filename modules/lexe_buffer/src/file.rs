// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::time;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, function::Opt, qjs, ArrayBuffer, Ctx, Exception, Result,
    Value,
};

use super::blob::{usv_string_from_js, Blob};

#[rquickjs::class]
#[derive(Trace, Clone, rquickjs::JsLifetime)]
pub struct File {
    #[qjs(skip_trace)]
    blob: Blob,
    filename: String,
    last_modified: i64,
}

/// Convert a JS value using WebIDL `long long` (ToNumber, non-finite -> 0, mod 2^64).
fn js_to_long_long<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<i64> {
    let mut num = 0.0_f64;
    let rc = unsafe { qjs::JS_ToFloat64(ctx.as_raw().as_ptr(), &mut num, value.as_raw()) };
    if rc < 0 {
        return Err(rquickjs::Error::Exception);
    }
    if !num.is_finite() || num == 0.0 {
        return Ok(0);
    }
    let int = num.trunc();
    // Modulo 2^64 into signed i64 (ECMAScript ToInt64 / WebIDL long long)
    let two_pow_64 = 18_446_744_073_709_551_616.0_f64;
    let rem = int.rem_euclid(two_pow_64);
    if rem >= 9_223_372_036_854_775_808.0_f64 {
        Ok((rem - two_pow_64) as i64)
    } else {
        Ok(rem as i64)
    }
}

#[rquickjs::methods]
impl File {
    #[qjs(constructor)]
    fn new<'js>(
        ctx: Ctx<'js>,
        data: Opt<Value<'js>>,
        filename: Opt<Value<'js>>,
        options: Opt<Value<'js>>,
    ) -> Result<Self> {
        let Some(data) = data.0 else {
            return Err(Exception::throw_type(
                &ctx,
                "Failed to construct 'File': 2 arguments required, but only 0 present.",
            ));
        };
        let Some(filename_val) = filename.0 else {
            return Err(Exception::throw_type(
                &ctx,
                "Failed to construct 'File': 2 arguments required, but only 1 present.",
            ));
        };
        if data.is_undefined() {
            return Err(Exception::throw_type(
                &ctx,
                "Failed to construct 'File': The provided value cannot be converted to a sequence.",
            ));
        }

        let filename = usv_string_from_js(&ctx, &filename_val)?;

        let mut last_modified = time::now_millis();

        if let Some(ref opts) = options.0 {
            if !opts.is_undefined() && !opts.is_null() {
                let Some(v) = opts.as_object() else {
                    return Err(Exception::throw_type(
                        &ctx,
                        "Failed to construct 'File': parameter 3 is not of type 'Object'.",
                    ));
                };
                let lm_val: Value<'js> = v.get("lastModified")?;
                if !lm_val.is_undefined() {
                    last_modified = js_to_long_long(&ctx, &lm_val)?;
                }
            }
        }

        let blob = Blob::new(ctx, Opt(Some(data)), options)?;

        Ok(Self {
            blob,
            filename,
            last_modified,
        })
    }

    #[qjs(get)]
    pub fn size(&self) -> usize {
        self.blob.size()
    }

    #[qjs(get)]
    pub fn name(&self) -> String {
        self.filename.clone()
    }

    #[qjs(get, rename = "type")]
    pub fn mime_type(&self) -> String {
        self.blob.mime_type()
    }

    #[qjs(get, rename = "lastModified")]
    pub fn last_modified(&self) -> i64 {
        self.last_modified
    }

    pub fn slice<'js>(
        &self,
        ctx: Ctx<'js>,
        start: Opt<Value<'js>>,
        end: Opt<Value<'js>>,
        content_type: Opt<Value<'js>>,
    ) -> Result<Blob> {
        self.blob.slice(ctx, start, end, content_type)
    }

    pub fn stream<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.blob.stream(ctx)
    }

    pub async fn text(&self) -> String {
        self.blob.text().await
    }

    #[qjs(rename = "arrayBuffer")]
    pub async fn array_buffer<'js>(&self, ctx: Ctx<'js>) -> Result<ArrayBuffer<'js>> {
        self.blob.array_buffer(ctx).await
    }

    pub async fn bytes<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.blob.bytes(ctx).await
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(File)
    }
}

impl File {
    pub fn from_bytes<'js>(
        _ctx: &Ctx<'js>,
        data: Vec<u8>,
        filename: String,
        mime_type: Option<String>,
    ) -> Result<Self> {
        let blob = Blob::from_bytes(
            data,
            Some(mime_type.unwrap_or_else(|| "application/octet-stream".into())),
        );

        Ok(Self {
            blob,
            filename,
            last_modified: time::now_millis(),
        })
    }

    pub fn from_blob(blob: Blob, filename: String) -> Self {
        Self {
            blob,
            filename,
            last_modified: time::now_millis(),
        }
    }

    pub fn get_blob(&self) -> Blob {
        self.blob.clone()
    }
}
