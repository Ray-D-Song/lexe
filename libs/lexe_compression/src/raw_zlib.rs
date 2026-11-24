// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZlibMode {
    Gzip,
    Gunzip,
    Deflate,
    Inflate,
    DeflateRaw,
    InflateRaw,
    Unzip,
}

impl ZlibMode {
    pub fn from_encoding(encoding: &str, decode: bool) -> io::Result<Self> {
        match (encoding, decode) {
            ("gzip", false) => Ok(Self::Gzip),
            ("gzip", true) | ("gunzip", true) => Ok(Self::Gunzip),
            ("deflate", false) => Ok(Self::Deflate),
            ("deflate", true) | ("inflate", true) => Ok(Self::Inflate),
            ("deflate-raw", false) => Ok(Self::DeflateRaw),
            ("deflate-raw", true) | ("inflate-raw", true) => Ok(Self::InflateRaw),
            ("unzip", true) => Ok(Self::Unzip),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported encoding: {encoding}"),
            )),
        }
    }

    #[cfg(feature = "flate2-c")]
    fn raw_window_bits(self, wb: i32) -> i32 {
        match self {
            Self::Deflate | Self::Inflate => wb,
            Self::DeflateRaw | Self::InflateRaw => -wb,
            Self::Gzip | Self::Gunzip => wb + 16,
            Self::Unzip => wb + 32,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ZlibOptions {
    pub level: i32,
    pub window_bits: i32,
    pub mem_level: i32,
    pub strategy: i32,
    pub dictionary: Option<Vec<u8>>,
}

impl Default for ZlibOptions {
    fn default() -> Self {
        Self {
            level: -1,
            window_bits: 15,
            mem_level: 8,
            strategy: 0,
            dictionary: None,
        }
    }
}

pub fn validate_zlib_options(opts: &ZlibOptions, is_encode: bool) -> io::Result<()> {
    if is_encode && !(-1..=9).contains(&opts.level) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid compression level: {}", opts.level),
        ));
    }
    if !(8..=15).contains(&opts.window_bits) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid windowBits: {}", opts.window_bits),
        ));
    }
    if is_encode && !(1..=9).contains(&opts.mem_level) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid memLevel: {}", opts.mem_level),
        ));
    }
    if is_encode && !(0..=4).contains(&opts.strategy) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid strategy: {}", opts.strategy),
        ));
    }
    Ok(())
}

#[cfg(feature = "flate2-c")]
mod imp {
    use std::ffi::CStr;
    use std::io;
    use std::mem::{size_of, MaybeUninit};
    use std::os::raw::c_int;
    use std::ptr;

    use libz_ng_sys as z;

    use super::{validate_zlib_options, ZlibMode, ZlibOptions};

    const ZLIB_VERSION_CSTR: &[u8] = b"1.2.11\0";

    fn alloc_zeroed_zstream() -> z::z_streamp {
        Box::into_raw(Box::new(MaybeUninit::<z::z_stream>::zeroed())).cast()
    }

    unsafe fn free_zstream(strm: z::z_streamp) {
        if !strm.is_null() {
            drop(Box::from_raw(strm.cast::<MaybeUninit<z::z_stream>>()));
        }
    }

    unsafe fn zlib_err(rc: c_int, strm: z::z_streamp) -> io::Error {
        let msg = if !strm.is_null() && !(*strm).msg.is_null() {
            CStr::from_ptr((*strm).msg).to_string_lossy().into_owned()
        } else {
            match rc {
                z::Z_STREAM_ERROR => "Z_STREAM_ERROR: stream error".to_string(),
                z::Z_DATA_ERROR => "Z_DATA_ERROR: incorrect data check".to_string(),
                z::Z_MEM_ERROR => "Z_MEM_ERROR: insufficient memory".to_string(),
                z::Z_BUF_ERROR => "unexpected end of file".to_string(),
                z::Z_VERSION_ERROR => "Z_VERSION_ERROR: incompatible zlib version".to_string(),
                _ => format!("zlib error {rc}"),
            }
        };
        let kind = match rc {
            z::Z_DATA_ERROR => io::ErrorKind::InvalidData,
            z::Z_BUF_ERROR => io::ErrorKind::UnexpectedEof,
            _ => io::ErrorKind::InvalidInput,
        };
        io::Error::new(kind, msg)
    }

    pub struct RawZlibEncoder {
        strm: *mut z::z_stream,
        initialized: bool,
        mode: ZlibMode,
        opts: ZlibOptions,
        total_in: u64,
    }

    unsafe impl Send for RawZlibEncoder {}

    impl Drop for RawZlibEncoder {
        fn drop(&mut self) {
            unsafe {
                if self.initialized {
                    let _ = z::deflateEnd(self.strm);
                    self.initialized = false;
                }
                free_zstream(self.strm);
            }
        }
    }

    impl RawZlibEncoder {
        pub fn new(mode: ZlibMode, opts: ZlibOptions) -> io::Result<Self> {
            validate_zlib_options(&opts, true)?;
            let strm = alloc_zeroed_zstream();
            let mut enc = Self {
                strm,
                initialized: false,
                mode,
                opts,
                total_in: 0,
            };
            let raw_wb = mode.raw_window_bits(enc.opts.window_bits);
            let rc = unsafe {
                z::deflateInit2_(
                    enc.strm,
                    enc.opts.level,
                    z::Z_DEFLATED,
                    raw_wb,
                    enc.opts.mem_level,
                    enc.opts.strategy,
                    ZLIB_VERSION_CSTR.as_ptr().cast(),
                    size_of::<z::z_stream>() as c_int,
                )
            };
            if rc != z::Z_OK {
                return Err(unsafe { zlib_err(rc, enc.strm) });
            }
            enc.initialized = true;
            enc.apply_dictionary()?;
            Ok(enc)
        }

        fn apply_dictionary(&mut self) -> io::Result<()> {
            if let Some(dict) = &self.opts.dictionary {
                if matches!(self.mode, ZlibMode::Deflate | ZlibMode::DeflateRaw) {
                    let len = u32::try_from(dict.len())
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "dictionary too large"))?;
                    let rc = unsafe { z::deflateSetDictionary(self.strm, dict.as_ptr(), len) };
                    if rc != z::Z_OK {
                        return Err(unsafe { zlib_err(rc, self.strm) });
                    }
                }
            }
            Ok(())
        }

        pub fn bytes_written(&self) -> u64 {
            self.total_in
        }

        pub fn compress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
            self.process(input, z::Z_NO_FLUSH)
        }

        pub fn flush_kind(&mut self, kind: i32) -> io::Result<Vec<u8>> {
            if !(0..=6).contains(&kind) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid flush value: {kind}"),
                ));
            }
            self.process(&[], kind)
        }

        pub fn params(&mut self, level: i32, strategy: i32) -> io::Result<Vec<u8>> {
            if !(-1..=9).contains(&level) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid compression level: {level}"),
                ));
            }
            if !(0..=4).contains(&strategy) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid strategy: {strategy}"),
                ));
            }
            let mut out = Vec::new();
            let mut scratch = [0u8; 16 * 1024];
            unsafe {
                (*self.strm).next_in = ptr::null_mut();
                (*self.strm).avail_in = 0;
            }
            loop {
                unsafe {
                    (*self.strm).next_out = scratch.as_mut_ptr();
                    (*self.strm).avail_out = scratch.len() as z::uInt;
                }
                let rc = unsafe { z::deflateParams(self.strm, level, strategy) };
                let written = scratch.len() - unsafe { (*self.strm).avail_out as usize };
                out.extend_from_slice(&scratch[..written]);
                match rc {
                    z::Z_OK => {
                        if unsafe { (*self.strm).avail_out } > 0 {
                            break;
                        }
                    },
                    z::Z_BUF_ERROR if written > 0 => continue,
                    z::Z_BUF_ERROR => break,
                    other => return Err(unsafe { zlib_err(other, self.strm) }),
                }
            }
            self.opts.level = level;
            self.opts.strategy = strategy;
            Ok(out)
        }

        pub fn reset(&mut self) -> io::Result<()> {
            let rc = unsafe { z::deflateReset(self.strm) };
            if rc != z::Z_OK {
                return Err(unsafe { zlib_err(rc, self.strm) });
            }
            self.apply_dictionary()
        }

        pub fn finish(mut self) -> io::Result<Vec<u8>> {
            self.process(&[], z::Z_FINISH)
        }

        fn process(&mut self, input: &[u8], flush: i32) -> io::Result<Vec<u8>> {
            let mut out = Vec::new();
            let mut scratch = [0u8; 16 * 1024];
            let max_chunk = 1024 * 1024 * 1024;
            let mut offset = 0usize;

            loop {
                let end = (offset + max_chunk).min(input.len());
                let slice = &input[offset..end];
                let is_last = end == input.len();
                let chunk_flush = if is_last { flush } else { z::Z_NO_FLUSH };

                unsafe {
                    (*self.strm).next_in = if slice.is_empty() {
                        ptr::null_mut()
                    } else {
                        slice.as_ptr().cast_mut()
                    };
                    (*self.strm).avail_in = slice.len() as z::uInt;
                }

                loop {
                    let before_avail_in = unsafe { (*self.strm).avail_in as usize };
                    unsafe {
                        (*self.strm).next_out = scratch.as_mut_ptr();
                        (*self.strm).avail_out = scratch.len() as z::uInt;
                    }
                    let rc = unsafe { z::deflate(self.strm, chunk_flush) };
                    let after_avail_in = unsafe { (*self.strm).avail_in as usize };
                    let consumed = before_avail_in.saturating_sub(after_avail_in);
                    self.total_in = self.total_in.saturating_add(consumed as u64);

                    let avail_out = unsafe { (*self.strm).avail_out as usize };
                    let written = scratch.len() - avail_out;
                    out.extend_from_slice(&scratch[..written]);

                    match rc {
                        z::Z_STREAM_END => break,
                        z::Z_OK => {
                            if after_avail_in == 0 && avail_out > 0 {
                                break;
                            }
                            if after_avail_in == 0 && written == 0 {
                                break;
                            }
                        },
                        z::Z_BUF_ERROR => {
                            if chunk_flush == z::Z_FINISH && written > 0 {
                                continue;
                            }
                            break;
                        },
                        other => return Err(unsafe { zlib_err(other, self.strm) }),
                    }
                }

                offset = end;
                if offset >= input.len() {
                    break;
                }
            }

            Ok(out)
        }
    }

    pub struct RawZlibDecoder {
        strm: *mut z::z_stream,
        initialized: bool,
        reached_end: bool,
        mode: ZlibMode,
        opts: ZlibOptions,
        total_in: u64,
    }

    unsafe impl Send for RawZlibDecoder {}

    impl Drop for RawZlibDecoder {
        fn drop(&mut self) {
            unsafe {
                if self.initialized {
                    let _ = z::inflateEnd(self.strm);
                    self.initialized = false;
                }
                free_zstream(self.strm);
            }
        }
    }

    impl RawZlibDecoder {
        pub fn new(mode: ZlibMode, opts: ZlibOptions) -> io::Result<Self> {
            validate_zlib_options(&opts, false)?;
            let strm = alloc_zeroed_zstream();
            let mut dec = Self {
                strm,
                initialized: false,
                reached_end: false,
                mode,
                opts,
                total_in: 0,
            };
            let raw_wb = mode.raw_window_bits(dec.opts.window_bits);
            let rc = unsafe {
                z::inflateInit2_(
                    dec.strm,
                    raw_wb,
                    ZLIB_VERSION_CSTR.as_ptr().cast(),
                    size_of::<z::z_stream>() as c_int,
                )
            };
            if rc != z::Z_OK {
                return Err(unsafe { zlib_err(rc, dec.strm) });
            }
            dec.initialized = true;
            dec.apply_raw_dictionary()?;
            Ok(dec)
        }

        fn apply_raw_dictionary(&mut self) -> io::Result<()> {
            if let Some(dict) = &self.opts.dictionary {
                if self.mode == ZlibMode::InflateRaw {
                    let len = u32::try_from(dict.len())
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "dictionary too large"))?;
                    let rc = unsafe { z::inflateSetDictionary(self.strm, dict.as_ptr(), len) };
                    if rc != z::Z_OK {
                        return Err(unsafe { zlib_err(rc, self.strm) });
                    }
                }
            }
            Ok(())
        }

        pub fn bytes_written(&self) -> u64 {
            self.total_in
        }

        pub fn decompress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
            self.process(input, z::Z_NO_FLUSH)
        }

        pub fn flush_kind(&mut self, kind: i32) -> io::Result<Vec<u8>> {
            if !(0..=6).contains(&kind) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid flush value: {kind}"),
                ));
            }
            self.process(&[], kind)
        }

        pub fn reset(&mut self) -> io::Result<()> {
            let rc = unsafe { z::inflateReset(self.strm) };
            if rc != z::Z_OK {
                return Err(unsafe { zlib_err(rc, self.strm) });
            }
            self.reached_end = false;
            self.apply_raw_dictionary()
        }

        pub fn finish(mut self) -> io::Result<Vec<u8>> {
            if self.reached_end {
                return Ok(Vec::new());
            }
            let tail = self.process(&[], z::Z_FINISH)?;
            if !self.reached_end {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected end of file",
                ));
            }
            Ok(tail)
        }

        fn process(&mut self, input: &[u8], flush: i32) -> io::Result<Vec<u8>> {
            let mut out = Vec::new();
            let mut scratch = [0u8; 16 * 1024];
            let max_chunk = 1024 * 1024 * 1024;
            let mut offset = 0usize;

            loop {
                let end = (offset + max_chunk).min(input.len());
                let slice = &input[offset..end];
                let is_last = end == input.len();
                let chunk_flush = if is_last { flush } else { z::Z_NO_FLUSH };

                unsafe {
                    (*self.strm).next_in = if slice.is_empty() {
                        ptr::null_mut()
                    } else {
                        slice.as_ptr().cast_mut()
                    };
                    (*self.strm).avail_in = slice.len() as z::uInt;
                }

                loop {
                    let before_avail_in = unsafe { (*self.strm).avail_in as usize };
                    unsafe {
                        (*self.strm).next_out = scratch.as_mut_ptr();
                        (*self.strm).avail_out = scratch.len() as z::uInt;
                    }
                    let rc = unsafe { z::inflate(self.strm, chunk_flush) };
                    let after_avail_in = unsafe { (*self.strm).avail_in as usize };
                    let consumed = before_avail_in.saturating_sub(after_avail_in);
                    self.total_in = self.total_in.saturating_add(consumed as u64);

                    let avail_out = unsafe { (*self.strm).avail_out as usize };
                    let written = scratch.len() - avail_out;
                    out.extend_from_slice(&scratch[..written]);

                    match rc {
                        z::Z_STREAM_END => {
                            self.reached_end = true;
                            break;
                        },
                        z::Z_NEED_DICT => {
                            let Some(dict) = &self.opts.dictionary else {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "Missing dictionary (Z_NEED_DICT)",
                                ));
                            };
                            let len = u32::try_from(dict.len()).map_err(|_| {
                                io::Error::new(io::ErrorKind::InvalidInput, "dictionary too large")
                            })?;
                            let drc = unsafe {
                                z::inflateSetDictionary(self.strm, dict.as_ptr(), len)
                            };
                            if drc != z::Z_OK {
                                return Err(unsafe { zlib_err(drc, self.strm) });
                            }
                        },
                        z::Z_OK => {
                            if after_avail_in == 0 && avail_out > 0 {
                                break;
                            }
                            if after_avail_in == 0 && written == 0 {
                                break;
                            }
                        },
                        z::Z_BUF_ERROR => {
                            if written > 0 {
                                continue;
                            }
                            break;
                        },
                        other => return Err(unsafe { zlib_err(other, self.strm) }),
                    }
                }

                offset = end;
                if offset >= input.len() || self.reached_end {
                    break;
                }
            }

            Ok(out)
        }
    }
}

#[cfg(all(not(feature = "flate2-c"), feature = "flate2-rust"))]
mod imp {
    use std::io::{self, Write};

    use flate2::{Compression, FlushCompress, FlushDecompress};

    use super::{validate_zlib_options, ZlibMode, ZlibOptions};

    fn map_flush_compress(kind: i32) -> io::Result<FlushCompress> {
        match kind {
            0 => Ok(FlushCompress::None),
            1 | 2 => Ok(FlushCompress::Sync),
            3 | 5 | 6 => Ok(FlushCompress::Full),
            4 => Ok(FlushCompress::Finish),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid flush value: {kind}"),
            )),
        }
    }

    pub struct RawZlibEncoder {
        inner: RustEncInner,
        mode: ZlibMode,
        opts: ZlibOptions,
        total_in: u64,
    }

    enum RustEncInner {
        Gzip(flate2::write::GzEncoder<Vec<u8>>),
        Compress(flate2::Compress),
    }

    impl RawZlibEncoder {
        pub fn new(mode: ZlibMode, opts: ZlibOptions) -> io::Result<Self> {
            validate_zlib_options(&opts, true)?;
            if opts.dictionary.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "dictionary option requires compression-c feature",
                ));
            }
            let level = if opts.level < 0 { 6 } else { opts.level as u32 };
            let comp = Compression::new(level);
            let inner = match mode {
                ZlibMode::Gzip => RustEncInner::Gzip(flate2::write::GzEncoder::new(Vec::new(), comp)),
                ZlibMode::Deflate => RustEncInner::Compress(flate2::Compress::new(comp, true)),
                ZlibMode::DeflateRaw => RustEncInner::Compress(flate2::Compress::new(comp, false)),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid mode for encoder",
                    ))
                },
            };
            Ok(Self {
                inner,
                mode,
                opts,
                total_in: 0,
            })
        }

        pub fn bytes_written(&self) -> u64 {
            self.total_in
        }

        pub fn compress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
            self.process(input, FlushCompress::None)
        }

        pub fn flush_kind(&mut self, kind: i32) -> io::Result<Vec<u8>> {
            let flush = map_flush_compress(kind)?;
            self.process(&[], flush)
        }

        pub fn params(&mut self, level: i32, strategy: i32) -> io::Result<Vec<u8>> {
            if !(-1..=9).contains(&level) || !(0..=4).contains(&strategy) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid params argument",
                ));
            }
            self.opts.level = level;
            self.opts.strategy = strategy;
            Ok(Vec::new())
        }

        pub fn reset(&mut self) -> io::Result<()> {
            let total = self.total_in;
            *self = Self::new(self.mode, self.opts.clone())?;
            self.total_in = total;
            Ok(())
        }

        pub fn finish(mut self) -> io::Result<Vec<u8>> {
            match self.inner {
                RustEncInner::Gzip(enc) => enc.finish(),
                RustEncInner::Compress(..) => self.process(&[], FlushCompress::Finish),
            }
        }

        fn process(&mut self, input: &[u8], flush: FlushCompress) -> io::Result<Vec<u8>> {
            self.total_in = self.total_in.saturating_add(input.len() as u64);
            match &mut self.inner {
                RustEncInner::Gzip(enc) => {
                    enc.write_all(input)?;
                    enc.flush()?;
                    Ok(std::mem::take(enc.get_mut()))
                },
                RustEncInner::Compress(comp) => {
                    let mut out = Vec::new();
                    let mut consumed = 0usize;
                    let mut scratch = [0u8; 16 * 1024];
                    loop {
                        let before_in = comp.total_in();
                        let before_out = comp.total_out();
                        let status = comp
                            .compress(&input[consumed..], &mut scratch, flush)
                            .map_err(|e| io::Error::other(e.to_string()))?;
                        consumed += (comp.total_in() - before_in) as usize;
                        let written = (comp.total_out() - before_out) as usize;
                        out.extend_from_slice(&scratch[..written]);
                        match status {
                            flate2::Status::StreamEnd => break,
                            flate2::Status::Ok | flate2::Status::BufError => {
                                if consumed >= input.len() && written == 0 {
                                    break;
                                }
                            },
                        }
                    }
                    Ok(out)
                },
            }
        }
    }

    enum RustDecInner {
        PendingUnzip(Vec<u8>),
        Gzip(flate2::write::GzDecoder<Vec<u8>>),
        Decompress(flate2::Decompress),
    }

    pub struct RawZlibDecoder {
        inner: RustDecInner,
        reached_end: bool,
        mode: ZlibMode,
        opts: ZlibOptions,
        total_in: u64,
    }

    impl RawZlibDecoder {
        pub fn new(mode: ZlibMode, opts: ZlibOptions) -> io::Result<Self> {
            validate_zlib_options(&opts, false)?;
            if opts.dictionary.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "dictionary option requires compression-c feature",
                ));
            }
            let inner = match mode {
                ZlibMode::Gunzip => RustDecInner::Gzip(flate2::write::GzDecoder::new(Vec::new())),
                ZlibMode::Inflate => RustDecInner::Decompress(flate2::Decompress::new(true)),
                ZlibMode::InflateRaw => RustDecInner::Decompress(flate2::Decompress::new(false)),
                ZlibMode::Unzip => RustDecInner::PendingUnzip(Vec::new()),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid mode for decoder",
                    ))
                },
            };
            Ok(Self {
                inner,
                reached_end: false,
                mode,
                opts,
                total_in: 0,
            })
        }

        pub fn bytes_written(&self) -> u64 {
            self.total_in
        }

        pub fn decompress_chunk(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
            self.total_in = self.total_in.saturating_add(input.len() as u64);
            if let RustDecInner::PendingUnzip(buf) = &mut self.inner {
                buf.extend_from_slice(input);
                if buf.len() < 2 {
                    return Ok(Vec::new());
                }
                let buffered = std::mem::take(buf);
                self.inner = if buffered.starts_with(&[0x1f, 0x8b]) {
                    RustDecInner::Gzip(flate2::write::GzDecoder::new(Vec::new()))
                } else {
                    RustDecInner::Decompress(flate2::Decompress::new(true))
                };
                return self.feed_inner(&buffered, FlushDecompress::None);
            }
            self.feed_inner(input, FlushDecompress::None)
        }

        pub fn flush_kind(&mut self, kind: i32) -> io::Result<Vec<u8>> {
            if !(0..=6).contains(&kind) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid flush value: {kind}"),
                ));
            }
            Ok(Vec::new())
        }

        pub fn reset(&mut self) -> io::Result<()> {
            let total = self.total_in;
            *self = Self::new(self.mode, self.opts.clone())?;
            self.total_in = total;
            Ok(())
        }

        pub fn finish(mut self) -> io::Result<Vec<u8>> {
            if let RustDecInner::PendingUnzip(buf) = &self.inner {
                if buf.is_empty() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "unexpected end of file",
                    ));
                }
            }
            match self.inner {
                RustDecInner::PendingUnzip(_) => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected end of file",
                )),
                RustDecInner::Gzip(dec) => dec.finish(),
                RustDecInner::Decompress(..) => {
                    let out = self.feed_inner(&[], FlushDecompress::Finish)?;
                    if !self.reached_end {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "unexpected end of file",
                        ));
                    }
                    Ok(out)
                },
            }
        }

        fn feed_inner(&mut self, input: &[u8], flush: FlushDecompress) -> io::Result<Vec<u8>> {
            match &mut self.inner {
                RustDecInner::PendingUnzip(_) => Ok(Vec::new()),
                RustDecInner::Gzip(dec) => {
                    dec.write_all(input)?;
                    dec.flush()?;
                    Ok(std::mem::take(dec.get_mut()))
                },
                RustDecInner::Decompress(decomp) => {
                    let mut out = Vec::new();
                    let mut consumed = 0usize;
                    let mut scratch = [0u8; 16 * 1024];
                    loop {
                        let before_in = decomp.total_in();
                        let before_out = decomp.total_out();
                        let status = decomp
                            .decompress(&input[consumed..], &mut scratch, flush)
                            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                        consumed += (decomp.total_in() - before_in) as usize;
                        let written = (decomp.total_out() - before_out) as usize;
                        out.extend_from_slice(&scratch[..written]);
                        match status {
                            flate2::Status::StreamEnd => {
                                self.reached_end = true;
                                break;
                            },
                            flate2::Status::Ok | flate2::Status::BufError => {
                                if consumed >= input.len() && written == 0 {
                                    break;
                                }
                            },
                        }
                    }
                    Ok(out)
                },
            }
        }
    }
}

#[cfg(any(feature = "flate2-c", feature = "flate2-rust"))]
pub use imp::{RawZlibDecoder, RawZlibEncoder};
