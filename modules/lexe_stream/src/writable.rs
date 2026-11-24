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
    Class, Ctx, Error, Exception, Function, JsLifetime, Result, Value,
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

        proto.set("write", Func::from(Self::write))?;

        proto.set("end", Func::from(Self::end))?;

        Ok(())
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

    fn write_flushed(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        cb: Opt<Function<'js>>,
    ) -> Result<()> {
        Self::do_write(this, ctx, value, cb, true)
    }

    fn write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        cb: Opt<Function<'js>>,
    ) -> Result<()> {
        Self::do_write(this, ctx, value, cb, false)
    }

    fn do_write(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
        cb: Opt<Function<'js>>,
        flush: bool,
    ) -> Result<()> {
        let callback = cb.0;

        if this
            .borrow()
            .inner()
            .command_tx
            .send(WriteCommand::Write(value, callback.clone(), flush))
            .is_err()
        {
            if let Some(cb) = callback {
                let err = Exception::throw_message(&ctx, "This stream has been ended")
                    .into_value(&ctx)?;

                () = cb.call((err,))?;
            }
        }

        Ok(())
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
                                        writer.shutdown().await.or_throw(&ctx3)?;
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
                Self::emit_close(this,&ctx2,had_error)?;
            }

            Ok::<_, Error>(had_error)
        })
    }
}
