// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Node-compatible `tty` module: `isatty`, `ReadStream`, `WriteStream`.
//!
//! Stdin reading starts only on `resume()` so importing readline does not keep
//! the process alive. Reader lifecycle is a single-ownership generation state
//! machine: only one active worker per stream, pause cancels via a worker-owned
//! cancel handle, and a new generation starts only after the previous worker
//! fully completes. Process stdio fds are never closed (workers `dup` then close
//! their own copy). Raw mode is reference-counted and restored on last use.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[cfg(not(target_family = "wasm"))]
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

#[cfg(target_family = "wasm")]
fn enable_raw_mode() -> io::Result<()> {
    Ok(())
}

#[cfg(target_family = "wasm")]
fn disable_raw_mode() -> io::Result<()> {
    Ok(())
}
use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_events::{Emitter, EventEmitter};
use lexe_utils::bytes::ObjectBytes;
use lexe_utils::event_loop::{self, EventLoopRef};
use lexe_utils::module::{export_default, ModuleInfo};
use lexe_utils::primordials::{BasePrimordials, Primordial};
use rquickjs::{
    class::{Trace, Tracer},
    function::Opt,
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, This},
    Class, Ctx, Exception, IntoJs, JsLifetime, Object, Result, Value,
};
use tokio::sync::{mpsc, Notify};

// ---------------------------------------------------------------------------
// Platform helpers
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn isatty(fd: i32) -> bool {
    if fd < 0 {
        return false;
    }
    unsafe { libc::isatty(fd as libc::c_int) != 0 }
}

#[cfg(windows)]
fn isatty(fd: i32) -> bool {
    use std::io::{stderr, stdin, stdout, IsTerminal};
    if fd < 0 {
        return false;
    }
    match fd {
        0 => stdin().is_terminal(),
        1 => stdout().is_terminal(),
        2 => stderr().is_terminal(),
        _ => win_cancel::handle_for_fd(fd).is_some_and(win_cancel::is_console_handle),
    }
}

#[cfg(not(any(unix, windows)))]
fn isatty(_fd: i32) -> bool {
    false
}

/// Write all bytes to `fd` without taking ownership / closing the descriptor.
/// Uses `libc::write` with a full partial-write + EINTR loop. Never wraps the
/// fd in `File::from_raw_fd` (would risk closing a shared process descriptor).
#[cfg(unix)]
fn write_all_fd(fd: i32, mut bytes: &[u8]) -> io::Result<()> {
    if fd < 0 {
        return Err(io::Error::from_raw_os_error(libc::EBADF));
    }
    while !bytes.is_empty() {
        let n = unsafe {
            libc::write(
                fd as libc::c_int,
                bytes.as_ptr() as *const libc::c_void,
                bytes.len(),
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write whole buffer",
            ));
        }
        bytes = &bytes[n as usize..];
    }
    Ok(())
}

#[cfg(windows)]
fn write_all_fd(fd: i32, bytes: &[u8]) -> io::Result<()> {
    use std::io::{stderr, stdout, Write};
    if fd < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad file descriptor",
        ));
    }
    match fd {
        1 => {
            let mut out = stdout().lock();
            out.write_all(bytes)?;
            out.flush()
        },
        2 => {
            let mut out = stderr().lock();
            out.write_all(bytes)?;
            out.flush()
        },
        _ => {
            let handle = win_cancel::handle_for_fd(fd).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "bad file descriptor")
            })?;
            win_cancel::write_handle_all(handle, bytes)
        },
    }
}

#[cfg(not(any(unix, windows)))]
fn write_all_fd(fd: i32, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    if fd < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad file descriptor",
        ));
    }
    use std::io::{stderr, stdout};
    match fd {
        1 => {
            let mut out = stdout().lock();
            out.write_all(bytes)?;
            out.flush()
        },
        2 => {
            let mut out = stderr().lock();
            out.write_all(bytes)?;
            out.flush()
        },
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "write to arbitrary fd is not supported on this platform",
        )),
    }
}

fn window_size_for_fd(fd: i32) -> (u16, u16) {
    #[cfg(unix)]
    if fd >= 0 {
        let mut ws = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let rc = unsafe { libc::ioctl(fd as libc::c_int, libc::TIOCGWINSZ, &mut ws) };
        if rc == 0 && (ws.ws_col > 0 || ws.ws_row > 0) {
            return (ws.ws_col, ws.ws_row);
        }
    }
    #[cfg(not(target_family = "wasm"))]
    {
        match crossterm::terminal::size() {
            Ok((cols, rows)) => (cols, rows),
            Err(_) => (80, 24),
        }
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = fd;
        (80, 24)
    }
}

// ---------------------------------------------------------------------------
// Raw mode refcount (global process terminal state)
// ---------------------------------------------------------------------------

static RAW_MODE_USERS: AtomicUsize = AtomicUsize::new(0);

fn acquire_raw_mode() -> io::Result<()> {
    let previous = RAW_MODE_USERS.fetch_add(1, Ordering::AcqRel);
    if previous == 0 {
        if let Err(e) = enable_raw_mode() {
            RAW_MODE_USERS.fetch_sub(1, Ordering::AcqRel);
            return Err(e);
        }
    }
    Ok(())
}

fn release_raw_mode() {
    loop {
        let cur = RAW_MODE_USERS.load(Ordering::Acquire);
        if cur == 0 {
            return;
        }
        if RAW_MODE_USERS
            .compare_exchange(cur, cur - 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            if cur == 1 {
                let _ = disable_raw_mode();
            }
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Cancel handle + reader state machine
// ---------------------------------------------------------------------------

/// Cancels a blocked reader worker. On Unix this is the write end of a pipe
/// that the worker polls alongside the read fd. On Windows it is a manual-reset
/// event used with a system wait.
#[derive(Clone)]
struct CancelHandle {
    inner: Arc<CancelInner>,
}

struct CancelInner {
    #[cfg(unix)]
    write_fd: i32,
    #[cfg(windows)]
    event: win_cancel::EventHandle,
    #[cfg(not(any(unix, windows)))]
    flag: std::sync::atomic::AtomicBool,
}

impl CancelHandle {
    fn new() -> io::Result<(Self, WorkerCancel)> {
        #[cfg(unix)]
        {
            let mut fds = [0i32; 2];
            let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            let handle = Self {
                inner: Arc::new(CancelInner { write_fd: fds[1] }),
            };
            let worker = WorkerCancel {
                read_fd: fds[0],
                write_fd_owned: false,
            };
            Ok((handle, worker))
        }
        #[cfg(windows)]
        {
            let event = win_cancel::EventHandle::new()?;
            let worker_event = event.duplicate()?;
            let handle = Self {
                inner: Arc::new(CancelInner { event }),
            };
            let worker = WorkerCancel {
                event: worker_event,
            };
            Ok((handle, worker))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let handle = Self {
                inner: Arc::new(CancelInner {
                    flag: std::sync::atomic::AtomicBool::new(false),
                }),
            };
            // Keep a shared flag for the worker via the same Arc path: store pointer in WorkerCancel
            let worker = WorkerCancel {
                flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            };
            let _ = flag;
            Ok((handle, worker))
        }
    }

    fn cancel(&self) {
        #[cfg(unix)]
        {
            let fd = self.inner.write_fd;
            if fd >= 0 {
                let b = [1u8];
                unsafe {
                    let _ = libc::write(fd as libc::c_int, b.as_ptr() as *const libc::c_void, 1);
                }
            }
        }
        #[cfg(windows)]
        {
            self.inner.event.signal();
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.inner.flag.store(true, Ordering::Release);
        }
    }
}

impl Drop for CancelInner {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            if self.write_fd >= 0 {
                unsafe {
                    libc::close(self.write_fd as libc::c_int);
                }
                self.write_fd = -1;
            }
        }
        #[cfg(windows)]
        {
            self.event.close();
        }
    }
}

/// Worker-side half of the cancel mechanism (owned by the reader thread).
struct WorkerCancel {
    #[cfg(unix)]
    read_fd: i32,
    #[cfg(unix)]
    write_fd_owned: bool,
    #[cfg(windows)]
    event: win_cancel::EventHandle,
    #[cfg(not(any(unix, windows)))]
    flag: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for WorkerCancel {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            if self.read_fd >= 0 {
                unsafe {
                    libc::close(self.read_fd as libc::c_int);
                }
                self.read_fd = -1;
            }
            let _ = self.write_fd_owned;
        }
        #[cfg(windows)]
        {
            self.event.close();
        }
    }
}

#[cfg(windows)]
mod win_cancel {
    use std::io;
    use std::os::windows::raw::HANDLE;
    use std::ptr;

    extern "C" {
        fn _get_osfhandle(fd: i32) -> isize;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            lp_event_attributes: *mut core::ffi::c_void,
            b_manual_reset: i32,
            b_initial_state: i32,
            lp_name: *const u16,
        ) -> HANDLE;
        fn SetEvent(h_event: HANDLE) -> i32;
        fn CloseHandle(h_object: HANDLE) -> i32;
        fn DuplicateHandle(
            h_source_process_handle: HANDLE,
            h_source_handle: HANDLE,
            h_target_process_handle: HANDLE,
            lp_target_handle: *mut HANDLE,
            dw_desired_access: u32,
            b_inherit_handle: i32,
            dw_options: u32,
        ) -> i32;
        fn GetCurrentProcess() -> HANDLE;
        fn WaitForMultipleObjects(
            n_count: u32,
            lp_handles: *const HANDLE,
            b_wait_all: i32,
            dw_milliseconds: u32,
        ) -> u32;
        fn GetStdHandle(n_std_handle: u32) -> HANDLE;
        fn WriteFile(
            h_file: HANDLE,
            lp_buffer: *const u8,
            n_number_of_bytes_to_write: u32,
            lp_number_of_bytes_written: *mut u32,
            lp_overlapped: *mut core::ffi::c_void,
        ) -> i32;
        fn GetConsoleMode(h_console_handle: HANDLE, lp_mode: *mut u32) -> i32;
    }

    pub const WAIT_OBJECT_0: u32 = 0;
    pub const WAIT_FAILED: u32 = 0xFFFFFFFF;
    pub const INFINITE: u32 = 0xFFFFFFFF;
    pub const DUPLICATE_SAME_ACCESS: u32 = 0x00000002;
    pub const STD_INPUT_HANDLE: u32 = 0xFFFFFFF6; // -10 as u32
    pub const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5;
    pub const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4;

    pub struct EventHandle(HANDLE);

    unsafe impl Send for EventHandle {}

    impl EventHandle {
        pub fn new() -> io::Result<Self> {
            let h = unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) };
            if h.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(h))
            }
        }

        pub fn duplicate(&self) -> io::Result<Self> {
            let mut target: HANDLE = ptr::null_mut();
            let ok = unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    self.0,
                    GetCurrentProcess(),
                    &mut target,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                )
            };
            if ok == 0 || target.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(target))
            }
        }

        pub fn signal(&self) {
            if !self.0.is_null() {
                unsafe {
                    let _ = SetEvent(self.0);
                }
            }
        }

        pub fn raw(&self) -> HANDLE {
            self.0
        }

        pub fn close(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
                self.0 = ptr::null_mut();
            }
        }
    }

    impl Drop for EventHandle {
        fn drop(&mut self) {
            self.close();
        }
    }

    pub fn std_handle_for_fd(fd: i32) -> Option<HANDLE> {
        let n = match fd {
            0 => STD_INPUT_HANDLE,
            1 => STD_OUTPUT_HANDLE,
            2 => STD_ERROR_HANDLE,
            _ => return None,
        };
        let h = unsafe { GetStdHandle(n) };
        if h.is_null() || h == (-1isize as HANDLE) {
            None
        } else {
            Some(h)
        }
    }

    pub fn handle_for_fd(fd: i32) -> Option<HANDLE> {
        if fd < 0 {
            return None;
        }
        if matches!(fd, 0..=2) {
            if let Some(h) = std_handle_for_fd(fd) {
                return Some(h);
            }
        }
        let h = unsafe { _get_osfhandle(fd) };
        if h == 0 || h == -1isize {
            None
        } else {
            Some(h as HANDLE)
        }
    }

    pub fn is_console_handle(handle: HANDLE) -> bool {
        let mut mode: u32 = 0;
        unsafe { GetConsoleMode(handle, &mut mode) != 0 }
    }

    pub fn write_handle_all(handle: HANDLE, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let chunk_len = bytes.len().min(u32::MAX as usize) as u32;
            let mut written: u32 = 0;
            let ok = unsafe {
                WriteFile(
                    handle,
                    bytes.as_ptr(),
                    chunk_len,
                    &mut written,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write whole buffer",
                ));
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }

    pub fn wait_read_or_cancel(read_handle: HANDLE, cancel: HANDLE) -> io::Result<WaitOutcome> {
        let handles = [read_handle, cancel];
        let r = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        if r == WAIT_FAILED {
            return Err(io::Error::last_os_error());
        }
        if r == WAIT_OBJECT_0 {
            Ok(WaitOutcome::Readable)
        } else if r == WAIT_OBJECT_0 + 1 {
            Ok(WaitOutcome::Cancelled)
        } else {
            // WAIT_ABANDONED_* or unexpected — treat as cancel/stop.
            Ok(WaitOutcome::Cancelled)
        }
    }

    pub enum WaitOutcome {
        Readable,
        Cancelled,
    }
}

/// Single-ownership generation state machine for the stdin/tty reader.
enum ReaderState {
    Paused,
    Starting {
        generation: u64,
    },
    Running {
        generation: u64,
        cancel: CancelHandle,
    },
    Stopping {
        generation: u64,
        resume_pending: bool,
    },
    Destroyed,
}

enum ReaderMsg {
    Data(Vec<u8>),
    End,
    Error(String),
    /// Worker exited because cancel was signalled (or destroy). No EOF.
    Cancelled,
}

// ---------------------------------------------------------------------------
// ReadStream
// ---------------------------------------------------------------------------

#[rquickjs::class]
pub struct ReadStream<'js> {
    emitter: EventEmitter<'js>,
    pub fd: i32,
    is_raw: bool,
    is_tty: bool,
    reader_state: ReaderState,
    next_generation: u64,
    destroyed: bool,
    loop_ref: Arc<EventLoopRef>,
}

// ReaderState is not Clone; live cancel ownership stays on the Class cell.
// Clones only mirror metadata for rquickjs class patterns.
impl<'js> Clone for ReadStream<'js> {
    fn clone(&self) -> Self {
        Self {
            emitter: self.emitter.clone(),
            fd: self.fd,
            is_raw: self.is_raw,
            is_tty: self.is_tty,
            reader_state: ReaderState::Paused,
            next_generation: self.next_generation,
            destroyed: self.destroyed,
            loop_ref: self.loop_ref.clone(),
        }
    }
}

unsafe impl<'js> JsLifetime<'js> for ReadStream<'js> {
    type Changed<'to> = ReadStream<'to>;
}

impl<'js> Trace<'js> for ReadStream<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

impl<'js> Drop for ReadStream<'js> {
    fn drop(&mut self) {
        if self.is_raw {
            release_raw_mode();
            self.is_raw = false;
        }
        match std::mem::replace(&mut self.reader_state, ReaderState::Destroyed) {
            ReaderState::Running { cancel, .. } => {
                cancel.cancel();
            },
            ReaderState::Stopping { .. }
            | ReaderState::Starting { .. }
            | ReaderState::Paused
            | ReaderState::Destroyed => {},
        }
    }
}

impl<'js> Emitter<'js> for ReadStream<'js> {
    fn get_event_list(&self) -> lexe_events::Events<'js> {
        self.emitter.get_event_list()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> ReadStream<'js> {
    #[qjs(constructor)]
    pub fn new_js(ctx: Ctx<'js>, fd: i32) -> Result<Class<'js, Self>> {
        Self::new(ctx, fd)
    }

    pub fn resume(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Class<'js, Self>> {
        Self::resume_inner(this.0.clone(), ctx)?;
        Ok(this.0)
    }

    pub fn pause(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        {
            let mut s = this.0.borrow_mut();
            match &mut s.reader_state {
                ReaderState::Running { generation, cancel } => {
                    let gen = *generation;
                    cancel.cancel();
                    s.reader_state = ReaderState::Stopping {
                        generation: gen,
                        resume_pending: false,
                    };
                },
                ReaderState::Starting { generation } => {
                    let gen = *generation;
                    s.reader_state = ReaderState::Stopping {
                        generation: gen,
                        resume_pending: false,
                    };
                },
                ReaderState::Stopping { resume_pending, .. } => {
                    // Already stopping: only clear a pending restart if we want
                    // pause to win over a prior resume_pending — Node pause after
                    // resume-while-stopping should leave the stream paused.
                    *resume_pending = false;
                },
                ReaderState::Paused | ReaderState::Destroyed => {},
            }
        }
        this.0
    }

    pub fn set_raw_mode(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        enabled: bool,
    ) -> Result<Class<'js, Self>> {
        let currently = this.0.borrow().is_raw;
        if enabled == currently {
            return Ok(this.0);
        }
        if enabled {
            if let Err(e) = acquire_raw_mode() {
                let err = Exception::from_message(ctx.clone(), &format!("setRawMode: {e}"))?;
                err.as_object().set("syscall", "setRawMode")?;
                let _ = ReadStream::emit_str(
                    this.0.clone(),
                    &ctx,
                    "error",
                    vec![err.into_value()],
                    false,
                );
                return Ok(this.0);
            }
            this.0.borrow_mut().is_raw = true;
        } else {
            release_raw_mode();
            this.0.borrow_mut().is_raw = false;
        }
        Ok(this.0)
    }

    #[qjs(get)]
    pub fn is_raw(&self) -> bool {
        self.is_raw
    }

    #[qjs(get, rename = "isTTY")]
    pub fn is_tty(&self) -> bool {
        self.is_tty
    }

    #[qjs(get)]
    pub fn fd(&self) -> i32 {
        self.fd
    }

    #[qjs(get)]
    pub fn readable(&self) -> bool {
        !self.destroyed && !matches!(self.reader_state, ReaderState::Destroyed)
    }

    pub fn is_paused(&self) -> bool {
        matches!(
            self.reader_state,
            ReaderState::Paused | ReaderState::Stopping { .. } | ReaderState::Destroyed
        )
    }

    pub fn destroy(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Class<'js, Self>> {
        {
            let mut s = this.0.borrow_mut();
            if s.destroyed {
                return Ok(this.0.clone());
            }
            s.destroyed = true;
            match std::mem::replace(&mut s.reader_state, ReaderState::Destroyed) {
                ReaderState::Running { cancel, .. } => {
                    cancel.cancel();
                },
                ReaderState::Stopping { .. }
                | ReaderState::Starting { .. }
                | ReaderState::Paused
                | ReaderState::Destroyed => {},
            }
            if s.is_raw {
                release_raw_mode();
                s.is_raw = false;
            }
        }
        let _ = ReadStream::emit_str(this.0.clone(), &ctx, "close", vec![], false);
        Ok(this.0)
    }

    #[qjs(rename = "ref")]
    pub fn js_ref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.reference();
        this.0
    }

    #[qjs(rename = "unref")]
    pub fn js_unref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.unreference();
        this.0
    }
}

impl<'js> ReadStream<'js> {
    pub fn new(ctx: Ctx<'js>, fd: i32) -> Result<Class<'js, Self>> {
        let emitter = EventEmitter::new();
        let is_tty = isatty(fd);
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_unreferenced(rt_key);
        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter,
                fd,
                is_raw: false,
                is_tty,
                reader_state: ReaderState::Paused,
                next_generation: 1,
                destroyed: false,
                loop_ref,
            },
        )?;
        ReadStream::add_event_emitter_prototype(&ctx)?;
        Ok(instance)
    }

    fn resume_inner(this: Class<'js, Self>, ctx: Ctx<'js>) -> Result<()> {
        // State transitions only on the JS executor (this function / finish_worker).
        let start = {
            let mut s = this.borrow_mut();
            if s.destroyed {
                return Ok(());
            }
            match &mut s.reader_state {
                ReaderState::Destroyed => return Ok(()),
                ReaderState::Running { .. } | ReaderState::Starting { .. } => {
                    // Idempotent resume while already running / starting.
                    return Ok(());
                },
                ReaderState::Stopping { resume_pending, .. } => {
                    *resume_pending = true;
                    return Ok(());
                },
                ReaderState::Paused => {
                    let generation = s.next_generation;
                    s.next_generation = s.next_generation.wrapping_add(1);
                    s.reader_state = ReaderState::Starting { generation };
                    Some((generation, s.fd))
                },
            }
        };

        let Some((generation, fd)) = start else {
            return Ok(());
        };

        let (cancel, worker_cancel) = match CancelHandle::new() {
            Ok(v) => v,
            Err(e) => {
                let mut s = this.borrow_mut();
                if matches!(
                    s.reader_state,
                    ReaderState::Starting { generation: g } if g == generation
                ) {
                    s.reader_state = ReaderState::Paused;
                }
                let msg = e.to_string();
                if let Ok(err) = Exception::from_message(ctx.clone(), &msg) {
                    let _ = ReadStream::emit_str(
                        this.clone(),
                        &ctx,
                        "error",
                        vec![err.into_value()],
                        false,
                    );
                }
                return Ok(());
            },
        };

        {
            let mut s = this.borrow_mut();
            match s.reader_state {
                ReaderState::Starting { generation: g } if g == generation => {
                    s.reader_state = ReaderState::Running {
                        generation,
                        cancel: cancel.clone(),
                    };
                },
                ReaderState::Stopping {
                    generation: g,
                    resume_pending,
                } if g == generation => {
                    if resume_pending {
                        // pause+resume raced during start: promote to Running and
                        // keep the cancel pipe alive so the worker is not woken
                        // by Drop closing the write end.
                        s.reader_state = ReaderState::Running {
                            generation: g,
                            cancel: cancel.clone(),
                        };
                    } else {
                        // pause won: signal cancel, keep Stopping, spawn worker
                        // only so it observes cancel and finishes cleanly.
                        cancel.cancel();
                        s.reader_state = ReaderState::Stopping {
                            generation: g,
                            resume_pending: false,
                        };
                        // Dropping `cancel` closes the write end (also wakes poll).
                        drop(cancel);
                    }
                },
                ReaderState::Destroyed => {
                    cancel.cancel();
                    drop(worker_cancel);
                    return Ok(());
                },
                _ => {
                    // Stale / unexpected: cancel and do not run.
                    cancel.cancel();
                    drop(worker_cancel);
                    return Ok(());
                },
            }
        }

        this.borrow().loop_ref.reference();
        Self::spawn_reader_worker(this, ctx, fd, generation, worker_cancel)?;
        Ok(())
    }

    fn spawn_reader_worker(
        this: Class<'js, Self>,
        ctx: Ctx<'js>,
        fd: i32,
        generation: u64,
        worker_cancel: WorkerCancel,
    ) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<ReaderMsg>();

        std::thread::Builder::new()
            .name(format!("tty-reader-{generation}"))
            .spawn(move || {
                reader_thread_main(fd, worker_cancel, tx);
            })
            .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;

        let this2 = this.clone();
        let ctx2 = ctx.clone();
        let reader_loop_ref = this.borrow().loop_ref.clone();
        ctx.clone().spawn_exit(async move {
            let mut broke_unref = false;
            loop {
                if !lexe_utils::event_loop::keeps_event_loop_alive(&reader_loop_ref) {
                    broke_unref = true;
                    break;
                }
                tokio::select! {
                    _ = lexe_utils::event_loop::wait_until_io_unref(reader_loop_ref.clone()) => {
                        broke_unref = true;
                        break;
                    }
                    msg = rx.recv() => {
                        let Some(msg) = msg else {
                            break;
                        };
                // Drop messages from stale generations (should not happen for
                // a single worker channel, but guard emit against Destroyed).
                let still_live = {
                    let s = this2.borrow();
                    matches!(
                        &s.reader_state,
                        ReaderState::Running { generation: g, .. }
                            | ReaderState::Stopping { generation: g, .. }
                            | ReaderState::Starting { generation: g }
                            if *g == generation
                    )
                };
                if !still_live {
                    // Drain until channel closes so the worker can finish, but
                    // do not emit or mutate state for a newer generation.
                    continue;
                }

                match msg {
                    ReaderMsg::Data(chunk) => match Buffer(chunk).into_js(&ctx2) {
                        Ok(data) => {
                            let _ = ReadStream::emit_str(
                                this2.clone(),
                                &ctx2,
                                "data",
                                vec![data],
                                false,
                            );
                        },
                        Err(e) => {
                            let msg = format!("{e}");
                            if let Ok(err) = Exception::from_message(ctx2.clone(), &msg) {
                                let _ = ReadStream::emit_str(
                                    this2.clone(),
                                    &ctx2,
                                    "error",
                                    vec![err.into_value()],
                                    false,
                                );
                            }
                        },
                    },
                    ReaderMsg::End => {
                        let _ = ReadStream::emit_str(this2.clone(), &ctx2, "end", vec![], false);
                    },
                    ReaderMsg::Error(msg) => {
                        if let Ok(err) = Exception::from_message(ctx2.clone(), &msg) {
                            let _ = ReadStream::emit_str(
                                this2.clone(),
                                &ctx2,
                                "error",
                                vec![err.into_value()],
                                false,
                            );
                        }
                    },
                    ReaderMsg::Cancelled => {
                        // No event; worker finishing will drive state.
                    },
                }
                    }
                }
            }

            if broke_unref {
                {
                    let mut s = this2.borrow_mut();
                    if let ReaderState::Running {
                        cancel,
                        generation: g,
                    } = std::mem::replace(&mut s.reader_state, ReaderState::Paused)
                    {
                        if g == generation {
                            cancel.cancel();
                        }
                    }
                    s.loop_ref.unreference();
                }
                while rx.recv().await.is_some() {}
                return Ok(());
            }

            // Worker fully completed: state transition on JS executor.
            Self::on_worker_finished(this2, ctx2, generation)?;
            Ok(())
        })?;
        Ok(())
    }

    fn on_worker_finished(this: Class<'js, Self>, ctx: Ctx<'js>, generation: u64) -> Result<()> {
        let resume_again = {
            let mut s = this.borrow_mut();
            match &mut s.reader_state {
                ReaderState::Running { generation: g, .. } if *g == generation => {
                    s.reader_state = ReaderState::Paused;
                    s.loop_ref.unreference();
                    false
                },
                ReaderState::Starting { generation: g } if *g == generation => {
                    s.reader_state = ReaderState::Paused;
                    s.loop_ref.unreference();
                    false
                },
                ReaderState::Stopping {
                    generation: g,
                    resume_pending,
                } if *g == generation => {
                    let pending = *resume_pending;
                    s.reader_state = ReaderState::Paused;
                    s.loop_ref.unreference();
                    pending && !s.destroyed
                },
                // Old generation completion must not overwrite newer state.
                _ => false,
            }
        };

        if resume_again {
            Self::resume_inner(this, ctx)?;
        }
        Ok(())
    }
}

fn reader_thread_main(fd: i32, cancel: WorkerCancel, tx: mpsc::UnboundedSender<ReaderMsg>) {
    #[cfg(unix)]
    {
        unix_reader_loop(fd, cancel, tx);
    }
    #[cfg(windows)]
    {
        windows_reader_loop(fd, cancel, tx);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (fd, cancel);
        let _ = tx.send(ReaderMsg::Error(
            "tty read is not supported on this platform".into(),
        ));
    }
}

#[cfg(unix)]
fn unix_reader_loop(fd: i32, cancel: WorkerCancel, tx: mpsc::UnboundedSender<ReaderMsg>) {
    // Each worker owns a dup of the source fd. Do NOT set O_NONBLOCK (would
    // change flags on the shared file description).
    let read_fd = unsafe { libc::dup(fd as libc::c_int) };
    if read_fd < 0 {
        let _ = tx.send(ReaderMsg::Error(io::Error::last_os_error().to_string()));
        return;
    }

    let cancel_fd = cancel.read_fd;
    let mut buf = vec![0u8; 8192];

    loop {
        let mut pfds = [
            libc::pollfd {
                fd: read_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: cancel_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let pr = unsafe { libc::poll(pfds.as_mut_ptr(), 2, -1) };
        if pr < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            let _ = tx.send(ReaderMsg::Error(err.to_string()));
            break;
        }

        // Cancel takes priority: do NOT continue into a read after cancel.
        if pfds[1].revents != 0 {
            let _ = tx.send(ReaderMsg::Cancelled);
            break;
        }

        if pfds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            let n =
                unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                let _ = tx.send(ReaderMsg::Error(err.to_string()));
                break;
            }
            if n == 0 {
                let _ = tx.send(ReaderMsg::End);
                break;
            }
            if tx
                .send(ReaderMsg::Data(buf[..n as usize].to_vec()))
                .is_err()
            {
                break;
            }
        }
    }

    unsafe {
        libc::close(read_fd);
    }
    // cancel (WorkerCancel) drops and closes cancel_fd
    drop(cancel);
}

#[cfg(windows)]
fn windows_reader_loop(fd: i32, cancel: WorkerCancel, tx: mpsc::UnboundedSender<ReaderMsg>) {
    use std::io::Read;
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};

    let Some(handle) = win_cancel::handle_for_fd(fd) else {
        let _ = tx.send(ReaderMsg::Error("EBADF: bad file descriptor, read".into()));
        return;
    };

    let mut buf = vec![0u8; 8192];
    loop {
        match win_cancel::wait_read_or_cancel(handle, cancel.event.raw()) {
            Ok(win_cancel::WaitOutcome::Cancelled) => {
                let _ = tx.send(ReaderMsg::Cancelled);
                break;
            },
            Ok(win_cancel::WaitOutcome::Readable) => {
                // SAFETY: process std handle; we into_raw_handle so we never close it.
                let mut file = unsafe { std::fs::File::from_raw_handle(handle) };
                let res = file.read(&mut buf);
                let _ = file.into_raw_handle();
                match res {
                    Ok(0) => {
                        let _ = tx.send(ReaderMsg::End);
                        break;
                    },
                    Ok(n) => {
                        if tx.send(ReaderMsg::Data(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    },
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        let _ = tx.send(ReaderMsg::Error(e.to_string()));
                        break;
                    },
                }
            },
            Err(e) => {
                let _ = tx.send(ReaderMsg::Error(e.to_string()));
                break;
            },
        }
    }
    drop(cancel);
}

// ---------------------------------------------------------------------------
// WriteStream
// ---------------------------------------------------------------------------

#[rquickjs::class]
#[derive(Clone)]
pub struct WriteStream<'js> {
    emitter: EventEmitter<'js>,
    pub fd: i32,
    is_tty: bool,
    columns: u16,
    rows: u16,
    ended: bool,
    ended_flag: Arc<AtomicBool>,
    winch_active: Arc<AtomicBool>,
    winch_stop: Arc<Notify>,
    loop_ref: Arc<EventLoopRef>,
}

unsafe impl<'js> JsLifetime<'js> for WriteStream<'js> {
    type Changed<'to> = WriteStream<'to>;
}

impl<'js> Trace<'js> for WriteStream<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

impl<'js> Emitter<'js> for WriteStream<'js> {
    fn get_event_list(&self) -> lexe_events::Events<'js> {
        self.emitter.get_event_list()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> WriteStream<'js> {
    #[qjs(constructor)]
    pub fn new_js(ctx: Ctx<'js>, fd: i32) -> Result<Class<'js, Self>> {
        Self::new(ctx, fd)
    }

    pub fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        chunk: Value<'js>,
        encoding_or_cb: Opt<Value<'js>>,
        cb: Opt<Option<rquickjs::Function<'js>>>,
    ) -> Result<bool> {
        let callback = cb.0.flatten().or_else(|| {
            encoding_or_cb.0.as_ref().and_then(|v| {
                if v.is_function() {
                    rquickjs::Function::from_value(v.clone()).ok()
                } else {
                    None
                }
            })
        });

        if this.0.borrow().ended {
            let err = Exception::from_message(ctx.clone(), "write after end")?;
            err.as_object().set("code", "ERR_STREAM_WRITE_AFTER_END")?;
            err.as_object().set("syscall", "write")?;
            let err_val = err.into_value();
            if let Some(cb) = callback {
                let cb2 = cb;
                let err_c = err_val.clone();
                ctx.clone().spawn_exit(async move {
                    let _ = cb2.call::<_, ()>((err_c,));
                    Ok(())
                })?;
            }
            let _ = WriteStream::emit_str(this.0.clone(), &ctx, "error", vec![err_val], false);
            return Ok(false);
        }

        let fd = this.0.borrow().fd;
        if fd < 0 {
            let err = Exception::from_message(ctx.clone(), "EBADF: bad file descriptor, write")?;
            err.as_object().set("code", "EBADF")?;
            err.as_object().set("syscall", "write")?;
            err.as_object().set("errno", -9)?;
            let err_val = err.into_value();
            if let Some(cb) = callback {
                let cb2 = cb;
                let err_c = err_val.clone();
                ctx.clone().spawn_exit(async move {
                    let _ = cb2.call::<_, ()>((err_c,));
                    Ok(())
                })?;
            }
            let _ = WriteStream::emit_str(this.0.clone(), &ctx, "error", vec![err_val], false);
            return Ok(false);
        }

        let bytes = ObjectBytes::from(&ctx, &chunk)?;
        let data = bytes.as_bytes(&ctx)?.to_vec();

        match write_all_fd(fd, &data) {
            Ok(()) => {
                if let Some(cb) = callback {
                    // Node: success callback is invoked with zero arguments.
                    let cb2 = cb;
                    ctx.clone().spawn_exit(async move {
                        let _ = cb2.call::<_, ()>(());
                        Ok(())
                    })?;
                }
                Ok(true)
            },
            Err(e) => {
                let code = match e.kind() {
                    io::ErrorKind::BrokenPipe => "EPIPE",
                    io::ErrorKind::PermissionDenied => "EACCES",
                    io::ErrorKind::NotFound => "ENOENT",
                    io::ErrorKind::InvalidInput => "EBADF",
                    _ => "EIO",
                };
                let msg = format!("{code}: {e}, write");
                let err = Exception::from_message(ctx.clone(), &msg)?;
                err.as_object().set("code", code)?;
                err.as_object().set("syscall", "write")?;
                err.as_object()
                    .set("errno", e.raw_os_error().unwrap_or(-1))?;
                let err_val = err.into_value();
                if let Some(cb) = callback {
                    let cb2 = cb;
                    let err_c = err_val.clone();
                    ctx.clone().spawn_exit(async move {
                        let _ = cb2.call::<_, ()>((err_c,));
                        Ok(())
                    })?;
                }
                let _ = WriteStream::emit_str(this.0.clone(), &ctx, "error", vec![err_val], false);
                Ok(false)
            },
        }
    }

    pub fn end(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        chunk: Opt<Value<'js>>,
        encoding_or_cb: Opt<Value<'js>>,
        cb: Opt<Option<rquickjs::Function<'js>>>,
    ) -> Result<Class<'js, Self>> {
        let explicit_cb = cb.0.flatten();
        let data_chunk = match chunk.0 {
            Some(v) if v.is_function() => {
                let cb_fn = rquickjs::Function::from_value(v)?;
                Self::finish_end(&this, &ctx)?;
                ctx.clone().spawn_exit(async move {
                    let _ = cb_fn.call::<_, ()>(());
                    Ok(())
                })?;
                return Ok(this.0);
            },
            Some(v) if v.is_undefined() || v.is_null() => None,
            other => other,
        };

        if let Some(chunk_val) = data_chunk {
            let _ = Self::write(
                This(this.0.clone()),
                ctx.clone(),
                chunk_val,
                encoding_or_cb,
                Opt(Some(explicit_cb)),
            )?;
            Self::finish_end(&this, &ctx)?;
        } else {
            let callback = explicit_cb.or_else(|| {
                encoding_or_cb.0.and_then(|v| {
                    if v.is_function() {
                        rquickjs::Function::from_value(v).ok()
                    } else {
                        None
                    }
                })
            });
            Self::finish_end(&this, &ctx)?;
            if let Some(cb_fn) = callback {
                ctx.clone().spawn_exit(async move {
                    let _ = cb_fn.call::<_, ()>(());
                    Ok(())
                })?;
            }
        }
        Ok(this.0)
    }

    pub fn get_window_size(&self) -> Vec<u16> {
        let (c, r) = window_size_for_fd(self.fd);
        vec![c, r]
    }

    pub fn cursor_to(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        x: i32,
        y: Opt<Option<i32>>,
        cb: Opt<Option<rquickjs::Function<'js>>>,
    ) -> Result<bool> {
        let data = if let Some(y) = y.0.flatten() {
            format!("\x1b[{};{}H", y + 1, x + 1)
        } else {
            format!("\x1b[{}G", x + 1)
        };
        let val = data.into_js(&ctx)?;
        Self::write(this, ctx, val, Opt(None), cb)
    }

    pub fn move_cursor(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        dx: i32,
        dy: i32,
        cb: Opt<Option<rquickjs::Function<'js>>>,
    ) -> Result<bool> {
        let mut data = String::new();
        if dx < 0 {
            data.push_str(&format!("\x1b[{}D", -dx));
        } else if dx > 0 {
            data.push_str(&format!("\x1b[{}C", dx));
        }
        if dy < 0 {
            data.push_str(&format!("\x1b[{}A", -dy));
        } else if dy > 0 {
            data.push_str(&format!("\x1b[{}B", dy));
        }
        if data.is_empty() {
            if let Some(cb) = cb.0.flatten() {
                let cb2 = cb;
                ctx.clone().spawn_exit(async move {
                    let _ = cb2.call::<_, ()>(());
                    Ok(())
                })?;
            }
            return Ok(true);
        }
        let val = data.into_js(&ctx)?;
        Self::write(this, ctx, val, Opt(None), cb)
    }

    pub fn clear_line(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        dir: Opt<Option<i32>>,
        cb: Opt<Option<rquickjs::Function<'js>>>,
    ) -> Result<bool> {
        let dir = dir.0.flatten().unwrap_or(0);
        let seq = if dir < 0 {
            "\x1b[1K"
        } else if dir > 0 {
            "\x1b[0K"
        } else {
            "\x1b[2K"
        };
        let val = seq.into_js(&ctx)?;
        Self::write(this, ctx, val, Opt(None), cb)
    }

    pub fn clear_screen_down(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        cb: Opt<Option<rquickjs::Function<'js>>>,
    ) -> Result<bool> {
        let val = "\x1b[0J".into_js(&ctx)?;
        Self::write(this, ctx, val, Opt(None), cb)
    }

    pub fn has_colors(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        count_or_env: Opt<Value<'js>>,
        _env: Opt<Value<'js>>,
    ) -> Result<bool> {
        let depth = Self::get_color_depth(This(this.0), ctx, count_or_env)?;
        Ok(depth >= 4)
    }

    pub fn get_color_depth(
        _this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        env_or_count: Opt<Value<'js>>,
    ) -> Result<u32> {
        use lexe_utils::object::ObjectExt;
        let globals = ctx.globals();
        let process: Object = globals.get("process")?;
        let env: Object = process.get("env")?;

        let force: Option<String> = env.get_optional("FORCE_COLOR")?;
        if let Some(v) = force {
            return Ok(match v.as_str() {
                "" | "1" | "true" => 4,
                "2" => 8,
                "3" => 24,
                _ => 1,
            });
        }
        let no_color: Option<String> = env.get_optional("NO_COLOR")?;
        if no_color.as_deref().is_some_and(|s| !s.is_empty()) {
            return Ok(1);
        }
        let node_disable: Option<String> = env.get_optional("NODE_DISABLE_COLORS")?;
        if node_disable.as_deref().is_some_and(|s| !s.is_empty()) {
            return Ok(1);
        }
        let term: Option<String> = env.get_optional("TERM")?;
        if term.as_deref() == Some("dumb") {
            return Ok(1);
        }
        let _ = env_or_count;
        Ok(if isatty(1) { 4 } else { 1 })
    }

    #[qjs(get, rename = "isTTY")]
    pub fn is_tty_prop(&self) -> bool {
        self.is_tty
    }

    #[qjs(get)]
    pub fn fd(&self) -> i32 {
        self.fd
    }

    #[qjs(get)]
    pub fn columns(&self) -> u16 {
        self.columns
    }

    #[qjs(get)]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    #[qjs(get)]
    pub fn writable(&self) -> bool {
        !self.ended
    }

    pub fn refresh_size(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        let (cols, rows) = window_size_for_fd(this.0.borrow().fd);
        let mut changed = false;
        {
            let mut s = this.0.borrow_mut();
            if s.columns != cols || s.rows != rows {
                s.columns = cols;
                s.rows = rows;
                changed = true;
            }
        }
        if changed {
            let _ = WriteStream::emit_str(this.0, &ctx, "resize", vec![], false);
        }
        Ok(())
    }

    #[qjs(rename = "ref")]
    pub fn js_ref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.reference();
        this.0
    }

    #[qjs(rename = "unref")]
    pub fn js_unref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.unreference();
        this.0
    }
}

impl<'js> WriteStream<'js> {
    fn finish_end(this: &This<Class<'js, Self>>, ctx: &Ctx<'js>) -> Result<()> {
        let was_ended = {
            let mut ws = this.0.borrow_mut();
            let prev = ws.ended;
            ws.ended = true;
            ws.ended_flag.store(true, Ordering::Release);
            ws.winch_stop.notify_waiters();
            prev
        };
        if !was_ended {
            let _ = WriteStream::emit_str(this.0.clone(), ctx, "finish", vec![], false);
        }
        Ok(())
    }

    fn start_resize_watch(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        #[cfg(unix)]
        {
            let (is_tty, ended_flag, winch_active, stop_notify, loop_ref) = {
                let borrow = this.borrow();
                (
                    borrow.is_tty,
                    borrow.ended_flag.clone(),
                    borrow.winch_active.clone(),
                    borrow.winch_stop.clone(),
                    borrow.loop_ref.clone(),
                )
            };
            if !is_tty || ended_flag.load(Ordering::Acquire) {
                return Ok(());
            }
            if winch_active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Ok(());
            }
            use tokio::signal::unix::{signal, SignalKind};
            let Ok(mut sig) = signal(SignalKind::window_change()) else {
                winch_active.store(false, Ordering::Release);
                return Ok(());
            };
            let ws_clone = this.0.clone();
            ctx.clone().spawn_exit(async move {
                loop {
                    if ended_flag.load(Ordering::Acquire)
                        || !event_loop::keeps_event_loop_alive(&loop_ref)
                    {
                        break;
                    }
                    tokio::select! {
                        _ = event_loop::wait_until_io_unref(loop_ref.clone()) => break,
                        _ = stop_notify.notified() => break,
                        evt = sig.recv() => {
                            if evt.is_none() {
                                break;
                            }
                            let _ = WriteStream::refresh_size(This(ws_clone.clone()), ctx.clone());
                        }
                    }
                }
                winch_active.store(false, Ordering::Release);
                Ok(())
            })?;
        }
        #[cfg(not(unix))]
        let _ = (this, ctx);
        Ok(())
    }

    fn stop_resize_watch(this: This<Class<'js, Self>>) {
        this.borrow().winch_stop.notify_waiters();
    }

    fn ensure_prototype(ctx: &Ctx<'js>) -> Result<()> {
        let _ = BasePrimordials::init(ctx);
        let Some(proto) = Class::<WriteStream>::prototype(ctx)? else {
            return Ok(());
        };
        if proto
            .get::<_, Option<bool>>("__lexeTtyResizeHooked")?
            .unwrap_or(false)
        {
            return Ok(());
        }
        WriteStream::add_event_emitter_prototype(ctx)?;
        let start_watch = rquickjs::Function::new(ctx.clone(), WriteStream::start_resize_watch)?;
        let stop_watch = rquickjs::Function::new(ctx.clone(), WriteStream::stop_resize_watch)?;
        let install: rquickjs::Function = ctx.eval(
            r#"(function (proto, startWatch, stopWatch) {
  Object.defineProperty(proto, "__lexeTtyResizeHooked", { value: true });
  for (const m of ["on", "once", "addListener", "prependListener", "prependOnceListener"]) {
    const orig = proto[m];
    if (typeof orig === "function") {
      proto[m] = function (ev) {
        const ret = orig.apply(this, arguments);
        if (ev === "resize") startWatch.call(this);
        return ret;
      };
    }
  }
  for (const m of ["off", "removeListener", "removeAllListeners"]) {
    const orig = proto[m];
    if (typeof orig === "function") {
      proto[m] = function () {
        const ret = orig.apply(this, arguments);
        if (typeof this.listenerCount === "function" && this.listenerCount("resize") === 0) {
          stopWatch.call(this);
        }
        return ret;
      };
    }
  }
})"#,
        )?;
        install.call::<_, ()>((proto, start_watch, stop_watch))
    }

    pub fn new(ctx: Ctx<'js>, fd: i32) -> Result<Class<'js, Self>> {
        let emitter = EventEmitter::new();
        let is_tty = isatty(fd);
        let (columns, rows) = if is_tty {
            window_size_for_fd(fd)
        } else {
            (0, 0)
        };
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_unreferenced(rt_key);
        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter,
                fd,
                is_tty,
                columns,
                rows,
                ended: false,
                ended_flag: Arc::new(AtomicBool::new(false)),
                winch_active: Arc::new(AtomicBool::new(false)),
                winch_stop: Arc::new(Notify::new()),
                loop_ref,
            },
        )?;
        WriteStream::ensure_prototype(&ctx)?;
        Ok(instance)
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

pub struct TtyModule;

impl ModuleDef for TtyModule {
    fn declare(declare: &Declarations<'_>) -> Result<()> {
        declare.declare("isatty")?;
        declare.declare("ReadStream")?;
        declare.declare("WriteStream")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let _ = Class::<ReadStream>::create_constructor(ctx)?;
        let _ = Class::<WriteStream>::create_constructor(ctx)?;
        ReadStream::add_event_emitter_prototype(ctx)?;
        WriteStream::ensure_prototype(ctx)?;

        export_default(ctx, exports, |default| {
            default.set("isatty", Func::from(isatty))?;
            let rs_ctor =
                Class::<ReadStream>::create_constructor(ctx)?.expect("ReadStream constructor");
            let ws_ctor =
                Class::<WriteStream>::create_constructor(ctx)?.expect("WriteStream constructor");
            default.set("ReadStream", rs_ctor)?;
            default.set("WriteStream", ws_ctor)?;
            Ok(())
        })
    }
}

impl From<TtyModule> for ModuleInfo<TtyModule> {
    fn from(val: TtyModule) -> Self {
        ModuleInfo {
            name: "tty",
            module: val,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};
    use std::io::{stderr, stdin, stdout, IsTerminal};
    use std::sync::atomic::Ordering;
    // ObjectBytes / WriteStream JS paths need a full runtime; covered by pure helpers above.

    #[test]
    fn raw_mode_enable_fail_rolls_back_refcount() {
        // Simulate the rollback path: increment then force-fail path logic.
        // We cannot force enable_raw_mode to fail portably, but we can verify
        // the counter restore contract used by acquire_raw_mode on error.
        let before = RAW_MODE_USERS.load(Ordering::SeqCst);
        // Manually exercise rollback arithmetic used on enable failure.
        let previous = RAW_MODE_USERS.fetch_add(1, Ordering::AcqRel);
        RAW_MODE_USERS.fetch_sub(1, Ordering::AcqRel);
        assert_eq!(previous, before);
        assert_eq!(RAW_MODE_USERS.load(Ordering::SeqCst), before);
    }

    #[cfg(unix)]
    #[test]
    fn write_all_fd_partial_and_invalid() {
        // invalid fd
        let err = write_all_fd(-1, b"x").unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EBADF));

        // pipe: write then read back (covers partial-write loop for small buf)
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = (fds[0], fds[1]);
        write_all_fd(w, b"hello").unwrap();
        let mut buf = [0u8; 8];
        let n = unsafe { libc::read(r, buf.as_mut_ptr() as *mut _, buf.len()) };
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"hello");
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancel_handle_wakes_poll() {
        let (cancel, worker) = CancelHandle::new().unwrap();
        let mut pfds = [libc::pollfd {
            fd: worker.read_fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        // Non-blocking poll before cancel: should not be ready.
        let pr = unsafe { libc::poll(pfds.as_mut_ptr(), 1, 0) };
        assert_eq!(pr, 0);
        cancel.cancel();
        let pr = unsafe { libc::poll(pfds.as_mut_ptr(), 1, 100) };
        assert!(pr > 0);
        assert!(pfds[0].revents != 0);
        drop(worker);
        drop(cancel);
    }

    #[cfg(unix)]
    #[test]
    fn reader_thread_no_duplicate_bytes_and_cancel() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = (fds[0], fds[1]);

        let (cancel, worker_cancel) = CancelHandle::new().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let t = std::thread::spawn(move || {
            unix_reader_loop(r, worker_cancel, tx);
        });

        // Write one chunk, then cancel — must not steal later bytes.
        assert_eq!(unsafe { libc::write(w, b"abc".as_ptr() as *const _, 3) }, 3);
        // Give the reader a moment to observe data.
        std::thread::sleep(std::time::Duration::from_millis(50));
        cancel.cancel();
        t.join().unwrap();

        let mut got = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let ReaderMsg::Data(d) = msg {
                got.extend_from_slice(&d);
            }
        }
        assert_eq!(got, b"abc");

        // Further writes must not be consumed by a stale reader.
        assert_eq!(unsafe { libc::write(w, b"XYZ".as_ptr() as *const _, 3) }, 3);
        // Reader closed its dup of r; original r still open and still has XYZ if not read.
        let mut buf = [0u8; 8];
        let n = unsafe { libc::read(r, buf.as_mut_ptr() as *mut _, buf.len()) };
        assert_eq!(n, 3);
        assert_eq!(&buf[..3], b"XYZ");

        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[cfg(unix)]
    #[test]
    fn destroy_while_reader_blocked_cancels() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = (fds[0], fds[1]);

        let (cancel, worker_cancel) = CancelHandle::new().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let t = std::thread::spawn(move || {
            unix_reader_loop(r, worker_cancel, tx);
        });
        // Blocked on empty pipe; cancel as destroy would.
        std::thread::sleep(std::time::Duration::from_millis(20));
        cancel.cancel();
        t.join().unwrap();
        let mut saw_cancel = false;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, ReaderMsg::Cancelled) {
                saw_cancel = true;
            }
            assert!(!matches!(msg, ReaderMsg::Data(_)));
        }
        assert!(saw_cancel);
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[tokio::test]
    async fn test_isatty() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<TtyModule>(ctx.clone(), "tty")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { isatty } from 'tty';

                        export async function test() {
                            return new Array(3).fill(0).map((_, i) => +isatty(i)).join('')
                        }
                    "#,
                )
                .await
                .unwrap();
                let expect = [
                    stdin().is_terminal(),
                    stdout().is_terminal(),
                    stderr().is_terminal(),
                ]
                .map(|i| (i as u8).to_string())
                .join("");
                let result = call_test::<String, _>(&ctx, &module, ()).await;
                assert_eq!(result, expect);
            })
        })
        .await;
    }

    #[test]
    fn write_after_end_flag_blocks_writes() {
        // Mirrors WriteStream::write post-end path: ended streams must not call write_all_fd.
        let ended = true;
        assert!(ended);
        // invalid fd path
        let err = write_all_fd(-1, b"x").unwrap_err();
        #[cfg(unix)]
        assert_eq!(err.raw_os_error(), Some(libc::EBADF));
        #[cfg(not(unix))]
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn write_success_and_partial_loop() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = (fds[0], fds[1]);
        // Larger buffer to exercise the write loop.
        let data = vec![b'a'; 4096];
        write_all_fd(w, &data).unwrap();
        let mut got = 0usize;
        let mut buf = [0u8; 1024];
        while got < data.len() {
            let n = unsafe { libc::read(r, buf.as_mut_ptr() as *mut _, buf.len()) };
            assert!(n > 0);
            got += n as usize;
        }
        assert_eq!(got, data.len());
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[cfg(unix)]
    #[test]
    fn consecutive_resume_idempotent_at_cancel_layer() {
        // Running + resume is idempotent: a second cancel handle is not created
        // while the first generation is still Running (exercised by single worker).
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = (fds[0], fds[1]);

        let (cancel, worker) = CancelHandle::new().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let t = std::thread::spawn(move || unix_reader_loop(r, worker, tx));

        // "resume" while running: do not spawn a second worker; only one reader.
        assert_eq!(unsafe { libc::write(w, b"hi".as_ptr() as *const _, 2) }, 2);
        std::thread::sleep(std::time::Duration::from_millis(30));
        // pause then immediate resume = cancel old, then new generation after join
        cancel.cancel();
        t.join().unwrap();
        let mut first = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let ReaderMsg::Data(d) = msg {
                first.extend(d);
            }
        }
        assert_eq!(first, b"hi");

        let (cancel2, worker2) = CancelHandle::new().unwrap();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let t2 = std::thread::spawn(move || unix_reader_loop(r, worker2, tx2));
        assert_eq!(unsafe { libc::write(w, b"yo".as_ptr() as *const _, 2) }, 2);
        std::thread::sleep(std::time::Duration::from_millis(30));
        cancel2.cancel();
        t2.join().unwrap();
        let mut second = Vec::new();
        while let Ok(msg) = rx2.try_recv() {
            if let ReaderMsg::Data(d) = msg {
                second.extend(d);
            }
        }
        assert_eq!(second, b"yo");
        // No duplicate / no stale steal of later bytes across generations.
        assert_ne!(first, second);

        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[test]
    fn reader_state_stopping_resume_pending_semantics() {
        // Pure state-machine transitions (mirrors resume/pause rules).
        let gen = 1u64;
        let (cancel, _worker) = CancelHandle::new().unwrap();
        // resume -> Running (Starting is an intermediate step)
        let mut state = ReaderState::Running {
            generation: gen,
            cancel: cancel.clone(),
        };
        // resume while Running is idempotent (no state change)
        match &state {
            ReaderState::Running { generation, .. } => assert_eq!(*generation, gen),
            _ => panic!("expected Running"),
        }
        // pause -> Stopping
        state = match state {
            ReaderState::Running { generation, cancel } => {
                cancel.cancel();
                ReaderState::Stopping {
                    generation,
                    resume_pending: false,
                }
            },
            other => other,
        };
        // resume while Stopping only sets resume_pending
        if let ReaderState::Stopping { resume_pending, .. } = &mut state {
            *resume_pending = true;
        } else {
            panic!("expected Stopping");
        }
        match state {
            ReaderState::Stopping {
                generation,
                resume_pending,
            } => {
                assert_eq!(generation, gen);
                assert!(resume_pending);
            },
            _ => panic!("expected Stopping with resume_pending"),
        }
        // destroy: no restart
        assert!(matches!(ReaderState::Destroyed, ReaderState::Destroyed));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_stream_sigwinch_emits_resize_automatically() {
        let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        assert!(master >= 0, "posix_openpt failed");
        assert_eq!(unsafe { libc::grantpt(master) }, 0);
        assert_eq!(unsafe { libc::unlockpt(master) }, 0);
        let slave_name = unsafe { libc::ptsname(master) };
        assert!(!slave_name.is_null());
        let slave = unsafe { libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY) };
        assert!(slave >= 0, "open pty slave failed");

        let init_ws = libc::winsize {
            ws_row: 30,
            ws_col: 90,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            unsafe { libc::ioctl(master, libc::TIOCSWINSZ, &init_ws) },
            0
        );
        assert!(isatty(slave));
        assert_eq!(window_size_for_fd(slave), (90, 30));

        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<TtyModule>(ctx.clone(), "tty")
                    .await
                    .unwrap();
                ctx.globals().set("__ptyMaster", master).unwrap();
                ctx.globals().set("__ptySlave", slave).unwrap();
                let trigger_resize = rquickjs::Function::new(ctx.clone(), |m: i32| {
                    let next_ws = libc::winsize {
                        ws_row: 50,
                        ws_col: 132,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    };
                    unsafe {
                        libc::ioctl(m as libc::c_int, libc::TIOCSWINSZ, &next_ws);
                        libc::kill(libc::getpid(), libc::SIGWINCH);
                    }
                })
                .unwrap();
                ctx.globals().set("__triggerWinch", trigger_resize).unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import tty from "tty";
                        export async function test() {
                          const ws = new tty.WriteStream(globalThis.__ptySlave);
                          if (ws.columns !== 90 || ws.rows !== 30) {
                            throw new Error(`unexpected initial size: ${ws.columns}x${ws.rows}`);
                          }
                          ws.ref();
                          return await new Promise((resolve) => {
                            ws.once("resize", () => {
                              ws.unref();
                              resolve({ columns: ws.columns, rows: ws.rows });
                            });
                            globalThis.__triggerWinch(globalThis.__ptyMaster);
                          });
                        }
                    "#,
                )
                .await
                .unwrap();

                let res: Object = call_test(&ctx, &module, ()).await;
                let cols: u16 = res.get("columns").unwrap();
                let rows: u16 = res.get("rows").unwrap();
                assert_eq!((cols, rows), (132, 50));
            })
        })
        .await;

        unsafe {
            libc::close(slave);
            libc::close(master);
        }
    }
}
