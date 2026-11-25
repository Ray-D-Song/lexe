use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use lexe_abort::AbortSignal;
use lexe_utils::result::ResultExt;
use rquickjs::{
    class::{OwnedBorrow, Trace, Tracer},
    prelude::This,
    Class, Coerced, Ctx, Exception, FromJs, Function, JsLifetime, Promise, Result, Value,
};

use crate::{
    readable::{
        controller::ReadableStreamControllerOwned,
        default_reader::{
            ReadableStreamDefaultReader, ReadableStreamDefaultReaderOwned,
            ReadableStreamReadRequest,
        },
        objects::{
            ReadableStreamClassObjects, ReadableStreamDefaultReaderObjects, ReadableStreamObjects,
        },
        reader::ReadableStreamReaderClass,
        stream::{ReadableStream, ReadableStreamOwned, ReadableStreamState},
    },
    utils::{
        promise::{
            make_traced_once_fn, on_promise_rejected_traced, promise_resolved_with,
            upon_promise_fulfilment_traced, upon_promise_traced, PromisePrimordials,
            ResolveablePromise, TracedCallback, TracedReaction,
        },
        UnwrapOrUndefined, ValueOrUndefined,
    },
    writable::{
        WritableStream, WritableStreamClassObjects, WritableStreamDefaultWriter,
        WritableStreamDefaultWriterOwned, WritableStreamObjects, WritableStreamOwned,
        WritableStreamState,
    },
};

impl<'js> ReadableStream<'js> {
    pub(super) fn readable_stream_pipe_to(
        ctx: Ctx<'js>,
        source: ReadableStreamOwned<'js>,
        dest: WritableStreamOwned<'js>,
        prevent_close: bool,
        prevent_abort: bool,
        prevent_cancel: bool,
        signal: Option<Class<'js, AbortSignal<'js>>>,
    ) -> Result<Promise<'js>> {
        let (source_stored_error, source_closed) = match source.state {
            ReadableStreamState::Errored(ref stored_error) => (Some(stored_error.clone()), false),
            ReadableStreamState::Closed => (None, true),
            _ => (None, false),
        };
        let dest_stored_error = dest.stored_error();
        let dest_closing = dest.writable_stream_close_queued_or_in_flight()
            || matches!(dest.state, WritableStreamState::Closed);

        let source_controller = source.controller.clone();

        let dest_controller = dest
            .controller
            .clone()
            .expect("pipeTo called on writable stream without controller");

        let (mut source, reader) =
            ReadableStreamReaderClass::acquire_readable_stream_default_reader(ctx.clone(), source)?;

        let source_closed_promise = reader.borrow().generic.closed_promise.promise.clone();

        let (dest, writer) =
            WritableStreamDefaultWriter::acquire_writable_stream_default_writer(&ctx, dest)?;

        let dest_closed_promise = writer.borrow().closed_promise.promise.clone();

        source.disturbed = true;

        let shared = Class::instance(
            ctx.clone(),
            PipeShared {
                current_write: RefCell::new(
                    source
                        .promise_primordials
                        .promise_resolved_with_undefined
                        .clone(),
                ),
                abort_callback: RefCell::new(None),
            },
        )?;

        let promise_primordials = source.promise_primordials.clone();
        let constructor_type_error = source.constructor_type_error.clone();

        let pipe_to = PipeTo {
            source_objects: ReadableStreamClassObjects {
                stream: source.into_inner(),
                controller: source_controller,
                reader,
            },
            dest_objects: WritableStreamClassObjects {
                stream: dest.into_inner(),
                controller: dest_controller,
                writer,
            },
            shared,
            shutting_down: Rc::new(AtomicBool::new(false)),
            signal,
            promise: ResolveablePromise::new(&ctx)?,
            promise_primordials: promise_primordials.clone(),
        };

        // If signal is not undefined,
        if let Some(signal) = &pipe_to.signal {
            let run_abort =
                |ctx: Ctx<'js>, pipe_to: &PipeTo<'js>, sig: &Class<'js, AbortSignal<'js>>| {
                    let error = sig.borrow().reason().unwrap_or_undefined(&ctx);
                    pipe_to.shutdown_with_action(
                        ctx,
                        PipeAction::AbortSignal {
                            prevent_abort,
                            prevent_cancel,
                            error: error.clone(),
                        },
                        Some(error),
                    )
                };

            {
                let signal_ref = signal.borrow();
                if signal_ref.aborted {
                    drop(signal_ref);
                    run_abort(ctx, &pipe_to, signal)?;
                    return Ok(pipe_to.promise.promise);
                }
            }

            #[derive(Trace)]
            struct PipeAbortSignalHandler<'js> {
                pipe_to: PipeTo<'js>,
                signal: Class<'js, AbortSignal<'js>>,
                prevent_abort: bool,
                prevent_cancel: bool,
            }

            impl<'js> TracedCallback<'js> for PipeAbortSignalHandler<'js> {
                fn call(self: Box<Self>, ctx: Ctx<'js>, _: Value<'js>) -> Result<Value<'js>> {
                    let error = self.signal.borrow().reason().unwrap_or_undefined(&ctx);
                    self.pipe_to.shutdown_with_action(
                        ctx.clone(),
                        PipeAction::AbortSignal {
                            prevent_abort: self.prevent_abort,
                            prevent_cancel: self.prevent_cancel,
                            error: error.clone(),
                        },
                        Some(error),
                    )?;
                    Ok(Value::new_undefined(ctx))
                }
            }

            let abort_cb = make_traced_once_fn(
                &ctx,
                PipeAbortSignalHandler {
                    pipe_to: pipe_to.clone(),
                    signal: signal.clone(),
                    prevent_abort,
                    prevent_cancel,
                },
            )?;
            *pipe_to.shared.borrow().abort_callback.borrow_mut() = Some(abort_cb.clone());
            AbortSignal::set_on_abort(This(signal.clone()), ctx.clone(), abort_cb)?;
        }

        // Errors must be propagated forward
        PipeTo::is_or_becomes_errored(
            ctx.clone(),
            source_stored_error,
            source_closed_promise.clone(),
            PipeToErrorKind::Forward {
                pipe_to: pipe_to.clone(),
                prevent_abort,
            },
        )?;

        // Errors must be propagated backward
        PipeTo::is_or_becomes_errored(
            ctx.clone(),
            dest_stored_error,
            dest_closed_promise,
            PipeToErrorKind::Backward {
                pipe_to: pipe_to.clone(),
                prevent_cancel,
            },
        )?;

        // Closing must be propagated forward
        PipeTo::is_or_becomes_closed(
            ctx.clone(),
            source_closed,
            source_closed_promise,
            pipe_to.clone(),
            prevent_close,
        )?;

        // Closing must be propagated backward
        if dest_closing {
            let dest_closed: Value<'js> = constructor_type_error.call((
                "the destination writable stream closed before all data could be piped to it",
            ))?;

            if !prevent_cancel {
                pipe_to.shutdown_with_action(
                    ctx.clone(),
                    PipeAction::CancelSource(dest_closed.clone()),
                    Some(dest_closed),
                )?;
            } else {
                pipe_to.shutdown(ctx.clone(), Some(dest_closed))?;
            }
        }

        let result_promise = pipe_to.promise.promise.clone();
        let pipe_loop_promise = pipe_to.pipe_loop(ctx)?;
        let _ = pipe_loop_promise.set_is_handled();

        Ok(result_promise)
    }
}

/// Mutable JS-visible state shared by every clone of a `PipeTo`. It lives in a single JS object so
/// the garbage collector sees exactly one owner for the promises inside (tracing a shared `Rc` from
/// several holders would decrement their reference counts more than once).
#[rquickjs::class]
pub(crate) struct PipeShared<'js> {
    current_write: RefCell<Promise<'js>>,
    abort_callback: RefCell<Option<Function<'js>>>,
}

unsafe impl<'js> JsLifetime<'js> for PipeShared<'js> {
    type Changed<'to> = PipeShared<'to>;
}

impl<'js> Trace<'js> for PipeShared<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        if let Ok(cw) = self.current_write.try_borrow() {
            cw.trace(tracer);
        }
        if let Ok(cb) = self.abort_callback.try_borrow() {
            cb.trace(tracer);
        }
    }
}

#[derive(Clone)]
struct PipeTo<'js> {
    source_objects: ReadableStreamClassObjects<
        'js,
        ReadableStreamControllerOwned<'js>,
        ReadableStreamDefaultReaderOwned<'js>,
    >,
    dest_objects: WritableStreamClassObjects<'js, WritableStreamDefaultWriterOwned<'js>>,
    shared: Class<'js, PipeShared<'js>>,
    shutting_down: Rc<AtomicBool>,
    signal: Option<Class<'js, AbortSignal<'js>>>,
    promise: ResolveablePromise<'js>,

    promise_primordials: PromisePrimordials<'js>,
}

impl<'js> Trace<'js> for PipeTo<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.source_objects.trace(tracer);
        self.dest_objects.trace(tracer);
        self.shared.trace(tracer);
        self.signal.trace(tracer);
        self.promise.trace(tracer);
        self.promise_primordials.trace(tracer);
    }
}

#[derive(Clone, Trace)]
enum PipeAction<'js> {
    AbortSignal {
        prevent_abort: bool,
        prevent_cancel: bool,
        error: Value<'js>,
    },
    AbortDest(Value<'js>),
    CancelSource(Value<'js>),
    CloseDest,
}

impl<'js> PipeAction<'js> {
    fn run(self, ctx: Ctx<'js>, pipe_to: &PipeTo<'js>) -> Result<Promise<'js>> {
        match self {
            PipeAction::AbortSignal {
                prevent_abort,
                prevent_cancel,
                error,
            } => {
                let mut promises: Vec<Promise<'js>> = Vec::new();
                if !prevent_abort {
                    let dest_objects =
                        WritableStreamObjects::from_class(pipe_to.dest_objects.clone());
                    let p = if matches!(dest_objects.stream.state, WritableStreamState::Writable) {
                        WritableStream::writable_stream_abort(
                            ctx.clone(),
                            dest_objects,
                            Some(error.clone()),
                        )?
                        .0
                    } else {
                        dest_objects
                            .stream
                            .promise_primordials
                            .promise_resolved_with_undefined
                            .clone()
                    };
                    promises.push(p);
                }
                if !prevent_cancel {
                    let source_objects =
                        ReadableStreamObjects::from_class(pipe_to.source_objects.clone());
                    let p = if let ReadableStreamState::Readable = source_objects.stream.state {
                        ReadableStream::readable_stream_cancel(ctx.clone(), source_objects, error)?
                            .0
                    } else {
                        source_objects
                            .stream
                            .promise_primordials
                            .promise_resolved_with_undefined
                            .clone()
                    };
                    promises.push(p);
                }
                pipe_to.promise_primordials.promise_all.call((
                    This(pipe_to.promise_primordials.promise_constructor.clone()),
                    promises,
                ))
            },
            PipeAction::AbortDest(stored_error) => {
                let dest_objects = WritableStreamObjects::from_class(pipe_to.dest_objects.clone());
                let (promise, _) =
                    WritableStream::writable_stream_abort(ctx, dest_objects, Some(stored_error))?;
                Ok(promise)
            },
            PipeAction::CancelSource(stored_error) => {
                let source_objects =
                    ReadableStreamObjects::from_class(pipe_to.source_objects.clone());
                let (promise, _) =
                    ReadableStream::readable_stream_cancel(ctx, source_objects, stored_error)?;
                Ok(promise)
            },
            PipeAction::CloseDest => {
                let dest_objects = WritableStreamObjects::from_class(pipe_to.dest_objects.clone());
                WritableStreamDefaultWriter::writable_stream_default_writer_close_with_error_propagation(
                    ctx,
                    dest_objects,
                )
            },
        }
    }
}

#[derive(Trace)]
enum PipeToErrorKind<'js> {
    Forward {
        pipe_to: PipeTo<'js>,
        prevent_abort: bool,
    },
    Backward {
        pipe_to: PipeTo<'js>,
        prevent_cancel: bool,
    },
}

impl<'js> TracedCallback<'js> for PipeToErrorKind<'js> {
    fn call(self: Box<Self>, ctx: Ctx<'js>, stored_error: Value<'js>) -> Result<Value<'js>> {
        match *self {
            PipeToErrorKind::Forward {
                pipe_to,
                prevent_abort,
            } => {
                if !prevent_abort {
                    pipe_to.shutdown_with_action(
                        ctx.clone(),
                        PipeAction::AbortDest(stored_error.clone()),
                        Some(stored_error),
                    )?;
                } else {
                    pipe_to.shutdown(ctx.clone(), Some(stored_error))?;
                }
            },
            PipeToErrorKind::Backward {
                pipe_to,
                prevent_cancel,
            } => {
                if !prevent_cancel {
                    pipe_to.shutdown_with_action(
                        ctx.clone(),
                        PipeAction::CancelSource(stored_error.clone()),
                        Some(stored_error),
                    )?;
                } else {
                    pipe_to.shutdown(ctx.clone(), Some(stored_error))?;
                }
            },
        }
        Ok(Value::new_undefined(ctx))
    }
}

#[derive(Trace)]
struct PipeClosedHandler<'js> {
    pipe_to: PipeTo<'js>,
    prevent_close: bool,
}

impl<'js> TracedCallback<'js> for PipeClosedHandler<'js> {
    fn call(self: Box<Self>, ctx: Ctx<'js>, _: Value<'js>) -> Result<Value<'js>> {
        if !self.prevent_close {
            self.pipe_to
                .shutdown_with_action(ctx.clone(), PipeAction::CloseDest, None)?;
        } else {
            self.pipe_to.shutdown(ctx.clone(), None)?;
        }
        Ok(Value::new_undefined(ctx))
    }
}

#[derive(Trace)]
struct PipeLoopNextHandler<'js> {
    pipe_to: PipeTo<'js>,
    loop_promise: ResolveablePromise<'js>,
}

impl<'js> TracedReaction<'js> for PipeLoopNextHandler<'js> {
    fn call(
        self: Box<Self>,
        ctx: Ctx<'js>,
        result: std::result::Result<Value<'js>, Value<'js>>,
    ) -> Result<Value<'js>> {
        match result {
            Ok(v) => {
                let done = v.as_bool().unwrap_or(false);
                self.pipe_to.next(ctx.clone(), done, self.loop_promise)?;
            },
            Err(err) => self.loop_promise.reject(err)?,
        }
        Ok(Value::new_undefined(ctx))
    }
}

#[derive(Trace)]
struct PipeStepReadyHandler<'js> {
    pipe_to: PipeTo<'js>,
}

impl<'js> TracedCallback<'js> for PipeStepReadyHandler<'js> {
    fn call(self: Box<Self>, ctx: Ctx<'js>, _: Value<'js>) -> Result<Value<'js>> {
        if self.pipe_to.shutting_down.load(Ordering::Acquire) {
            return Ok(Value::new_bool(ctx, true));
        }

        let read_promise = ResolveablePromise::new(&ctx)?;

        struct ReadRequest<'js> {
            dest_objects: WritableStreamClassObjects<'js, WritableStreamDefaultWriterOwned<'js>>,
            shared: Class<'js, PipeShared<'js>>,
            shutting_down: Rc<AtomicBool>,
            read_promise: ResolveablePromise<'js>,
        }

        impl<'js> Trace<'js> for ReadRequest<'js> {
            fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
                self.dest_objects.trace(tracer);
                self.shared.trace(tracer);
                self.read_promise.trace(tracer);
            }
        }

        impl<'js> ReadableStreamReadRequest<'js> for ReadRequest<'js> {
            fn chunk_steps(
                &self,
                objects: ReadableStreamDefaultReaderObjects<'js>,
                chunk: Value<'js>,
            ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
                let ctx = chunk.ctx().clone();
                if self.shutting_down.load(Ordering::Acquire) {
                    self.read_promise.resolve(Value::new_bool(ctx, true))?;
                    return Ok(objects);
                }

                // calling write can trigger user code; ensure we don't hold locks
                let objects = objects.into_inner();

                let dest_objects = WritableStreamObjects::from_class(self.dest_objects.clone());
                let write_promise =
                    WritableStreamDefaultWriter::writable_stream_default_writer_write(
                        ctx.clone(),
                        dest_objects,
                        chunk,
                    )?;

                let _: Promise<'js> = write_promise.catch()?.call((
                    This(write_promise.clone()),
                    Function::new(ctx.clone(), || {}),
                ))?;

                *self.shared.borrow().current_write.borrow_mut() = write_promise;
                self.read_promise.resolve(Value::new_bool(ctx, false))?;

                Ok(ReadableStreamObjects::from_class(objects))
            }

            fn close_steps(
                &self,
                ctx: &Ctx<'js>,
                objects: ReadableStreamDefaultReaderObjects<'js>,
            ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
                self.read_promise
                    .resolve(Value::new_bool(ctx.clone(), true))?;

                Ok(objects)
            }

            fn error_steps(
                &self,
                objects: ReadableStreamDefaultReaderObjects<'js>,
                reason: Value<'js>,
            ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
                self.read_promise.reject(reason)?;

                Ok(objects)
            }
        }

        let objects = ReadableStreamObjects::from_class(self.pipe_to.source_objects.clone());
        let promise = read_promise.promise.clone();

        ReadableStreamDefaultReader::readable_stream_default_reader_read(
            &ctx,
            objects,
            ReadRequest {
                dest_objects: self.pipe_to.dest_objects,
                shared: self.pipe_to.shared,
                shutting_down: self.pipe_to.shutting_down,
                read_promise,
            },
        )?;

        Ok(promise.into_value())
    }
}

#[derive(Trace)]
struct PipeFinalizeHandler<'js> {
    pipe_to: PipeTo<'js>,
    original_error: Option<Value<'js>>,
}

impl<'js> TracedReaction<'js> for PipeFinalizeHandler<'js> {
    fn call(
        self: Box<Self>,
        ctx: Ctx<'js>,
        result: std::result::Result<Value<'js>, Value<'js>>,
    ) -> Result<Value<'js>> {
        match result {
            Ok(_) => self.pipe_to.finalize(ctx.clone(), self.original_error)?,
            Err(new_error) => self.pipe_to.finalize(ctx.clone(), Some(new_error))?,
        }
        Ok(Value::new_undefined(ctx))
    }
}

#[derive(Trace)]
struct PipeShutdownWaitHandler<'js> {
    pipe_to: PipeTo<'js>,
    action: Option<PipeAction<'js>>,
    original_error: Option<Value<'js>>,
}

impl<'js> TracedReaction<'js> for PipeShutdownWaitHandler<'js> {
    fn call(
        self: Box<Self>,
        ctx: Ctx<'js>,
        _: std::result::Result<Value<'js>, Value<'js>>,
    ) -> Result<Value<'js>> {
        if let Some(action) = self.action {
            let action_promise = action.run(ctx.clone(), &self.pipe_to)?;
            upon_promise_traced(
                &ctx,
                action_promise,
                PipeFinalizeHandler {
                    pipe_to: self.pipe_to,
                    original_error: self.original_error,
                },
            )?;
        } else {
            self.pipe_to.finalize(ctx.clone(), self.original_error)?;
        }
        Ok(Value::new_undefined(ctx))
    }
}

struct PipeWaitWriteLoopHandler<'js> {
    old_current_write: Promise<'js>,
    shared: Class<'js, PipeShared<'js>>,
}

impl<'js> Trace<'js> for PipeWaitWriteLoopHandler<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.old_current_write.trace(tracer);
        self.shared.trace(tracer);
    }
}

impl<'js> TracedReaction<'js> for PipeWaitWriteLoopHandler<'js> {
    fn call(
        self: Box<Self>,
        ctx: Ctx<'js>,
        _: std::result::Result<Value<'js>, Value<'js>>,
    ) -> Result<Value<'js>> {
        if !self
            .old_current_write
            .eq(&self.shared.borrow().current_write.borrow())
        {
            Ok(PipeTo::wait_for_writes_to_finish(ctx, self.shared)?.into_value())
        } else {
            Ok(Value::new_undefined(ctx))
        }
    }
}

impl<'js> PipeTo<'js> {
    fn pipe_loop(self, ctx: Ctx<'js>) -> Result<ResolveablePromise<'js>> {
        let loop_promise = ResolveablePromise::new(&ctx)?;
        self.next(ctx, false, loop_promise.clone())?;
        Ok(loop_promise)
    }

    fn next(&self, ctx: Ctx<'js>, done: bool, loop_promise: ResolveablePromise<'js>) -> Result<()> {
        if done {
            loop_promise.resolve_undefined()?
        } else {
            let pipe_step_promise = self.pipe_step(ctx.clone())?;
            upon_promise_traced(
                &ctx,
                pipe_step_promise,
                PipeLoopNextHandler {
                    pipe_to: self.clone(),
                    loop_promise,
                },
            )?;
        }
        Ok(())
    }

    fn pipe_step(&self, ctx: Ctx<'js>) -> Result<Promise<'js>> {
        if self.shutting_down.load(Ordering::Acquire) {
            return promise_resolved_with(
                &ctx,
                &self.promise_primordials,
                Ok(Value::new_bool(ctx.clone(), true)),
            );
        }

        let writer_ready = self
            .dest_objects
            .writer
            .borrow()
            .ready_promise
            .promise
            .clone();

        upon_promise_fulfilment_traced(
            &ctx,
            writer_ready,
            PipeStepReadyHandler {
                pipe_to: self.clone(),
            },
        )
    }

    fn is_or_becomes_errored(
        ctx: Ctx<'js>,
        stored_error: Option<Value<'js>>,
        promise: Promise<'js>,
        handler: PipeToErrorKind<'js>,
    ) -> Result<()> {
        if let Some(stored_error) = stored_error {
            Box::new(handler).call(ctx, stored_error)?;
            Ok(())
        } else {
            on_promise_rejected_traced(&ctx, promise, handler)
        }
    }

    fn is_or_becomes_closed(
        ctx: Ctx<'js>,
        already_closed: bool,
        promise: Promise<'js>,
        pipe_to: PipeTo<'js>,
        prevent_close: bool,
    ) -> Result<()> {
        let handler = PipeClosedHandler {
            pipe_to,
            prevent_close,
        };
        if already_closed {
            let v = Value::new_undefined(ctx.clone());
            Box::new(handler).call(ctx, v)?;
        } else {
            upon_promise_fulfilment_traced(&ctx, promise, handler)?;
        }
        Ok(())
    }

    fn shutdown_with_action(
        &self,
        ctx: Ctx<'js>,
        action: PipeAction<'js>,
        original_error: Option<Value<'js>>,
    ) -> Result<()> {
        if self.shutting_down.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        let writable = {
            let dest_stream = OwnedBorrow::from_class(self.dest_objects.stream.clone());
            matches!(dest_stream.state, WritableStreamState::Writable)
                && !dest_stream.writable_stream_close_queued_or_in_flight()
        };

        if writable {
            let wait_promise = Self::wait_for_writes_to_finish(ctx.clone(), self.shared.clone())?;
            upon_promise_traced(
                &ctx,
                wait_promise,
                PipeShutdownWaitHandler {
                    pipe_to: self.clone(),
                    action: Some(action),
                    original_error,
                },
            )?;
        } else {
            let action_promise = action.run(ctx.clone(), self)?;
            upon_promise_traced(
                &ctx,
                action_promise,
                PipeFinalizeHandler {
                    pipe_to: self.clone(),
                    original_error,
                },
            )?;
        }
        Ok(())
    }

    fn shutdown(&self, ctx: Ctx<'js>, error: Option<Value<'js>>) -> Result<()> {
        if self.shutting_down.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        let writable = {
            let dest_stream = OwnedBorrow::from_class(self.dest_objects.stream.clone());
            matches!(dest_stream.state, WritableStreamState::Writable)
                && !dest_stream.writable_stream_close_queued_or_in_flight()
        };

        if writable {
            let wait_promise = Self::wait_for_writes_to_finish(ctx.clone(), self.shared.clone())?;
            upon_promise_traced(
                &ctx,
                wait_promise,
                PipeShutdownWaitHandler {
                    pipe_to: self.clone(),
                    action: None,
                    original_error: error,
                },
            )?;
        } else {
            self.finalize(ctx, error)?;
        }
        Ok(())
    }

    fn wait_for_writes_to_finish(
        ctx: Ctx<'js>,
        shared: Class<'js, PipeShared<'js>>,
    ) -> Result<Promise<'js>> {
        let old_current_write: Promise<'js> = shared.borrow().current_write.borrow().clone();

        upon_promise_traced(
            &ctx,
            old_current_write.clone(),
            PipeWaitWriteLoopHandler {
                old_current_write,
                shared,
            },
        )
    }

    fn finalize(&self, ctx: Ctx<'js>, error: Option<Value<'js>>) -> Result<()> {
        let source_objects = ReadableStreamObjects::from_class(self.source_objects.clone());
        let dest_objects = WritableStreamObjects::from_class(self.dest_objects.clone());

        WritableStreamDefaultWriter::writable_stream_default_writer_release(dest_objects)?;
        ReadableStreamDefaultReader::readable_stream_default_reader_release(source_objects)?;

        let abort_callback = self.shared.borrow().abort_callback.borrow().clone();
        if let (Some(signal), Some(abort_callback)) = (&self.signal, abort_callback) {
            AbortSignal::remove_on_abort(This(signal.clone()), ctx.clone(), abort_callback)?;
        }

        if let Some(error) = error {
            self.promise.reject(error)
        } else {
            self.promise.resolve_undefined()
        }
    }
}

#[derive(Default)]
pub struct StreamPipeOptions<'js> {
    pub prevent_close: bool,
    pub prevent_abort: bool,
    pub prevent_cancel: bool,
    pub signal: Option<Class<'js, AbortSignal<'js>>>,
}

impl<'js> FromJs<'js> for StreamPipeOptions<'js> {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        if value.is_undefined() || value.is_null() {
            return Ok(Self::default());
        }
        let Some(obj) = value.as_object() else {
            return Err(Exception::throw_type(ctx, "options must be an object"));
        };

        let get_bool = |key| {
            Result::Ok(
                obj.get_value_or_undefined::<_, Coerced<bool>>(key)?
                    .map(|b| b.0)
                    .unwrap_or(false),
            ) // missing is treated as false
        };

        let prevent_abort = get_bool("preventAbort")?;
        let prevent_cancel = get_bool("preventCancel")?;
        let prevent_close = get_bool("preventClose")?;

        let signal = match obj.get_value_or_undefined::<_, Value<'js>>("signal")? {
            Some(signal) => Some(
                Class::<AbortSignal>::from_js(ctx, signal)
                    .or_throw_type(ctx, "Invalid signal argument")?,
            ),
            None => None,
        };

        Ok(Self {
            prevent_close,
            prevent_abort,
            prevent_cancel,
            signal,
        })
    }
}
