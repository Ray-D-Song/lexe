// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{mem::MaybeUninit, slice};

use crate::codec::{
    decoded_to_js, is_falsy, resolve_encoding, string_units, throw_coded, unknown_encoding,
    validate_offset,
};
use raster_runtime_encoding::{Encoder, NodeEncoding};
use raster_runtime_utils::{
    bytes::{get_array_bytes, get_start_end_indexes, ObjectBytes},
    error_messages::{ERROR_MSG_ARRAY_BUFFER_DETACHED, ERROR_MSG_NOT_ARRAY_BUFFER},
    iterable_enum,
    primordials::Primordial,
    result::ResultExt,
    string::get_coerced_string,
};
use rquickjs::{
    atom::PredefinedAtom,
    convert::Coerced,
    function::{Constructor, Opt},
    prelude::{Func, Rest, This},
    Array, ArrayBuffer, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Result, TypedArray,
    Value,
};

#[derive(JsLifetime)]
pub struct BufferPrimordials<'js> {
    constructor: Constructor<'js>,
}

impl<'js> Primordial<'js> for BufferPrimordials<'js> {
    fn new(ctx: &Ctx<'js>) -> Result<Self>
    where
        Self: Sized,
    {
        let constructor: Constructor = ctx.globals().get(stringify!(Buffer))?;

        Ok(Self { constructor })
    }
}

pub struct Buffer(pub Vec<u8>);

impl<'js> IntoJs<'js> for Buffer {
    fn into_js(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        let array_buffer = ArrayBuffer::new(ctx.clone(), self.0)?;
        Self::from_array_buffer(ctx, array_buffer)
    }
}

impl<'js> Buffer {
    pub fn alloc(length: usize) -> Self {
        Self(vec![0; length])
    }

    pub fn to_string(&self, ctx: &Ctx<'js>, encoding: &str) -> Result<String> {
        if let Some(node) = NodeEncoding::parse(encoding) {
            return Ok(match node.decode_bytes(self.0.as_ref()) {
                raster_runtime_encoding::DecodedText::Str(s) => s,
                raster_runtime_encoding::DecodedText::Units(u) => String::from_utf16_lossy(&u),
            });
        }
        Encoder::from_str(encoding)
            .and_then(|enc| enc.encode_to_string(self.0.as_ref(), true))
            .or_throw(ctx)
    }

    fn from_array_buffer(ctx: &Ctx<'js>, buffer: ArrayBuffer<'js>) -> Result<Value<'js>> {
        BufferPrimordials::get(ctx)?
            .constructor
            .construct((buffer,))
    }

    fn from_array_buffer_offset_length(
        ctx: &Ctx<'js>,
        array_buffer: ArrayBuffer<'js>,
        offset: usize,
        length: usize,
    ) -> Result<Value<'js>> {
        BufferPrimordials::get(ctx)?
            .constructor
            .construct((array_buffer, offset, length))
    }

    fn from_encoding(ctx: &Ctx<'js>, bytes: Vec<u8>) -> Result<Value<'js>> {
        Buffer(bytes).into_js(ctx)
    }

    fn from_units(ctx: &Ctx<'js>, units: &[u16], encoding: Option<&str>) -> Result<Value<'js>> {
        let node = match encoding {
            Some(label) if !label.is_empty() => {
                NodeEncoding::parse(label).ok_or_else(|| unknown_encoding(ctx, label))?
            },
            _ => NodeEncoding::Utf8,
        };
        Buffer(node.encode_units(units)).into_js(ctx)
    }
}

// Static Methods
fn alloc<'js>(
    ctx: Ctx<'js>,
    length: usize,
    fill: Opt<Value<'js>>,
    encoding: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    if let Some(value) = fill.0 {
        if let Some(value) = value.as_string() {
            let units = string_units(value)?;
            if units.is_empty() {
                return Buffer(vec![0; length]).into_js(&ctx);
            }
            let node = match encoding.0.as_ref() {
                Some(e) if is_falsy(e) => NodeEncoding::Utf8,
                Some(e) => resolve_encoding(&ctx, e)?,
                None => NodeEncoding::Utf8,
            };
            let bytes = node.encode_units(&units);
            if bytes.is_empty() {
                return Err(throw_coded(
                    &ctx,
                    false,
                    "ERR_INVALID_ARG_VALUE",
                    format!(
                        "The argument 'value' is invalid. Received '{}'",
                        String::from_utf16_lossy(&units)
                    ),
                ));
            }
            return alloc_byte_ref(&ctx, &bytes, length);
        }
        if let Some(value) = value.as_int() {
            let bytes = vec![value as u8; length];
            return Buffer(bytes).into_js(&ctx);
        }
        if let Some(obj) = value.as_object() {
            if let Some(ob) = ObjectBytes::from_array_buffer(obj)? {
                let bytes = ob.as_bytes(&ctx)?;
                return alloc_byte_ref(&ctx, bytes, length);
            }
        }
    }

    Buffer(vec![0; length]).into_js(&ctx)
}

fn alloc_byte_ref<'js>(ctx: &Ctx<'js>, byte_ref: &[u8], length: usize) -> Result<Value<'js>> {
    let mut bytes = vec![0; length];
    let byte_ref_length = byte_ref.len();
    if byte_ref_length == 0 {
        return Buffer(bytes).into_js(ctx);
    }
    for i in 0..length {
        bytes[i] = byte_ref[i % byte_ref_length];
    }
    Buffer(bytes).into_js(ctx)
}

fn alloc_unsafe(ctx: Ctx<'_>, size: usize) -> Result<Value<'_>> {
    let mut bytes: Vec<MaybeUninit<u8>> = Vec::with_capacity(size);
    unsafe {
        bytes.set_len(size);
    }

    Buffer(maybeuninit_to_u8(bytes)).into_js(&ctx)
}

fn maybeuninit_to_u8(vec: Vec<MaybeUninit<u8>>) -> Vec<u8> {
    let len = vec.len();
    let capacity = vec.capacity();
    let ptr = vec.as_ptr() as *mut u8;

    std::mem::forget(vec);

    // This conversion is safe because MaybeUninit has the same memory layout as u8, meaning the underlying bytes are identical.
    // Since Vec<MaybeUninit> and Vec share the same memory representation, a simple reinterpretation of the pointer is valid.
    // Additionally, Vec::from_raw_parts correctly reconstructs the vector using the original length and capacity, ensuring that memory ownership remains consistent.
    // The call to std::mem::forget(vec) prevents the original Vec<MaybeUninit> from being dropped, avoiding double frees or memory corruption.
    // However, this conversion is only safe if all elements of MaybeUninit are properly initialized.
    // If any uninitialized values exist, reading them as u8 would lead to undefined behavior.
    unsafe { Vec::from_raw_parts(ptr, len, capacity) }
}

fn alloc_unsafe_slow(ctx: Ctx<'_>, size: usize) -> Result<Value<'_>> {
    let layout = std::alloc::Layout::array::<u8>(size).or_throw(&ctx)?;

    let bytes = unsafe {
        let ptr = std::alloc::alloc(layout);
        if ptr.is_null() {
            return Err(Exception::throw_internal(&ctx, "Memory allocation failed"));
        }
        Vec::from_raw_parts(ptr, size, size)
    };
    Buffer(bytes).into_js(&ctx)
}

fn byte_length<'js>(ctx: Ctx<'js>, value: Value<'js>, encoding: Opt<Value<'js>>) -> Result<usize> {
    if let Some(js) = value.as_string() {
        let units = string_units(js)?;
        let node = match encoding.0.as_ref().filter(|e| !is_falsy(e)) {
            Some(e) => {
                let label = e.as_string().map(|s| s.to_string()).unwrap_or_else(|| {
                    Ok(e.get::<Coerced<String>>().map(|c| c.0).unwrap_or_default())
                })?;
                NodeEncoding::parse(&label).unwrap_or(NodeEncoding::Utf8)
            },
            None => NodeEncoding::Utf8,
        };
        return Ok(node.byte_length(&units));
    }

    if value.is_array() {
        let array = value.as_array().unwrap();

        for val in array.iter::<u8>() {
            val.or_throw_msg(&ctx, "array value is not u8")?;
        }

        return Ok(array.len());
    }

    if let Some(obj) = value.as_object() {
        if let Some(ob) = ObjectBytes::from_array_buffer(obj)? {
            return Ok(ob.as_bytes(&ctx)?.len());
        }
    }

    Err(Exception::throw_message(
        &ctx,
        "value must be typed DataView, Buffer, ArrayBuffer, Uint8Array or string",
    ))
}

fn concat<'js>(ctx: Ctx<'js>, list: Array<'js>, max_length: Opt<usize>) -> Result<Value<'js>> {
    let mut bytes = Vec::new();
    let mut total_length = 0;
    let mut length;
    for value in list.iter::<Object>() {
        let typed_array = TypedArray::<u8>::from_object(value?)?;
        let bytes_ref: &[u8] = typed_array.as_ref();

        length = bytes_ref.len();

        if length == 0 {
            continue;
        }

        if let Some(max_length) = max_length.0 {
            total_length += length;
            if total_length > max_length {
                let diff = max_length - (total_length - length);
                bytes.extend_from_slice(&bytes_ref[0..diff]);
                break;
            }
        }
        bytes.extend_from_slice(bytes_ref);
    }

    Buffer(bytes).into_js(&ctx)
}

fn from<'js>(
    ctx: Ctx<'js>,
    value: Value<'js>,
    offset_or_encoding: Opt<Value<'js>>,
    length: Opt<usize>,
) -> Result<Value<'js>> {
    let mut encoding: Option<String> = None;
    let mut offset = 0;

    if let Some(offset_or_encoding) = offset_or_encoding.0 {
        if offset_or_encoding.is_string() {
            encoding = Some(offset_or_encoding.get()?);
        } else if offset_or_encoding.is_number() {
            offset = offset_or_encoding.get()?;
        }
    }

    if let Some(js) = value.as_string() {
        let units = string_units(js)?;
        return Buffer::from_units(&ctx, &units, encoding.as_deref());
    }
    if let Some(bytes) = get_array_bytes(&value, offset, length.0)? {
        return Buffer::from_encoding(&ctx, bytes);
    }

    if let Some(obj) = value.as_object() {
        if let Some(ab_bytes) = ObjectBytes::from_array_buffer(obj)? {
            let bytes = ab_bytes.as_bytes(&ctx)?;
            let (start, end) = get_start_end_indexes(bytes.len(), length.0, offset);

            //buffers from buffer should be copied
            if obj
                .get::<_, Option<String>>(PredefinedAtom::Meta)?
                .as_deref()
                == Some(stringify!(Buffer))
                || encoding.is_some()
            {
                let bytes = bytes.into();
                return Buffer::from_encoding(&ctx, bytes);
            } else {
                let (array_buffer, _, source_offset) = ab_bytes.get_array_buffer()?.unwrap(); //we know it's an array buffer
                return Buffer::from_array_buffer_offset_length(
                    &ctx,
                    array_buffer,
                    start + source_offset,
                    end - start,
                );
            }
        }
    }

    if let Some(string) = get_coerced_string(&value) {
        let units: Vec<u16> = string.encode_utf16().collect();
        return Buffer::from_units(&ctx, &units, encoding.as_deref());
    }

    Err(Exception::throw_message(
        &ctx,
        "value must be typed DataView, Buffer, ArrayBuffer, Uint8Array or interpretable as string",
    ))
}

fn is_buffer<'js>(ctx: Ctx<'js>, value: Value<'js>) -> Result<bool> {
    if let Some(object) = value.as_object() {
        let constructor = BufferPrimordials::get(&ctx)?;
        return Ok(object.is_instance_of(&constructor.constructor));
    }

    Ok(false)
}

fn is_encoding(value: Value) -> Result<bool> {
    if let Some(js_string) = value.as_string() {
        let std_string = js_string.to_string()?;
        return Ok(NodeEncoding::parse(std_string.as_str()).is_some());
    }

    Ok(false)
}

/// Number.MAX_SAFE_INTEGER (2^53 - 1) / Number.MIN_SAFE_INTEGER.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const MIN_SAFE_INTEGER: f64 = -9_007_199_254_740_991.0;

/// ECMAScript `IsIntegralNumber`.
fn is_integral_number(n: f64) -> bool {
    n.is_finite() && n.fract() == 0.0
}

/// Node `buffer.js` `toInteger(n, defaultVal)` with defaultVal always `0` for copy indices.
fn to_integer_default_zero(n: f64) -> i64 {
    if n.is_nan() || !n.is_finite() || n < MIN_SAFE_INTEGER || n > MAX_SAFE_INTEGER {
        return 0;
    }
    // Math.floor (not trunc): -0.5 → -1.
    n.floor() as i64
}

fn preserve_integral_number(n: f64) -> i64 {
    if n >= i64::MAX as f64 {
        return i64::MAX;
    }
    if n <= i64::MIN as f64 {
        return i64::MIN;
    }
    n as i64
}

/// Normalize a Buffer.copy index following Node `copyImpl`.
///
/// - `undefined` → positional default (`0` / `0` / `source.length`)
/// - **raw** JS Number that is already integral (incl. `2**53`) → keep as-is
///   (`NumberIsInteger` on the original value, *before* ToNumber coercion)
/// - otherwise ToNumber + `toInteger(n, 0)`: NaN / ±Infinity / out-of-safe-range → `0`;
///   other non-integers use `Math.floor`
fn normalize_copy_index(
    _ctx: &Ctx<'_>,
    value: Option<Value<'_>>,
    default_if_undefined: i64,
) -> Result<i64> {
    let Some(value) = value else {
        return Ok(default_if_undefined);
    };
    if value.is_undefined() {
        return Ok(default_if_undefined);
    }
    // Node: NumberIsInteger on the *original* value first.
    // Strings / objects with valueOf go through ToNumber → toInteger(..., 0).
    if let Some(n) = value.as_number() {
        if is_integral_number(n) {
            return Ok(preserve_integral_number(n));
        }
    }
    // ToNumber coercion (null→0, true→1, "1"→1, Symbol throws).
    let n = value.get::<rquickjs::Coerced<f64>>()?.0;
    Ok(to_integer_default_zero(n))
}

// Prototype Methods
fn copy<'js>(
    this: This<Object<'js>>,
    ctx: Ctx<'js>,
    target: Value<'js>,
    target_start: Opt<Value<'js>>,
    source_start: Opt<Value<'js>>,
    source_end: Opt<Value<'js>>,
) -> Result<usize> {
    // Resolve TypedArray objects only. Index coercion may run user valueOf() which
    // can transfer/resize/detach backing buffers; raw pointers must not span that.
    // Validation order matches Node `copyImpl`: convert + check each arg immediately
    // so later args (e.g. Symbol) are not evaluated after an earlier RangeError.
    let source_view = TypedArray::<u8>::from_object(this.0.clone()).or_throw(&ctx)?;

    let target_obj = target
        .as_object()
        .ok_or_else(|| Exception::throw_type(&ctx, "Target must be a Uint8Array or Buffer"))?
        .clone();
    let target_view = TypedArray::<u8>::from_object(target_obj).or_throw(&ctx)?;

    // 1) targetStart — convert then immediately check negative.
    let target_start = normalize_copy_index(&ctx, target_start.0, 0)?;
    if target_start < 0 {
        return Err(Exception::throw_range(
            &ctx,
            "The value of \"targetStart\" is out of range.",
        ));
    }

    // 2) sourceStart — convert then check against *current* source length.
    let source_start = normalize_copy_index(&ctx, source_start.0, 0)?;
    // Length only (no retained raw pointer across further JS).
    let source_length_for_check = source_view.as_raw().map(|r| r.len).unwrap_or(0);
    if source_start < 0 || source_start > source_length_for_check as i64 {
        return Err(Exception::throw_range(
            &ctx,
            "The value of \"sourceStart\" is out of range.",
        ));
    }

    // 3) sourceEnd — convert or default; check negative immediately.
    //    Default uses length observed after sourceStart (Node: source.byteLength).
    let source_end = match source_end.0 {
        Some(value) if !value.is_undefined() => {
            let e = normalize_copy_index(&ctx, Some(value), 0)?;
            if e < 0 {
                return Err(Exception::throw_range(
                    &ctx,
                    "The value of \"sourceEnd\" is out of range.",
                ));
            }
            e
        },
        _ => source_length_for_check as i64,
    };

    // Re-acquire view-local regions after all JS-executing conversions.
    // Detached (or otherwise unreadable) buffers: Node-compatible Ok(0) for
    // the final copy step when length checks would no-op, or when as_raw fails.
    let Some(source_raw) = source_view.as_raw() else {
        return Ok(0);
    };
    let Some(target_raw) = target_view.as_raw() else {
        return Ok(0);
    };

    let source_length = source_raw.len;
    let target_length = target_raw.len;

    // Safe to cast: negatives already rejected.
    let target_start_u = target_start as u64;
    let source_start_u = source_start as u64;
    let source_end_u = source_end as u64;
    let source_length_u = source_length as u64;
    let target_length_u = target_length as u64;

    // Final gates (Node: targetStart >= target.byteLength || sourceStart >= sourceEnd).
    if source_start_u >= source_end_u {
        return Ok(0);
    }
    if target_start_u >= target_length_u {
        return Ok(0);
    }

    // Clamp sourceEnd to current source length (Node _copyActual).
    let source_end_u = source_end_u.min(source_length_u);
    if source_end_u <= source_start_u {
        return Ok(0);
    }

    let target_start = target_start_u as usize;
    let source_start = source_start_u as usize;
    let source_end = source_end_u as usize;

    let copyable = (source_end - source_start)
        .min(source_length - source_start)
        .min(target_length - target_start);
    if copyable == 0 {
        return Ok(0);
    }

    if source_start
        .checked_add(copyable)
        .map(|end| end > source_raw.len)
        .unwrap_or(true)
        || target_start
            .checked_add(copyable)
            .map(|end| end > target_raw.len)
            .unwrap_or(true)
    {
        return Ok(0);
    }

    // View-local pointers from post-coercion as_raw(); ptr::copy is memmove-safe for overlap.
    unsafe {
        let src = source_raw.ptr.as_ptr().add(source_start);
        let dst = target_raw.ptr.as_ptr().add(target_start);
        std::ptr::copy(src, dst, copyable);
    }

    // Keep views live across the unsafe copy.
    let _ = (source_view, target_view);

    Ok(copyable)
}

fn equals<'js>(this: This<Object<'js>>, ctx: Ctx<'js>, other: Value<'js>) -> Result<bool> {
    let this_view = TypedArray::<u8>::from_object(this.0.clone()).map_err(|_| {
        Exception::throw_type(
            &ctx,
            "The receiver must be an instance of Buffer or Uint8Array",
        )
    })?;

    if *this.0.as_value() == other {
        return Ok(true);
    }

    let other_obj = other.as_object().ok_or_else(|| {
        Exception::throw_type(
            &ctx,
            "The \"otherBuffer\" argument must be an instance of Buffer or Uint8Array",
        )
    })?;
    let other_view = TypedArray::<u8>::from_object(other_obj.clone()).map_err(|_| {
        Exception::throw_type(
            &ctx,
            "The \"otherBuffer\" argument must be an instance of Buffer or Uint8Array",
        )
    })?;

    let this_bytes: &[u8] = this_view.as_ref();
    let other_bytes: &[u8] = other_view.as_ref();
    if this_bytes.len() != other_bytes.len() {
        return Ok(false);
    }
    if this_bytes.is_empty() {
        return Ok(true);
    }
    Ok(this_bytes == other_bytes)
}

fn subarray<'js>(
    this: This<Object<'js>>,
    ctx: Ctx<'js>,
    start: Opt<isize>,
    end: Opt<isize>,
) -> Result<Value<'js>> {
    let view = TypedArray::<u8>::from_object(this.0.clone())?;

    let array_buffer = view.arraybuffer()?;
    // Use internal TypedArray region (len + as_raw) — not shadowed byteOffset/byteLength.
    let view_raw = view
        .as_raw()
        .ok_or(ERROR_MSG_ARRAY_BUFFER_DETACHED)
        .or_throw(&ctx)?;
    let ab_raw = array_buffer
        .as_raw()
        .ok_or(ERROR_MSG_ARRAY_BUFFER_DETACHED)
        .or_throw(&ctx)?;
    let view_offset = unsafe { view_raw.ptr.as_ptr().offset_from(ab_raw.ptr.as_ptr()) };
    if view_offset < 0 {
        return Err(Exception::throw_message(
            &ctx,
            ERROR_MSG_ARRAY_BUFFER_DETACHED,
        ));
    }
    let view_offset = view_offset as isize;
    // Prefer as_raw().len (internal) over JS "length" which can be shadowed.
    let view_length = view_raw.len as isize;

    let start_index = start.map_or(0, |s| {
        if s < 0 {
            (view_length + s).max(0)
        } else {
            s.min(view_length)
        }
    });

    let end_index = end.map_or(view_length, |e| {
        if e < 0 {
            (view_length + e).max(0)
        } else {
            e.min(view_length)
        }
    });

    let length = (end_index - start_index).max(0) as usize;
    let new_offset = (view_offset + start_index).max(0) as usize;

    Buffer::from_array_buffer_offset_length(&ctx, array_buffer, new_offset, length)
}

fn to_string<'js>(
    this: This<Object<'js>>,
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<Value<'js>> {
    let len = TypedArray::<u8>::from_object(this.0.clone())?.len() as f64;

    let number = |v: &Value<'js>| -> Result<f64> { Ok(v.get::<Coerced<f64>>()?.0) };
    let undefined = Value::new_undefined(ctx.clone());
    let encoding = args.0.first().unwrap_or(&undefined);
    let start_arg = args.0.get(1).unwrap_or(&undefined);
    let end_arg = args.0.get(2).unwrap_or(&undefined);

    let (start, end) = if args.0.is_empty() {
        (0.0, len)
    } else {
        let s = number(start_arg)?;
        let start = if s <= 0.0 {
            0.0
        } else if s >= len {
            return rquickjs::String::from_str(ctx.clone(), "").map(|s| s.into_value());
        } else {
            let t = s.trunc();
            if t.is_nan() {
                0.0
            } else {
                t
            }
        };
        let end = if end_arg.is_undefined() {
            len
        } else {
            let e = number(end_arg)?;
            if e > len {
                len
            } else {
                let t = e.trunc();
                if t.is_nan() {
                    0.0
                } else {
                    t
                }
            }
        };
        if end <= start {
            return rquickjs::String::from_str(ctx.clone(), "").map(|s| s.into_value());
        }
        (start, end)
    };

    let node = if encoding.is_undefined() {
        NodeEncoding::Utf8
    } else {
        resolve_encoding(&ctx, encoding)?
    };

    let typed_array = TypedArray::<u8>::from_object(this.0)?;
    let bytes: &[u8] = typed_array.as_ref();
    let end = (end as usize).min(bytes.len());
    let start = (start as usize).min(end);
    decoded_to_js(&ctx, node.decode_bytes(&bytes[start..end]))
}

fn write<'js>(this: This<Object<'js>>, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<usize> {
    let undefined = Value::new_undefined(ctx.clone());
    let string = args.0.first().unwrap_or(&undefined);
    let offset_arg = args.0.get(1).unwrap_or(&undefined);
    let length_arg = args.0.get(2).unwrap_or(&undefined);
    let encoding_arg = args.0.get(3).unwrap_or(&undefined);

    let target = ObjectBytes::from(&ctx, this.0.as_inner())?;
    let Some((array_buffer, view_len, view_offset)) = target.get_array_buffer()? else {
        return Ok(0);
    };
    let buf_len = view_len as f64;

    let not_string = || {
        throw_coded(
            &ctx,
            false,
            "ERR_INVALID_ARG_TYPE",
            "argument must be a string".to_string(),
        )
    };

    let offset;
    let length;
    let node;
    if offset_arg.is_undefined() {
        offset = 0.0;
        length = buf_len;
        node = NodeEncoding::Utf8;
        if !string.is_string() {
            return Err(not_string());
        }
    } else {
        let mut encoding = encoding_arg.clone();
        if length_arg.is_undefined() && offset_arg.is_string() {
            encoding = offset_arg.clone();
            length = buf_len;
            offset = 0.0;
        } else {
            offset = validate_offset(&ctx, offset_arg, "offset", 0.0, buf_len)?;
            let remaining = buf_len - offset;
            if length_arg.is_undefined() {
                length = remaining;
            } else if length_arg.is_string() {
                encoding = length_arg.clone();
                length = remaining;
            } else {
                let l = validate_offset(&ctx, length_arg, "length", 0.0, buf_len)?;
                length = l.min(remaining);
            }
        }
        node = if is_falsy(&encoding)
            || encoding
                .as_string()
                .is_some_and(|s| s.to_string().is_ok_and(|s| s == "utf8"))
        {
            NodeEncoding::Utf8
        } else {
            resolve_encoding(&ctx, &encoding)?
        };
        if !string.is_string() {
            return Err(not_string());
        }
    }

    let units = string_units(string.as_string().unwrap())?;
    let encoded = node.encode_units(&units);
    let written = node.writable_len(&encoded, length as usize);

    let raw = array_buffer
        .as_raw()
        .ok_or(ERROR_MSG_ARRAY_BUFFER_DETACHED)
        .or_throw(&ctx)?;
    let target_bytes = unsafe { slice::from_raw_parts_mut(raw.ptr.as_ptr(), raw.len) };
    let start = view_offset + offset as usize;
    target_bytes[start..start + written].copy_from_slice(&encoded[..written]);

    Ok(written)
}

#[derive(Clone, Copy)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NumberKind {
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Float32,
    Float64,
    BigInt,
    BigUInt,
}

impl NumberKind {
    pub fn bits(&self) -> u8 {
        match self {
            NumberKind::Int8 => 8,
            NumberKind::UInt8 => 8,
            NumberKind::Int16 => 16,
            NumberKind::UInt16 => 16,
            NumberKind::Int32 => 32,
            NumberKind::UInt32 => 32,
            NumberKind::Float32 => 32,
            NumberKind::Float64 => 64,
            NumberKind::BigInt => 64,
            NumberKind::BigUInt => 64,
        }
    }

    pub fn is_signed(&self) -> bool {
        matches!(
            self,
            NumberKind::Int8 | NumberKind::Int16 | NumberKind::Int32
        )
    }

    pub fn prototype(&self) -> &'static [(Endian, &'static str, Option<&'static str>)] {
        match self {
            NumberKind::Int8 => &[(Endian::Little, "Int8", None)],
            NumberKind::UInt8 => &[(Endian::Little, "UInt8", Some("Uint8"))],
            NumberKind::Int16 => &[
                (Endian::Little, "Int16LE", None),
                (Endian::Big, "Int16BE", None),
            ],
            NumberKind::UInt16 => &[
                (Endian::Little, "UInt16LE", Some("Uint16LE")),
                (Endian::Big, "UInt16BE", Some("Uint16BE")),
            ],
            NumberKind::Int32 => &[
                (Endian::Little, "Int32LE", None),
                (Endian::Big, "Int32BE", None),
            ],
            NumberKind::UInt32 => &[
                (Endian::Little, "UInt32LE", Some("Uint32LE")),
                (Endian::Big, "UInt32BE", Some("Uint32BE")),
            ],
            NumberKind::Float32 => &[
                (Endian::Little, "FloatLE", None),
                (Endian::Big, "FloatBE", None),
            ],
            NumberKind::Float64 => &[
                (Endian::Little, "DoubleLE", None),
                (Endian::Big, "DoubleBE", None),
            ],
            NumberKind::BigInt => &[
                (Endian::Little, "BigInt64LE", None),
                (Endian::Big, "BigInt64BE", None),
            ],
            NumberKind::BigUInt => &[
                (Endian::Little, "BigUInt64LE", Some("BigUint64LE")),
                (Endian::Big, "BigUInt64BE", Some("BigUint64BE")),
            ],
        }
    }
}

iterable_enum!(
    NumberKind, Int8, UInt8, Int16, UInt16, Int32, UInt32, Float32, Float64, BigInt, BigUInt
);

#[allow(clippy::too_many_arguments)]
fn write_buf<'js>(
    this: &This<Object<'js>>,
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    offset: &Opt<usize>,
    endian: Endian,
    kind: NumberKind,
) -> Result<usize> {
    let offset = offset.0.unwrap_or_default();

    // Extract and convert value
    let (byte_count, bytes) = match kind {
        NumberKind::BigInt => {
            let Some(bigint) = value.as_big_int() else {
                return Err(Exception::throw_type(ctx, "Expected BigInt"));
            };
            let (byte_count, val) = (8, bigint.clone().to_i64().or_throw(ctx)? as u64);
            (byte_count, endian_bytes(val, endian))
        },
        NumberKind::BigUInt => {
            return Err(Exception::throw_type(ctx, "Uint64 is not supported"));
        },
        NumberKind::Float32 => {
            // Accept both int-tagged and float-tagged JS Numbers (e.g. 7 and 7.5).
            let Some(float_val) = value.as_number() else {
                return Err(Exception::throw_type(ctx, "Expected number"));
            };
            match endian {
                Endian::Big => (4, (float_val as f32).to_bits().to_be_bytes().to_vec()),
                Endian::Little => (4, (float_val as f32).to_bits().to_le_bytes().to_vec()),
            }
        },
        NumberKind::Float64 => {
            let Some(float_val) = value.as_number() else {
                return Err(Exception::throw_type(ctx, "Expected number"));
            };
            match endian {
                Endian::Big => (8, float_val.to_bits().to_be_bytes().to_vec()),
                Endian::Little => (8, float_val.to_bits().to_le_bytes().to_vec()),
            }
        },
        NumberKind::Int8
        | NumberKind::UInt8
        | NumberKind::Int16
        | NumberKind::UInt16
        | NumberKind::Int32
        | NumberKind::UInt32 => {
            let Some(int_val) = value.as_number() else {
                return Err(Exception::throw_type(ctx, "Expected number"));
            };
            let int_val = int_val as i64;
            let bit_mask = (1i64 << kind.bits()) - 1;
            let max_val = if kind.is_signed() {
                (1i64 << (kind.bits() - 1)) - 1
            } else {
                bit_mask
            };
            let min_val = if kind.is_signed() { -max_val - 1 } else { 0 };

            if int_val < min_val || int_val > max_val {
                return Err(Exception::throw_range(ctx, "Value out of range"));
            }

            let masked = int_val & bit_mask;
            (
                (kind.bits() / 8) as usize,
                shifted_bytes(masked as u64, kind.bits(), endian),
            )
        },
    };

    let target = ObjectBytes::from(ctx, this.0.as_inner())?;
    let Some((array_buffer, view_length, view_offset)) = target.get_array_buffer()? else {
        return Err(Exception::throw_message(ctx, ERROR_MSG_NOT_ARRAY_BUFFER));
    };

    if offset
        .checked_add(byte_count)
        .map(|end| end > view_length)
        .unwrap_or(true)
    {
        return Err(Exception::throw_range(
            ctx,
            "The specified offset is out of range",
        ));
    }

    let raw = array_buffer
        .as_raw()
        .ok_or(ERROR_MSG_ARRAY_BUFFER_DETACHED)
        .or_throw(ctx)?;

    if view_offset
        .checked_add(view_length)
        .map(|end| end > raw.len)
        .unwrap_or(true)
    {
        return Err(Exception::throw_range(
            ctx,
            "The specified offset is out of range",
        ));
    }

    let abs_start = view_offset + offset;
    let abs_end = abs_start + byte_count;
    let target_bytes = unsafe { slice::from_raw_parts_mut(raw.ptr.as_ptr(), raw.len) };
    target_bytes[abs_start..abs_end].copy_from_slice(&bytes);

    // Return the next offset relative to the view, not the backing buffer.
    Ok(offset + byte_count)
}

fn read_buf<'js>(
    this: &This<Object<'js>>,
    ctx: &Ctx<'js>,
    offset: &Opt<usize>,
    endian: Endian,
    kind: NumberKind,
) -> Result<Value<'js>> {
    // Retrieve the array buffer with the TypedArray view bounds.
    let target = ObjectBytes::from(ctx, this.0.as_inner())?;
    let Some((array_buffer, view_length, view_offset)) = target.get_array_buffer()? else {
        return Err(Exception::throw_message(ctx, ERROR_MSG_NOT_ARRAY_BUFFER));
    };
    let raw = array_buffer
        .as_raw()
        .ok_or(ERROR_MSG_ARRAY_BUFFER_DETACHED)
        .or_throw(ctx)?;
    let target_bytes = unsafe { slice::from_raw_parts_mut(raw.ptr.as_ptr(), raw.len) };

    if view_offset
        .checked_add(view_length)
        .map(|end| end > raw.len)
        .unwrap_or(true)
    {
        return Err(Exception::throw_range(
            ctx,
            "The value of \"offset\" is out of range",
        ));
    }

    // Enforce bounds relative to the view, not the backing buffer.
    let start = offset.0.unwrap_or_default();
    let byte_count = (kind.bits() / 8) as usize;
    let end = start
        .checked_add(byte_count)
        .ok_or_else(|| Exception::throw_range(ctx, "The value of \"offset\" is out of range"))?;
    if end > view_length {
        return Err(Exception::throw_range(
            ctx,
            "The value of \"offset\" is out of range",
        ));
    }

    let abs_start = view_offset + start;
    let abs_end = abs_start + byte_count;
    let bytes = &target_bytes[abs_start..abs_end];

    let value = match kind {
        NumberKind::BigInt => {
            let value = match endian {
                Endian::Big => i64::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => i64::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_big_int(ctx.clone(), value)?
        },
        NumberKind::BigUInt => {
            return Err(Exception::throw_type(ctx, "Uint64 is not supported"));
        },
        NumberKind::Float32 => {
            let value = match endian {
                Endian::Big => f32::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => f32::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_float(ctx.clone(), value as f64)
        },
        NumberKind::Float64 => {
            let value = match endian {
                Endian::Big => f64::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => f64::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_float(ctx.clone(), value)
        },
        NumberKind::Int8 => {
            let value = match endian {
                Endian::Big => i8::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => i8::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_int(ctx.clone(), value as i32)
        },
        NumberKind::UInt8 => {
            let value = match endian {
                Endian::Big => u8::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => u8::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_int(ctx.clone(), value as i32)
        },
        NumberKind::Int16 => {
            let value = match endian {
                Endian::Big => i16::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => i16::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_int(ctx.clone(), value as i32)
        },
        NumberKind::UInt16 => {
            let value = match endian {
                Endian::Big => u16::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => u16::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_int(ctx.clone(), value as i32)
        },
        NumberKind::Int32 => {
            let value = match endian {
                Endian::Big => i32::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => i32::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_int(ctx.clone(), value)
        },
        NumberKind::UInt32 => {
            let value = match endian {
                Endian::Big => u32::from_be_bytes(bytes.try_into().unwrap()),
                Endian::Little => u32::from_le_bytes(bytes.try_into().unwrap()),
            };
            Value::new_float(ctx.clone(), value as f64)
        },
    };
    Ok(value)
}

// Pure mathematical byte generation
fn endian_bytes(mut val: u64, endian: Endian) -> Vec<u8> {
    let mut bytes = vec![0u8; 8];

    #[allow(clippy::needless_range_loop)]
    for i in 0..8 {
        bytes[i] = match endian {
            Endian::Big => (val >> (56 - i * 8)) as u8,
            Endian::Little => (val >> (i * 8)) as u8,
        };
        // Clear processed bits
        match endian {
            Endian::Big => val &= !(0xFF << ((7 - i) * 8)),
            Endian::Little => val &= !(0xFF << (i * 8)),
        }
    }
    bytes
}

fn shifted_bytes(mut val: u64, bits: u8, endian: Endian) -> Vec<u8> {
    let byte_count = (bits / 8) as usize;
    let mut bytes = vec![0u8; byte_count];

    #[allow(clippy::needless_range_loop)]
    for i in 0..byte_count {
        let shift = match endian {
            Endian::Big => (byte_count - 1 - i) * 8,
            Endian::Little => i * 8,
        };
        bytes[i] = (val >> shift) as u8;
        val &= !(0xFF << shift); // Clear processed bits
    }
    bytes
}

pub(crate) fn set_prototype<'js>(ctx: &Ctx<'js>, constructor: Object<'js>) -> Result<()> {
    let _ = &constructor.set("alloc", Func::from(alloc))?;
    let _ = &constructor.set("allocUnsafe", Func::from(alloc_unsafe))?;
    let _ = &constructor.set("allocUnsafeSlow", Func::from(alloc_unsafe_slow))?;
    let _ = &constructor.set("byteLength", Func::from(byte_length))?;
    let _ = &constructor.set("concat", Func::from(concat))?;
    let _ = &constructor.set(PredefinedAtom::From, Func::from(from))?;
    let _ = &constructor.set("isBuffer", Func::from(is_buffer))?;
    let _ = &constructor.set("isEncoding", Func::from(is_encoding))?;

    let prototype: &Object = &constructor.get(PredefinedAtom::Prototype)?;
    prototype.set("copy", Func::from(copy))?;
    prototype.set("equals", Func::from(equals))?;
    prototype.set("subarray", Func::from(subarray))?;
    prototype.set(PredefinedAtom::ToString, Func::from(to_string))?;
    prototype.set("write", Func::from(write))?;

    // Set all write and read methods
    for kind in NumberKind::iter() {
        for (endian, name, alias) in kind.prototype() {
            let write_func = Function::new(ctx.clone(), |t, c, v, o| {
                write_buf(&t, &c, &v, &o, *endian, *kind)
            })?;
            let read_func =
                Function::new(ctx.clone(), |t, c, o| read_buf(&t, &c, &o, *endian, *kind))?;
            if let Some(alias) = alias {
                prototype.set(["write", alias].concat(), write_func.clone())?;
                prototype.set(["read", alias].concat(), read_func.clone())?;
            }
            prototype.set(["write", name].concat(), write_func)?;
            prototype.set(["read", name].concat(), read_func)?;
        }
    }

    //not assessable from js
    prototype.prop(PredefinedAtom::Meta, stringify!(Buffer))?;

    ctx.globals().set(stringify!(Buffer), constructor)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use raster_runtime_test::{call_test, test_async_with, ModuleEvaluator};

    use crate::BufferModule;

    #[tokio::test]
    async fn test_subarray() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<BufferModule>(ctx.clone(), "buffer")
                    .await
                    .unwrap();

                let data = "hello world".to_string().into_bytes();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { Buffer } from 'buffer';

                        export async function test(data) {
                            let buffer = Buffer.from(data);
                            let sub = buffer.subarray(6, 11); // "world" part
                            return sub.toString();
                        }
                    "#,
                )
                .await
                .unwrap();
                let result = call_test::<String, _>(&ctx, &module, (data,)).await;
                assert_eq!(result, "world");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_subarray_partial() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<BufferModule>(ctx.clone(), "buffer")
                    .await
                    .unwrap();

                let data = "hello world".to_string().into_bytes();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { Buffer } from 'buffer';

                        export async function test(data) {
                            let buffer = Buffer.from(data);
                            let sub = buffer.subarray(0, 5); // "hello" part
                            return sub.toString();
                        }
                    "#,
                )
                .await
                .unwrap();
                let result = call_test::<String, _>(&ctx, &module, (data,)).await;
                assert_eq!(result, "hello");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_subarray_out_of_bounds() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<BufferModule>(ctx.clone(), "buffer")
                    .await
                    .unwrap();

                let data = "hello world".to_string().into_bytes();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { Buffer } from 'buffer';

                        export async function test(data) {
                            let buffer = Buffer.from(data);
                            let sub = buffer.subarray(6, 20); // "world" part but goes out of bounds
                            return sub.toString();
                        }
                    "#,
                )
                .await
                .unwrap();
                let result = call_test::<String, _>(&ctx, &module, (data,)).await;
                assert_eq!(result, "world");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_read_int_32_be() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<BufferModule>(ctx.clone(), "buffer")
                    .await
                    .unwrap();

                let data = "hello world".to_string().into_bytes();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { Buffer } from 'buffer';

                        export async function test(data) {
                            const buf = Buffer.from([1, 2, 3, 4, 0, 0, 0, 0]);
                            return buf.readInt32BE();
                        }
                    "#,
                )
                .await
                .unwrap();
                let result = call_test::<i32, _>(&ctx, &module, (data,)).await;
                assert_eq!(result, 0x01020304);
            })
        })
        .await;
    }
}
