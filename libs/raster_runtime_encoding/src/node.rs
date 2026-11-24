//! Codecs with the exact semantics of Node's `Buffer` encodings.
//!
//! Strings are handled as UTF-16 code units because JS strings may contain
//! lone surrogates, which `latin1`, `ascii` and `utf16le` must see unmodified.
use std::borrow::Cow;

use crate::{bytes_to_b64_string, bytes_to_b64_url_safe_string, bytes_to_hex_string};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeEncoding {
    Utf8,
    Utf16le,
    Latin1,
    Ascii,
    Base64,
    Base64Url,
    Hex,
}

pub enum DecodedText {
    Str(String),
    Units(Vec<u16>),
}

impl NodeEncoding {
    /// Resolves an encoding name the way `Buffer.isEncoding` does: exact
    /// match or ASCII case-insensitive match, with no trimming.
    pub fn parse(label: &str) -> Option<Self> {
        let lower: Cow<str> = if label.bytes().any(|b| b.is_ascii_uppercase()) {
            Cow::Owned(label.to_ascii_lowercase())
        } else {
            Cow::Borrowed(label)
        };
        match lower.as_ref() {
            "utf8" | "utf-8" => Some(Self::Utf8),
            "ucs2" | "ucs-2" | "utf16le" | "utf-16le" => Some(Self::Utf16le),
            "latin1" | "binary" => Some(Self::Latin1),
            "ascii" => Some(Self::Ascii),
            "base64" => Some(Self::Base64),
            "base64url" => Some(Self::Base64Url),
            "hex" => Some(Self::Hex),
            _ => None,
        }
    }

    /// Bytes to string (`buf.toString(encoding)`).
    pub fn decode_bytes(self, bytes: &[u8]) -> DecodedText {
        match self {
            Self::Utf8 => DecodedText::Str(String::from_utf8_lossy(bytes).into_owned()),
            Self::Utf16le => DecodedText::Units(
                bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect(),
            ),
            Self::Latin1 => DecodedText::Str(bytes.iter().map(|&b| b as char).collect()),
            Self::Ascii => DecodedText::Str(bytes.iter().map(|&b| (b & 0x7f) as char).collect()),
            Self::Base64 => DecodedText::Str(bytes_to_b64_string(bytes)),
            Self::Base64Url => DecodedText::Str(bytes_to_b64_url_safe_string(bytes)),
            Self::Hex => DecodedText::Str(bytes_to_hex_string(bytes)),
        }
    }

    /// String to bytes (`Buffer.from(string, encoding)`).
    pub fn encode_units(self, units: &[u16]) -> Vec<u8> {
        match self {
            Self::Utf8 => String::from_utf16_lossy(units).into_bytes(),
            Self::Utf16le => units.iter().flat_map(|u| u.to_le_bytes()).collect(),
            Self::Latin1 | Self::Ascii => units.iter().map(|&u| u as u8).collect(),
            Self::Hex => hex_decode(units),
            Self::Base64 | Self::Base64Url => base64_decode(units),
        }
    }

    /// `Buffer.byteLength(string, encoding)`, computed without decoding.
    pub fn byte_length(self, units: &[u16]) -> usize {
        let len = units.len();
        if len == 0 {
            return 0;
        }
        match self {
            Self::Utf8 => utf8_length(units),
            Self::Utf16le => len * 2,
            Self::Latin1 | Self::Ascii => len,
            Self::Hex => len >> 1,
            Self::Base64 | Self::Base64Url => {
                let mut n = len;
                if units[n - 1] == b'=' as u16 {
                    n -= 1;
                }
                if n > 1 && units[n - 1] == b'=' as u16 {
                    n -= 1;
                }
                (n * 3) >> 2
            },
        }
    }

    /// How many of `encoded` bytes `buf.write` stores when `capacity` bytes
    /// are available: utf8 never splits a character, utf16le never a unit.
    pub fn writable_len(self, encoded: &[u8], capacity: usize) -> usize {
        if encoded.len() <= capacity {
            return encoded.len();
        }
        match self {
            Self::Utf8 => {
                let mut end = capacity;
                while end > 0 && (encoded[end] & 0xC0) == 0x80 {
                    end -= 1;
                }
                end
            },
            Self::Utf16le => capacity & !1,
            _ => capacity,
        }
    }
}

fn utf8_length(units: &[u16]) -> usize {
    let mut total = 0;
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        total += match u {
            0..=0x7f => 1,
            0x80..=0x7ff => 2,
            0xD800..=0xDBFF if matches!(units.get(i + 1), Some(0xDC00..=0xDFFF)) => {
                i += 1;
                4
            },
            _ => 3,
        };
        i += 1;
    }
    total
}

fn hex_value(unit: u16) -> Option<u8> {
    match unit {
        0x30..=0x39 => Some((unit - 0x30) as u8),
        0x41..=0x46 => Some((unit - 0x41 + 10) as u8),
        0x61..=0x66 => Some((unit - 0x61 + 10) as u8),
        _ => None,
    }
}

fn hex_decode(units: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(units.len() / 2);
    for pair in units.chunks_exact(2) {
        match (hex_value(pair[0]), hex_value(pair[1])) {
            (Some(hi), Some(lo)) => out.push(hi << 4 | lo),
            _ => break,
        }
    }
    out
}

fn base64_value(unit: u16) -> Option<u8> {
    match unit {
        0x41..=0x5a => Some((unit - 0x41) as u8),
        0x61..=0x7a => Some((unit - 0x61 + 26) as u8),
        0x30..=0x39 => Some((unit - 0x30 + 52) as u8),
        0x2b | 0x2d => Some(62),
        0x2f | 0x5f => Some(63),
        _ => None,
    }
}

/// Accepts both alphabets, skips characters outside them and stops at `=`.
fn base64_decode(units: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(units.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &unit in units {
        if unit == b'=' as u16 {
            break;
        }
        let Some(v) = base64_value(unit) else {
            continue;
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn latin1_maps_every_byte_to_the_same_code_point() {
        let all: Vec<u8> = (0..=255).collect();
        let DecodedText::Str(s) = NodeEncoding::Latin1.decode_bytes(&all) else {
            panic!("latin1 must decode to a string");
        };
        let cps: Vec<u32> = s.chars().map(|c| c as u32).collect();
        assert_eq!(cps, (0..=255).collect::<Vec<u32>>());
    }

    #[test]
    fn latin1_encode_keeps_low_byte() {
        let bytes = NodeEncoding::Latin1.encode_units(&[0xe9, 0x100, 0x141, 0xd83d, 0xde00]);
        assert_eq!(bytes, [0xe9, 0x00, 0x41, 0x3d, 0x00]);
    }

    #[test]
    fn ascii_masks_high_bit() {
        let DecodedText::Str(s) = NodeEncoding::Ascii.decode_bytes(&[0xc8, 0x41]) else {
            panic!()
        };
        assert_eq!(s, "HA");
    }

    #[test]
    fn utf16le_keeps_lone_surrogates_and_drops_odd_byte() {
        let DecodedText::Units(u) = NodeEncoding::Utf16le.decode_bytes(&[0x00, 0xd8, 0x41]) else {
            panic!()
        };
        assert_eq!(u, [0xd800]);
    }

    #[test]
    fn hex_decode_stops_at_first_bad_pair() {
        assert_eq!(hex_decode(&units("0aFFg1")), [0x0a, 0xff]);
        assert_eq!(hex_decode(&units("abc")), [0xab]);
        assert!(hex_decode(&units("0g")).is_empty());
    }

    #[test]
    fn base64_decode_matches_node() {
        assert_eq!(base64_decode(&units("AQID/+-_=")), [1, 2, 3, 255, 239, 191]);
        assert_eq!(base64_decode(&units("A Q\nI D")), [1, 2, 3]);
        assert_eq!(base64_decode(&units("AQ==AQ==")), [1]);
        assert!(base64_decode(&units("=YQ")).is_empty());
        assert!(base64_decode(&units("A")).is_empty());
    }

    #[test]
    fn byte_lengths() {
        assert_eq!(NodeEncoding::Utf8.byte_length(&[0xd800]), 3);
        assert_eq!(
            NodeEncoding::Utf8.byte_length(&units("h\u{e9}\u{20ac}\u{1f600}x")),
            11
        );
        assert_eq!(NodeEncoding::Base64.byte_length(&units("====")), 1);
        assert_eq!(NodeEncoding::Hex.byte_length(&units("abc")), 1);
    }

    #[test]
    fn writable_len_respects_character_boundaries() {
        let euro = "\u{20ac}\u{20ac}\u{20ac}".as_bytes();
        assert_eq!(NodeEncoding::Utf8.writable_len(euro, 5), 3);
        assert_eq!(NodeEncoding::Utf16le.writable_len(&[0; 8], 5), 4);
        assert_eq!(NodeEncoding::Latin1.writable_len(&[0; 8], 5), 5);
    }
}
