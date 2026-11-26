// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    any::Any,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
};

use lexe_context::CtxExtension;
use lexe_events::{EmitError, Emitter, EventEmitter, EventList};
use lexe_utils::{
    bytes::ObjectBytes,
    error::ErrorExtensions,
    event_loop::{self, EventLoopRef},
    result::ResultExt,
};
use rquickjs::{
    class::{Trace, Tracer},
    prelude::{Func, Opt, This},
    Class, Ctx, Error, Exception, Function, IntoJs, JsLifetime, Result, Value,
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt, BufWriter},
    sync::{
        broadcast::{self, Sender},
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot::{self, Receiver},
    },
};

use super::{impl_stream_events, set_destroyed_and_error, SteamEvents};

pub struct WritableStreamInner<'js> {
    emitter: EventEmitter<'js>,
    command_tx: UnboundedSender<WriteCommand<'js>>,
    command_rx: Option<UnboundedReceiver<WriteCommand<'js>>>,
    is_finished: bool,
    is_closed: bool,
    #[allow(dead_code)]
    errored: bool,
    emit_close: bool,
    is_destroyed: bool,
    destroy_tx: Sender<Option<Value<'js>>>,
    handoff_waiter: Arc<Mutex<Option<Box<dyn Any + Send>>>>,
}

impl<'js> Trace<'js> for WritableStreamInner<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

impl<'js> WritableStreamInner<'js> {
    pub fn new(emitter: EventEmitter<'js>, emit_close: bool) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        let (destroy_tx, _) = broadcast::channel::<Option<Value<'js>>>(1);

        Self {
            command_tx: tx,
            command_rx: Some(rx),
            emitter,
            is_finished: false,
            is_closed: false,
            is_destroyed: false,
            destroy_tx,
            emit_close,
            errored: false,
            handoff_waiter: Arc::new(Mutex::new(None)),
        }
    }

    /// Whether the writable side has finished (end() completed).
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.is_finished
    }

    /// Whether the writable side has been destroyed.
    #[inline]
    pub fn is_destroyed(&self) -> bool {
        self.is_destroyed
    }

    /// Whether the writable side has closed.
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.is_closed
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum WriteCommand<'js> {
    End,
    Write(Value<'js>, Option<Function<'js>>, bool),
    Flush,
    FlushBarrier(oneshot::Sender<()>),
    Handoff,
}

#[rquickjs::class]
#[derive(rquickjs::class::Trace)]
pub struct DefaultWritableStream<'js> {
    inner: WritableStreamInner<'js>,
}

unsafe impl<'js> JsLifetime<'js> for DefaultWritableStream<'js> {
    type Changed<'to> = DefaultWritableStream<'to>;
}

impl<'js> DefaultWritableStream<'js> {
    fn with_emitter(ctx: Ctx<'js>, emitter: EventEmitter<'js>) -> Result<Class<'js, Self>> {
        Class::instance(
            ctx,
            Self {
                inner: WritableStreamInner::new(emitter, true),
            },
        )
    }

    pub fn new(ctx: Ctx<'js>) -> Result<Class<'js, Self>> {
        Self::with_emitter(ctx, EventEmitter::new())
    }
}

impl_stream_events!(DefaultWritableStream);
impl<'js> Emitter<'js> for DefaultWritableStream<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.inner.emitter.get_event_list()
    }
}

impl<'js> WritableStream<'js> for DefaultWritableStream<'js> {
    fn inner_mut(&mut self) -> &mut WritableStreamInner<'js> {
        &mut self.inner
    }

    fn inner(&self) -> &WritableStreamInner<'js> {
        &self.inner
    }
}

pub trait WritableStream<'js>
where
    Self: Emitter<'js> + SteamEvents<'js>,
{
    fn inner_mut(&mut self) -> &mut WritableStreamInner<'js>;

    fn inner(&self) -> &WritableStreamInner<'js>;

    fn add_writable_stream_prototype(ctx: &Ctx<'js>) -> Result<()> {
        let proto = Class::<Self>::prototype(ctx)?
            .or_throw_msg(ctx, &["Prototype for ", Self::NAME, " not found"].concat())?;

        proto.set("write", Func::from(Self::js_write))?;

        proto.set("end", Func::from(Self::js_end))?;

        proto.set("destroy", Func::from(Self::destroy))?;

        let state_fn = Function::new(ctx.clone(), Self::get_writable_flags)?;
        let install: Function = ctx.eval(
            r#"(function (proto, stateFn) {
  Object.defineProperties(proto, {
    writable: { get() { const s = stateFn.call(this); return !s[0] && !s[1]; }, enumerable: true, configurable: true },
    writableEnded: { get() { return stateFn.call(this)[0]; }, enumerable: true, configurable: true },
    writableFinished: { get() { return stateFn.call(this)[0]; }, enumerable: true, configurable: true },
    destroyed: { get() { return stateFn.call(this)[1]; }, enumerable: true, configurable: true },
    closed: { get() { return stateFn.call(this)[2]; }, enumerable: true, configurable: true },
  });
  if (typeof proto.cork !== "function") proto.cork = function () {};
  if (typeof proto.uncork !== "function") proto.uncork = function () {};
  if (typeof proto.setDefaultEncoding !== "function") {
    proto.setDefaultEncoding = function (_encoding) { return this; };
  }
})"#,
        )?;
        install.call::<_, ()>((proto, state_fn))?;

        Ok(())
    }

    fn get_writable_flags(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let borrow = this.borrow();
        let inner = borrow.inner();
        let arr = rquickjs::Array::new(ctx.clone())?;
        arr.set(0, inner.is_finished)?;
        arr.set(1, inner.is_destroyed)?;
        arr.set(2, inner.is_closed)?;
        arr.into_js(&ctx)
    }

    fn destroy(this: This<Class<'js, Self>>, error: Opt<Value<'js>>) -> Class<'js, Self> {
        if !this.borrow().inner().is_finished {
            let mut borrow = this.borrow_mut();
            let inner = borrow.inner_mut();
            inner.is_finished = true;
            inner.is_destroyed = true;
            let tx = inner.destroy_tx.clone();
            drop(borrow);
            //it doesn't matter if channel is closed because then writable is already closed
            let _ = tx.send(error.0);
        } else {
            this.borrow_mut().inner_mut().is_destroyed = true;
        }
        this.0
    }

    fn end(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        if !this.borrow().inner().is_finished {
            let mut borrow = this.borrow_mut();
            let inner = borrow.inner_mut();
            inner.is_finished = true;
            let tx = inner.command_tx.clone();
            drop(borrow);
            //it doesn't matter if channel is closed because then writable is already closed
            let _ = tx.send(WriteCommand::End);
        }
        this.0
    }

    fn js_end(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        chunk: Opt<Value<'js>>,
        encoding_or_cb: Opt<Value<'js>>,
        cb: Opt<Option<Function<'js>>>,
    ) -> Result<Class<'js, Self>> {
        let explicit_cb = cb.0.flatten();
        let (data_chunk, callback) = match chunk.0 {
            Some(v) if v.is_function() => (None, Some(Function::from_value(v)?)),
            Some(v) if v.is_undefined() || v.is_null() => {
                let callback = explicit_cb.or_else(|| {
                    encoding_or_cb
                        .0
                        .as_ref()
                        .and_then(|ev| ev.as_function().cloned())
                });
                (None, callback)
            },
            Some(v) => {
                let callback = explicit_cb.or_else(|| {
                    encoding_or_cb
                        .0
                        .as_ref()
                        .and_then(|ev| ev.as_function().cloned())
                });
                (Some((v, encoding_or_cb.0)), callback)
            },
            None => (None, explicit_cb),
        };

        if let Some((chunk_val, enc_opt)) = data_chunk {
            let enc_val = enc_opt.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
            let _ = Self::do_write_encoded(
                This(this.0.clone()),
                ctx.clone(),
                chunk_val,
                Some(enc_val),
                None,
                false,
            )?;
        }

        let finish_event_pending = !this.borrow().inner().is_finished;
        if let Some(cb_fn) = callback {
            if finish_event_pending {
                if let Ok(once_fn) = this.0.get::<_, Function>("once") {
                    let _ = once_fn.call::<_, Value>((This(this.0.clone()), "finish", cb_fn));
                }
            } else {
                ctx.clone().spawn_exit(async move {
                    let _ = cb_fn.call::<_, ()>(());
                    Ok(())
                })?;
            }
        }

        Ok(Self::end(this))
    }

    #[allow(dead_code)]
    fn flush(this: Class<'js, Self>, ctx: &Ctx<'js>) -> Result<()> {
        let _ = this
            .borrow()
            .inner()
            .command_tx
            .send(WriteCommand::Flush)
            .or_throw(ctx);
        Ok(())
    }

    fn js_write_flushed(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        encoding_or_cb: Opt<Value<'js>>,
        cb: Opt<Option<Function<'js>>>,
    ) -> Result<bool> {
        Self::do_write_encoded(this, ctx, value, encoding_or_cb.0, cb.0.flatten(), true)
    }

    fn js_write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        encoding_or_cb: Opt<Value<'js>>,
        cb: Opt<Option<Function<'js>>>,
    ) -> Result<bool> {
        Self::do_write_encoded(this, ctx, value, encoding_or_cb.0, cb.0.flatten(), false)
    }

    fn write_flushed(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        cb: Opt<Function<'js>>,
    ) -> Result<()> {
        let _ = Self::do_write(this, ctx, value, Opt(Some(cb.0)), true)?;
        Ok(())
    }

    #[allow(dead_code)]
    fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        cb: Opt<Function<'js>>,
    ) -> Result<()> {
        let _ = Self::do_write(this, ctx, value, Opt(Some(cb.0)), false)?;
        Ok(())
    }

    fn do_write_encoded(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        mut value: Value<'js>,
        encoding_or_cb: Option<Value<'js>>,
        cb: Option<Function<'js>>,
        flush: bool,
    ) -> Result<bool> {
        let mut encoding: Option<String> = None;
        let callback = if let Some(explicit) = cb {
            if let Some(enc_val) = encoding_or_cb {
                if let Some(s) = enc_val.as_string() {
                    encoding = Some(s.to_string()?);
                }
            }
            Some(explicit)
        } else if let Some(enc_or_cb) = encoding_or_cb {
            if let Some(f) = enc_or_cb.as_function().cloned() {
                Some(f)
            } else {
                if let Some(s) = enc_or_cb.as_string() {
                    encoding = Some(s.to_string()?);
                }
                None
            }
        } else {
            None
        };

        if value.is_string() {
            if let Some(ref enc) = encoding {
                if !enc.is_empty()
                    && !enc.eq_ignore_ascii_case("utf8")
                    && !enc.eq_ignore_ascii_case("utf-8")
                {
                    if let Ok(encode_fn) = ctx.eval::<Function, _>(
                        "(str, enc) => typeof Buffer !== 'undefined' ? Buffer.from(str, enc) : str",
                    ) {
                        value = encode_fn.call((value, enc.as_str()))?;
                    }
                }
            }
        }

        Self::do_write(this, ctx, value, Opt(Some(callback)), flush)
    }

    fn do_write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        cb: Opt<Option<Function<'js>>>,
        flush: bool,
    ) -> Result<bool> {
        let callback = cb.0.flatten();

        let already_ended = {
            let borrow = this.borrow();
            let inner = borrow.inner();
            inner.is_finished || inner.is_destroyed
        };
        if already_ended {
            if let Some(cb_fn) = callback {
                let err = Exception::from_message(ctx.clone(), "write after end")?.into_value();
                ctx.clone().spawn_exit(async move {
                    let _ = cb_fn.call::<_, ()>((err,));
                    Ok(())
                })?;
            }
            return Ok(false);
        }

        if this
            .borrow()
            .inner()
            .command_tx
            .send(WriteCommand::Write(value, callback.clone(), flush))
            .is_err()
        {
            if let Some(cb_fn) = callback {
                let err = Exception::from_message(ctx.clone(), "write after end")?.into_value();
                ctx.clone().spawn_exit(async move {
                    let _ = cb_fn.call::<_, ()>((err,));
                    Ok(())
                })?;
            }
            return Ok(false);
        }

        Ok(true)
    }

    fn request_flush_barrier(this: Class<'js, Self>, ctx: &Ctx<'js>) -> Result<Receiver<()>> {
        let (tx, rx) = oneshot::channel();
        this.borrow()
            .inner()
            .command_tx
            .send(WriteCommand::FlushBarrier(tx))
            .or_throw(ctx)?;
        Ok(rx)
    }

    fn request_handoff<T: Send + 'static>(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
    ) -> Result<Receiver<T>> {
        let (tx, rx) = oneshot::channel();
        let mut borrow = this.borrow_mut();
        let inner = borrow.inner_mut();
        *inner.handoff_waiter.lock().unwrap() = Some(Box::new(tx));
        inner.command_tx.send(WriteCommand::Handoff).or_throw(ctx)?;
        drop(borrow);
        Ok(rx)
    }

    fn process<T: AsyncWrite + Send + 'static + 'js + Unpin>(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
        writable: T,
    ) -> Result<Receiver<bool>> {
        Self::process_loop(this, ctx, writable, None)
    }

    fn process_loop<T: AsyncWrite + Send + 'static + 'js + Unpin>(
        this: Class<'js, Self>,
        ctx: &Ctx<'js>,
        writable: T,
        io_loop_ref: Option<Arc<EventLoopRef>>,
    ) -> Result<Receiver<bool>> {
        let mut borrow = this.borrow_mut();
        let inner = borrow.inner_mut();
        let mut is_destroyed = inner.is_destroyed;
        let emit_close = inner.emit_close;
        let mut command_rx = inner
            .command_rx
            .take()
            .expect("rx from writable process already taken!");
        let mut destroy_rx = inner.destroy_tx.subscribe();
        let handoff_waiter = inner.handoff_waiter.clone();
        let mut error_value = None;

        drop(borrow);
        let ctx2 = ctx.clone();
        let handoff_completed = Arc::new(AtomicBool::new(false));
        let handoff_completed2 = handoff_completed.clone();
        let io_loop_ref = io_loop_ref;

        ctx.spawn_exit(async move {
            let ctx3 = ctx2.clone();
            let ctx4 = ctx2.clone();
            let this2 = this.clone();
            let write_function = async move {
                let mut writer = BufWriter::new(writable);
                let io_loop_ref = io_loop_ref;

                // `end()` may be called before a transport is attached (for
                // example immediately from a HTTP upgrade handler).  In that
                // case queued writes must still be flushed before the End
                // command shuts the transport down.
                if !is_destroyed {
                    loop {
                        if let Some(ref lr) = io_loop_ref {
                            if !event_loop::keeps_event_loop_alive(lr) {
                                break;
                            }
                        }

                        tokio::select! {
                            _ = async {
                                if let Some(lr) = io_loop_ref.clone() {
                                    event_loop::wait_until_io_unref(lr).await;
                                }
                            }, if io_loop_ref.is_some() => {
                                break;
                            }
                            command = command_rx.recv() => {
                                 match command {
                                    Some(WriteCommand::Write(value, cb, flush)) => {
                                        let bytes = ObjectBytes::from(&ctx3, &value)?;
                                        let data = bytes.as_bytes(&ctx4)?;
                                        let result = async {
                                            writer.write_all(data).await?;
                                            if flush {
                                                writer.flush().await.or_throw(&ctx3)?;
                                            }
                                            Ok::<_, Error>(())
                                        }.await;

                                        match result {
                                            Ok(_) => {
                                                if let Some(cb) = cb {
                                                    () = cb.call(())?;
                                                }
                                            },
                                            Err(err) => {
                                                let err2 = Exception::throw_message(&ctx3, &err.to_string());
                                                if let Some(cb) = cb {
                                                    let err = err.into_value(&ctx3)?;
                                                    () = cb.call((err,))?;
                                                }
                                                return Err(err2);
                                            }
                                        }
                                    },
                                    Some(WriteCommand::End) => {
                                        writer.flush().await.or_throw(&ctx3)?;
                                        // On macOS/BSD, shutting down a socket whose peer already closed returns ENOTCONN.
                                        if let Err(err) = writer.shutdown().await {
                                            if err.kind() != std::io::ErrorKind::NotConnected {
                                                return Err(err).or_throw(&ctx3);
                                            }
                                        }
                                        break;
                                    },
                                    Some(WriteCommand::Flush) => writer.flush().await.or_throw(&ctx3)?,
                                    Some(WriteCommand::FlushBarrier(ack)) => {
                                        writer.flush().await.or_throw(&ctx3)?;
                                        let _ = ack.send(());
                                    },
                                    Some(WriteCommand::Handoff) => {
                                        writer.flush().await.or_throw(&ctx3)?;
                                        let inner_writer = writer.into_inner();
                                        if let Some(boxed) = handoff_waiter.lock().unwrap().take() {
                                            let tx = *boxed
                                                .downcast::<oneshot::Sender<T>>()
                                                .expect("handoff waiter type mismatch");
                                            let _ = tx.send(inner_writer);
                                        }
                                        handoff_completed2.store(true, Ordering::SeqCst);
                                        return Ok(());
                                    },
                                    None => break,
                                }
                            },
                            error = destroy_rx.recv() => {
                                set_destroyed_and_error(&mut is_destroyed,  &mut error_value, error);
                                break;
                            }
                        }
                    }
                }

                drop(writer);

                if !is_destroyed {
                    Self::emit_str(this2, &ctx3, "finish", vec![], false)?;
                }

                if let Some(error_value) = error_value{
                    return Err(ctx3.throw(error_value));
                }

                Ok::<_, Error>(())
            }
            .await;

            let had_error = write_function.emit_error("writable", &ctx2, this.clone())?;

            if emit_close && !handoff_completed.load(Ordering::SeqCst) {
                this.borrow_mut().inner_mut().is_closed = true;
                Self::emit_close(this,&ctx2,had_error)?;
            }

            Ok::<_, Error>(had_error)
        })
    }
}
