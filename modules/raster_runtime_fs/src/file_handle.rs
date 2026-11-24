// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::uninlined_format_args)]

use std::borrow::Cow;
use std::io;
use std::path::PathBuf;

use either::Either;
use raster_runtime_buffer::{ArrayBufferView, Buffer};
use raster_runtime_encoding::Encoder;
use raster_runtime_utils::{
    object::ObjectExt,
    result::{OptionExt, ResultExt},
};
use rquickjs::function::Opt;
use rquickjs::{Ctx, Error, Exception, FromJs, Null, Object, Result, Value};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};

use super::{read_file, Stats};

const DEFAULT_BUFFER_SIZE: usize = 16384;
const DEFAULT_ENCODING: &str = "utf8";

/// CRT open flags matching MSVC `<fcntl.h>` (used by `_open_osfhandle`).
/// Defined on all platforms so unit tests can lock the mapping without Windows.
pub mod crt_flags {
    pub const O_RDONLY: i32 = 0x0000;
    pub const O_WRONLY: i32 = 0x0001;
    pub const O_RDWR: i32 = 0x0002;
    pub const O_APPEND: i32 = 0x0008;
    pub const O_BINARY: i32 = 0x8000;
}

/// Map Node-style open flags to CRT `_open_osfhandle` access flags.
pub fn crt_flags_for_node_open(flags: &str) -> i32 {
    use crt_flags::*;
    match flags {
        "r" => O_RDONLY | O_BINARY,
        "r+" => O_RDWR | O_BINARY,
        "w" | "wx" => O_WRONLY | O_BINARY,
        "w+" | "wx+" => O_RDWR | O_BINARY,
        "a" | "ax" => O_WRONLY | O_APPEND | O_BINARY,
        "a+" => O_RDWR | O_APPEND | O_BINARY,
        // Default read (same as open default "r").
        _ => O_RDONLY | O_BINARY,
    }
}

#[cfg(windows)]
extern "C" {
    fn _open_osfhandle(handle: isize, flags: i32) -> i32;
    fn _close(fd: i32) -> i32;
    fn _get_osfhandle(fd: i32) -> isize;
}

/// Owns a CRT file descriptor created via `_open_osfhandle` on a duplicated HANDLE.
#[cfg(windows)]
struct OwnedCrtFd(i32);

#[cfg(windows)]
impl Drop for OwnedCrtFd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            unsafe {
                _close(self.0);
            }
            self.0 = -1;
        }
    }
}

#[cfg(windows)]
fn duplicate_as_crt_fd(file: &File, crt_flags: i32) -> io::Result<OwnedCrtFd> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{
        CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let src = file.as_raw_handle() as HANDLE;
        let mut dup: HANDLE = std::ptr::null_mut();
        let ok = DuplicateHandle(
            GetCurrentProcess(),
            src,
            GetCurrentProcess(),
            &mut dup,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        );
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = _open_osfhandle(dup as isize, crt_flags);
        if fd < 0 {
            CloseHandle(dup);
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "_open_osfhandle failed",
            ));
        }
        // CRT now owns `dup`; do not CloseHandle(dup).
        Ok(OwnedCrtFd(fd))
    }
}

#[allow(dead_code)]
#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct FileHandle {
    #[qjs(skip_trace)]
    file: Option<File>,
    #[qjs(skip_trace)]
    path: PathBuf,
    /// Real CRT file descriptor (Windows only); dropped via `_close`.
    #[cfg(windows)]
    #[qjs(skip_trace)]
    crt_fd: Option<OwnedCrtFd>,
}

/// A `dup` of a tokio `File`'s descriptor, sharing its file offset.
///
/// `tokio::fs::File::try_clone` is async; this only needs the descriptor
/// number, so it duplicates straight from the fd.
#[cfg(not(windows))]
fn try_clone_sync(file: &File) -> io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let raw = file.as_raw_fd();
    let dup = unsafe { libc::dup(raw) };
    if dup < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(dup) })
}

impl FileHandle {
    /// Create a FileHandle. On Windows, `crt_flags` must match open access
    /// (`crt_flags_for_node_open`); ignored on Unix.
    pub fn new(file: File, path: PathBuf) -> io::Result<Self> {
        Self::new_with_crt_flags(file, path, {
            #[cfg(windows)]
            {
                crt_flags::O_RDONLY | crt_flags::O_BINARY
            }
            #[cfg(not(windows))]
            {
                0
            }
        })
    }

    pub fn new_with_crt_flags(file: File, path: PathBuf, crt_flags: i32) -> io::Result<Self> {
        #[cfg(windows)]
        {
            let crt_fd = duplicate_as_crt_fd(&file, crt_flags)?;
            Ok(Self {
                file: Some(file),
                path,
                crt_fd: Some(crt_fd),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = crt_flags;
            // Register the descriptor so the sync fd family (`readSync` and
            // friends) works on a handle opened through `fs.promises`. The
            // duplicate shares the file offset, which is what Node does: measured
            // there, `readSync` and `handle.read` advance the same cursor.
            if let Ok(shared) = try_clone_sync(&file) {
                use std::os::fd::AsRawFd;
                crate::fd_sync::register(file.as_raw_fd(), shared, path.clone());
            }
            Ok(Self {
                file: Some(file),
                path,
            })
        }
    }

    fn file(&self, ctx: &Ctx<'_>) -> Result<&File> {
        self.file.as_ref().or_throw_msg(ctx, "FileHandle is closed")
    }

    fn file_mut(&mut self, ctx: &Ctx<'_>) -> Result<&mut File> {
        self.file.as_mut().or_throw_msg(ctx, "FileHandle is closed")
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl FileHandle {
    #[allow(unused_variables)]
    async fn chmod(&self, ctx: Ctx<'_>, mode: u32) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perm = std::fs::Permissions::from_mode(mode);
            self.file(&ctx)?
                .set_permissions(perm)
                .await
                .or_throw_msg(&ctx, "Can't modify file permissions")?;
        }
        Ok(())
    }

    #[allow(unused_variables)]
    async fn chown(&self, ctx: Ctx<'_>, uid: u32, gid: u32) -> Result<()> {
        #[cfg(unix)]
        {
            let path = self.path.clone();
            tokio::task::spawn_blocking(move || {
                std::os::unix::fs::chown(&path, Some(uid), Some(gid))
            })
            .await
            .or_throw(&ctx)?
            .or_throw_msg(&ctx, "Can't modify file owner")?;
        }
        Ok(())
    }

    async fn close(&mut self) {
        // Drop CRT fd first (closes the duplicated HANDLE), then the Rust File.
        #[cfg(windows)]
        {
            self.crt_fd = None;
        }
        if let Some(file) = self.file.take() {
            drop(file.into_std().await);
        }
    }

    async fn datasync(&self, ctx: Ctx<'_>) -> Result<()> {
        self.file(&ctx)?
            .sync_data()
            .await
            .or_throw_msg(&ctx, "Can't sync file data")?;
        Ok(())
    }

    /// Synchronous file descriptor (Node `filehandle.fd`).
    ///
    /// Unix: real OS fd. Windows: real CRT fd from `_open_osfhandle` on a
    /// duplicated HANDLE (interoperable; not a truncated pointer cast).
    #[qjs(get)]
    fn fd(&self) -> Result<i32> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            self.file
                .as_ref()
                .map(AsRawFd::as_raw_fd)
                .ok_or_else(|| Error::new_from_js("closed FileHandle", "fd"))
        }
        #[cfg(windows)]
        {
            self.crt_fd
                .as_ref()
                .map(|fd| fd.0)
                .ok_or_else(|| Error::new_from_js("closed FileHandle", "fd"))
        }
        #[cfg(not(any(unix, windows)))]
        {
            if self.file.is_none() {
                return Err(Error::new_from_js("closed FileHandle", "fd"));
            }
            Ok(0)
        }
    }

    async fn read<'js>(
        &mut self,
        ctx: Ctx<'js>,
        buffer_or_options: Opt<Either<ArrayBufferView<'js>, ReadOptions<'js>>>,
        options_or_offset: Opt<Either<ReadOptions<'js>, usize>>,
        length: Opt<usize>,
        position: Opt<Option<u64>>, // -1 is not supported
    ) -> Result<Object<'js>> {
        let options_1 = match buffer_or_options.0 {
            Some(Either::Left(buffer)) => ReadOptions {
                buffer: Some(buffer),
                ..Default::default()
            },
            Some(Either::Right(options)) => options,
            None => ReadOptions::default(),
        };
        let options_2 = match options_or_offset.0 {
            Some(Either::Left(options)) => options,
            Some(Either::Right(offset)) => ReadOptions {
                offset: Some(offset),
                ..Default::default()
            },
            None => ReadOptions::default(),
        };

        let mut buffer = options_1
            .buffer
            .or(options_2.buffer)
            .unwrap_or_else_ok(|| {
                ArrayBufferView::from_buffer(&ctx, Buffer::alloc(DEFAULT_BUFFER_SIZE))
            })?;
        let offset = options_1.offset.or(options_2.offset).unwrap_or(0);
        let length = options_1
            .length
            .or(options_2.length)
            .or(length.0)
            .unwrap_or_else(|| buffer.len() - offset);
        let position = options_1
            .position
            .or(options_2.position)
            .or(position.0.flatten());
        validate_length_offset(&ctx, length, offset, buffer.len())?;

        // It is not safe to pass the buffer from `ArrayBufferView` to `File::read`
        // since the read is done in a different thread and we cannot garantee
        // that multiple read calls are not done with the same buffer.
        // Ideally, we should make our own version of `BufReader` to reuse the buffer
        // instead of doing an allocation on each read.
        let mut buf = vec![0u8; length];
        let file = self.file_mut(&ctx)?;

        // Tokio doesn't offer an API for positional reads. This means we have
        // to seek to the position, read the file, and then seek back to the original
        // position. See https://github.com/tokio-rs/tokio/issues/699
        let mut cursor = None;
        if let Some(position) = position {
            cursor = Some(
                file.seek(SeekFrom::Current(0))
                    .await
                    .or_throw_msg(&ctx, "Can't get cursor")?,
            );
            file.seek(SeekFrom::Start(position))
                .await
                .or_throw_msg(&ctx, "Can't seek file")?;
        }

        let bytes_read = file
            .read(&mut buf)
            .await
            .or_throw_msg(&ctx, "Failed to read file")?;

        // Reset the file at the original position. If there is an error while
        // resetting the cursor, we close the file pre-emptively since future
        // reads would be invalid.
        if let Some(cursor) = cursor {
            if let Err(err) = file
                .seek(SeekFrom::Start(cursor))
                .await
                .or_throw_msg(&ctx, "Failed to reset cursor")
            {
                self.close().await;
                return Err(err);
            }
        }

        let dst_buf = buffer
            .as_bytes_mut()
            .or_throw_msg(&ctx, "Buffer is detached")?;
        dst_buf[offset..].copy_from_slice(&buf);

        let result = Object::new(ctx)?;
        result.set("bytesRead", bytes_read)?;
        result.set("buffer", buffer)?;
        Ok(result)
    }

    async fn read_file<'js>(
        &mut self,
        ctx: Ctx<'js>,
        options: Opt<Either<String, read_file::ReadFileOptions>>,
    ) -> Result<Value<'js>> {
        let size = self
            .file(&ctx)?
            .metadata()
            .await
            .map(|m| m.len() as usize)
            .ok();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size.unwrap_or(0))
            .or_throw_msg(&ctx, "Out of memory")?;

        self.file_mut(&ctx)?
            .read_to_end(&mut bytes)
            .await
            .or_throw_msg(&ctx, "Failed to read file")?;
        read_file::handle_read_file_bytes(&ctx, options, bytes)
    }

    async fn stat(&self, ctx: Ctx<'_>) -> Result<Stats> {
        let metadata = self
            .file(&ctx)?
            .metadata()
            .await
            .or_throw_msg(&ctx, "Can't stat file")?;
        Ok(Stats::new(metadata))
    }

    async fn sync(&self, ctx: Ctx<'_>) -> Result<()> {
        self.file(&ctx)?
            .sync_all()
            .await
            .or_throw_msg(&ctx, "Can't sync file")
    }

    async fn truncate(&mut self, ctx: Ctx<'_>, len: Opt<u64>) -> Result<()> {
        let len = len.0.unwrap_or(0);
        self.file_mut(&ctx)?
            .set_len(len)
            .await
            .or_throw_msg(&ctx, "Can't truncate file")
    }

    // Setting times not supported in tokio
    // See https://github.com/tokio-rs/tokio/issues/6368
    // async fn utimes(&mut self,  ctx: Ctx<'_>, atime: Value<'_>, mtime: Value<'_>) -> Result<()>

    async fn write<'js>(
        &mut self,
        ctx: Ctx<'js>,
        buffer_or_string: Either<ArrayBufferView<'js>, String>,
        offset_or_options_or_position: Opt<Either<Either<usize, Null>, WriteOptions>>,
        length_or_encoding: Opt<Either<usize, String>>,
        position: Opt<Option<u64>>,
    ) -> Result<Object<'js>> {
        let mut options = match offset_or_options_or_position.0 {
            Some(Either::Left(Either::Left(offset_or_position))) => {
                if buffer_or_string.is_left() {
                    WriteOptions {
                        offset: Some(offset_or_position),
                        ..Default::default()
                    }
                } else {
                    WriteOptions::default()
                }
            },
            Some(Either::Right(options)) => options,
            _ => WriteOptions::default(),
        };
        if let Some(Either::Left(length)) = length_or_encoding.0 {
            options.length = Some(length);
        }

        let buffer = match &buffer_or_string {
            Either::Left(buffer) => {
                let buffer = buffer.as_bytes().or_throw_msg(&ctx, "Buffer is detached")?;
                Cow::Borrowed(buffer)
            },
            Either::Right(string) => {
                let encoding = length_or_encoding
                    .0
                    .and_then(|e| e.right())
                    .unwrap_or_else(|| DEFAULT_ENCODING.to_string());
                let buffer = Encoder::from_str(&encoding)
                    .and_then(|enc| enc.decode_from_string(string.clone()))
                    .or_throw(&ctx)?;
                Cow::Owned(buffer)
            },
        };

        let offset = options.offset.unwrap_or(0);
        let length = options.length.unwrap_or(buffer.len() - offset);
        let position = options.position.or(position.0.flatten());
        validate_length_offset(&ctx, length, offset, buffer.len())?;

        let file = self.file_mut(&ctx)?;

        // Tokio doesn't offer an API for positional writes. This means we have
        // to seek to the position, write to the file, and then seek back to the original
        // position. See https://github.com/tokio-rs/tokio/issues/699
        let mut cursor = None;
        if let Some(position) = position {
            cursor = Some(
                file.seek(SeekFrom::Current(0))
                    .await
                    .or_throw_msg(&ctx, "Can't get cursor")?,
            );
            file.seek(SeekFrom::Start(position))
                .await
                .or_throw_msg(&ctx, "Can't seek file")?;
        }

        file.write_all(&buffer[offset..length])
            .await
            .or_throw_msg(&ctx, "Failed to write to file")?;

        // Reset the file at the original position. If there is an error while
        // resetting the cursor, we close the file pre-emptively since future
        // writes would be invalid.
        if let Some(cursor) = cursor {
            if let Err(err) = file
                .seek(SeekFrom::Start(cursor))
                .await
                .or_throw_msg(&ctx, "Failed to reset cursor")
            {
                self.close().await;
                return Err(err);
            }
        }

        let result = Object::new(ctx)?;
        result.set("bytesWritten", length)?;
        result.set("buffer", buffer_or_string)?;
        Ok(result)
    }

    async fn write_file<'js>(
        &mut self,
        ctx: Ctx<'js>,
        data: Either<ArrayBufferView<'js>, String>,
        options_or_encoding: Opt<Either<WriteFileOptions, String>>,
    ) -> Result<()> {
        let file = self.file_mut(&ctx)?;

        // Always overwrite the whole file
        file.set_len(0)
            .await
            .or_throw_msg(&ctx, "Failed to truncate file")?;

        let encoding = match options_or_encoding.0 {
            Some(Either::Left(options)) => options.encoding,
            Some(Either::Right(encoding)) => Some(encoding),
            _ => None,
        }
        .unwrap_or_else(|| DEFAULT_ENCODING.to_string());

        let buffer = match &data {
            Either::Left(buffer) => {
                let buffer = buffer.as_bytes().or_throw_msg(&ctx, "Buffer is detached")?;
                Cow::Borrowed(buffer)
            },
            Either::Right(string) => {
                let buffer = Encoder::from_str(&encoding)
                    .and_then(|enc| enc.decode_from_string(string.clone()))
                    .or_throw(&ctx)?;
                Cow::Owned(buffer)
            },
        };

        file.write_all(&buffer)
            .await
            .or_throw_msg(&ctx, "Failed to write to file")?;
        Ok(())
    }
}

fn validate_length_offset(
    ctx: &Ctx<'_>,
    length: usize,
    offset: usize,
    buffer_length: usize,
) -> Result<()> {
    if offset > buffer_length {
        return Err(Exception::throw_range(
            ctx,
            &format!("offset ({}) <= {}", offset, buffer_length),
        ));
    }
    if length > buffer_length - offset {
        return Err(Exception::throw_range(
            ctx,
            &format!("length ({}) <= {}", length, buffer_length - offset),
        ));
    }
    Ok(())
}

#[derive(Default)]
struct ReadOptions<'js> {
    buffer: Option<ArrayBufferView<'js>>,
    offset: Option<usize>,
    length: Option<usize>,
    position: Option<u64>,
}

impl<'js> FromJs<'js> for ReadOptions<'js> {
    fn from_js(_ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let ty_name = value.type_name();
        let obj = value
            .as_object()
            .ok_or(Error::new_from_js(ty_name, "Object"))?;

        let buffer = obj.get_optional::<_, ArrayBufferView<'js>>("buffer")?;
        let offset = obj.get_optional::<_, usize>("offset")?;
        let length = obj.get_optional::<_, usize>("length")?;
        let position = obj.get_optional::<_, u64>("position")?;

        Ok(Self {
            buffer,
            offset,
            length,
            position,
        })
    }
}

#[derive(Default)]
struct WriteOptions {
    offset: Option<usize>,
    length: Option<usize>,
    position: Option<u64>,
}

impl<'js> FromJs<'js> for WriteOptions {
    fn from_js(_ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let ty_name = value.type_name();
        let obj = value
            .as_object()
            .ok_or(Error::new_from_js(ty_name, "Object"))?;

        let offset = obj.get_optional::<_, usize>("offset")?;
        let length = obj.get_optional::<_, usize>("length")?;
        let position = obj.get_optional::<_, u64>("position")?;

        Ok(Self {
            offset,
            length,
            position,
        })
    }
}

#[derive(Default)]
struct WriteFileOptions {
    encoding: Option<String>,
}

impl<'js> FromJs<'js> for WriteFileOptions {
    fn from_js(_ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let ty_name = value.type_name();
        let obj = value
            .as_object()
            .ok_or(Error::new_from_js(ty_name, "Object"))?;

        let encoding = obj.get_optional::<_, String>("encoding")?;

        Ok(Self { encoding })
    }
}

#[cfg(test)]
mod tests {
    use raster_runtime_buffer as buffer;
    use raster_runtime_test::{call_test, call_test_err, test_async_with, ModuleEvaluator};
    use rquickjs::{CatchResultExt, CaughtError};
    use tokio::fs::OpenOptions;

    use super::*;

    async fn given_file(content: &str, options: &mut OpenOptions) -> (File, PathBuf) {
        // Create file
        let path = raster_runtime_test::given_file(content).await;

        // Open in right mode
        let file = options.open(&path).await.unwrap();
        (file, path)
    }

    #[tokio::test]
    async fn test_file_handle_read() {
        let (file, path) = given_file("Hello World", OpenOptions::new().read(true)).await;
        let path_1 = path.clone();

        test_async_with(|ctx| {
            Box::pin(async move {
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            const buffer = new ArrayBuffer(4096);
                            const view = new Uint8Array(buffer);
                            const read = await filehandle.read(view);
                            return Array.from(view);
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<Vec<u8>, _>(
                    &ctx,
                    &module,
                    (FileHandle::new(file, path_1).unwrap(),),
                )
                .await;

                assert!(result.starts_with(b"Hello World"));
            })
        })
        .await;

        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_file_handle_read_concurrent() {
        let (file_a, path_a) = given_file(&"a".repeat(20000), OpenOptions::new().read(true)).await;
        let (file_b, path_b) = given_file(&"b".repeat(20000), OpenOptions::new().read(true)).await;
        let path_a_1 = path_a.clone();
        let path_b_1 = path_b.clone();

        test_async_with(|ctx| {
            Box::pin(async move {

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandleA, filehandleB) {
                            const buffer = new ArrayBuffer(10000);
                            const view = new Uint8Array(buffer);
                            const read = await Promise.all([filehandleA.read(view), filehandleB.read(view)]);
                            return Array.from(view);
                        }
                    "#,
                )
                .await
                .unwrap();

                let result =
                    call_test::<Vec<u8>, _>(&ctx, &module, (FileHandle::new(file_a, path_a_1).unwrap(), FileHandle::new(file_b, path_b_1).unwrap())).await;

                assert_eq!(result.len(), 10000);
                if result.iter().all(|&b| b == b'a') {
                    println!("All a");
                } else if result.iter().all(|&b| b == b'b') {
                    println!("All b");
                } else {
                    println!("Mixed");
                }
            })
        })
        .await;

        tokio::fs::remove_file(&path_a).await.unwrap();
        tokio::fs::remove_file(&path_b).await.unwrap();
    }

    #[tokio::test]
    async fn test_file_handle_read_position() {
        let (file, path) = given_file("Hello World", OpenOptions::new().read(true)).await;
        let path_1 = path.clone();

        test_async_with(|ctx| {
            Box::pin(async move {
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            const buffer = new ArrayBuffer(4096);
                            const view = new Uint8Array(buffer);
                            await filehandle.read(view, { position: 6 });
                            await filehandle.read(view, { offset: 5 });
                            return Array.from(view);
                        }
                    "#,
                )
                .await
                .catch(&ctx)
                .unwrap();

                let result = call_test::<Vec<u8>, _>(
                    &ctx,
                    &module,
                    (FileHandle::new(file, path_1).unwrap(),),
                )
                .await;

                assert!(result.starts_with(b"WorldHello World"));
            })
        })
        .await;

        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_file_handle_read_subarray() {
        let (file, path) = given_file("Hello World", OpenOptions::new().read(true)).await;
        let path_1 = path.clone();

        test_async_with(|ctx| {
            Box::pin(async move {
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            const buffer = new ArrayBuffer(4096);
                            const view = new Uint8Array(buffer);
                            const subarray = view.subarray(3, 8);
                            const read = await filehandle.read(subarray);
                            return Array.from(view);
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<Vec<u8>, _>(
                    &ctx,
                    &module,
                    (FileHandle::new(file, path_1).unwrap(),),
                )
                .await;

                assert!(result.starts_with(b"\x00\x00\x00Hello\x00"));
            })
        })
        .await;

        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_file_handle_read_buffer() {
        let (file, path) = given_file("Hello World", OpenOptions::new().read(true)).await;
        let path_1 = path.clone();

        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            const buffer = new ArrayBuffer(4096);
                            const view = new Uint8Array(buffer);
                            await filehandle.read(view, { length: 2000, offset: 3000 });
                        }
                    "#,
                )
                .await
                .unwrap();

                let error = call_test_err::<(), _>(
                    &ctx,
                    &module,
                    (FileHandle::new(file, path_1).unwrap(),),
                )
                .await
                .unwrap_err();

                let CaughtError::Exception(exception) = error else {
                    panic!("Expected exception");
                };

                assert_eq!(exception.message().unwrap(), "length (2000) <= 1096");
            })
        })
        .await;

        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_file_handle_read_out_of_range() {
        let (file, path) = given_file("Hello World", OpenOptions::new().read(true)).await;
        let path_1 = path.clone();

        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            const buffer = Buffer.alloc(4096);
                            const read = await filehandle.read(buffer);
                            return Array.from(buffer);
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<Vec<u8>, _>(
                    &ctx,
                    &module,
                    (FileHandle::new(file, path_1).unwrap(),),
                )
                .await;

                assert!(result.starts_with(b"Hello World"));
            })
        })
        .await;

        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_file_handle_read_file() {
        let (file, path) = given_file("Hello World", OpenOptions::new().read(true)).await;
        let path_1 = path.clone();

        test_async_with(|ctx| {
            Box::pin(async move {
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            const data = await filehandle.readFile("utf8");
                            return data;
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<String, _>(
                    &ctx,
                    &module,
                    (FileHandle::new(file, path_1).unwrap(),),
                )
                .await;

                assert_eq!(result, "Hello World");
            })
        })
        .await;

        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_file_handle_write() {
        let (file, path) = given_file("", OpenOptions::new().write(true)).await;
        let path_1 = path.clone();

        test_async_with(|ctx| {
            Box::pin(async move {

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            const { bytesWritten } = await filehandle.write("Hello World", null, "utf8");
                            await filehandle.sync();
                            return bytesWritten;
                        }
                    "#,
                )
                .await
                .unwrap();

                let result =
                    call_test::<u32, _>(&ctx, &module, (FileHandle::new(file, path_1).unwrap(),)).await;

                assert_eq!(result, 11);
            })
        })
        .await;

        let file_content = tokio::fs::read(&path).await.unwrap();
        tokio::fs::remove_file(&path).await.unwrap();
        assert_eq!(file_content, b"Hello World");
    }

    #[tokio::test]
    async fn test_file_handle_write_position() {
        let (file, path) = given_file("", OpenOptions::new().write(true)).await;
        let path_1 = path.clone();
        test_async_with(|ctx| {
            Box::pin(async move {
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            const { bytesWritten } = await filehandle.write("Hello World", null, "utf8", 4);
                            await filehandle.write("a", null, "utf8");
                            await filehandle.sync();
                            return bytesWritten;
                        }
                    "#,
                )
                .await
                .unwrap();

                let result =
                    call_test::<u32, _>(&ctx, &module, (FileHandle::new(file, path_1).unwrap(),)).await;

                assert_eq!(result, 11);
            })
        })
        .await;

        let file_content = tokio::fs::read(&path).await.unwrap();
        tokio::fs::remove_file(&path).await.unwrap();
        assert_eq!(file_content, b"a\x00\x00\x00Hello World");
    }

    #[tokio::test]
    async fn test_file_handle_write_out_of_range() {
        let (file, path) = given_file("", OpenOptions::new().write(true)).await;
        let path_1 = path.clone();
        test_async_with(|ctx| {
            Box::pin(async move {
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            await filehandle.write("Hello World", { offset: 5, length: 20 });
                        }
                    "#,
                )
                .await
                .unwrap();

                let error = call_test_err::<(), _>(
                    &ctx,
                    &module,
                    (FileHandle::new(file, path_1).unwrap(),),
                )
                .await
                .unwrap_err();

                let CaughtError::Exception(exception) = error else {
                    panic!("Expected exception");
                };

                assert_eq!(exception.message().unwrap(), "length (20) <= 6");
            })
        })
        .await;

        let file_content = tokio::fs::read(&path).await.unwrap();
        tokio::fs::remove_file(&path).await.unwrap();
        assert_eq!(file_content, b"");
    }

    #[tokio::test]
    async fn test_file_handle_write_file() {
        let (file, path) = given_file(
            "Other very very very very long Data",
            OpenOptions::new().write(true),
        )
        .await;
        let path_1 = path.clone();
        test_async_with(|ctx| {
            Box::pin(async move {
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            await filehandle.writeFile("Hello World", "utf8");
                            await filehandle.sync();
                        }
                    "#,
                )
                .await
                .unwrap();

                call_test::<(), _>(&ctx, &module, (FileHandle::new(file, path_1).unwrap(),)).await;
            })
        })
        .await;

        let file_content = tokio::fs::read(&path).await.unwrap();
        tokio::fs::remove_file(&path).await.unwrap();
        assert_eq!(file_content, b"Hello World");
    }

    #[tokio::test]
    async fn test_file_handle_fd() {
        let (file, path) = given_file("", OpenOptions::new().read(true)).await;
        let path_1 = path.clone();
        test_async_with(|ctx| {
            Box::pin(async move {
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        export async function test(filehandle) {
                            return filehandle.fd;
                        }
                    "#,
                )
                .await
                .unwrap();

                let result =
                    call_test::<i32, _>(&ctx, &module, (FileHandle::new(file, path_1).unwrap(),))
                        .await;

                assert!(result > 0);
            })
        })
        .await;

        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[test]
    fn crt_flags_match_node_open_access() {
        use super::crt_flags::*;
        assert_eq!(crt_flags_for_node_open("r"), O_RDONLY | O_BINARY);
        assert_eq!(crt_flags_for_node_open("r+"), O_RDWR | O_BINARY);
        assert_eq!(crt_flags_for_node_open("w"), O_WRONLY | O_BINARY);
        assert_eq!(crt_flags_for_node_open("w+"), O_RDWR | O_BINARY);
        assert_eq!(crt_flags_for_node_open("a"), O_WRONLY | O_APPEND | O_BINARY);
        assert_eq!(crt_flags_for_node_open("a+"), O_RDWR | O_APPEND | O_BINARY);
        // Write modes must not be read-only.
        assert_ne!(crt_flags_for_node_open("w") & O_WRONLY, 0);
        assert_ne!(crt_flags_for_node_open("r+") & O_RDWR, 0);
    }

    /// Windows: fd must be a real CRT descriptor, not a truncated HANDLE cast.
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_crt_fd_is_valid_osfhandle() {
        use std::os::windows::io::AsRawHandle;
        use tokio::fs::OpenOptions;

        let (file, path) =
            given_file("crt-fd-test", OpenOptions::new().read(true).write(true)).await;
        let raw_handle = file.as_raw_handle() as isize;
        // RDWR matches r+ / w+ — not the default O_RDONLY used by FileHandle::new.
        let handle =
            FileHandle::new_with_crt_flags(file, path.clone(), crt_flags_for_node_open("r+"))
                .unwrap();
        let fd = handle.fd().unwrap();
        assert!(fd >= 0);
        // Must not be the raw HANDLE truncated into i32.
        let truncated = raw_handle as i32;
        assert_ne!(fd, truncated, "fd must not be a truncated RawHandle cast");
        let osf = unsafe { super::_get_osfhandle(fd) };
        assert_ne!(osf, -1, "_get_osfhandle(filehandle.fd) must succeed");
        // Closing FileHandle drops CRT fd.
        drop(handle);
        let osf_after = unsafe { super::_get_osfhandle(fd) };
        assert_eq!(osf_after, -1, "CRT fd invalid after FileHandle drop");
        tokio::fs::remove_file(&path).await.unwrap();
    }

    /// Windows: write-mode CRT fd allows CRT `_write` (not open read-only).
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_crt_fd_write_mode_allows_crt_write() {
        use std::ffi::c_void;
        use tokio::fs::OpenOptions;

        extern "C" {
            fn _write(fd: i32, buffer: *const c_void, count: u32) -> i32;
        }

        let (file, path) =
            given_file("", OpenOptions::new().read(true).write(true).truncate(true)).await;
        let handle =
            FileHandle::new_with_crt_flags(file, path.clone(), crt_flags_for_node_open("w+"))
                .unwrap();
        let fd = handle.fd().unwrap();
        let msg = b"crt-write";
        let n = unsafe { _write(fd, msg.as_ptr() as *const c_void, msg.len() as u32) };
        assert_eq!(n, msg.len() as i32, "CRT _write must succeed on RDWR fd");
        drop(handle);
        let content = tokio::fs::read(&path).await.unwrap();
        assert_eq!(&content[..msg.len()], msg.as_slice());
        tokio::fs::remove_file(&path).await.unwrap();
    }
}
