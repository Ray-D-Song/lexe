// SPDX-License-Identifier: Apache-2.0
//! Native helpers behind `buffer.transcode`, `buffer.isUtf8` and `buffer.isAscii`.
//!
//! `transcode` mirrors the observable behaviour of Node's ICU based
//! implementation (`node_i18n.cc`), including its quirks: the `ascii` source is
//! not a 7-bit decoder when the target is `ucs2`, invalid UTF-8 is an error only
//! when the target is `ucs2`, and a lone surrogate is an error only when the
//! target is `utf8`.

use rquickjs::{prelude::Opt, Ctx, IntoJs, Value};

use crate::{ArrayBufferView, Buffer};

pub const U_ILLEGAL_ARGUMENT_ERROR: i32 = 1;
pub const U_INVALID_CHAR_FOUND: i32 = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Enc {
    Utf8,
    Ucs2,
    Latin1,
    Ascii,
}

impl Enc {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "utf8" => Some(Enc::Utf8),
            "utf16le" => Some(Enc::Ucs2),
            "latin1" => Some(Enc::Latin1),
            "ascii" => Some(Enc::Ascii),
            _ => None,
        }
    }
}

const REPLACEMENT: u32 = 0xFFFD;

fn is_surrogate(cp: u32) -> bool {
    (0xD800..=0xDFFF).contains(&cp)
}

fn decode_utf16le(src: &[u8]) -> Vec<u32> {
    let units: Vec<u16> = src
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let u = units[i] as u32;
        if (0xD800..0xDC00).contains(&u) && i + 1 < units.len() {
            let lo = units[i + 1] as u32;
            if (0xDC00..0xE000).contains(&lo) {
                out.push(0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00));
                i += 2;
                continue;
            }
        }
        out.push(u);
        i += 1;
    }
    out
}

pub fn transcode_bytes(src: &[u8], from: Enc, to: Enc) -> Result<Vec<u8>, i32> {
    let mut trailing_odd_byte = false;
    let cps: Vec<u32> = match from {
        Enc::Utf8 => match std::str::from_utf8(src) {
            Ok(s) => s.chars().map(|c| c as u32).collect(),
            Err(_) if to == Enc::Ucs2 => return Err(U_INVALID_CHAR_FOUND),
            Err(_) => String::from_utf8_lossy(src)
                .chars()
                .map(|c| c as u32)
                .collect(),
        },
        Enc::Ucs2 => {
            trailing_odd_byte = src.len() % 2 == 1;
            decode_utf16le(src)
        },
        Enc::Latin1 => src.iter().map(|&b| b as u32).collect(),
        Enc::Ascii => src
            .iter()
            .map(|&b| {
                if b < 0x80 {
                    b as u32
                } else if to == Enc::Ucs2 {
                    b as u32
                } else {
                    REPLACEMENT
                }
            })
            .collect(),
    };

    let mut out = Vec::with_capacity(cps.len());
    match to {
        Enc::Utf8 => {
            let mut tmp = [0u8; 4];
            for &cp in &cps {
                if is_surrogate(cp) {
                    return Err(U_INVALID_CHAR_FOUND);
                }
                let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
            }
        },
        Enc::Ucs2 => {
            let mut push = |cp: u32| {
                let ch = if is_surrogate(cp) {
                    '\u{FFFD}'
                } else {
                    char::from_u32(cp).unwrap_or('\u{FFFD}')
                };
                let mut units = [0u16; 2];
                for u in ch.encode_utf16(&mut units) {
                    out.extend_from_slice(&u.to_le_bytes());
                }
            };
            for &cp in &cps {
                push(cp);
            }
            if trailing_odd_byte && from == Enc::Ucs2 {
                push(REPLACEMENT);
            }
        },
        // ICU's single-byte converters drop U+FEFF instead of substituting it.
        Enc::Latin1 => out.extend(cps.iter().filter(|&&cp| cp != 0xFEFF).map(|&cp| {
            if cp <= 0xFF {
                cp as u8
            } else {
                b'?'
            }
        })),
        Enc::Ascii => out.extend(cps.iter().filter(|&&cp| cp != 0xFEFF).map(|&cp| {
            if cp <= 0x7F {
                cp as u8
            } else {
                b'?'
            }
        })),
    }
    Ok(out)
}

/// Returns a `Buffer` on success or the ICU error number as a JS number.
pub fn transcode_native<'js>(
    ctx: Ctx<'js>,
    source: ArrayBufferView<'js>,
    from: Opt<Value<'js>>,
    to: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let parse = |v: Opt<Value<'js>>| {
        v.0.and_then(|v| v.as_string().and_then(|s| s.to_string().ok()))
            .and_then(|s| Enc::parse(&s))
    };
    let (Some(from), Some(to)) = (parse(from), parse(to)) else {
        return U_ILLEGAL_ARGUMENT_ERROR.into_js(&ctx);
    };
    let bytes = source.as_bytes().unwrap_or(&[]);
    match transcode_bytes(bytes, from, to) {
        Ok(out) => Buffer(out).into_js(&ctx),
        Err(code) => code.into_js(&ctx),
    }
}

pub fn is_utf8_native(view: ArrayBufferView<'_>) -> bool {
    std::str::from_utf8(view.as_bytes().unwrap_or(&[])).is_ok()
}

pub fn is_ascii_native(view: ArrayBufferView<'_>) -> bool {
    view.as_bytes().unwrap_or(&[]).is_ascii()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(src: &[u8], from: Enc, to: Enc) -> Result<Vec<u8>, i32> {
        transcode_bytes(src, from, to)
    }

    #[test]
    fn utf8_invalid_is_replaced_except_for_ucs2() {
        let bad = [0x61, 0xff, 0x62];
        assert_eq!(
            t(&bad, Enc::Utf8, Enc::Utf8).unwrap(),
            "a\u{FFFD}b".as_bytes()
        );
        assert_eq!(t(&bad, Enc::Utf8, Enc::Latin1).unwrap(), b"a?b");
        assert_eq!(t(&bad, Enc::Utf8, Enc::Ucs2), Err(U_INVALID_CHAR_FOUND));
    }

    #[test]
    fn ucs2_lone_surrogate_errors_only_for_utf8() {
        let lone = [0x00, 0xd8, 0x41, 0x00];
        assert_eq!(t(&lone, Enc::Ucs2, Enc::Utf8), Err(U_INVALID_CHAR_FOUND));
        assert_eq!(
            t(&lone, Enc::Ucs2, Enc::Ucs2).unwrap(),
            [0xfd, 0xff, 0x41, 0x00]
        );
        assert_eq!(t(&lone, Enc::Ucs2, Enc::Ascii).unwrap(), b"?A");
    }

    #[test]
    fn ascii_source_depends_on_target() {
        let hi = [0x80, 0x41];
        assert_eq!(
            t(&hi, Enc::Ascii, Enc::Ucs2).unwrap(),
            [0x80, 0x00, 0x41, 0x00]
        );
        assert_eq!(
            t(&hi, Enc::Ascii, Enc::Utf8).unwrap(),
            "\u{FFFD}A".as_bytes()
        );
        assert_eq!(t(&hi, Enc::Ascii, Enc::Latin1).unwrap(), b"?A");
    }

    #[test]
    fn byte_order_mark_is_dropped_by_single_byte_targets() {
        let bom = "\u{FEFF}a".as_bytes();
        assert_eq!(t(bom, Enc::Utf8, Enc::Latin1).unwrap(), b"a");
        assert_eq!(t(bom, Enc::Utf8, Enc::Utf8).unwrap(), bom);
    }

    #[test]
    fn odd_ucs2_tail() {
        let odd = [0x61, 0x00, 0x62];
        assert_eq!(t(&odd, Enc::Ucs2, Enc::Utf8).unwrap(), b"a");
        assert_eq!(
            t(&odd, Enc::Ucs2, Enc::Ucs2).unwrap(),
            [0x61, 0x00, 0xfd, 0xff]
        );
    }
}
