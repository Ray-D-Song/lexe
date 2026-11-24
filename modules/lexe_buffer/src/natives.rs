//! Native helpers behind `methods.js`: the byte-level parts of Node's
//! `internalBinding('buffer')` (compare, indexOf, fill, slice, write, swap).
use std::slice;

use lexe_encoding::NodeEncoding;
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{prelude::Func, Ctx, Exception, Object, Result, Value};

use crate::codec::{decoded_to_js, string_units, throw_coded};

fn view<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<(*mut u8, usize)> {
    let bytes = ObjectBytes::from(ctx, value)?;
    let Some((array_buffer, len, offset)) = bytes.get_array_buffer()? else {
        return Err(Exception::throw_type(
            ctx,
            "argument must be an ArrayBufferView",
        ));
    };
    let raw = array_buffer
        .as_raw()
        .ok_or_else(|| Exception::throw_type(ctx, "ArrayBuffer is detached"))?;
    Ok((unsafe { raw.ptr.as_ptr().add(offset) }, len))
}

fn bytes<'a, 'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<&'a [u8]> {
    let (ptr, len) = view(ctx, value)?;
    Ok(unsafe { slice::from_raw_parts(ptr, len) })
}

fn bytes_mut<'a, 'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<&'a mut [u8]> {
    let (ptr, len) = view(ctx, value)?;
    Ok(unsafe { slice::from_raw_parts_mut(ptr, len) })
}

fn normalize_compare(cmp: std::cmp::Ordering, a_len: usize, b_len: usize) -> i32 {
    use std::cmp::Ordering::*;
    match cmp {
        Greater => 1,
        Less => -1,
        Equal => match a_len.cmp(&b_len) {
            Greater => 1,
            Less => -1,
            Equal => 0,
        },
    }
}

fn compare<'js>(ctx: Ctx<'js>, a: Value<'js>, b: Value<'js>) -> Result<i32> {
    let a = bytes(&ctx, &a)?;
    let b = bytes(&ctx, &b)?;
    let n = a.len().min(b.len());
    Ok(normalize_compare(a[..n].cmp(&b[..n]), a.len(), b.len()))
}

fn compare_offset<'js>(
    ctx: Ctx<'js>,
    source: Value<'js>,
    target: Value<'js>,
    target_start: f64,
    source_start: f64,
    target_end: f64,
    source_end: f64,
) -> Result<i32> {
    let source = bytes(&ctx, &source)?;
    let target = bytes(&ctx, &target)?;
    let (target_start, source_start) = (target_start as usize, source_start as usize);
    let (target_end, source_end) = (target_end as usize, source_end as usize);
    if source_start > source.len() {
        return Err(throw_coded(
            &ctx,
            true,
            "ERR_OUT_OF_RANGE",
            "The value of \"sourceStart\" is out of range.".to_string(),
        ));
    }
    if target_start > target.len() {
        return Err(throw_coded(
            &ctx,
            true,
            "ERR_OUT_OF_RANGE",
            "The value of \"targetStart\" is out of range.".to_string(),
        ));
    }
    let to_cmp = (source_end - source_start)
        .min(target_end - target_start)
        .min(source.len() - source_start);
    let ord = source[source_start..source_start + to_cmp]
        .cmp(&target[target_start..target_start + to_cmp]);
    Ok(normalize_compare(
        ord,
        source_end - source_start,
        target_end - target_start,
    ))
}

fn index_of_offset(length: usize, offset: i64, needle_length: usize, forward: bool) -> i64 {
    let length = length as i64;
    let needle_length = needle_length as i64;
    if offset < 0 {
        if offset + length >= 0 {
            length + offset
        } else if forward || needle_length == 0 {
            0
        } else {
            -1
        }
    } else if offset + needle_length <= length {
        offset
    } else if needle_length == 0 {
        length
    } else if forward {
        -1
    } else {
        length - 1
    }
}

/// `nbytes::SearchString`: first match at or after `start` going forward, last
/// match at or before `start` going backward; positions are in `unit`-byte steps.
fn search(hay: &[u8], needle: &[u8], start: usize, forward: bool, unit: usize) -> Option<usize> {
    let hay_units = hay.len() / unit;
    let needle_units = needle.len() / unit;
    if hay_units < needle_units {
        return None;
    }
    let diff = hay_units - needle_units;
    let matches = |i: usize| hay[i * unit..(i + needle_units) * unit] == *needle;
    if forward {
        (start..=diff).find(|&i| matches(i)).map(|i| i * unit)
    } else {
        (0..=start.min(diff))
            .rev()
            .find(|&i| matches(i))
            .map(|i| i * unit)
    }
}

/// Shared tail of `IndexOfString` / `IndexOfBuffer`.
fn index_of_bytes(
    hay: &[u8],
    needle: &[u8],
    offset: i64,
    forward: bool,
    ucs2: bool,
    hay_len: usize,
) -> f64 {
    let needle_len = needle.len();
    let opt_offset = index_of_offset(hay_len, offset, needle_len, forward);
    if needle_len == 0 {
        return opt_offset as f64;
    }
    if hay_len == 0 || opt_offset <= -1 {
        return -1.0;
    }
    let offset = opt_offset as usize;
    if (forward && needle_len + offset > hay_len) || needle_len > hay_len {
        return -1.0;
    }
    let unit = if ucs2 { 2 } else { 1 };
    if ucs2 && (hay.len() < 2 || needle_len < 2) {
        return -1.0;
    }
    let hay = &hay[..hay_len];
    let needle = &needle[..needle_len - needle_len % unit];
    match search(hay, needle, offset / unit, forward, unit) {
        Some(p) => p as f64,
        None => -1.0,
    }
}

fn index_of_string<'js>(
    ctx: Ctx<'js>,
    buf: Value<'js>,
    needle: rquickjs::String<'js>,
    offset: f64,
    encoding: String,
    forward: bool,
) -> Result<f64> {
    let hay = bytes(&ctx, &buf)?;
    let units = string_units(&needle)?;
    let enc = NodeEncoding::parse(&encoding).unwrap_or(NodeEncoding::Utf8);
    let ucs2 = enc == NodeEncoding::Utf16le;
    let needle_bytes = match enc {
        NodeEncoding::Latin1 | NodeEncoding::Ascii => enc.encode_units(&units),
        NodeEncoding::Utf16le => enc.encode_units(&units),
        _ => NodeEncoding::Utf8.encode_units(&units),
    };
    let hay_len = if ucs2 { hay.len() & !1 } else { hay.len() };
    Ok(index_of_bytes(
        hay,
        &needle_bytes,
        offset as i64,
        forward,
        ucs2,
        hay_len,
    ))
}

fn index_of_buffer<'js>(
    ctx: Ctx<'js>,
    buf: Value<'js>,
    needle: Value<'js>,
    offset: f64,
    encoding: String,
    forward: bool,
) -> Result<f64> {
    let hay = bytes(&ctx, &buf)?;
    let needle = bytes(&ctx, &needle)?;
    let ucs2 = NodeEncoding::parse(&encoding) == Some(NodeEncoding::Utf16le);
    Ok(index_of_bytes(
        hay,
        needle,
        offset as i64,
        forward,
        ucs2,
        hay.len(),
    ))
}

fn index_of_number<'js>(
    ctx: Ctx<'js>,
    buf: Value<'js>,
    needle: f64,
    offset: f64,
    forward: bool,
) -> Result<i32> {
    let hay = bytes(&ctx, &buf)?;
    let needle = needle as u32 as u8;
    let opt_offset = index_of_offset(hay.len(), offset as i64, 1, forward);
    if opt_offset <= -1 || hay.is_empty() {
        return Ok(-1);
    }
    let offset = opt_offset as usize;
    let found = if forward {
        hay[offset..]
            .iter()
            .position(|&b| b == needle)
            .map(|p| p + offset)
    } else {
        hay[..=offset].iter().rposition(|&b| b == needle)
    };
    Ok(found.map_or(-1, |p| p as i32))
}

/// `Buffer::Fill` for string and ArrayBufferView values; returns -1 when the
/// value yields no bytes and -2 when the range is out of bounds.
fn fill<'js>(
    ctx: Ctx<'js>,
    buf: Value<'js>,
    value: Value<'js>,
    start: f64,
    end: f64,
    encoding: Value<'js>,
) -> Result<i32> {
    let target = bytes_mut(&ctx, &buf)?;
    let (start, end) = (start as usize, end as usize);
    if start > end || end > target.len() {
        return Ok(-2);
    }
    let fill_length = end - start;

    let pattern: Vec<u8> = if let Some(js) = value.as_string() {
        let units = string_units(js)?;
        let enc = match encoding.as_string() {
            Some(e) => NodeEncoding::parse(&e.to_string()?).unwrap_or(NodeEncoding::Utf8),
            None => NodeEncoding::Utf8,
        };
        enc.encode_units(&units)
    } else {
        bytes(&ctx, &value)?.to_vec()
    };

    let copy = pattern.len().min(fill_length);
    target[start..start + copy].copy_from_slice(&pattern[..copy]);
    if pattern.len() >= fill_length {
        return Ok(0);
    }
    if pattern.is_empty() {
        return Ok(-1);
    }
    let mut filled = pattern.len();
    while filled < fill_length {
        let n = filled.min(fill_length - filled);
        target.copy_within(start..start + n, start + filled);
        filled += n;
    }
    Ok(0)
}

fn slice<'js>(
    ctx: Ctx<'js>,
    buf: Value<'js>,
    encoding: String,
    start: f64,
    end: f64,
) -> Result<Value<'js>> {
    let data = bytes(&ctx, &buf)?;
    let enc = NodeEncoding::parse(&encoding).unwrap_or(NodeEncoding::Utf8);
    decoded_to_js(&ctx, enc.decode_bytes(&data[start as usize..end as usize]))
}

fn write<'js>(
    ctx: Ctx<'js>,
    buf: Value<'js>,
    encoding: String,
    string: rquickjs::String<'js>,
    offset: f64,
    max_length: f64,
) -> Result<f64> {
    let target = bytes_mut(&ctx, &buf)?;
    let enc = NodeEncoding::parse(&encoding).unwrap_or(NodeEncoding::Utf8);
    let encoded = enc.encode_units(&string_units(&string)?);
    let written = enc.writable_len(&encoded, max_length as usize);
    let offset = offset as usize;
    target[offset..offset + written].copy_from_slice(&encoded[..written]);
    Ok(written as f64)
}

fn swap<'js>(ctx: Ctx<'js>, buf: Value<'js>, width: usize) -> Result<Value<'js>> {
    let data = bytes_mut(&ctx, &buf)?;
    for chunk in data.chunks_exact_mut(width) {
        chunk.reverse();
    }
    Ok(buf)
}

pub(crate) fn natives<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let natives = Object::new(ctx.clone())?;
    natives.set("compare", Func::from(compare))?;
    natives.set("compareOffset", Func::from(compare_offset))?;
    natives.set("indexOfString", Func::from(index_of_string))?;
    natives.set("indexOfBuffer", Func::from(index_of_buffer))?;
    natives.set("indexOfNumber", Func::from(index_of_number))?;
    natives.set("fill", Func::from(fill))?;
    natives.set("slice", Func::from(slice))?;
    natives.set("write", Func::from(write))?;
    natives.set("swap", Func::from(swap))?;
    Ok(natives)
}
