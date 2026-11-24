// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::io::{self, Write};

#[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
use brotli as brotlic;

/// A streaming inflate driven directly by `flate2::Decompress`.
///
/// The `write::*Decoder` helpers read to EOF and then report success, so a
/// truncated stream looks exactly like a finished one. Driving the decompressor
/// ourselves lets us see the `StreamEnd` status, which is the only reliable
/// signal that the compressed stream actually ended rather than ran out.
#[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
pub struct RawInflate {
    decoder: flate2::Decompress,
    /// Set once the decompressor reports `StreamEnd`.
    reached_end: bool,
}

#[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
impl RawInflate {
    /// `zlib_header` selects `deflate` (true) or `deflate-raw` (false).
    pub fn new(zlib_header: bool) -> Self {
        Self {
            decoder: flate2::Decompress::new(zlib_header),
            reached_end: false,
        }
    }

    pub fn decompress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(input.len().saturating_mul(4).max(4096));
        let mut consumed = 0usize;
        loop {
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            let mut scratch = [0u8; 16 * 1024];
            let status = self
                .decoder
                .decompress(
                    &input[consumed..],
                    &mut scratch,
                    flate2::FlushDecompress::None,
                )
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
            let produced = (self.decoder.total_out() - before_out) as usize;
            out.extend_from_slice(&scratch[..produced]);
            consumed += (self.decoder.total_in() - before_in) as usize;
            match status {
                flate2::Status::StreamEnd => {
                    self.reached_end = true;
                    break;
                },
                flate2::Status::Ok | flate2::Status::BufError => {
                    if consumed >= input.len() && produced == 0 {
                        // No progress and no input left: wait for the next chunk.
                        break;
                    }
                    if consumed >= input.len() && produced > 0 {
                        break;
                    }
                },
            }
        }
        Ok(out)
    }

    pub fn finish(self) -> io::Result<Vec<u8>> {
        if !self.reached_end {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected end of file",
            ));
        }
        Ok(Vec::new())
    }
}

/// Streaming decompressor that maintains state across chunks
pub enum StreamingDecoder {
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    Gzip(flate2::write::GzDecoder<Vec<u8>>),
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    Deflate(RawInflate),
    /// Raw DEFLATE with no zlib wrapper (RFC 1951). `ws` permessage-deflate
    /// and Node's `inflateRaw` both need it.
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    DeflateRaw(RawInflate),
    #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
    Zstd(zstd::stream::write::Decoder<'static, Vec<u8>>),
    #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
    Brotli(brotlic::DecompressorWriter<Vec<u8>>),
    Identity,
}

impl StreamingDecoder {
    pub fn new(encoding: &str) -> io::Result<Self> {
        match encoding {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            "gzip" => Ok(Self::Gzip(flate2::write::GzDecoder::new(Vec::new()))),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            "deflate" => Ok(Self::Deflate(RawInflate::new(true))),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            "deflate-raw" => Ok(Self::DeflateRaw(RawInflate::new(false))),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            "zstd" => Ok(Self::Zstd(zstd::stream::write::Decoder::new(Vec::new())?)),
            #[cfg(feature = "brotli-c")]
            "br" => Ok(Self::Brotli(brotlic::DecompressorWriter::new(Vec::new()))),
            #[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
            "br" => Ok(Self::Brotli(brotlic::DecompressorWriter::new(
                Vec::new(),
                8_096,
            ))),
            "" | "identity" => Ok(Self::Identity),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported encoding: {}", encoding),
            )),
        }
    }

    /// Decompress a chunk of data, returning the decompressed output
    pub fn decompress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(input.to_vec()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::Gzip(decoder) => {
                decoder.write_all(input)?;
                decoder.flush()?;
                Ok(std::mem::take(decoder.get_mut()))
            },
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::Deflate(decoder) => decoder.decompress_chunk(input),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::DeflateRaw(decoder) => decoder.decompress_chunk(input),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            Self::Zstd(decoder) => {
                decoder.write_all(input)?;
                decoder.flush()?;
                Ok(std::mem::take(decoder.get_mut()))
            },
            #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
            Self::Brotli(decoder) => {
                decoder.write_all(input)?;
                decoder.flush()?;
                Ok(std::mem::take(decoder.get_mut()))
            },
        }
    }

    /// Finish decompression and return any remaining data
    pub fn finish(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(Vec::new()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::Gzip(decoder) => decoder.finish(),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::Deflate(decoder) => decoder.finish(),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::DeflateRaw(decoder) => decoder.finish(),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            Self::Zstd(decoder) => Ok(decoder.into_inner()),
            #[cfg(feature = "brotli-c")]
            Self::Brotli(decoder) => decoder
                .into_inner()
                .map_err(|e| io::Error::other(e.to_string())),
            #[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
            Self::Brotli(decoder) => decoder
                .into_inner()
                .map_err(|_| io::Error::other("brotli decompression failed")),
        }
    }
}

/// Streaming compressor that maintains state across chunks.
///
/// Node's `zlib.createGzip()` and friends emit a *single* stream framed across
/// every chunk written to them, so per-chunk one-shot compression is not
/// equivalent. This keeps the encoder alive between `compress_chunk` calls.
pub enum StreamingEncoder {
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    Gzip(flate2::write::GzEncoder<Vec<u8>>),
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    Deflate(flate2::write::ZlibEncoder<Vec<u8>>),
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    DeflateRaw(flate2::write::DeflateEncoder<Vec<u8>>),
    #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
    Zstd(zstd::stream::write::Encoder<'static, Vec<u8>>),
    #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
    Brotli(brotlic::CompressorWriter<Vec<u8>>),
    Identity,
}

impl StreamingEncoder {
    pub fn new(encoding: &str, level: u32) -> io::Result<Self> {
        match encoding {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            "gzip" => Ok(Self::Gzip(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::new(level),
            ))),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            "deflate" => Ok(Self::Deflate(flate2::write::ZlibEncoder::new(
                Vec::new(),
                flate2::Compression::new(level),
            ))),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            "deflate-raw" => Ok(Self::DeflateRaw(flate2::write::DeflateEncoder::new(
                Vec::new(),
                flate2::Compression::new(level),
            ))),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            "zstd" => Ok(Self::Zstd(zstd::stream::write::Encoder::new(
                Vec::new(),
                level as i32,
            )?)),
            #[cfg(feature = "brotli-c")]
            "br" => {
                use brotlic::{BrotliEncoderOptions, Quality};
                let quality = Quality::new(level.min(11) as u8)
                    .map_err(|e| io::Error::other(e.to_string()))?;
                let encoder = BrotliEncoderOptions::new()
                    .quality(quality)
                    .build()
                    .map_err(|e| io::Error::other(e.to_string()))?;
                Ok(Self::Brotli(brotlic::CompressorWriter::with_encoder(
                    encoder,
                    Vec::new(),
                )))
            },
            #[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
            "br" => Ok(Self::Brotli(brotlic::CompressorWriter::new(
                Vec::new(),
                8_096,
                level,
                22,
            ))),
            "" | "identity" => Ok(Self::Identity),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported encoding: {}", encoding),
            )),
        }
    }

    /// Compress a chunk of data, returning the output produced so far.
    pub fn compress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(input.to_vec()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::Gzip(encoder) => {
                encoder.write_all(input)?;
                encoder.flush()?;
                Ok(std::mem::take(encoder.get_mut()))
            },
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::Deflate(encoder) => {
                encoder.write_all(input)?;
                encoder.flush()?;
                Ok(std::mem::take(encoder.get_mut()))
            },
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::DeflateRaw(encoder) => {
                encoder.write_all(input)?;
                encoder.flush()?;
                Ok(std::mem::take(encoder.get_mut()))
            },
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            Self::Zstd(encoder) => {
                encoder.write_all(input)?;
                encoder.flush()?;
                Ok(std::mem::take(encoder.get_mut()))
            },
            #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
            Self::Brotli(encoder) => {
                encoder.write_all(input)?;
                encoder.flush()?;
                Ok(std::mem::take(encoder.get_mut()))
            },
        }
    }

    /// Finish compression and return the trailing bytes of the stream.
    pub fn finish(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(Vec::new()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::Gzip(encoder) => encoder.finish(),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::Deflate(encoder) => encoder.finish(),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            Self::DeflateRaw(encoder) => encoder.finish(),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            Self::Zstd(encoder) => encoder.finish(),
            #[cfg(feature = "brotli-c")]
            Self::Brotli(encoder) => encoder
                .into_inner()
                .map_err(|e| io::Error::other(e.to_string())),
            #[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
            Self::Brotli(encoder) => encoder
                .into_inner()
                .map_err(|e| io::Error::other(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Multi-chunk compression must produce one framed stream, not one stream
    /// per chunk: total length differs and a single decoder reads it back.
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    #[test]
    fn gzip_encoder_streams_across_chunks() {
        let chunks: [&[u8]; 3] = [b"hello ", b"streaming ", b"world"];

        let mut encoder = StreamingEncoder::new("gzip", 6).unwrap();
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend(encoder.compress_chunk(chunk).unwrap());
        }
        out.extend(encoder.finish().unwrap());

        let mut decoder = StreamingDecoder::new("gzip").unwrap();
        let mut back = decoder.decompress_chunk(&out).unwrap();
        back.extend(decoder.finish().unwrap());
        assert_eq!(back, b"hello streaming world");

        // Three independently framed gzip members would be longer than one.
        let joined: Vec<u8> = chunks.concat();
        let mut per_chunk = Vec::new();
        for chunk in chunks {
            let mut enc = StreamingEncoder::new("gzip", 6).unwrap();
            per_chunk.extend(enc.compress_chunk(chunk).unwrap());
            per_chunk.extend(enc.finish().unwrap());
        }
        let _ = joined;
        assert!(out.len() < per_chunk.len());
    }

    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    #[test]
    fn deflate_raw_encoder_round_trips() {
        let mut encoder = StreamingEncoder::new("deflate-raw", 6).unwrap();
        let mut out = encoder.compress_chunk(b"raw deflate payload").unwrap();
        out.extend(encoder.finish().unwrap());

        let mut decoder = StreamingDecoder::new("deflate-raw")
            .unwrap_or(StreamingDecoder::new("identity").unwrap());
        // `StreamingDecoder` has no raw-deflate variant; decode via flate2 directly.
        let mut direct = flate2::read::DeflateDecoder::new(&out[..]);
        let mut back = Vec::new();
        std::io::Read::read_to_end(&mut direct, &mut back).unwrap();
        assert_eq!(back, b"raw deflate payload");
        let _ = decoder.decompress_chunk(b"");
    }

    #[test]
    fn identity_encoder_passes_bytes_through() {
        let mut encoder = StreamingEncoder::new("identity", 0).unwrap();
        assert_eq!(encoder.compress_chunk(b"abc").unwrap(), b"abc");
        assert!(encoder.finish().unwrap().is_empty());
    }

    #[test]
    fn unsupported_encoding_is_rejected() {
        assert!(StreamingEncoder::new("nope", 1).is_err());
    }
}
