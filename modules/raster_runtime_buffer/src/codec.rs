use std::{mem::MaybeUninit, slice};

use raster_runtime_encoding::{DecodedText, NodeEncoding};
use raster_runtime_utils::primordials::{BasePrimordials, Primordial};
use rquickjs::{qjs, Coerced, Ctx, Error, Object, Result, Type, Value};

/// Reads a JS string as UTF-16 code units, keeping lone surrogates intact
/// (`rquickjs::String::to_string` fails on them).
pub(crate) fn string_units(js: &rquickjs::String<'_>) -> Result<Vec<u16>> {
    let ctx = js.ctx();
    let mut len = MaybeUninit::uninit();
    let ptr = unsafe {
        qjs::JS_ToCStringLen2(ctx.as_raw().as_ptr(), len.as_mut_ptr(), js.as_raw(), true)
    };
    if ptr.is_null() {
        return Err(Error::Unknown);
    }
    let len = unsafe { len.assume_init() } as usize;
    let bytes: &[u8] = unsafe { slice::from_raw_parts(ptr as *const u8, len) };
    let units = cesu8_to_units(bytes);
    unsafe { qjs::JS_FreeCString(ctx.as_raw().as_ptr(), ptr) };
    Ok(units)
}

fn cesu8_to_units(bytes: &[u8]) -> Vec<u16> {
    if bytes.is_ascii() {
        return bytes.iter().map(|&b| b as u16).collect();
    }
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let get = |n: usize| bytes.get(i + n).map_or(0, |b| (b & 0x3f) as u32);
        match b0 {
            0x00..=0x7f => {
                units.push(b0 as u16);
                i += 1;
            },
            0xc0..=0xdf => {
                units.push((((b0 & 0x1f) as u32) << 6 | get(1)) as u16);
                i += 2;
            },
            0xe0..=0xef => {
                units.push((((b0 & 0x0f) as u32) << 12 | get(1) << 6 | get(2)) as u16);
                i += 3;
            },
            _ => {
                let cp = ((b0 & 0x07) as u32) << 18 | get(1) << 12 | get(2) << 6 | get(3);
                let cp = cp - 0x10000;
                units.push(0xd800 | (cp >> 10) as u16);
                units.push(0xdc00 | (cp & 0x3ff) as u16);
                i += 4;
            },
        }
    }
    units
}

pub(crate) fn units_to_js<'js>(ctx: &Ctx<'js>, units: &[u16]) -> Result<Value<'js>> {
    let raw =
        unsafe { qjs::JS_NewStringUTF16(ctx.as_raw().as_ptr(), units.as_ptr(), units.len() as _) };
    let value = unsafe { Value::from_raw(ctx.clone(), raw) };
    if value.is_exception() {
        return Err(Error::Exception);
    }
    Ok(value)
}

pub(crate) fn decoded_to_js<'js>(ctx: &Ctx<'js>, text: DecodedText) -> Result<Value<'js>> {
    match text {
        DecodedText::Str(s) => rquickjs::String::from_str(ctx.clone(), &s).map(|s| s.into_value()),
        DecodedText::Units(u) => units_to_js(ctx, &u),
    }
}

pub(crate) fn throw_coded(ctx: &Ctx<'_>, range: bool, code: &str, message: String) -> Error {
    let primordials = BasePrimordials::get(ctx).expect("primordials");
    let constructor = if range {
        &primordials.constructor_range_error
    } else {
        &primordials.constructor_type_error
    };
    let err: Object = constructor.construct((message,)).expect("error");
    err.set("code", code).expect("code");
    ctx.throw(err.into_value())
}

pub(crate) fn unknown_encoding(ctx: &Ctx<'_>, label: &str) -> Error {
    throw_coded(
        ctx,
        false,
        "ERR_UNKNOWN_ENCODING",
        format!("Unknown encoding: {label}"),
    )
}

/// `getEncodingOps` of lib/buffer.js: the value is stringified, then looked up.
pub(crate) fn resolve_encoding(ctx: &Ctx<'_>, value: &Value<'_>) -> Result<NodeEncoding> {
    let label = match value.as_string() {
        Some(s) => s.to_string()?,
        None => value
            .get::<Coerced<String>>()
            .map(|c| c.0)
            .unwrap_or_default(),
    };
    NodeEncoding::parse(&label).ok_or_else(|| unknown_encoding(ctx, &label))
}

pub(crate) fn is_falsy(value: &Value<'_>) -> bool {
    match value.type_of() {
        Type::Undefined | Type::Null | Type::Uninitialized => true,
        Type::Bool => value.as_bool() == Some(false),
        Type::Int => value.as_int() == Some(0),
        Type::Float => value.as_float().is_some_and(|f| f == 0.0 || f.is_nan()),
        Type::String => value
            .as_string()
            .is_some_and(|s| s.to_string().is_ok_and(|s| s.is_empty())),
        _ => false,
    }
}

fn numeric_separators(value: f64) -> String {
    let digits = format!("{}", value.abs() as u64);
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('_');
        }
        out.push(c);
    }
    if value < 0.0 {
        out.insert(0, '-');
    }
    out
}

fn describe_received(value: &Value<'_>) -> String {
    match value.type_of() {
        Type::Undefined => "undefined".into(),
        Type::Null => "null".into(),
        Type::Bool => format!("type boolean ({})", value.as_bool().unwrap_or(false)),
        Type::Object | Type::Array => "an instance of Object".into(),
        Type::Function | Type::Constructor => "function".into(),
        Type::Symbol => "type symbol".into(),
        _ => "type unknown".into(),
    }
}

/// `validateOffset` of lib/buffer.js (integer between `min` and `max`).
pub(crate) fn validate_offset(
    ctx: &Ctx<'_>,
    value: &Value<'_>,
    name: &str,
    min: f64,
    max: f64,
) -> Result<f64> {
    let Some(n) = value.as_number() else {
        return Err(throw_coded(
            ctx,
            false,
            "ERR_INVALID_ARG_TYPE",
            format!(
                "The \"{name}\" argument must be of type number. Received {}",
                describe_received(value)
            ),
        ));
    };
    let received = if n.fract() == 0.0 && n.abs() > 4_294_967_296.0 {
        numeric_separators(n)
    } else if n.is_finite() && n.fract() == 0.0 {
        format!("{}", n as i64)
    } else if n.is_nan() {
        "NaN".to_string()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else {
        format!("{n}")
    };
    if !n.is_finite() || n.fract() != 0.0 {
        return Err(throw_coded(
            ctx,
            true,
            "ERR_OUT_OF_RANGE",
            format!("The value of \"{name}\" is out of range. It must be an integer. Received {received}"),
        ));
    }
    if n < min || n > max {
        return Err(throw_coded(
            ctx,
            true,
            "ERR_OUT_OF_RANGE",
            format!(
                "The value of \"{name}\" is out of range. It must be >= {min} && <= {max}. Received {received}"
            ),
        ));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cesu8_decoding_keeps_lone_surrogates() {
        // "a", U+00E9, U+20AC, lone high surrogate D800, then a pair as two 3-byte units
        let bytes = [
            0x61, 0xc3, 0xa9, 0xe2, 0x82, 0xac, 0xed, 0xa0, 0x80, 0xed, 0xa0, 0xbd, 0xed, 0xb8,
            0x80,
        ];
        assert_eq!(
            cesu8_to_units(&bytes),
            [0x61, 0xe9, 0x20ac, 0xd800, 0xd83d, 0xde00]
        );
    }

    #[test]
    fn numeric_separators_match_node() {
        assert_eq!(numeric_separators(4_294_967_297.0), "4_294_967_297");
        assert_eq!(numeric_separators(-12_345_678_901.0), "-12_345_678_901");
    }
}
