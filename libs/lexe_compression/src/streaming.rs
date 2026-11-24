// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Streaming encoder and decoder handles used by `node:zlib` and `CompressionStream`.

use std::io::{self, Write};

#[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
use brotli as brotlic;

use crate::raw_zlib::{ZlibMode, ZlibOptions};
#[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
use crate::raw_zlib::{RawZlibDecoder, RawZlibEncoder};

pub enum StreamingDecoderInner {
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    Zlib(RawZlibDecoder),
    #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
    Zstd(zstd::stream::write::Decoder<'static, Vec<u8>>),
    #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
    Brotli(brotlic::DecompressorWriter<Vec<u8>>),
    Identity,
}

/// Streaming decompressor that maintains state across chunks.
pub struct StreamingDecoder {
    inner: StreamingDecoderInner,
    encoding: String,
    opts: ZlibOptions,
    bytes_in: u64,
}

impl StreamingDecoder {
    pub fn new(encoding: &str) -> io::Result<Self> {
        Self::new_with_options(encoding, ZlibOptions::default())
    }

    pub fn new_with_options(encoding: &str, opts: ZlibOptions) -> io::Result<Self> {
        let inner = match encoding {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            "gzip" | "gunzip" | "deflate" | "inflate" | "deflate-raw" | "inflate-raw" | "unzip" => {
                let mode = ZlibMode::from_encoding(encoding, true)?;
                StreamingDecoderInner::Zlib(RawZlibDecoder::new(mode, opts.clone())?)
            },
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            "zstd" => StreamingDecoderInner::Zstd(zstd::stream::write::Decoder::new(Vec::new())?),
            #[cfg(feature = "brotli-c")]
            "br" => StreamingDecoderInner::Brotli(brotlic::DecompressorWriter::new(Vec::new())),
            #[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
            "br" => StreamingDecoderInner::Brotli(brotlic::DecompressorWriter::new(
                Vec::new(),
                8_096,
            )),
            "" | "identity" => StreamingDecoderInner::Identity,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported encoding: {encoding}"),
                ))
            },
        };
        Ok(Self {
            inner,
            encoding: encoding.to_string(),
            opts,
            bytes_in: 0,
        })
    }

    pub fn bytes_written(&self) -> u64 {
        match &self.inner {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingDecoderInner::Zlib(dec) => dec.bytes_written(),
            _ => self.bytes_in,
        }
    }

    pub fn decompress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        self.bytes_in = self.bytes_in.saturating_add(input.len() as u64);
        match &mut self.inner {
            StreamingDecoderInner::Identity => Ok(input.to_vec()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingDecoderInner::Zlib(dec) => dec.decompress_chunk(input),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            StreamingDecoderInner::Zstd(dec) => {
                dec.write_all(input)?;
                dec.flush()?;
                Ok(std::mem::take(dec.get_mut()))
            },
            #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
            StreamingDecoderInner::Brotli(dec) => {
                dec.write_all(input)?;
                dec.flush()?;
                Ok(std::mem::take(dec.get_mut()))
            },
        }
    }

    pub fn flush_kind(&mut self, kind: i32) -> io::Result<Vec<u8>> {
        match &mut self.inner {
            StreamingDecoderInner::Identity => Ok(Vec::new()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingDecoderInner::Zlib(dec) => dec.flush_kind(kind),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            StreamingDecoderInner::Zstd(dec) => {
                let _ = kind;
                dec.flush()?;
                Ok(std::mem::take(dec.get_mut()))
            },
            #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
            StreamingDecoderInner::Brotli(dec) => {
                let _ = kind;
                dec.flush()?;
                Ok(std::mem::take(dec.get_mut()))
            },
        }
    }

    pub fn reset(&mut self) -> io::Result<()> {
        match &mut self.inner {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingDecoderInner::Zlib(dec) => dec.reset(),
            _ => {
                let total = self.bytes_in;
                *self = Self::new_with_options(&self.encoding, self.opts.clone())?;
                self.bytes_in = total;
                Ok(())
            },
        }
    }

    pub fn finish(self) -> io::Result<Vec<u8>> {
        match self.inner {
            StreamingDecoderInner::Identity => Ok(Vec::new()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingDecoderInner::Zlib(dec) => dec.finish(),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            StreamingDecoderInner::Zstd(dec) => Ok(dec.into_inner()),
            #[cfg(feature = "brotli-c")]
            StreamingDecoderInner::Brotli(dec) => dec
                .into_inner()
                .map_err(|e| io::Error::other(e.to_string())),
            #[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
            StreamingDecoderInner::Brotli(dec) => dec
                .into_inner()
                .map_err(|_| io::Error::other("brotli decompression failed")),
        }
    }
}

pub enum StreamingEncoderInner {
    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    Zlib(RawZlibEncoder),
    #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
    Zstd(zstd::stream::write::Encoder<'static, Vec<u8>>),
    #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
    Brotli(brotlic::CompressorWriter<Vec<u8>>),
    Identity,
}

/// Streaming compressor that maintains state across chunks.
pub struct StreamingEncoder {
    inner: StreamingEncoderInner,
    encoding: String,
    opts: ZlibOptions,
    bytes_in: u64,
}

impl StreamingEncoder {
    pub fn new(encoding: &str, level: u32) -> io::Result<Self> {
        let mut opts = ZlibOptions::default();
        opts.level = level as i32;
        Self::new_with_options(encoding, opts)
    }

    pub fn new_with_options(encoding: &str, opts: ZlibOptions) -> io::Result<Self> {
        let inner = match encoding {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            "gzip" | "deflate" | "deflate-raw" => {
                let mode = ZlibMode::from_encoding(encoding, false)?;
                StreamingEncoderInner::Zlib(RawZlibEncoder::new(mode, opts.clone())?)
            },
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            "zstd" => {
                let level = if opts.level < 0 { 3 } else { opts.level };
                StreamingEncoderInner::Zstd(zstd::stream::write::Encoder::new(Vec::new(), level)?)
            },
            #[cfg(feature = "brotli-c")]
            "br" => {
                use brotlic::{BrotliEncoderOptions, Quality};
                let q = if opts.level < 0 { 11 } else { opts.level.clamp(0, 11) as u8 };
                let quality = Quality::new(q).map_err(|e| io::Error::other(e.to_string()))?;
                let encoder = BrotliEncoderOptions::new()
                    .quality(quality)
                    .build()
                    .map_err(|e| io::Error::other(e.to_string()))?;
                StreamingEncoderInner::Brotli(brotlic::CompressorWriter::with_encoder(
                    encoder,
                    Vec::new(),
                ))
            },
            #[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
            "br" => {
                let q = if opts.level < 0 { 11 } else { opts.level.clamp(0, 11) as u32 };
                StreamingEncoderInner::Brotli(brotlic::CompressorWriter::new(
                    Vec::new(),
                    8_096,
                    q,
                    22,
                ))
            },
            "" | "identity" => StreamingEncoderInner::Identity,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported encoding: {encoding}"),
                ))
            },
        };
        Ok(Self {
            inner,
            encoding: encoding.to_string(),
            opts,
            bytes_in: 0,
        })
    }

    pub fn bytes_written(&self) -> u64 {
        match &self.inner {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingEncoderInner::Zlib(enc) => enc.bytes_written(),
            _ => self.bytes_in,
        }
    }

    pub fn compress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        self.bytes_in = self.bytes_in.saturating_add(input.len() as u64);
        match &mut self.inner {
            StreamingEncoderInner::Identity => Ok(input.to_vec()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingEncoderInner::Zlib(enc) => enc.compress_chunk(input),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            StreamingEncoderInner::Zstd(enc) => {
                enc.write_all(input)?;
                enc.flush()?;
                Ok(std::mem::take(enc.get_mut()))
            },
            #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
            StreamingEncoderInner::Brotli(enc) => {
                enc.write_all(input)?;
                enc.flush()?;
                Ok(std::mem::take(enc.get_mut()))
            },
        }
    }

    pub fn flush_kind(&mut self, kind: i32) -> io::Result<Vec<u8>> {
        match &mut self.inner {
            StreamingEncoderInner::Identity => Ok(Vec::new()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingEncoderInner::Zlib(enc) => enc.flush_kind(kind),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            StreamingEncoderInner::Zstd(enc) => {
                let _ = kind;
                enc.flush()?;
                Ok(std::mem::take(enc.get_mut()))
            },
            #[cfg(any(feature = "brotli-c", feature = "brotli-rust"))]
            StreamingEncoderInner::Brotli(enc) => {
                let _ = kind;
                enc.flush()?;
                Ok(std::mem::take(enc.get_mut()))
            },
        }
    }

    pub fn params(&mut self, level: i32, strategy: i32) -> io::Result<Vec<u8>> {
        match &mut self.inner {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingEncoderInner::Zlib(enc) => enc.params(level, strategy),
            _ => {
                let _ = (level, strategy);
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "params only supported for zlib streams",
                ))
            },
        }
    }

    pub fn reset(&mut self) -> io::Result<()> {
        match &mut self.inner {
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingEncoderInner::Zlib(enc) => enc.reset(),
            _ => {
                let total = self.bytes_in;
                *self = Self::new_with_options(&self.encoding, self.opts.clone())?;
                self.bytes_in = total;
                Ok(())
            },
        }
    }

    pub fn finish(self) -> io::Result<Vec<u8>> {
        match self.inner {
            StreamingEncoderInner::Identity => Ok(Vec::new()),
            #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
            StreamingEncoderInner::Zlib(enc) => enc.finish(),
            #[cfg(any(feature = "zstd-c", feature = "zstd-rust"))]
            StreamingEncoderInner::Zstd(enc) => enc.finish(),
            #[cfg(feature = "brotli-c")]
            StreamingEncoderInner::Brotli(enc) => enc
                .into_inner()
                .map_err(|e| io::Error::other(e.to_string())),
            #[cfg(all(not(feature = "brotli-c"), feature = "brotli-rust"))]
            StreamingEncoderInner::Brotli(enc) => Ok(enc.into_inner()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    #[test]
    fn gzip_encoder_streams_across_chunks() {
        let chunks: [&[u8]; 3] = [b"hello ", b"streaming ", b"world"];

        let mut encoder = StreamingEncoder::new("gzip", 6).unwrap();
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend(encoder.compress_chunk(chunk).unwrap());
        }
        assert_eq!(encoder.bytes_written(), 21);
        out.extend(encoder.finish().unwrap());

        let mut decoder = StreamingDecoder::new("gzip").unwrap();
        let mut back = decoder.decompress_chunk(&out).unwrap();
        back.extend(decoder.finish().unwrap());
        assert_eq!(back, b"hello streaming world");

        let mut per_chunk = Vec::new();
        for chunk in chunks {
            let mut enc = StreamingEncoder::new("gzip", 6).unwrap();
            per_chunk.extend(enc.compress_chunk(chunk).unwrap());
            per_chunk.extend(enc.finish().unwrap());
        }
        assert!(out.len() < per_chunk.len());
    }

    #[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
    #[test]
    fn deflate_raw_and_unzip_round_trip() {
        let mut encoder = StreamingEncoder::new("deflate-raw", 6).unwrap();
        let mut out = encoder.compress_chunk(b"raw deflate payload").unwrap();
        out.extend(encoder.finish().unwrap());

        let mut decoder = StreamingDecoder::new("deflate-raw").unwrap();
        let mut back = decoder.decompress_chunk(&out).unwrap();
        back.extend(decoder.finish().unwrap());
        assert_eq!(back, b"raw deflate payload");

        let mut gz = StreamingEncoder::new("gzip", 6).unwrap();
        let mut gz_bytes = gz.compress_chunk(b"unzip auto").unwrap();
        gz_bytes.extend(gz.finish().unwrap());
        let mut unzip = StreamingDecoder::new("unzip").unwrap();
        let mut u_back = unzip.decompress_chunk(&gz_bytes).unwrap();
        u_back.extend(unzip.finish().unwrap());
        assert_eq!(u_back, b"unzip auto");
    }

    #[cfg(feature = "flate2-c")]
    #[test]
    fn dictionary_window_bits_flush_and_params_work() {
        let dict = b"the quick brown fox jumps over the lazy dog".to_vec();
        let opts = ZlibOptions {
            level: 9,
            window_bits: 12,
            mem_level: 5,
            strategy: 0,
            dictionary: Some(dict.clone()),
        };
        let mut enc = StreamingEncoder::new_with_options("deflate", opts.clone()).unwrap();
        let mut compressed = enc.compress_chunk(b"the quick ").unwrap();
        compressed.extend(enc.flush_kind(2).unwrap()); // Z_SYNC_FLUSH
        assert!(!compressed.is_empty());

        let mut dec = StreamingDecoder::new_with_options("deflate", opts.clone()).unwrap();
        let partial = dec.decompress_chunk(&compressed).unwrap();
        assert_eq!(partial, b"the quick ");

        compressed.extend(enc.params(1, 0).unwrap());
        compressed.extend(enc.compress_chunk(b"brown fox").unwrap());
        let tail = enc.finish().unwrap();
        compressed.extend(&tail);

        let mut full_dec = StreamingDecoder::new_with_options("deflate", opts).unwrap();
        let mut restored = full_dec.decompress_chunk(&compressed).unwrap();
        restored.extend(full_dec.finish().unwrap());
        assert_eq!(restored, b"the quick brown fox");
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
