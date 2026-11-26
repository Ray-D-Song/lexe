// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::ops::RangeInclusive;

use lexe_utils::{
    bytes::ObjectBytes,
    primordials::{BasePrimordials, Primordial},
};
use rquickjs::{
    atom::PredefinedAtom, class::Trace, function::Opt, qjs, Array, ArrayBuffer, Class, Ctx,
    Exception, Function, Result, Symbol, TypedArray, Value,
};

use super::codec::string_units;
use super::file::File;

static CONSTRUCT_ERROR: &str =
    "Failed to construct 'Blob': The provided value cannot be converted to a sequence.";

static BLOB_STREAM_FACTORY_JS: &str = r#"(function (bytes) {
  let offset = 0;
  return new ReadableStream({
    type: "bytes",
    pull(controller) {
      if (offset >= bytes.byteLength) {
        const req = controller.byobRequest;
        controller.close();
        if (req) req.respond(0);
        return;
      }
      const req = controller.byobRequest;
      if (req && req.view) {
        const view = req.view;
        const n = Math.min(view.byteLength, bytes.byteLength - offset);
        const dest = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
        dest.set(bytes.subarray(offset, offset + n));
        offset += n;
        if (offset >= bytes.byteLength) {
          controller.close();
        }
        req.respond(n);
        return;
      }
      const chunk = bytes.slice(offset);
      offset = bytes.byteLength;
      controller.enqueue(chunk);
      controller.close();
    }
  });
})"#;

#[derive(Clone, Copy)]
pub(crate) enum EndingType {
    Native,
    Transparent,
}

#[cfg(windows)]
const LINE_ENDING: &[u8] = b"\r\n";
#[cfg(not(windows))]
const LINE_ENDING: &[u8] = b"\n";

#[rquickjs::class]
#[derive(Trace, Clone, rquickjs::JsLifetime)]
pub struct Blob {
    #[qjs(skip_trace)]
    data: Vec<u8>,
    mime_type: String,
}

/// Convert any JS value to a WebIDL `USVString` (replacing lone surrogates with U+FFFD).
pub(crate) fn usv_string_from_js<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<String> {
    let raw = unsafe { qjs::JS_ToString(ctx.as_raw().as_ptr(), value.as_raw()) };
    let js_val = unsafe { Value::from_raw(ctx.clone(), raw) };
    if js_val.is_exception() {
        return Err(rquickjs::Error::Exception);
    }
    let js_str = js_val
        .into_string()
        .ok_or_else(|| Exception::throw_type(ctx, "Expected string"))?;
    let units = string_units(&js_str)?;
    Ok(char::decode_utf16(units)
        .map(|r| r.unwrap_or('\u{FFFD}'))
        .collect())
}

/// Convert a JS value using WebIDL `[Clamp] long long`.
fn js_to_clamped_i64<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<i64> {
    let mut num = 0.0_f64;
    let rc = unsafe { qjs::JS_ToFloat64(ctx.as_raw().as_ptr(), &mut num, value.as_raw()) };
    if rc < 0 {
        return Err(rquickjs::Error::Exception);
    }
    if num.is_nan() {
        return Ok(0);
    }
    if num >= i64::MAX as f64 {
        return Ok(i64::MAX);
    }
    if num <= i64::MIN as f64 {
        return Ok(i64::MIN);
    }
    // WebIDL round-half-to-even for [Clamp] integers
    let rounded = num.round_ties_even();
    Ok(rounded as i64)
}

pub(crate) fn normalize_type(mut mime_type: String) -> String {
    static INVALID_RANGE: RangeInclusive<u8> = 0x0020..=0x007E;

    let bytes = unsafe { mime_type.as_bytes_mut() };
    for byte in bytes {
        if !INVALID_RANGE.contains(byte) {
            return String::new();
        }
        byte.make_ascii_lowercase();
    }
    mime_type
}

pub(crate) fn parse_blob_options<'js>(
    ctx: &Ctx<'js>,
    options: &Option<Value<'js>>,
) -> Result<(EndingType, String)> {
    let mut endings = EndingType::Transparent;
    let mut mime_type = String::new();

    if let Some(opts) = options {
        if opts.is_undefined() || opts.is_null() {
            return Ok((endings, mime_type));
        }
        let Some(v) = opts.as_object() else {
            return Err(Exception::throw_type(
                ctx,
                "Failed to construct 'Blob': parameter 2 is not of type 'Object'.",
            ));
        };
        let endings_val: Value<'js> = v.get("endings")?;
        if !endings_val.is_undefined() {
            let endings_opt = usv_string_from_js(ctx, &endings_val)?;
            if endings_opt == "native" {
                endings = EndingType::Native;
            } else if endings_opt != "transparent" {
                return Err(Exception::throw_type(
                    ctx,
                    r#"expected 'endings' to be either 'transparent' or 'native'"#,
                ));
            }
        }
        let type_val: Value<'js> = v.get("type")?;
        if !type_val.is_undefined() {
            mime_type = normalize_type(usv_string_from_js(ctx, &type_val)?);
        }
    }

    Ok((endings, mime_type))
}

#[rquickjs::methods]
impl Blob {
    #[qjs(constructor)]
    pub fn new<'js>(
        ctx: Ctx<'js>,
        parts: Opt<Value<'js>>,
        options: Opt<Value<'js>>,
    ) -> Result<Self> {
        let (endings, mime_type) = parse_blob_options(&ctx, &options.0)?;

        let data = match parts.0 {
            Some(parts) if !parts.is_undefined() => bytes_from_parts(&ctx, parts, endings)?,
            _ => Vec::new(),
        };

        Ok(Self { data, mime_type })
    }

    #[qjs(get)]
    pub fn size(&self) -> usize {
        self.data.len()
    }

    #[qjs(get, rename = "type")]
    pub fn mime_type(&self) -> String {
        self.mime_type.clone()
    }

    pub async fn text(&self) -> String {
        let bytes = self
            .data
            .strip_prefix(b"\xEF\xBB\xBF")
            .unwrap_or(&self.data);
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[qjs(rename = "arrayBuffer")]
    pub async fn array_buffer<'js>(&self, ctx: Ctx<'js>) -> Result<ArrayBuffer<'js>> {
        ArrayBuffer::new(ctx, self.data.to_vec())
    }

    pub async fn bytes<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        TypedArray::new(ctx, self.data.to_vec()).map(|m| m.into_value())
    }

    pub fn stream<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let bytes = TypedArray::new(ctx.clone(), self.data.clone())?.into_value();
        let factory: Function<'js> = ctx.eval(BLOB_STREAM_FACTORY_JS)?;
        factory.call((bytes,))
    }

    pub fn slice<'js>(
        &self,
        ctx: Ctx<'js>,
        start: Opt<Value<'js>>,
        end: Opt<Value<'js>>,
        content_type: Opt<Value<'js>>,
    ) -> Result<Blob> {
        let len = self.data.len() as i64;
        let start = match start.0 {
            Some(v) if !v.is_undefined() => js_to_clamped_i64(&ctx, &v)?,
            _ => 0,
        };
        let end = match end.0 {
            Some(v) if !v.is_undefined() => js_to_clamped_i64(&ctx, &v)?,
            _ => len,
        };

        let relative_start = if start < 0 {
            (len as i128 + start as i128).max(0) as usize
        } else {
            len.min(start) as usize
        };
        let relative_end = if end < 0 {
            (len as i128 + end as i128).max(0) as usize
        } else {
            len.min(end) as usize
        };

        let span = relative_end.saturating_sub(relative_start);
        let data = self.data[relative_start..relative_start + span].to_vec();

        let mime_type = match content_type.0 {
            Some(v) if !v.is_undefined() => normalize_type(usv_string_from_js(&ctx, &v)?),
            _ => String::new(),
        };

        Ok(Blob { mime_type, data })
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(Blob)
    }
}

impl Blob {
    pub fn from_bytes(data: Vec<u8>, content_type: Option<String>) -> Self {
        let mime_type = content_type.map(normalize_type).unwrap_or_default();
        Self { mime_type, data }
    }

    pub fn get_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    //FIXME: cant use procedural macro for Symbol rename + static, see https://github.com/DelSkayn/rquickjs/issues/315
    pub fn has_instance(value: Value<'_>) -> bool {
        if let Some(obj) = value.as_object() {
            return obj.instance_of::<Self>() || obj.instance_of::<File>();
        }
        false
    }
}

fn convert_line_endings_native(input: &str, out: &mut Vec<u8>) {
    let bytes = input.as_bytes();
    out.reserve(bytes.len());
    let mut i = 0;
    let mut start = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                out.extend_from_slice(&bytes[start..i]);
                out.extend_from_slice(LINE_ENDING);
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    i += 2;
                } else {
                    i += 1;
                }
                start = i;
            },
            b'\n' => {
                out.extend_from_slice(&bytes[start..i]);
                out.extend_from_slice(LINE_ENDING);
                i += 1;
                start = i;
            },
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        out.extend_from_slice(&bytes[start..]);
    }
}

fn bytes_from_parts<'js>(
    ctx: &Ctx<'js>,
    parts: Value<'js>,
    endings: EndingType,
) -> Result<Vec<u8>> {
    let array: Array<'js> = if let Some(obj) = parts.as_object() {
        let iter_val: Value<'js> = obj.get(Symbol::iterator(ctx.clone()))?;
        if iter_val.is_undefined() || iter_val.is_null() || !iter_val.is_function() {
            return Err(Exception::throw_type(ctx, CONSTRUCT_ERROR));
        }
        BasePrimordials::get(ctx)?
            .function_array_from
            .call((parts.clone(),))?
    } else {
        return Err(Exception::throw_type(ctx, CONSTRUCT_ERROR));
    };

    let mut data = Vec::new();
    for elem in array.iter::<Value>() {
        let elem = elem?;
        if let Ok(blob) = Class::<Blob>::from_value(&elem) {
            data.extend_from_slice(&blob.borrow().data);
            continue;
        }
        if let Ok(file) = Class::<File>::from_value(&elem) {
            data.extend_from_slice(&file.borrow().get_blob().data);
            continue;
        }
        if let Some(object) = elem.as_object() {
            if let Some(view) = ObjectBytes::from_array_buffer(object)? {
                let bytes = view.as_bytes(ctx).map_err(|_| {
                    Exception::throw_type(ctx, "Cannot create a blob with detached buffer")
                })?;
                data.extend_from_slice(bytes);
                continue;
            }
            if let Some(ab) = ArrayBuffer::from_object(object.clone()) {
                data.extend_from_slice(ab.as_bytes().ok_or_else(|| {
                    Exception::throw_type(ctx, "Cannot create a blob with detached buffer")
                })?);
                continue;
            }
        }

        let string = usv_string_from_js(ctx, &elem)?;
        match endings {
            EndingType::Transparent => data.extend_from_slice(string.as_bytes()),
            EndingType::Native => convert_line_endings_native(&string, &mut data),
        }
    }
    Ok(data)
}
