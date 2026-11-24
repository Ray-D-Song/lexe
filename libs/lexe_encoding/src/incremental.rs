// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{bytes_to_utf8_string, Encoder, Endian};

const REPLACEMENT: char = '\u{FFFD}';

#[derive(Debug, Default)]
pub struct IncrementalDecoder {
    pending: Vec<u8>,
    bom_resolved: bool,
    pending_bom: Option<Vec<u8>>,
    utf16_odd_byte: Option<u8>,
    utf16_lead_surrogate: Option<u16>,
}

impl IncrementalDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.bom_resolved = false;
        self.pending_bom = None;
        self.utf16_odd_byte = None;
        self.utf16_lead_surrogate = None;
    }

    pub fn decode_chunk(
        &mut self,
        encoder: &Encoder,
        input: &[u8],
        lossy: bool,
        stream: bool,
        ignore_bom: bool,
    ) -> Result<String, String> {
        let output = self.decode_bytes(encoder, input, lossy, stream, ignore_bom, false)?;
        if !stream {
            self.reset();
        }
        Ok(output)
    }

    pub fn flush(
        &mut self,
        encoder: &Encoder,
        lossy: bool,
        ignore_bom: bool,
    ) -> Result<String, String> {
        let mut output = String::new();

        if !ignore_bom && !self.bom_resolved {
            if let Some(pending) = self.pending_bom.take() {
                self.bom_resolved = true;
                output.push_str(&self.decode_bytes(encoder, &pending, lossy, false, true, true)?);
            }
        }

        output.push_str(&self.decode_bytes(encoder, &[], lossy, true, ignore_bom, true)?);

        match encoder {
            Encoder::Utf8 | Encoder::Windows1252 => {
                if !self.pending.is_empty() {
                    if lossy {
                        output.push(REPLACEMENT);
                    } else {
                        return Err("Incomplete UTF-8 sequence".to_string());
                    }
                    self.pending.clear();
                }
            },
            Encoder::Utf16le | Encoder::Utf16be => {
                if self.utf16_lead_surrogate.is_some() && self.utf16_odd_byte.is_some() {
                    if lossy {
                        output.push(REPLACEMENT);
                    } else {
                        return Err("Incomplete UTF-16 sequence".to_string());
                    }
                    self.utf16_lead_surrogate = None;
                    self.utf16_odd_byte = None;
                } else {
                    if self.utf16_odd_byte.is_some() {
                        if lossy {
                            output.push(REPLACEMENT);
                        } else {
                            return Err("Incomplete UTF-16 sequence".to_string());
                        }
                        self.utf16_odd_byte = None;
                    }
                    if self.utf16_lead_surrogate.is_some() {
                        if lossy {
                            output.push(REPLACEMENT);
                        } else {
                            return Err("Incomplete UTF-16 sequence".to_string());
                        }
                        self.utf16_lead_surrogate = None;
                    }
                }
            },
            Encoder::Hex | Encoder::Base64 => {
                if !self.pending.is_empty() {
                    output.push_str(&encoder.encode_to_string(&self.pending, lossy)?);
                    self.pending.clear();
                }
            },
        }

        self.reset();
        Ok(output)
    }

    fn decode_bytes(
        &mut self,
        encoder: &Encoder,
        input: &[u8],
        lossy: bool,
        stream: bool,
        ignore_bom: bool,
        bom_finalize: bool,
    ) -> Result<String, String> {
        let mut combined = Vec::with_capacity(self.pending.len() + input.len());
        combined.extend_from_slice(&self.pending);
        combined.extend_from_slice(input);
        self.pending.clear();

        let bytes = if self.bom_resolved || ignore_bom {
            combined
        } else {
            process_bom(
                encoder,
                combined,
                stream,
                bom_finalize,
                &mut self.bom_resolved,
                &mut self.pending_bom,
            )
        };

        match encoder {
            Encoder::Utf8 | Encoder::Windows1252 => {
                decode_utf8_like(&bytes, lossy, stream, &mut self.pending)
            },
            Encoder::Utf16le => decode_utf16_stream(
                &bytes,
                Endian::Little,
                lossy,
                stream,
                &mut self.utf16_odd_byte,
                &mut self.utf16_lead_surrogate,
            ),
            Encoder::Utf16be => decode_utf16_stream(
                &bytes,
                Endian::Big,
                lossy,
                stream,
                &mut self.utf16_odd_byte,
                &mut self.utf16_lead_surrogate,
            ),
            Encoder::Hex | Encoder::Base64 => {
                if stream {
                    self.pending = bytes;
                    Ok(String::new())
                } else {
                    encoder.encode_to_string(&bytes, lossy)
                }
            },
        }
    }
}

fn bom_prefix(encoder: &Encoder) -> &'static [u8] {
    match encoder {
        Encoder::Utf8 => &[0xEF, 0xBB, 0xBF],
        Encoder::Utf16le => &[0xFF, 0xFE],
        Encoder::Utf16be => &[0xFE, 0xFF],
        _ => &[],
    }
}

fn process_bom(
    encoder: &Encoder,
    bytes: Vec<u8>,
    stream: bool,
    bom_finalize: bool,
    bom_resolved: &mut bool,
    pending_bom: &mut Option<Vec<u8>>,
) -> Vec<u8> {
    let prefix = bom_prefix(encoder);
    if prefix.is_empty() {
        *bom_resolved = true;
        return bytes;
    }

    let data = if let Some(pending) = pending_bom.take() {
        let mut combined = pending;
        combined.extend(bytes);
        combined
    } else {
        bytes
    };

    if data.starts_with(prefix) {
        *bom_resolved = true;
        return data[prefix.len()..].to_vec();
    }

    if stream && !bom_finalize {
        let shared = data
            .iter()
            .zip(prefix.iter())
            .take_while(|(left, right)| left == right)
            .count();
        if shared == data.len() && shared < prefix.len() {
            *pending_bom = Some(data);
            return Vec::new();
        }
    }

    *bom_resolved = true;
    data
}

fn decode_utf8_like(
    bytes: &[u8],
    lossy: bool,
    stream: bool,
    pending: &mut Vec<u8>,
) -> Result<String, String> {
    if !stream {
        return bytes_to_utf8_string(bytes, lossy);
    }

    let mut output = String::new();
    let mut pos = 0;

    while pos < bytes.len() {
        match std::str::from_utf8(&bytes[pos..]) {
            Ok(valid) => {
                output.push_str(valid);
                break;
            },
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    output.push_str(unsafe {
                        std::str::from_utf8_unchecked(&bytes[pos..pos + valid_up_to])
                    });
                    pos += valid_up_to;
                }

                match error.error_len() {
                    Some(invalid_len) => {
                        if !lossy {
                            return Err(error.to_string());
                        }
                        output.push(REPLACEMENT);
                        pos += invalid_len;
                    },
                    None => {
                        pending.extend_from_slice(&bytes[pos..]);
                        break;
                    },
                }
            },
        }
    }

    Ok(output)
}

fn read_u16(bytes: &[u8], index: usize, endian: Endian) -> u16 {
    match endian {
        Endian::Little => u16::from_le_bytes([bytes[index], bytes[index + 1]]),
        Endian::Big => u16::from_be_bytes([bytes[index], bytes[index + 1]]),
    }
}

fn decode_utf16_stream(
    bytes: &[u8],
    endian: Endian,
    lossy: bool,
    stream: bool,
    odd_byte: &mut Option<u8>,
    lead_surrogate: &mut Option<u16>,
) -> Result<String, String> {
    let mut input = Vec::new();
    if let Some(byte) = odd_byte.take() {
        input.push(byte);
    }
    input.extend_from_slice(bytes);

    let mut code_units = Vec::new();
    let mut index = 0usize;

    while index < input.len() {
        if index + 1 == input.len() {
            if stream {
                *odd_byte = Some(input[index]);
            } else if lossy {
                code_units.push(REPLACEMENT as u16);
            } else {
                return Err("Incomplete UTF-16 sequence".to_string());
            }
            break;
        }

        let mut unit = read_u16(&input, index, endian);
        index += 2;

        if let Some(lead) = lead_surrogate.take() {
            if (0xDC00..=0xDFFF).contains(&unit) {
                code_units.push(lead);
                code_units.push(unit);
                continue;
            }
            if !lossy {
                return Err("Invalid UTF-16 sequence".to_string());
            }
            code_units.push(REPLACEMENT as u16);
        }

        if (0xD800..=0xDBFF).contains(&unit) {
            if index + 1 < input.len() {
                let next = read_u16(&input, index, endian);
                if (0xDC00..=0xDFFF).contains(&next) {
                    code_units.push(unit);
                    code_units.push(next);
                    index += 2;
                    continue;
                }
                if !lossy {
                    return Err("Invalid UTF-16 sequence".to_string());
                }
                code_units.push(REPLACEMENT as u16);
                unit = next;
                index += 2;
            } else if index < input.len() {
                if stream {
                    *lead_surrogate = Some(unit);
                    *odd_byte = Some(input[index]);
                    break;
                }
                if !lossy {
                    return Err("Invalid UTF-16 sequence".to_string());
                }
                code_units.push(REPLACEMENT as u16);
            } else if stream {
                *lead_surrogate = Some(unit);
                break;
            } else if !lossy {
                return Err("Invalid UTF-16 sequence".to_string());
            } else {
                code_units.push(REPLACEMENT as u16);
                continue;
            }
        }

        if (0xDC00..=0xDFFF).contains(&unit) {
            if !lossy {
                return Err("Invalid UTF-16 sequence".to_string());
            }
            code_units.push(REPLACEMENT as u16);
            continue;
        }

        code_units.push(unit);
    }

    if lossy {
        Ok(String::from_utf16_lossy(&code_units))
    } else {
        String::from_utf16(&code_units).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_stream_splits_multibyte_character() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf8;
        let char_bytes = "é".as_bytes();
        let first = decoder
            .decode_chunk(&encoder, &char_bytes[..1], true, true, true)
            .unwrap();
        assert!(first.is_empty());
        let second = decoder
            .decode_chunk(&encoder, &char_bytes[1..], true, false, true)
            .unwrap();
        assert_eq!(format!("{first}{second}"), "é");
    }

    #[test]
    fn utf8_fatal_errors_on_invalid_sequence() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf8;
        assert!(decoder
            .decode_chunk(&encoder, &[0xFF], false, false, true)
            .is_err());
    }

    #[test]
    fn utf8_flush_replaces_incomplete_sequence_when_lossy() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf8;
        let _ = decoder
            .decode_chunk(&encoder, &[0xE2, 0x82], true, true, true)
            .unwrap();
        let flushed = decoder.flush(&encoder, true, true).unwrap();
        assert_eq!(flushed, "\u{FFFD}");
    }

    #[test]
    fn utf8_bom_is_stripped_across_streaming_chunks() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf8;
        let first = decoder
            .decode_chunk(&encoder, &[0xEF], true, true, false)
            .unwrap();
        assert!(first.is_empty());
        let second = decoder
            .decode_chunk(&encoder, &[0xBB, 0xBF, b'a'], true, false, false)
            .unwrap();
        assert_eq!(second, "a");
    }

    #[test]
    fn utf8_flush_replaces_incomplete_bom_prefix_when_lossy() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf8;
        let _ = decoder
            .decode_chunk(&encoder, &[0xEF], true, true, false)
            .unwrap();
        assert_eq!(decoder.flush(&encoder, true, false).unwrap(), "\u{FFFD}");

        let mut decoder = IncrementalDecoder::new();
        let _ = decoder
            .decode_chunk(&encoder, &[0xEF, 0xBB], true, true, false)
            .unwrap();
        assert_eq!(decoder.flush(&encoder, true, false).unwrap(), "\u{FFFD}");
    }

    #[test]
    fn utf8_flush_errors_on_incomplete_bom_prefix_when_fatal() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf8;
        let _ = decoder
            .decode_chunk(&encoder, &[0xEF], false, true, false)
            .unwrap();
        assert!(decoder.flush(&encoder, false, false).is_err());
    }

    #[test]
    fn utf16le_flush_replaces_incomplete_bom_prefix_when_lossy() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16le;
        let _ = decoder
            .decode_chunk(&encoder, &[0xFF], true, true, false)
            .unwrap();
        assert_eq!(decoder.flush(&encoder, true, false).unwrap(), "\u{FFFD}");
    }

    #[test]
    fn utf16be_flush_replaces_incomplete_bom_prefix_when_lossy() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16be;
        let _ = decoder
            .decode_chunk(&encoder, &[0xFE], true, true, false)
            .unwrap();
        assert_eq!(decoder.flush(&encoder, true, false).unwrap(), "\u{FFFD}");
    }

    #[test]
    fn utf16_flush_errors_on_incomplete_bom_prefix_when_fatal() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16le;
        let _ = decoder
            .decode_chunk(&encoder, &[0xFF], false, true, false)
            .unwrap();
        assert!(decoder.flush(&encoder, false, false).is_err());
    }

    #[test]
    fn utf16_stream_flush_with_empty_input_does_not_panic() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16le;
        let _ = decoder
            .decode_chunk(&encoder, &[0x41, 0x00], true, true, true)
            .unwrap();
        let flushed = decoder.flush(&encoder, true, true).unwrap();
        assert!(flushed.is_empty());
    }

    #[test]
    fn utf16_stream_replaces_orphaned_lead_surrogate_on_flush() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16le;
        let lead = 0xD800u16.to_le_bytes();
        let _ = decoder
            .decode_chunk(&encoder, &lead, true, true, true)
            .unwrap();
        let flushed = decoder.flush(&encoder, true, true).unwrap();
        assert_eq!(flushed, "\u{FFFD}");
    }

    #[test]
    fn utf16_stream_replaces_lead_followed_by_non_trail() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16le;
        let bytes = [0x00, 0xD8, 0x41, 0x00];
        let output = decoder
            .decode_chunk(&encoder, &bytes, true, false, true)
            .unwrap();
        assert_eq!(output, "\u{FFFD}A");
    }

    #[test]
    fn utf16le_stream_splits_lead_surrogate_and_odd_byte_across_chunks() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16le;
        let first = decoder
            .decode_chunk(&encoder, &[0x00, 0xD8, 0x41], true, true, true)
            .unwrap();
        assert!(first.is_empty());
        let second = decoder
            .decode_chunk(&encoder, &[0x00], true, false, true)
            .unwrap();
        assert_eq!(second, "\u{FFFD}A");
    }

    #[test]
    fn utf16le_flush_replaces_combined_lead_surrogate_and_odd_byte() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16le;
        let _ = decoder
            .decode_chunk(&encoder, &[0x00, 0xD8, 0x41], true, true, true)
            .unwrap();
        assert_eq!(decoder.flush(&encoder, true, true).unwrap(), "\u{FFFD}");
    }

    #[test]
    fn utf16be_flush_replaces_combined_lead_surrogate_and_odd_byte() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16be;
        let _ = decoder
            .decode_chunk(&encoder, &[0xD8, 0x00, 0x00], true, true, true)
            .unwrap();
        assert_eq!(decoder.flush(&encoder, true, true).unwrap(), "\u{FFFD}");
    }

    #[test]
    fn utf16be_stream_splits_lead_surrogate_and_odd_byte_across_chunks() {
        let mut decoder = IncrementalDecoder::new();
        let encoder = Encoder::Utf16be;
        let first = decoder
            .decode_chunk(&encoder, &[0xD8, 0x00, 0x00], true, true, true)
            .unwrap();
        assert!(first.is_empty());
        let second = decoder
            .decode_chunk(&encoder, &[0x41], true, false, true)
            .unwrap();
        assert_eq!(second, "\u{FFFD}A");
    }
}
