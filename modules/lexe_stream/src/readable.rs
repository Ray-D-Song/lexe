// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    any::Any,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
};

use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_events::{EmitError, Emitter, EventEmitter, EventKey, EventList};
use lexe_utils::{
    bytearray_buffer::BytearrayBuffer,
    event_loop::{self, EventLoopRef},
    result::ResultExt,
};
use rquickjs::{
    class::{Trace, Tracer},
    prelude::{Func, Opt, This},
    Class, Ctx, Error, IntoJs, JsLifetime, Null, Object, Result, Value,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, BufReader},
    sync::{
        broadcast::{self, Sender},
        oneshot::{self, Receiver},
    },
};

use super::{impl_stream_events, set_destroyed_and_error, SteamEvents, DEFAULT_BUFFER_SIZE};

fn complete_readable_handoff<T: AsyncRead + Unpin + Send + 'static>(
    reader: &mut Option<BufReader<T>>,
    ba_buffer: &BytearrayBuffer,
    buffer: &mut Vec<u8>,
    handoff_waiter: &Arc<Mutex<Option<Box<dyn Any + Send>>>>,
) -> Result<bool> {
    if handoff_waiter.lock().unwrap().is_none() {
        return Ok(false);
    }

    let reader = reader.take().expect("reader available for handoff");
    let mut prefix = Vec::new();
    if let Some(stored) = ba_buffer.read(None) {
        prefix.extend(stored);
    }
    prefix.extend(buffer.drain(..));
    prefix.extend_from_slice(reader.buffer());
    let inner_reader = reader.into_inner();
    if let Some(boxed) = handoff_waiter.lock().unwrap().take() {
        let tx = *boxed
            .downcast::<oneshot::Sender<(T, Vec<u8>)>>()
            .expect("handoff waiter type mismatch");
        let _ = tx.send((inner_reader, prefix));
    }
    Ok(true)
}

#[derive(PartialEq, Clone, Debug)]
pub enum ReadableState {
    Init,
    Flowing,
    Paused,
}

#[allow(dead_code)]
pub struct ReadableStreamInner<'js> {
    emitter: EventEmitter<'js>,
    destroy_tx: Sender<Option<Value<'js>>>,
    is_ended: bool,
    is_destroyed: bool,
    errored: bool,
    buffer: BytearrayBuffer,
    emit_close: bool,
    state: ReadableState,
    high_water_mark: AtomicUsize,
    listener: Option<&'static str>,
    data_listener_attached_tx: Sender<()>,
    paused: Arc<AtomicBool>,
    handoff_waiter: Arc<Mutex<Option<Box<dyn Any + Send>>>>,
    handoff_notify: Sender<()>,
}

impl<'js> Trace<'js> for ReadableStreamInner<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

impl<'js> ReadableStreamInner<'js> {
    pub fn on_event_changed(&mut self, event: EventKey<'js>, added: bool) -> Result<()> {
        if let EventKey::String(event) = event {
            match event.as_ref() {
                "data" => {
                    if added {
                        if self.state == ReadableState::Paused {
                            let _ = self.data_listener_attached_tx.send(());
                        }
                        self.state = ReadableState::Flowing;
                        self.listener = Some("data");
                    } else {
                        self.listener = None;
                        if self.state == ReadableState::Flowing {
                            self.state = ReadableState::Paused;
                        }
                    }
                },
                "readable" => {
                    if added {
                        self.state = ReadableState::Paused;
                        self.listener = Some("readable");
                    } else {
                        self.listener = None;
                    }
                },
                _ => {},
            }
        }
        Ok(())
    }

    pub fn new(emitter: EventEmitter<'js>, emit_close: bool) -> Self {
        let (destroy_tx, _) = broadcast::channel::<Option<Value<'js>>>(1);
        let (listener_attached_tx, _) = broadcast::channel::<()>(1);
        let (handoff_notify, _) = broadcast::channel::<()>(1);
        Self {
            emitter,
            destroy_tx,
            is_ended: false,
            data_listener_attached_tx: listener_attached_tx,
            buffer: BytearrayBuffer::new(DEFAULT_BUFFER_SIZE),
            state: ReadableState::Init,
            high_water_mark: DEFAULT_BUFFER_SIZE.into(),
            listener: None,
            is_destroyed: false,
            emit_close,
            errored: false,
            paused: Arc::new(AtomicBool::new(false)),
            handoff_waiter: Arc::new(Mutex::new(None)),
            handoff_notify,
        }
    }

    /// Whether the readable side has reached EOF / end-of-stream.
    #[inline]
    pub fn is_ended(&self) -> bool {
        self.is_ended
    }

    /// Whether the readable side has been destroyed.
    #[inline]
    pub fn is_destroyed(&self) -> bool {
        self.is_destroyed
    }

    /// Whether JS is consuming this stream: a `data`/`readable` listener is
    /// attached, or the stream is in flowing mode.
    #[inline]
    pub fn has_consumer(&self) -> bool {
        self.listener.is_some() || self.state == ReadableState::Flowing
    }

    /// Park [`ReadableStream::process_callback_loop_duplex`] before reading more bytes.
    pub fn pause_reading(&self) {
        self.paused.store(true, Ordering::Release);
    }

    /// Wake a parked [`ReadableStream::process_callback_loop_duplex`] reader.
    ///
    /// On the duplex path the JS `Duplex` owns buffering and backpressure, so
    /// the native loop parks as soon as `push` returns `false` and only this
    /// signal (from `_read` / `resume`) lets it read again. Reuses the
    /// "consumer wants data" channel the classic loop already has; that loop
    /// reads it only to clear its byte array, so sending here cannot change its
    /// behaviour.
    pub fn resume_reading(&self) {
        self.paused.store(false, Ordering::Release);
        let _ = self.data_listener_attached_tx.send(());
    }
}

#[rquickjs::class]
#[derive(rquickjs::class::Trace)]
pub struct DefaultReadableStream<'js> {
    inner: ReadableStreamInner<'js>,
}

unsafe impl<'js> JsLifetime<'js> for DefaultReadableStream<'js> {
    type Changed<'to> = DefaultReadableStream<'to>;
}

impl<'js> DefaultReadableStream<'js> {
    fn with_emitter(ctx: Ctx<'js>, emitter: EventEmitter<'js>) -> Result<Class<'js, Self>> {
        Class::instance(
            ctx,
            Self {
                inner: ReadableStreamInner::new(emitter, true),
            },
        )
    }

    pub fn new(ctx: Ctx<'js>) -> Result<Class<'js, Self>> {
        Self::with_emitter(ctx, EventEmitter::new())
    }
}

impl_stream_events!(DefaultReadableStream);
impl<'js> Emitter<'js> for DefaultReadableStream<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.inner.emitter.get_event_list()
    }

    fn on_event_changed(&mut self, event: EventKey<'js>, added: bool) -> Result<()> {
        self.inner.on_event_changed(event, added)
    }
}
impl<'js> ReadableStream<'js> for DefaultReadableStream<'js> {
    fn inner_mut(&mut self) -> &mut ReadableStreamInner<'js> {
        &mut self.inner
    }

    fn inner(&self) -> &ReadableStreamInner<'js> {
        &self.inner
    }
}

pub trait ReadableStream<'js>
where
    Self: Emitter<'js> + SteamEvents<'js>,
{
    fn inner_mut(&mut self) -> &mut ReadableStreamInner<'js>;

    fn inner(&self) -> &ReadableStreamInner<'js>;

    fn add_readable_stream_prototype(ctx: &Ctx<'js>) -> Result<()> {
        let proto = Class::<Self>::prototype(ctx)?
            .or_throw_msg(ctx, &["Prototype for ", Self::NAME, " not found"].concat())?;

        proto.set("read", Func::from(Self::read))?;

        proto.set("destroy", Func::from(Self::destroy))?;

        Ok(())
    }

    fn destroy(this: This<Class<'js, Self>>, error: Opt<Value<'js>>) -> Class<'js, Self> {
        let mut borrow = this.borrow_mut();
        let inner = borrow.inner_mut();
        inner.is_destroyed = true;
        let _ = inner.destroy_tx.send(error.0);
        drop(borrow);
        this.0
    }

    fn read(this: This<Class<'js, Self>>, ctx: Ctx<'js>, size: Opt<usize>) -> Result<Value<'js>> {
        if let Some(data) = this.borrow().inner().buffer.read(size.0) {
            return Buffer(data).into_js(&ctx);
        }

        Ok(Null.into_value(ctx))
    }

    fn drain(this: Class<'js, Self>, ctx: &Ctx<'js>) -> Result<()> {
        let this2 = this.clone();
        let borrow = this2.borrow();
        let inner = borrow.inner();
        let listener = inner.listener;

        if let Some(listener) = listener {
            let ba_buffer = inner.buffer.clone();
            if !ba_buffer.is_empty() {
                drop(borrow);
                let args = match listener {
                    "data" => {
                        let buffer = ba_buffer.read(None).unwrap_or_default();
                        if buffer.is_empty() {
                            return Ok(());
                        }
                        vec![Buffer(buffer).into_js(ctx)?]
                    },
                    "readable" => {
                        vec![]
                    },
                    _ => {
                        vec![]
                    },
                };
                Self::emit_str(this, ctx, listener, args, false)?;
            }
        }
        Ok(())
    }

    fn process<T: AsyncRead + Send + 'static + 'js + Unpin>(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
        readable: T,
    ) -> Result<Receiver<bool>> {
        Self::do_process(this, ctx, readable, || {}, None)
    }

    fn process_callback<T: AsyncRead + Send + 'static + 'js + Unpin, C: FnOnce() + Sized + 'js>(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
        readable: T,
        on_end: C,
    ) -> Result<Receiver<bool>> {
        Self::do_process(this, ctx, readable, on_end, None)
    }

    fn process_callback_loop<
        T: AsyncRead + Send + 'static + 'js + Unpin,
        C: FnOnce() + Sized + 'js,
    >(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
        readable: T,
        on_end: C,
        io_loop_ref: Arc<EventLoopRef>,
    ) -> Result<Receiver<bool>> {
        Self::do_process(this, ctx, readable, on_end, Some(io_loop_ref))
    }

    /// Push-driven read loop for objects whose JS owner is a `Duplex`.
    ///
    /// [`ReadableStream::process_callback_loop`] stays the authority for plain
    /// streams and is deliberately left untouched. A JS `Duplex` (see
    /// `net.Socket`) already implements flowing/paused and high-water-mark
    /// backpressure, so running that state machine alongside it gives two
    /// disagreeing owners of "should we keep reading". This loop instead hands
    /// every chunk to `owner.push()`: the return value is the consumer's
    /// backpressure signal, and `false` parks the reader until
    /// [`ReadableStreamInner::resume_reading`] fires from `_read`/`resume`.
    fn process_callback_loop_duplex<
        T: AsyncRead + Send + 'static + 'js + Unpin,
        C: FnOnce() + Sized + 'js,
    >(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
        readable: T,
        on_end: C,
        owner: Object<'js>,
        io_loop_ref: Arc<EventLoopRef>,
    ) -> Result<Receiver<bool>> {
        let ctx2 = ctx.clone();
        let handoff_completed = Arc::new(AtomicBool::new(false));
        let handoff_completed2 = handoff_completed.clone();

        ctx.spawn_exit(async move {
            let this2 = this.clone();
            let ctx3 = ctx2.clone();

            let borrow = this2.borrow();
            let inner = borrow.inner();
            let mut destroy_rx = inner.destroy_tx.subscribe();
            let is_ended = inner.is_ended;
            let mut is_destroyed = inner.is_destroyed;
            let mut resume_rx = inner.data_listener_attached_tx.subscribe();
            let paused = inner.paused.clone();
            let ba_buffer = inner.buffer.clone();
            let handoff_waiter = inner.handoff_waiter.clone();
            let mut handoff_rx = inner.handoff_notify.subscribe();
            drop(borrow);

            let read_function = async move {
                let mut reader = Some(BufReader::new(readable));
                let mut buffer = Vec::<u8>::with_capacity(DEFAULT_BUFFER_SIZE);
                let mut error_value = None;
                let io_loop_ref = io_loop_ref;
                let mut reached_eof = false;

                if !is_ended && !is_destroyed {
                    if complete_readable_handoff(
                        &mut reader,
                        &ba_buffer,
                        &mut buffer,
                        &handoff_waiter,
                    )? {
                        handoff_completed2.store(true, Ordering::SeqCst);
                        return Ok(());
                    }

                    'read: loop {
                        if !event_loop::keeps_event_loop_alive(&io_loop_ref) {
                            break;
                        }

                        while paused.load(Ordering::Acquire) {
                            tokio::select! {
                                error = destroy_rx.recv() => {
                                    set_destroyed_and_error(&mut is_destroyed, &mut error_value, error);
                                    break 'read;
                                }
                                _ = resume_rx.recv() => {},
                                _ = handoff_rx.recv() => {
                                    if complete_readable_handoff(
                                        &mut reader,
                                        &ba_buffer,
                                        &mut buffer,
                                        &handoff_waiter,
                                    )? {
                                        handoff_completed2.store(true, Ordering::SeqCst);
                                        return Ok(());
                                    }
                                }
                            }
                        }

                        tokio::select! {
                            result = reader.as_mut().expect("reader").read_buf(&mut buffer) => {
                                let bytes_read = result.or_throw(&ctx3)?;
                                if bytes_read == 0 {
                                    reached_eof = true;
                                    break;
                                }

                                // Hand the chunk over. `false` means the Duplex
                                // buffered it and is at its high-water mark:
                                // accepted, but read no further until the
                                // consumer asks for more.
                                match duplex_push(&owner, &ctx3, &buffer)? {
                                    Some(true) => {},
                                    Some(false) => {
                                        // Buffered, but the consumer is at its
                                        // high-water mark. Park until
                                        // `_read`/`resume` says otherwise.
                                        tokio::select! {
                                            error = destroy_rx.recv() => {
                                                set_destroyed_and_error(&mut is_destroyed, &mut error_value, error);
                                                break 'read;
                                            }
                                            _ = resume_rx.recv() => {},
                                            _ = handoff_rx.recv() => {
                                                if complete_readable_handoff(
                                                    &mut reader,
                                                    &ba_buffer,
                                                    &mut buffer,
                                                    &handoff_waiter,
                                                )? {
                                                    handoff_completed2.store(true, Ordering::SeqCst);
                                                    return Ok(());
                                                }
                                            }
                                        }
                                    }
                                    None => {
                                        // No `push` on the owner after all;
                                        // fall back to the classic signal.
                                        Self::emit_str(
                                            this2.clone(),
                                            &ctx3,
                                            "data",
                                            vec![Buffer(buffer.clone()).into_js(&ctx3)?],
                                            false
                                        )?;
                                    }
                                }
                                buffer.clear();

                                if complete_readable_handoff(
                                    &mut reader,
                                    &ba_buffer,
                                    &mut buffer,
                                    &handoff_waiter,
                                )? {
                                    handoff_completed2.store(true, Ordering::SeqCst);
                                    return Ok(());
                                }
                            }
                            error = destroy_rx.recv() => {
                                set_destroyed_and_error(&mut is_destroyed, &mut error_value, error);
                                break;
                            }
                            _ = handoff_rx.recv() => {
                                if complete_readable_handoff(
                                    &mut reader,
                                    &ba_buffer,
                                    &mut buffer,
                                    &handoff_waiter,
                                )? {
                                    handoff_completed2.store(true, Ordering::SeqCst);
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                if handoff_completed2.load(Ordering::SeqCst) {
                    return Ok(());
                }

                {
                    let mut borrow = this2.borrow_mut();
                    let inner = borrow.inner_mut();
                    if is_destroyed {
                        inner.is_destroyed = true;
                    } else {
                        inner.is_ended = true;
                    }
                }

                if !is_destroyed {
                    // EOF: `push(null)` is what ends the JS readable side; the
                    // Duplex emits `end` when the consumer reaches it.
                    if reached_eof {
                        let _ = duplex_end(&owner, &ctx3);
                        on_end();
                    } else {
                        let _ = duplex_end(&owner, &ctx3);
                    }
                }

                if let Some(error_value) = error_value {
                    return Err(ctx3.throw(error_value));
                }

                Ok::<_, Error>(())
            }
            .await;

            let had_error = read_function.emit_error("readable", &ctx2, this.clone())?;

            Ok::<_, Error>(had_error)
        })
    }

    fn request_handoff<T: Send + 'static>(
        this: Class<'js, Self>,
        _ctx: &Ctx<'js>,
    ) -> Result<Receiver<(T, Vec<u8>)>> {
        let (tx, rx) = oneshot::channel();
        let mut borrow = this.borrow_mut();
        let inner = borrow.inner_mut();
        *inner.handoff_waiter.lock().unwrap() = Some(Box::new(tx));
        let _ = inner.handoff_notify.send(());
        drop(borrow);
        Ok(rx)
    }

    fn process_handoff<T: AsyncRead + Send + 'static + 'js + Unpin>(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
        readable: T,
    ) -> Result<(Receiver<bool>, Receiver<(T, Vec<u8>)>)> {
        let (handoff_tx, handoff_rx) = oneshot::channel();
        {
            let mut borrow = this.borrow_mut();
            let inner = borrow.inner_mut();
            *inner.handoff_waiter.lock().unwrap() = Some(Box::new(handoff_tx));
        }
        let completion_rx = Self::do_process(this, ctx, readable, || {}, None)?;
        Ok((completion_rx, handoff_rx))
    }

    fn do_process<T: AsyncRead + Send + 'static + 'js + Unpin, C: FnOnce() + Sized + 'js>(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
        readable: T,
        on_end: C,
        io_loop_ref: Option<Arc<EventLoopRef>>,
    ) -> Result<Receiver<bool>> {
        let ctx2 = ctx.clone();
        let handoff_completed = Arc::new(AtomicBool::new(false));
        let handoff_completed2 = handoff_completed.clone();
        let io_loop_ref = io_loop_ref;

        ctx.spawn_exit(async move {
            let this2 = this.clone();
            let ctx3 = ctx2.clone();

            let borrow = this2.borrow();
            let inner = borrow.inner();
            let mut destroy_rx = inner.destroy_tx.subscribe();
            let is_ended = inner.is_ended;
            let mut is_destroyed = inner.is_destroyed;
            let emit_close = inner.emit_close;

            let mut listener_attached_tx = inner.data_listener_attached_tx.subscribe();
            let ba_buffer = inner.buffer.clone();
            let handoff_waiter = inner.handoff_waiter.clone();
            let mut handoff_rx = inner.handoff_notify.subscribe();
            let mut has_data = false;
            drop(borrow);

            let read_function = async move {
                let mut reader = Some(BufReader::new(readable));
                let mut buffer = Vec::<u8>::with_capacity(DEFAULT_BUFFER_SIZE);
                let mut last_state = ReadableState::Init;
                let mut error_value = None;
                let io_loop_ref = io_loop_ref;

                if !is_ended && !is_destroyed {
                    if complete_readable_handoff(
                        &mut reader,
                        &ba_buffer,
                        &mut buffer,
                        &handoff_waiter,
                    )?
                    {
                        handoff_completed2.store(true, Ordering::SeqCst);
                        return Ok(());
                    }

                    loop {
                        if let Some(ref lr) = io_loop_ref {
                            if !event_loop::keeps_event_loop_alive(lr) {
                                break;
                            }
                        }

                        if complete_readable_handoff(
                            &mut reader,
                            &ba_buffer,
                            &mut buffer,
                            &handoff_waiter,
                        )?
                        {
                            handoff_completed2.store(true, Ordering::SeqCst);
                            return Ok(());
                        }

                        tokio::select! {
                            _ = async {
                                if let Some(lr) = io_loop_ref.clone() {
                                    event_loop::wait_until_io_unref(lr).await;
                                }
                            }, if io_loop_ref.is_some() => {
                                break;
                            }
                            result = reader.as_mut().expect("reader").read_buf(&mut buffer) => {
                                let bytes_read = result.or_throw(&ctx3)?;

                                let mut state = this2.borrow().inner().state.clone();
                                if !has_data && state == ReadableState::Init && bytes_read > 0 {
                                    this2.borrow_mut().inner_mut().state = ReadableState::Paused;
                                    state =  ReadableState::Paused;
                                    has_data = true;
                                }

                                match state {
                                    ReadableState::Flowing => {
                                        if last_state == ReadableState::Paused {
                                            if let Some(empty_buffer) = ba_buffer.read(None) {
                                                buffer.extend(empty_buffer);
                                            }
                                        }

                                        if buffer.is_empty() {
                                            if complete_readable_handoff(
                                                &mut reader,
                                                &ba_buffer,
                                                &mut buffer,
                                                &handoff_waiter,
                                            )?
                                            {
                                                handoff_completed2.store(true, Ordering::SeqCst);
                                                return Ok(());
                                            }
                                            break;
                                        }

                                        Self::emit_str(
                                            this2.clone(),
                                            &ctx3,
                                            "data",
                                            vec![Buffer(buffer.clone()).into_js(&ctx3)?],
                                            false
                                        )?;
                                        buffer.clear();
                                    },
                                    ReadableState::Paused => {
                                        if bytes_read == 0 {
                                            if complete_readable_handoff(
                                                &mut reader,
                                                &ba_buffer,
                                                &mut buffer,
                                                &handoff_waiter,
                                            )?
                                            {
                                                handoff_completed2.store(true, Ordering::SeqCst);
                                                return Ok(());
                                            }
                                            break;
                                        } else if !this2.borrow().inner().has_consumer() {
                                            // Nobody consumes this stream: buffering
                                            // past the high-water mark would block on
                                            // backpressure until the child exits, and
                                            // the exit path itself waits for this loop.
                                            // Node still emits `close` here, so drop the
                                            // bytes and keep draining to EOF.
                                            buffer.clear();
                                            buffer.reserve(DEFAULT_BUFFER_SIZE);
                                        } else {
                                            let write_buffer_future = ba_buffer.write_consuming(&mut buffer);
                                            Self::emit_str(
                                                this2.clone(),
                                                &ctx3,
                                                "readable",
                                                vec![],
                                                false
                                            )?;
                                            let mut handoff_requested = false;
                                            tokio::select!{
                                                capacity = write_buffer_future => {
                                                    buffer.clear();
                                                    buffer.reserve(buffer.capacity()-capacity);
                                                }
                                                error = destroy_rx.recv()  => {
                                                    set_destroyed_and_error(&mut is_destroyed,  &mut error_value, error);
                                                    break;
                                                }
                                                _ = listener_attached_tx.recv() => {
                                                    ba_buffer.clear().await
                                                }
                                                _ = handoff_rx.recv() => {
                                                    handoff_requested = true;
                                                }
                                            }
                                            if handoff_requested
                                                && complete_readable_handoff(
                                                    &mut reader,
                                                    &ba_buffer,
                                                    &mut buffer,
                                                    &handoff_waiter,
                                                )?
                                            {
                                                handoff_completed2.store(true, Ordering::SeqCst);
                                                return Ok(());
                                            }
                                        }
                                    },
                                    _ => {
                                        // Init (or an unexpected state): no consumer has
                                        // switched us to Flowing yet. Once the pipe hits
                                        // EOF `read` keeps returning 0 forever, so stop
                                        // instead of spinning at 100% CPU and stalling the
                                        // whole event loop (which also blocks the child
                                        // reap path that emits `exit`).
                                        if bytes_read == 0 {
                                            break;
                                        }
                                        if !this2.borrow().inner().has_consumer() {
                                            // Nobody will read these bytes; drain and drop.
                                            buffer.clear();
                                            buffer.reserve(DEFAULT_BUFFER_SIZE);
                                        }
                                    },
                                }

                                last_state = state;


                            }
                            error = destroy_rx.recv()  => {
                                set_destroyed_and_error(&mut is_destroyed,  &mut error_value, error);
                                break;
                            },
                            _ = handoff_rx.recv() => {
                                if complete_readable_handoff(
                                    &mut reader,
                                    &ba_buffer,
                                    &mut buffer,
                                    &handoff_waiter,
                                )?
                                {
                                    handoff_completed2.store(true, Ordering::SeqCst);
                                    return Ok(());
                                }
                            },
                        }
                    }
                }

                if handoff_completed2.load(Ordering::SeqCst) {
                    return Ok(());
                }

                let mut borrow = this2.borrow_mut();
                let inner = borrow.inner_mut();
                inner.buffer.close().await;
                if is_destroyed {
                    inner.is_destroyed = true;
                } else {
                    inner.is_ended = true;
                }

                drop(borrow);

                if !is_destroyed {
                    on_end();
                    Self::emit_str(this2.clone(), &ctx3, "end", vec![], false)?;
                }

                if let Some(error_value) = error_value{
                    return Err(ctx3.throw(error_value));
                }

                Ok::<_, Error>(())
            }
            .await;

            let had_error = read_function.emit_error("readable",&ctx2, this.clone())?;

            if emit_close && !handoff_completed.load(Ordering::SeqCst) {
                Self::emit_close(this,&ctx2,had_error)?;
            }

            Ok::<_, Error>(had_error)
        })
    }
}

/// Hand `chunk` to the JS `Duplex` owner via `push`.
///
/// `Some(true)` — accepted and the consumer wants more.
/// `Some(false)` — accepted, but the Duplex is at its high-water mark.
/// `None` — the owner has no usable `push`; caller falls back to emitting.
fn duplex_push<'js>(owner: &Object<'js>, ctx: &Ctx<'js>, chunk: &[u8]) -> Result<Option<bool>> {
    let Some(push) = owner.get::<_, Value>("push")?.as_function().cloned() else {
        return Ok(None);
    };
    let payload = Buffer(chunk.to_vec()).into_js(ctx)?;
    Ok(Some(push.call((This(owner.clone()), payload))?))
}

/// Signal end-of-stream to the JS `Duplex` readable side.
///
/// Prefers the owner's `_onEof` hook, which also resumes a stream that has no
/// consumer reading (so `end` still fires for a socket that only listens for
/// it). Falls back to `push(null)` for objects without the hook.
fn duplex_end<'js>(owner: &Object<'js>, ctx: &Ctx<'js>) -> Result<()> {
    if let Some(hook) = owner.get::<_, Value>("_onEof")?.as_function().cloned() {
        hook.call::<_, ()>((This(owner.clone()),))?;
        return Ok(());
    }
    if let Some(push) = owner.get::<_, Value>("push")?.as_function().cloned() {
        push.call::<_, bool>((This(owner.clone()), Null))?;
    }
    let _ = ctx;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc};

    use lexe_events::{EventEmitter, EventKey};
    use tokio::io::AsyncBufReadExt;
    use tokio::sync::{broadcast, oneshot};

    use super::*;

    #[test]
    fn data_listener_removed_returns_to_paused() {
        let emitter = EventEmitter {
            events: Arc::new(RwLock::new(Vec::new())),
        };
        let mut inner = ReadableStreamInner::new(emitter, true);
        inner.state = ReadableState::Flowing;
        inner.listener = Some("data");

        let key = EventKey::String(std::rc::Rc::from("data"));
        inner.on_event_changed(key, false).unwrap();

        assert_eq!(inner.state, ReadableState::Paused);
        assert_eq!(inner.listener, None);
    }

    #[tokio::test]
    async fn handoff_prefix_preserves_buffer_order() {
        let ba_buffer = BytearrayBuffer::new(8);
        ba_buffer.write_forced(b"ba");

        let mut local = b"lo".to_vec();
        let mut reader = Some(BufReader::new(Cursor::new(b"reader".to_vec())));
        reader.as_mut().unwrap().fill_buf().await.unwrap();

        let handoff_waiter: Arc<Mutex<Option<Box<dyn std::any::Any + Send>>>> =
            Arc::new(Mutex::new(None));
        let (tx, mut rx) = oneshot::channel::<(Cursor<Vec<u8>>, Vec<u8>)>();
        *handoff_waiter.lock().unwrap() = Some(Box::new(tx));

        assert!(complete_readable_handoff::<Cursor<Vec<u8>>>(
            &mut reader,
            &ba_buffer,
            &mut local,
            &handoff_waiter,
        )
        .unwrap());
        assert!(reader.is_none());

        let (_inner, prefix) = rx.try_recv().unwrap();
        assert_eq!(prefix, b"baloreader");
    }

    #[tokio::test]
    async fn handoff_waiter_available_without_broadcast_subscriber() {
        let (notify_tx, _) = broadcast::channel(1);
        let handoff_waiter: Arc<Mutex<Option<Box<dyn std::any::Any + Send>>>> =
            Arc::new(Mutex::new(None));
        let (tx, mut rx) = oneshot::channel::<(Cursor<Vec<u8>>, Vec<u8>)>();
        *handoff_waiter.lock().unwrap() = Some(Box::new(tx));

        assert!(notify_tx.send(()).is_err());

        let mut reader = Some(BufReader::new(Cursor::new(b"payload".to_vec())));
        reader.as_mut().unwrap().fill_buf().await.unwrap();
        let mut local = Vec::new();
        let ba_buffer = BytearrayBuffer::new(16);

        assert!(complete_readable_handoff::<Cursor<Vec<u8>>>(
            &mut reader,
            &ba_buffer,
            &mut local,
            &handoff_waiter,
        )
        .unwrap());

        let (_, prefix) = rx.try_recv().unwrap();
        assert_eq!(prefix, b"payload");
    }

    #[tokio::test]
    async fn handoff_prefix_no_duplicate_after_cancelled_partial_write() {
        let ba_buffer = BytearrayBuffer::new(4);
        ba_buffer.write_forced(&[1, 2, 3]);

        let mut local = vec![4u8, 5, 6, 7];
        let handoff_waiter: Arc<Mutex<Option<Box<dyn std::any::Any + Send>>>> =
            Arc::new(Mutex::new(None));
        let (tx, mut rx) = oneshot::channel::<(Cursor<Vec<u8>>, Vec<u8>)>();
        *handoff_waiter.lock().unwrap() = Some(Box::new(tx));

        {
            let write = ba_buffer.write_consuming(&mut local);
            tokio::pin!(write);
            tokio::select! {
                _ = &mut write => {},
                _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {},
            }
        }

        let mut reader = Some(BufReader::new(Cursor::new(Vec::new())));
        assert!(complete_readable_handoff::<Cursor<Vec<u8>>>(
            &mut reader,
            &ba_buffer,
            &mut local,
            &handoff_waiter,
        )
        .unwrap());

        let (_, prefix) = rx.try_recv().unwrap();
        assert_eq!(prefix, vec![1, 2, 3, 4, 5, 6, 7]);
    }
}
