// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
#[cfg(unix)]
use std::time::Duration;
use std::{fs::Metadata, time::SystemTime};

use lexe_context::CtxExtension;
use lexe_utils::result::ResultExt;
use rquickjs::{
    function::Rest, prelude::Opt, BigInt, Ctx, Exception, FromJs, Function, IntoJs, Result, Value,
};
use tokio::fs;

use crate::errors::{create_fs_error, defer_fs_callback, throw_fs_error};

// The Stats implementation is very much based on Unix. The Windows implementation
// tries its best to mimic the implementation of libuv since it is the standard.
// See: https://github.com/libuv/libuv/blob/90648ea3e55125a5a819b32106da6462da310da6/src/win/fs.c
//
// By comparison, the Deno implementation is very basic and doesn't even try much.
// See: https://github.com/denoland/deno/blob/c9da27e147d0681724dd647593abbaa46417feb7/ext/io/fs.rs#L114-L182
//
// This implementation doesn't handle files created before UNIX_EPOCH in all non-Unix cases.

#[derive(Clone, Copy, Default)]
pub(crate) struct StatOptions {
    pub bigint: bool,
    pub throw_if_no_entry: bool,
}

pub(crate) fn parse_stat_options(options: Option<Value<'_>>) -> StatOptions {
    let mut parsed = StatOptions {
        bigint: false,
        throw_if_no_entry: true,
    };
    let Some(options) = options else {
        return parsed;
    };
    if options.is_undefined() || options.is_null() {
        return parsed;
    }
    let Some(obj) = options.as_object() else {
        return parsed;
    };
    if let Ok(Some(bigint)) = obj.get::<_, Option<bool>>("bigint") {
        parsed.bigint = bigint;
    }
    if let Ok(Some(throw_if_no_entry)) = obj.get::<_, Option<bool>>("throwIfNoEntry") {
        parsed.throw_if_no_entry = throw_if_no_entry;
    }
    parsed
}

pub(crate) fn metadata_to_stats<'js>(
    ctx: &Ctx<'js>,
    metadata: Metadata,
    bigint: bool,
) -> Result<Value<'js>> {
    if bigint {
        BigIntStats::new(metadata).into_js(ctx)
    } else {
        Stats::new(metadata).into_js(ctx)
    }
}

pub(crate) fn zeroed_stats<'js>(ctx: &Ctx<'js>, bigint: bool) -> Result<Value<'js>> {
    if bigint {
        BigIntStats::zeroed().into_js(ctx)
    } else {
        Stats::zeroed().into_js(ctx)
    }
}

fn meta_dev(meta: Option<&Metadata>) -> u64 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.dev()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0
    }
}

fn meta_ino(meta: Option<&Metadata>) -> u64 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0
    }
}

fn meta_mode(meta: Option<&Metadata>) -> u32 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.mode()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0o666
    }
}

fn meta_nlink(meta: Option<&Metadata>) -> u64 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.nlink()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        1
    }
}

fn meta_uid(meta: Option<&Metadata>) -> u32 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.uid()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0
    }
}

fn meta_gid(meta: Option<&Metadata>) -> u32 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.gid()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0
    }
}

fn meta_rdev(meta: Option<&Metadata>) -> u64 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.rdev()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0
    }
}

fn meta_size(meta: Option<&Metadata>) -> u64 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.size()
    }
    #[cfg(windows)]
    {
        if m.is_dir() {
            0
        } else {
            m.file_size()
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = m;
        0
    }
}

fn meta_blksize(meta: Option<&Metadata>) -> u64 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.blksize()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        4096
    }
}

fn meta_blocks(meta: Option<&Metadata>) -> u64 {
    let Some(m) = meta else { return 0 };
    #[cfg(unix)]
    {
        m.blocks()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0
    }
}

fn system_time_to_nanos(time: SystemTime) -> i64 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64)
            .saturating_mul(1_000_000_000)
            .saturating_add(d.subsec_nanos() as i64),
        Err(e) => {
            let d = e.duration();
            -(d.as_secs() as i64)
                .saturating_mul(1_000_000_000)
                .saturating_sub(d.subsec_nanos() as i64)
        },
    }
}

fn meta_atime_ns(meta: Option<&Metadata>, ctx: &Ctx<'_>) -> Result<i64> {
    let Some(m) = meta else { return Ok(0) };
    #[cfg(unix)]
    {
        let _ = ctx;
        Ok(m.atime()
            .saturating_mul(1_000_000_000)
            .saturating_add(m.atime_nsec()))
    }
    #[cfg(not(unix))]
    {
        m.accessed().map(system_time_to_nanos).or_throw(ctx)
    }
}

fn meta_mtime_ns(meta: Option<&Metadata>, ctx: &Ctx<'_>) -> Result<i64> {
    let Some(m) = meta else { return Ok(0) };
    #[cfg(unix)]
    {
        let _ = ctx;
        Ok(m.mtime()
            .saturating_mul(1_000_000_000)
            .saturating_add(m.mtime_nsec()))
    }
    #[cfg(not(unix))]
    {
        m.modified().map(system_time_to_nanos).or_throw(ctx)
    }
}

fn meta_ctime_ns(meta: Option<&Metadata>, ctx: &Ctx<'_>) -> Result<i64> {
    let Some(m) = meta else { return Ok(0) };
    #[cfg(unix)]
    {
        let _ = ctx;
        Ok(m.ctime()
            .saturating_mul(1_000_000_000)
            .saturating_add(m.ctime_nsec()))
    }
    #[cfg(not(unix))]
    {
        m.modified().map(system_time_to_nanos).or_throw(ctx)
    }
}

fn meta_birthtime_ns(meta: Option<&Metadata>) -> i64 {
    let Some(m) = meta else { return 0 };
    match m.created() {
        Ok(created) => system_time_to_nanos(created),
        Err(_) => 0,
    }
}

fn meta_atime(meta: Option<&Metadata>, ctx: &Ctx<'_>) -> Result<SystemTime> {
    let Some(m) = meta else {
        return Ok(SystemTime::UNIX_EPOCH);
    };
    m.accessed().or_throw(ctx)
}

fn meta_mtime(meta: Option<&Metadata>, ctx: &Ctx<'_>) -> Result<SystemTime> {
    let Some(m) = meta else {
        return Ok(SystemTime::UNIX_EPOCH);
    };
    m.modified().or_throw(ctx)
}

fn meta_ctime(meta: Option<&Metadata>, ctx: &Ctx<'_>) -> Result<SystemTime> {
    let Some(m) = meta else {
        return Ok(SystemTime::UNIX_EPOCH);
    };
    #[cfg(unix)]
    {
        let _ = ctx;
        // `ctime_nsec` is the sub-second part only; the seconds come from
        // `ctime()`. Using the nanoseconds alone gave a time in 1970.
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(m.ctime().max(0) as u64)
            + Duration::from_nanos(m.ctime_nsec().max(0) as u64))
    }
    #[cfg(not(unix))]
    {
        m.modified().or_throw(ctx)
    }
}

fn meta_birthtime(meta: Option<&Metadata>) -> SystemTime {
    let Some(m) = meta else {
        return SystemTime::UNIX_EPOCH;
    };
    m.created().unwrap_or(SystemTime::UNIX_EPOCH)
}

#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct Stats {
    #[qjs(skip_trace)]
    metadata: Option<Metadata>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Stats {
    #[qjs(skip)]
    pub fn new(metadata: Metadata) -> Self {
        Self {
            metadata: Some(metadata),
        }
    }

    #[qjs(skip)]
    pub fn zeroed() -> Self {
        Self { metadata: None }
    }

    #[qjs(get, enumerable)]
    pub fn dev(&self) -> u64 {
        meta_dev(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn ino(&self) -> u64 {
        meta_ino(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn mode(&self) -> u32 {
        meta_mode(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn nlink(&self) -> u64 {
        meta_nlink(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn uid(&self) -> u32 {
        meta_uid(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn gid(&self) -> u32 {
        meta_gid(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn rdev(&self) -> u64 {
        meta_rdev(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn size(&self) -> u64 {
        meta_size(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn blksize(&self) -> u64 {
        meta_blksize(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn blocks(&self) -> u64 {
        meta_blocks(self.metadata.as_ref())
    }

    #[qjs(get, enumerable)]
    pub fn atime_ms(&self, ctx: Ctx<'_>) -> Result<f64> {
        let Some(m) = self.metadata.as_ref() else {
            return Ok(0.0);
        };
        #[cfg(unix)]
        {
            let _ = ctx;
            Ok(ms_from_parts(m.atime(), m.atime_nsec()))
        }
        #[cfg(not(unix))]
        {
            m.accessed().map(to_msec_f64).or_throw(&ctx)
        }
    }

    #[qjs(get, enumerable)]
    pub fn mtime_ms(&self, ctx: Ctx<'_>) -> Result<f64> {
        let Some(m) = self.metadata.as_ref() else {
            return Ok(0.0);
        };
        #[cfg(unix)]
        {
            let _ = ctx;
            Ok(ms_from_parts(m.mtime(), m.mtime_nsec()))
        }
        #[cfg(not(unix))]
        {
            m.modified().map(to_msec_f64).or_throw(&ctx)
        }
    }

    #[qjs(get, enumerable)]
    pub fn ctime_ms(&self, ctx: Ctx<'_>) -> Result<f64> {
        let Some(m) = self.metadata.as_ref() else {
            return Ok(0.0);
        };
        #[cfg(unix)]
        {
            let _ = ctx;
            Ok(ms_from_parts(m.ctime(), m.ctime_nsec()))
        }
        #[cfg(not(unix))]
        {
            m.modified().map(to_msec_f64).or_throw(&ctx)
        }
    }

    /// Milliseconds since the epoch, with sub-millisecond precision, matching
    /// Node. When the filesystem cannot report a creation time, Node returns
    /// epoch zero rather than falling back to `ctime` — measured on `/proc` and
    /// `/sys` files, where `birthtimeMs` is `0` and `birthtime.getTime()` is `0`.
    #[qjs(get, enumerable)]
    pub fn birthtime_ms(&self, ctx: Ctx<'_>) -> Result<f64> {
        let _ = &ctx;
        let Some(m) = self.metadata.as_ref() else {
            return Ok(0.0);
        };
        Ok(match m.created() {
            Ok(created) => to_msec_f64(created),
            Err(_) => 0.0,
        })
    }

    #[qjs(get, enumerable)]
    pub fn atime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        meta_atime(self.metadata.as_ref(), &ctx)
    }

    #[qjs(get, enumerable)]
    pub fn mtime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        meta_mtime(self.metadata.as_ref(), &ctx)
    }

    #[qjs(get, enumerable)]
    pub fn ctime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        meta_ctime(self.metadata.as_ref(), &ctx)
    }

    /// The creation time, or the epoch when the filesystem does not report one
    /// — same fallback as [`Self::birthtime_ms`], and the same value Node gives.
    #[qjs(get, enumerable)]
    pub fn birthtime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        let _ = &ctx;
        Ok(meta_birthtime(self.metadata.as_ref()))
    }

    pub fn is_file(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_file)
    }

    /// @deprecated Use `is_directory` instead
    pub fn is_dir(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_dir)
    }

    pub fn is_directory(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_dir)
    }

    /// @deprecated Use `is_symbolic_link` instead
    pub fn is_symlink(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_symlink)
    }

    pub fn is_symbolic_link(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_symlink)
    }

    #[qjs(rename = "isFIFO")]
    pub fn is_fifo(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata
                .as_ref()
                .is_some_and(|m| m.file_type().is_fifo())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_block_device(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata
                .as_ref()
                .is_some_and(|m| m.file_type().is_block_device())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_character_device(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata
                .as_ref()
                .is_some_and(|m| m.file_type().is_char_device())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_socket(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata
                .as_ref()
                .is_some_and(|m| m.file_type().is_socket())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct BigIntStats {
    #[qjs(skip_trace)]
    metadata: Option<Metadata>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl BigIntStats {
    #[qjs(skip)]
    pub fn new(metadata: Metadata) -> Self {
        Self {
            metadata: Some(metadata),
        }
    }

    #[qjs(skip)]
    pub fn zeroed() -> Self {
        Self { metadata: None }
    }

    #[qjs(get, enumerable)]
    pub fn dev<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, meta_dev(self.metadata.as_ref()))
    }

    #[qjs(get, enumerable)]
    pub fn ino<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, meta_ino(self.metadata.as_ref()))
    }

    #[qjs(get, enumerable)]
    pub fn mode<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, u64::from(meta_mode(self.metadata.as_ref())))
    }

    #[qjs(get, enumerable)]
    pub fn nlink<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, meta_nlink(self.metadata.as_ref()))
    }

    #[qjs(get, enumerable)]
    pub fn uid<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, u64::from(meta_uid(self.metadata.as_ref())))
    }

    #[qjs(get, enumerable)]
    pub fn gid<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, u64::from(meta_gid(self.metadata.as_ref())))
    }

    #[qjs(get, enumerable)]
    pub fn rdev<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, meta_rdev(self.metadata.as_ref()))
    }

    #[qjs(get, enumerable)]
    pub fn size<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, meta_size(self.metadata.as_ref()))
    }

    #[qjs(get, enumerable)]
    pub fn blksize<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, meta_blksize(self.metadata.as_ref()))
    }

    #[qjs(get, enumerable)]
    pub fn blocks<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        BigInt::from_u64(ctx, meta_blocks(self.metadata.as_ref()))
    }

    #[qjs(get, enumerable)]
    pub fn atime_ms<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let ns = meta_atime_ns(self.metadata.as_ref(), &ctx)?;
        BigInt::from_i64(ctx, ns / 1_000_000)
    }

    #[qjs(get, enumerable)]
    pub fn mtime_ms<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let ns = meta_mtime_ns(self.metadata.as_ref(), &ctx)?;
        BigInt::from_i64(ctx, ns / 1_000_000)
    }

    #[qjs(get, enumerable)]
    pub fn ctime_ms<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let ns = meta_ctime_ns(self.metadata.as_ref(), &ctx)?;
        BigInt::from_i64(ctx, ns / 1_000_000)
    }

    #[qjs(get, enumerable)]
    pub fn birthtime_ms<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let ns = meta_birthtime_ns(self.metadata.as_ref());
        BigInt::from_i64(ctx, ns / 1_000_000)
    }

    #[qjs(get, enumerable)]
    pub fn atime_ns<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let ns = meta_atime_ns(self.metadata.as_ref(), &ctx)?;
        BigInt::from_i64(ctx, ns)
    }

    #[qjs(get, enumerable)]
    pub fn mtime_ns<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let ns = meta_mtime_ns(self.metadata.as_ref(), &ctx)?;
        BigInt::from_i64(ctx, ns)
    }

    #[qjs(get, enumerable)]
    pub fn ctime_ns<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let ns = meta_ctime_ns(self.metadata.as_ref(), &ctx)?;
        BigInt::from_i64(ctx, ns)
    }

    #[qjs(get, enumerable)]
    pub fn birthtime_ns<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let ns = meta_birthtime_ns(self.metadata.as_ref());
        BigInt::from_i64(ctx, ns)
    }

    #[qjs(get, enumerable)]
    pub fn atime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        meta_atime(self.metadata.as_ref(), &ctx)
    }

    #[qjs(get, enumerable)]
    pub fn mtime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        meta_mtime(self.metadata.as_ref(), &ctx)
    }

    #[qjs(get, enumerable)]
    pub fn ctime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        meta_ctime(self.metadata.as_ref(), &ctx)
    }

    #[qjs(get, enumerable)]
    pub fn birthtime(&self, ctx: Ctx<'_>) -> Result<SystemTime> {
        let _ = &ctx;
        Ok(meta_birthtime(self.metadata.as_ref()))
    }

    pub fn is_file(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_file)
    }

    pub fn is_dir(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_dir)
    }

    pub fn is_directory(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_dir)
    }

    pub fn is_symlink(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_symlink)
    }

    pub fn is_symbolic_link(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_symlink)
    }

    #[qjs(rename = "isFIFO")]
    pub fn is_fifo(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata
                .as_ref()
                .is_some_and(|m| m.file_type().is_fifo())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_block_device(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata
                .as_ref()
                .is_some_and(|m| m.file_type().is_block_device())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_character_device(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata
                .as_ref()
                .is_some_and(|m| m.file_type().is_char_device())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn is_socket(&self) -> bool {
        #[cfg(unix)]
        {
            self.metadata
                .as_ref()
                .is_some_and(|m| m.file_type().is_socket())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

async fn metadata_impl(path: &str, follow_symlinks: bool) -> std::io::Result<Metadata> {
    if follow_symlinks {
        fs::metadata(path).await
    } else {
        fs::symlink_metadata(path).await
    }
}

fn metadata_impl_sync(path: &str, follow_symlinks: bool) -> std::io::Result<Metadata> {
    if follow_symlinks {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    }
}

pub async fn stat_fn<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let opts = parse_stat_options(options.0);
    match metadata_impl(&path, true).await {
        Ok(metadata) => metadata_to_stats(&ctx, metadata, opts.bigint),
        Err(err) => Err(throw_fs_error(&ctx, err, "stat", &path)),
    }
}

pub fn stat_fn_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let opts = parse_stat_options(options.0);
    match metadata_impl_sync(&path, true) {
        Ok(metadata) => metadata_to_stats(&ctx, metadata, opts.bigint),
        Err(err) => {
            if !opts.throw_if_no_entry
                && matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                )
            {
                return Ok(Value::new_undefined(ctx));
            }
            Err(throw_fs_error(&ctx, err, "stat", &path))
        },
    }
}

pub async fn lstat_fn<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let opts = parse_stat_options(options.0);
    match metadata_impl(&path, false).await {
        Ok(metadata) => metadata_to_stats(&ctx, metadata, opts.bigint),
        Err(err) => Err(throw_fs_error(&ctx, err, "lstat", &path)),
    }
}

pub fn lstat_fn_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let opts = parse_stat_options(options.0);
    match metadata_impl_sync(&path, false) {
        Ok(metadata) => metadata_to_stats(&ctx, metadata, opts.bigint),
        Err(err) => {
            if !opts.throw_if_no_entry
                && matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                )
            {
                return Ok(Value::new_undefined(ctx));
            }
            Err(throw_fs_error(&ctx, err, "lstat", &path))
        },
    }
}

fn parse_stat_callback_args<'js>(
    ctx: &Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<(Option<Value<'js>>, Function<'js>)> {
    let mut args = args.0;
    match args.len() {
        0 => Err(Exception::throw_type(
            ctx,
            "The \"cb\" argument must be of type function",
        )),
        1 => {
            let only = args.remove(0);
            if only.as_function().is_some() {
                Ok((None, Function::from_js(ctx, only)?))
            } else {
                Err(Exception::throw_type(
                    ctx,
                    "The \"cb\" argument must be of type function",
                ))
            }
        },
        _ => {
            let options_or_cb = args.remove(0);
            if options_or_cb.as_function().is_some() {
                Ok((None, Function::from_js(ctx, options_or_cb)?))
            } else {
                let callback = Function::from_js(ctx, args.remove(0)).map_err(|_| {
                    Exception::throw_type(ctx, "The \"cb\" argument must be of type function")
                })?;
                Ok((Some(options_or_cb), callback))
            }
        },
    }
}

/// Callback-style `fs.stat(path[, options], callback)`.
pub fn stat_callback<'js>(ctx: Ctx<'js>, path: String, args: Rest<Value<'js>>) -> Result<()> {
    let (options, callback) = parse_stat_callback_args(&ctx, args)?;
    let opts = parse_stat_options(options);

    ctx.clone().spawn_exit_simple(async move {
        match metadata_impl(&path, true).await {
            Ok(metadata) => {
                let stats = metadata_to_stats(&ctx, metadata, opts.bigint)?;
                defer_fs_callback(&ctx, callback, None, Some(stats))?;
            },
            Err(err) => {
                let exception = create_fs_error(&ctx, err, "stat", &path)?;
                defer_fs_callback(&ctx, callback, Some(exception), None)?;
            },
        }
        Ok(())
    });

    Ok(())
}

/// Callback-style `fs.lstat(path[, options], callback)`.
pub fn lstat_callback<'js>(ctx: Ctx<'js>, path: String, args: Rest<Value<'js>>) -> Result<()> {
    let (options, callback) = parse_stat_callback_args(&ctx, args)?;
    let opts = parse_stat_options(options);

    ctx.clone().spawn_exit_simple(async move {
        match metadata_impl(&path, false).await {
            Ok(metadata) => {
                let stats = metadata_to_stats(&ctx, metadata, opts.bigint)?;
                defer_fs_callback(&ctx, callback, None, Some(stats))?;
            },
            Err(err) => {
                let exception = create_fs_error(&ctx, err, "lstat", &path)?;
                defer_fs_callback(&ctx, callback, Some(exception), None)?;
            },
        }
        Ok(())
    });

    Ok(())
}

/// Milliseconds since the epoch as a float, keeping sub-millisecond precision.
///
/// Node's `*Ms` fields are floats, not integers: setting a timestamp of
/// `1234567890.123456` gives `1234567890123.456`, while the matching `Date`'s
/// `getTime()` truncates to `1234567890123`. The two are deliberately not equal.
#[allow(dead_code)]
fn to_msec_f64(time: SystemTime) -> f64 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64() * 1000.0,
        Err(e) => -(e.duration().as_secs_f64() * 1000.0),
    }
}

/// Build epoch milliseconds from a `time_t` seconds part and a nanoseconds part.
///
/// `MetadataExt::*_nsec` gives only the sub-second remainder, so the seconds must
/// be added separately — dropping them put every timestamp in 1970.
#[cfg(unix)]
fn ms_from_parts(secs: i64, nsec: i64) -> f64 {
    secs as f64 * 1000.0 + nsec as f64 / 1_000_000.0
}
