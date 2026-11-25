use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use rquickjs::{
    class::{OwnedBorrowMut, Trace, Tracer},
    function::Constructor,
    prelude::{List, OnceFn},
    ArrayBuffer, Class, Ctx, Error, Function, IntoJs, JsLifetime, Promise, Result, Value,
};

use crate::{
    readable::{
        byob_reader::{ReadableStreamBYOBReader, ReadableStreamReadIntoRequest, ViewBytes},
        byte_controller::{
            ReadableByteStreamController, ReadableByteStreamControllerClass,
            ReadableByteStreamControllerOwned,
        },
        controller::ReadableStreamController,
        default_controller::{
            ReadableStreamDefaultController, ReadableStreamDefaultControllerOwned,
        },
        default_reader::{
            ReadableStreamDefaultReader, ReadableStreamDefaultReaderOwned,
            ReadableStreamReadRequest,
        },
        objects::{ReadableByteStreamObjects, ReadableStreamDefaultControllerObjects},
        objects::{
            ReadableStreamBYOBObjects, ReadableStreamClassObjects,
            ReadableStreamDefaultReaderObjects, ReadableStreamObjects,
        },
        reader::{
            ReadableStreamReader, ReadableStreamReaderClass, ReadableStreamReaderOwned,
            UndefinedReader,
        },
        stream::{
            algorithms::{CancelAlgorithm, PullAlgorithm, StartAlgorithm},
            ReadableStream, ReadableStreamClass,
        },
    },
    utils::promise::{upon_promise_traced, ResolveablePromise, TracedReaction},
};

/// State for tee() operation, stored in a Class for GC tracing
#[rquickjs::class]
pub(crate) struct TeeState<'js> {
    pub(super) stream: ReadableStreamClass<'js>,
    pub(super) controller: Class<'js, ReadableStreamDefaultController<'js>>,
    pub(super) reader: Class<'js, ReadableStreamDefaultReader<'js>>,
    pub(super) cancel_promise: ResolveablePromise<'js>,
    pub(super) reading: Rc<AtomicBool>,
    pub(super) read_again: Rc<AtomicBool>,
    pub(super) reason_1: Rc<OnceCell<Value<'js>>>,
    pub(super) reason_2: Rc<OnceCell<Value<'js>>>,
    pub(super) branch_1: Rc<
        OnceCell<
            ReadableStreamClassObjects<
                'js,
                ReadableStreamDefaultControllerOwned<'js>,
                UndefinedReader,
            >,
        >,
    >,
    pub(super) branch_2: Rc<
        OnceCell<
            ReadableStreamClassObjects<
                'js,
                ReadableStreamDefaultControllerOwned<'js>,
                UndefinedReader,
            >,
        >,
    >,
}

unsafe impl<'js> JsLifetime<'js> for TeeState<'js> {
    type Changed<'to> = TeeState<'to>;
}

impl<'js> Trace<'js> for TeeState<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.stream.trace(tracer);
        self.controller.trace(tracer);
        self.reader.trace(tracer);
        self.cancel_promise.trace(tracer);
        if let Some(r) = self.reason_1.get() {
            r.trace(tracer);
        }
        if let Some(r) = self.reason_2.get() {
            r.trace(tracer);
        }
        if let Some(b) = self.branch_1.get() {
            b.trace(tracer);
        }
        if let Some(b) = self.branch_2.get() {
            b.trace(tracer);
        }
    }
}

type ReadableStreamPair<'js> = (ReadableStreamClass<'js>, ReadableStreamClass<'js>);

impl<'js> ReadableStream<'js> {
    pub(super) fn readable_stream_tee<C: ReadableStreamController<'js>>(
        ctx: Ctx<'js>,
        objects: ReadableStreamObjects<'js, C, UndefinedReader>,
    ) -> Result<ReadableStreamPair<'js>> {
        let (streams, _) = objects.with_controller(
            ctx,
            |ctx, objects| {
                let (streams, objects) = Self::readable_stream_default_tee(ctx, objects)?;
                Ok((streams, objects.clear_reader()))
            },
            |ctx, objects| {
                // If stream.[[controller]] implements ReadableByteStreamController, return ? ReadableByteStreamTee(stream).
                Self::readable_byte_stream_tee(ctx, objects)
            },
        )?;

        Ok(streams)
    }

    fn readable_stream_default_tee(
        ctx: Ctx<'js>,
        mut objects: ReadableStreamDefaultControllerObjects<'js, UndefinedReader>,
    ) -> Result<(
        ReadableStreamPair<'js>,
        ReadableStreamDefaultControllerObjects<'js, ReadableStreamDefaultReaderOwned<'js>>,
    )> {
        // Let reader be ? AcquireReadableStreamDefaultReader(stream).
        let (stream, reader) = ReadableStreamReaderClass::acquire_readable_stream_default_reader(
            ctx.clone(),
            objects.stream,
        )?;
        objects.stream = stream;
        // Let reading be false.
        let reading = Rc::new(AtomicBool::new(false));
        // Let readAgain be false.
        let read_again = Rc::new(AtomicBool::new(false));
        // Let canceled1 be false.
        // Let canceled2 be false.
        // Let reason1 be undefined.
        let reason_1 = Rc::new(OnceCell::new());
        // Let reason2 be undefined.
        let reason_2 = Rc::new(OnceCell::new());
        // Let branch1 be undefined.
        let branch_1: Rc<
            OnceCell<
                ReadableStreamClassObjects<
                    'js,
                    ReadableStreamDefaultControllerOwned<'js>,
                    UndefinedReader,
                >,
            >,
        > = Rc::new(OnceCell::new());
        // Let branch2 be undefined.
        let branch_2: Rc<
            OnceCell<
                ReadableStreamClassObjects<
                    'js,
                    ReadableStreamDefaultControllerOwned<'js>,
                    UndefinedReader,
                >,
            >,
        > = Rc::new(OnceCell::new());
        // Let cancelPromise be a new promise.
        let cancel_promise = ResolveablePromise::new(&ctx)?;

        // Let startAlgorithm be an algorithm that returns undefined.
        let start_algorithm = StartAlgorithm::ReturnUndefined;

        let objects_class: ReadableStreamClassObjects<
            'js,
            ReadableStreamDefaultControllerOwned<'js>,
            ReadableStreamDefaultReaderOwned<'js>,
        > = objects.into_inner().set_reader(reader);

        // Create TeeState to hold all JS values for GC tracing
        let tee_state = Class::instance(
            ctx.clone(),
            TeeState {
                stream: objects_class.stream.clone(),
                controller: objects_class.controller.clone(),
                reader: objects_class.reader.clone(),
                cancel_promise: cancel_promise.clone(),
                reading: reading.clone(),
                read_again: read_again.clone(),
                reason_1: reason_1.clone(),
                reason_2: reason_2.clone(),
                branch_1: branch_1.clone(),
                branch_2: branch_2.clone(),
            },
        )?;

        let pull_algorithm = PullAlgorithm::from_tee_state(tee_state.clone());
        let cancel_algorithm_1 = CancelAlgorithm::from_tee_state_1(tee_state.clone());
        let cancel_algorithm_2 = CancelAlgorithm::from_tee_state_2(tee_state.clone());

        // Set branch1 to ! CreateReadableStream(startAlgorithm, pullAlgorithm, cancel1Algorithm).
        let branch_1_objects = {
            let objects = Self::create_readable_stream(
                ctx.clone(),
                start_algorithm.clone(),
                pull_algorithm.clone(),
                cancel_algorithm_1,
                None,
                None,
            )?;
            _ = branch_1.set(objects.clone());
            objects
        };

        // Set branch2 to ! CreateReadableStream(startAlgorithm, pullAlgorithm, cancel2Algorithm).
        let branch_2_objects = {
            let objects = Self::create_readable_stream(
                ctx.clone(),
                start_algorithm,
                pull_algorithm,
                cancel_algorithm_2,
                None,
                None,
            )?;
            _ = branch_2.set(objects.clone());
            objects
        };

        #[derive(Trace)]
        struct TeeClosedHandler<'js> {
            tee_state: Class<'js, TeeState<'js>>,
            branch_1_objects: ReadableStreamClassObjects<
                'js,
                ReadableStreamDefaultControllerOwned<'js>,
                UndefinedReader,
            >,
            branch_2_objects: ReadableStreamClassObjects<
                'js,
                ReadableStreamDefaultControllerOwned<'js>,
                UndefinedReader,
            >,
        }

        impl<'js> TracedReaction<'js> for TeeClosedHandler<'js> {
            fn call(
                self: Box<Self>,
                ctx: Ctx<'js>,
                result: std::result::Result<Value<'js>, Value<'js>>,
            ) -> Result<Value<'js>> {
                if let Err(reason) = result {
                    let objects_1 =
                        ReadableStreamObjects::from_class_no_reader(self.branch_1_objects)
                            .refresh_reader();
                    ReadableStreamDefaultController::readable_stream_default_controller_error(
                        objects_1,
                        reason.clone(),
                    )?;

                    let objects_2 =
                        ReadableStreamObjects::from_class_no_reader(self.branch_2_objects)
                            .refresh_reader();
                    ReadableStreamDefaultController::readable_stream_default_controller_error(
                        objects_2, reason,
                    )?;

                    let state = self.tee_state.borrow();
                    if state.reason_1.get().is_none() || state.reason_2.get().is_none() {
                        state.cancel_promise.resolve_undefined()?;
                    }
                }
                Ok(Value::new_undefined(ctx))
            }
        }

        upon_promise_traced(
            &ctx,
            objects_class
                .reader
                .borrow()
                .generic
                .closed_promise
                .promise
                .clone(),
            TeeClosedHandler {
                tee_state,
                branch_1_objects: branch_1_objects.clone(),
                branch_2_objects: branch_2_objects.clone(),
            },
        )?;

        Ok((
            (branch_1_objects.stream, branch_2_objects.stream),
            ReadableStreamObjects::from_class(objects_class),
        ))
    }
}

/// Pull algorithm for tee - called from PullAlgorithm::Tee
pub fn tee_pull_algorithm<'js>(
    ctx: Ctx<'js>,
    state: Class<'js, TeeState<'js>>,
) -> Result<Promise<'js>> {
    let state_ref = state.borrow();

    // If reading is true, set readAgain to true and return resolved promise
    if state_ref.reading.load(Ordering::Acquire) {
        state_ref.read_again.store(true, Ordering::Release);
        return Ok(state_ref
            .stream
            .borrow()
            .promise_primordials
            .promise_resolved_with_undefined
            .clone());
    }

    // Set reading to true
    state_ref.reading.store(true, Ordering::Release);

    let objects_class: ReadableStreamClassObjects<
        'js,
        ReadableStreamDefaultControllerOwned<'js>,
        ReadableStreamDefaultReaderOwned<'js>,
    > = ReadableStreamClassObjects {
        stream: state_ref.stream.clone(),
        controller: state_ref.controller.clone(),
        reader: state_ref.reader.clone(),
    };
    drop(state_ref);

    let mut objects = ReadableStreamObjects::from_class(objects_class.clone());

    // ReadRequest that just holds TeeState
    #[derive(Clone)]
    struct TeeReadRequest<'js>(Class<'js, TeeState<'js>>);

    impl<'js> Trace<'js> for TeeReadRequest<'js> {
        fn trace<'a>(&self, tracer: rquickjs::class::Tracer<'a, 'js>) {
            self.0.trace(tracer);
        }
    }

    impl<'js> ReadableStreamReadRequest<'js> for TeeReadRequest<'js> {
        fn chunk_steps(
            &self,
            objects: ReadableStreamDefaultReaderObjects<'js>,
            chunk: Value<'js>,
        ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
            let ctx = chunk.ctx().clone();
            let state = self.0.clone();

            objects.with_assert_default_controller(|objects| {
                let objects_class = objects.into_inner();
                let f = {
                    let ctx = ctx.clone();
                    let _objects_class = objects_class.clone();
                    move || -> Result<()> {
                        let s = state.borrow();
                        s.read_again.store(false, Ordering::Release);

                        let chunk_1 = chunk.clone();
                        let chunk_2 = chunk;

                        if s.reason_1.get().is_none() {
                            let objects_1 = ReadableStreamObjects::from_class(
                                s.branch_1.get().cloned().expect("branch1 not set"),
                            )
                            .refresh_reader();
                            ReadableStreamDefaultController::readable_stream_default_controller_enqueue(
                                ctx.clone(),
                                objects_1,
                                chunk_1,
                            )?;
                        }

                        if s.reason_2.get().is_none() {
                            let objects_2 = ReadableStreamObjects::from_class(
                                s.branch_2.get().cloned().expect("branch2 not set"),
                            )
                            .refresh_reader();
                            ReadableStreamDefaultController::readable_stream_default_controller_enqueue(
                                ctx.clone(),
                                objects_2,
                                chunk_2,
                            )?;
                        }

                        s.reading.store(false, Ordering::Release);

                        if s.read_again.load(Ordering::Acquire) {
                            drop(s);
                            tee_pull_algorithm(ctx, state)?;
                        }

                        Ok(())
                    }
                };

                let () = Function::new(ctx, OnceFn::new(f))?.defer(())?;
                Ok(ReadableStreamObjects::from_class(objects_class))
            })
        }

        fn close_steps(
            &self,
            ctx: &Ctx<'js>,
            objects: ReadableStreamDefaultReaderObjects<'js>,
        ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
            let s = self.0.borrow();
            s.reading.store(false, Ordering::Release);

            if s.reason_1.get().is_none() {
                let objects_1 = ReadableStreamObjects::from_class(
                    s.branch_1.get().cloned().expect("branch1 not set"),
                )
                .refresh_reader();
                ReadableStreamDefaultController::readable_stream_default_controller_close(
                    ctx.clone(),
                    objects_1,
                )?;
            }

            if s.reason_2.get().is_none() {
                let objects_2 = ReadableStreamObjects::from_class(
                    s.branch_2.get().cloned().expect("branch2 not set"),
                )
                .refresh_reader();
                ReadableStreamDefaultController::readable_stream_default_controller_close(
                    ctx.clone(),
                    objects_2,
                )?;
            }

            if s.reason_1.get().is_none() || s.reason_2.get().is_none() {
                s.cancel_promise.resolve_undefined()?;
            }

            Ok(objects)
        }

        fn error_steps(
            &self,
            objects: ReadableStreamDefaultReaderObjects<'js>,
            _e: Value<'js>,
        ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
            self.0.borrow().reading.store(false, Ordering::Release);
            Ok(objects)
        }
    }

    objects = ReadableStreamDefaultReader::readable_stream_default_reader_read(
        &ctx,
        objects,
        TeeReadRequest(state),
    )?;

    Ok(objects
        .stream
        .promise_primordials
        .promise_resolved_with_undefined
        .clone())
}

/// Cancel algorithm for tee - called from CancelAlgorithm::Tee1/Tee2
pub fn tee_cancel_algorithm<'js>(
    ctx: Ctx<'js>,
    state: Class<'js, TeeState<'js>>,
    reason: Value<'js>,
    branch: usize,
) -> Result<Promise<'js>> {
    let state_ref = state.borrow();
    let objects_class: ReadableStreamClassObjects<
        'js,
        ReadableStreamDefaultControllerOwned<'js>,
        ReadableStreamDefaultReaderOwned<'js>,
    > = ReadableStreamClassObjects {
        stream: state_ref.stream.clone(),
        controller: state_ref.controller.clone(),
        reader: state_ref.reader.clone(),
    };
    let objects = ReadableStreamObjects::from_class(objects_class);
    ReadableStream::tee_cancel_algorithm_impl(
        ctx,
        objects,
        [&state_ref.reason_1, &state_ref.reason_2],
        state_ref.cancel_promise.clone(),
        reason,
        branch,
    )
}

impl<'js> ReadableStream<'js> {
    // Cancel algorithm for tee - handles both branches
    fn tee_cancel_algorithm_impl(
        ctx: Ctx<'js>,
        objects: ReadableStreamObjects<
            'js,
            impl ReadableStreamController<'js>,
            impl ReadableStreamReader<'js>,
        >,
        reasons: [&Rc<OnceCell<Value<'js>>>; 2],
        cancel_promise: ResolveablePromise<'js>,
        reason: Value<'js>,
        branch: usize,
    ) -> Result<Promise<'js>> {
        let other = 1 - branch;

        // Set canceled[branch] to true, set reason[branch] to reason
        reasons[branch]
            .set(reason.clone())
            .expect("tee stream already has a cancel reason");

        // If other branch is also canceled
        if let Some(other_reason) = reasons[other].get().cloned() {
            // CreateArrayFromList with reasons in correct order
            let composite_reason = if branch == 0 {
                List((reason, other_reason))
            } else {
                List((other_reason, reason))
            };
            let (cancel_result, _) = ReadableStream::readable_stream_cancel(
                ctx.clone(),
                objects,
                composite_reason.into_js(&ctx)?,
            )?;
            cancel_promise.resolve(cancel_result)?;
        }

        Ok(cancel_promise.promise)
    }

    fn readable_byte_stream_tee(
        ctx: Ctx<'js>,
        mut objects: ReadableByteStreamObjects<'js, UndefinedReader>,
    ) -> Result<(
        ReadableStreamPair<'js>,
        ReadableByteStreamObjects<'js, UndefinedReader>,
    )> {
        // Let reader be ? AcquireReadableStreamDefaultReader(stream).
        let (stream, reader) = ReadableStreamReaderClass::acquire_readable_stream_default_reader(
            ctx.clone(),
            objects.stream,
        )?;
        objects.stream = stream;
        let reader: Rc<RefCell<ReadableStreamReaderClass<'js>>> =
            Rc::new(RefCell::new(reader.into()));
        let reading = Rc::new(AtomicBool::new(false));
        let read_again_for_branch_1 = Rc::new(AtomicBool::new(false));
        let read_again_for_branch_2 = Rc::new(AtomicBool::new(false));
        let reason_1 = Rc::new(OnceCell::new());
        let reason_2 = Rc::new(OnceCell::new());
        let branch_1: Rc<
            OnceCell<
                ReadableStreamClassObjects<
                    'js,
                    ReadableByteStreamControllerOwned<'js>,
                    UndefinedReader,
                >,
            >,
        > = Rc::new(OnceCell::new());
        let branch_2: Rc<
            OnceCell<
                ReadableStreamClassObjects<
                    'js,
                    ReadableByteStreamControllerOwned<'js>,
                    UndefinedReader,
                >,
            >,
        > = Rc::new(OnceCell::new());
        let cancel_promise = ResolveablePromise::new(&ctx)?;

        let objects_class = objects.into_inner();

        let state = Class::instance(
            ctx.clone(),
            ByteTeeState {
                stream: objects_class.stream.clone(),
                controller: objects_class.controller.clone(),
                reader: reader.clone(),
                reading,
                read_again_for_branch_1,
                read_again_for_branch_2,
                reason_1,
                reason_2,
                branch_1: branch_1.clone(),
                branch_2: branch_2.clone(),
                cancel_promise,
            },
        )?;

        let pull_1_algorithm = PullAlgorithm::ByteTee1(state.clone());
        let pull_2_algorithm = PullAlgorithm::ByteTee2(state.clone());
        let cancel_algorithm_1 = CancelAlgorithm::ByteTee1(state.clone());
        let cancel_algorithm_2 = CancelAlgorithm::ByteTee2(state.clone());

        let start_algorithm = StartAlgorithm::ReturnUndefined;

        // Set branch1 to ! CreateReadableByteStream(startAlgorithm, pull1Algorithm, cancel1Algorithm).
        let objects_1 = {
            let (s, c) = Self::create_readable_byte_stream(
                ctx.clone(),
                start_algorithm.clone(),
                pull_1_algorithm,
                cancel_algorithm_1,
            )?;
            let objs = ReadableStreamClassObjects {
                stream: s,
                controller: c,
                reader: UndefinedReader,
            };
            let _ = branch_1.set(objs.clone());
            objs
        };

        // Set branch2 to ! CreateReadableByteStream(startAlgorithm, pull2Algorithm, cancel2Algorithm).
        let objects_2 = {
            let (s, c) = Self::create_readable_byte_stream(
                ctx.clone(),
                start_algorithm,
                pull_2_algorithm,
                cancel_algorithm_2,
            )?;
            let objs = ReadableStreamClassObjects {
                stream: s,
                controller: c,
                reader: UndefinedReader,
            };
            let _ = branch_2.set(objs.clone());
            objs
        };

        let this_reader = reader.borrow().clone();
        Self::readable_byte_stream_forward_reader_error(&ctx, state, this_reader)?;

        Ok((
            (objects_1.stream, objects_2.stream),
            ReadableStreamObjects::from_class(objects_class),
        ))
    }

    fn readable_byte_stream_forward_reader_error(
        ctx: &Ctx<'js>,
        state: Class<'js, ByteTeeState<'js>>,
        this_reader: ReadableStreamReaderClass<'js>,
    ) -> Result<()> {
        #[derive(Trace)]
        struct ByteTeeErrorForwarder<'js> {
            state: Class<'js, ByteTeeState<'js>>,
            this_reader: ReadableStreamReaderClass<'js>,
        }

        impl<'js> TracedReaction<'js> for ByteTeeErrorForwarder<'js> {
            fn call(
                self: Box<Self>,
                ctx: Ctx<'js>,
                result: std::result::Result<Value<'js>, Value<'js>>,
            ) -> Result<Value<'js>> {
                if let Err(r) = result {
                    let s = self.state.borrow();
                    if !s.reader.borrow().eq(&self.this_reader) {
                        return Ok(Value::new_undefined(ctx));
                    }
                    let b1 = s.branch_1.get().cloned().expect("branch1 not set");
                    let b2 = s.branch_2.get().cloned().expect("branch2 not set");
                    let r1_none = s.reason_1.get().is_none();
                    let r2_none = s.reason_2.get().is_none();
                    let cancel_promise = s.cancel_promise.clone();
                    drop(s);

                    let objects_1 =
                        ReadableStreamObjects::from_class_no_reader(b1).refresh_reader();
                    ReadableByteStreamController::readable_byte_stream_controller_error(
                        objects_1,
                        r.clone(),
                    )?;

                    let objects_2 =
                        ReadableStreamObjects::from_class_no_reader(b2).refresh_reader();
                    ReadableByteStreamController::readable_byte_stream_controller_error(
                        objects_2, r,
                    )?;

                    if r1_none || r2_none {
                        cancel_promise.resolve_undefined()?;
                    }
                }
                Ok(Value::new_undefined(ctx))
            }
        }

        upon_promise_traced(
            ctx,
            this_reader.closed_promise(),
            ByteTeeErrorForwarder { state, this_reader },
        )?;
        Ok(())
    }

    fn readable_byte_stream_pull_with_default_reader(
        ctx: Ctx<'js>,
        state: Class<'js, ByteTeeState<'js>>,
    ) -> Result<ReadableByteStreamObjects<'js, UndefinedReader>> {
        let s = state.borrow();
        let source_stream = s.stream.clone();
        let source_controller = s.controller.clone();
        let reader_rc = s.reader.clone();
        let current_reader = reader_rc.borrow().clone();
        drop(s);

        let mut objects = ReadableStreamObjects::from_class(ReadableStreamClassObjects {
            stream: source_stream,
            controller: source_controller,
            reader: UndefinedReader,
        });

        let current_reader = match current_reader {
            ReadableStreamReaderClass::ReadableStreamBYOBReader(r) => {
                let byob_reader = OwnedBorrowMut::from_class(r);
                objects = ReadableStreamBYOBReader::readable_stream_byob_reader_release(
                    objects.set_reader(byob_reader),
                )?
                .clear_reader();

                let (s, new_reader) =
                    ReadableStreamReaderClass::acquire_readable_stream_default_reader(
                        ctx.clone(),
                        objects.stream,
                    )?;
                objects.stream = s;
                reader_rc.replace(new_reader.clone().into());
                Self::readable_byte_stream_forward_reader_error(
                    &ctx,
                    state.clone(),
                    new_reader.clone().into(),
                )?;
                new_reader
            },
            ReadableStreamReaderClass::ReadableStreamDefaultReader(r) => r,
        };

        #[derive(Clone, Trace)]
        struct ReadRequest<'js> {
            state: Class<'js, ByteTeeState<'js>>,
        }

        impl<'js> ReadableStreamReadRequest<'js> for ReadRequest<'js> {
            fn chunk_steps(
                &self,
                objects: ReadableStreamDefaultReaderObjects<'js>,
                chunk: Value<'js>,
            ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
                let ctx = chunk.ctx().clone();
                let state = self.state.clone();

                objects.with_assert_byte_controller(|objects| {
                    let constructor_uint8array = objects
                        .controller
                        .array_constructor_primordials
                        .constructor_uint8array
                        .clone();
                    let function_array_buffer_is_view =
                        objects.controller.function_array_buffer_is_view.clone();
                    let chunk =
                        ViewBytes::from_value(&ctx, &function_array_buffer_is_view, Some(&chunk))?;
                    let objects_class = objects.into_inner();
                    let f = {
                        let ctx = ctx.clone();
                        let objects_class = objects_class.clone();
                        move || -> Result<()> {
                            let s = state.borrow();
                            s.read_again_for_branch_1.store(false, Ordering::Release);
                            s.read_again_for_branch_2.store(false, Ordering::Release);
                            let b1 = s.branch_1.get().cloned().expect("branch1 not set");
                            let b2 = s.branch_2.get().cloned().expect("branch2 not set");
                            let r1_none = s.reason_1.get().is_none();
                            let r2_none = s.reason_2.get().is_none();
                            let reading = s.reading.clone();
                            let ra1 = s.read_again_for_branch_1.clone();
                            let ra2 = s.read_again_for_branch_2.clone();
                            let cancel_promise = s.cancel_promise.clone();
                            drop(s);

                            let chunk_1 = chunk.clone();
                            let mut chunk_2 = chunk.clone();

                            if r1_none && r2_none {
                                match clone_as_uint8_array(
                                    ctx.clone(),
                                    &constructor_uint8array,
                                    &function_array_buffer_is_view,
                                    chunk,
                                ) {
                                    Err(Error::Exception) => {
                                        let err = ctx.catch();
                                        let objects_1 = ReadableStreamObjects::from_class(b1);
                                        ReadableByteStreamController::readable_byte_stream_controller_error(
                                            objects_1,
                                            err.clone(),
                                        )?;
                                        let objects_2 = ReadableStreamObjects::from_class(b2);
                                        ReadableByteStreamController::readable_byte_stream_controller_error(
                                            objects_2,
                                            err.clone(),
                                        )?;
                                        let (promise, _) = ReadableStream::readable_stream_cancel(
                                            ctx,
                                            ReadableStreamObjects::from_class(objects_class),
                                            err,
                                        )?;
                                        cancel_promise.resolve(promise)?;
                                        return Ok(());
                                    },
                                    Ok(clone_result) => chunk_2 = clone_result,
                                    Err(err) => return Err(err),
                                }
                            }

                            if r1_none {
                                let objects_1 =
                                    ReadableStreamObjects::from_class_no_reader(b1).refresh_reader();
                                ReadableByteStreamController::readable_byte_stream_controller_enqueue(
                                    &ctx, objects_1, chunk_1,
                                )?;
                            }

                            if r2_none {
                                let objects_2 =
                                    ReadableStreamObjects::from_class_no_reader(b2).refresh_reader();
                                ReadableByteStreamController::readable_byte_stream_controller_enqueue(
                                    &ctx, objects_2, chunk_2,
                                )?;
                            }

                            reading.store(false, Ordering::Release);

                            if ra1.load(Ordering::Acquire) {
                                byte_tee_pull_algorithm(ctx, state, false)?;
                            } else if ra2.load(Ordering::Acquire) {
                                byte_tee_pull_algorithm(ctx, state, true)?;
                            }

                            Ok(())
                        }
                    };

                    let () = Function::new(ctx, OnceFn::new(f))?.defer(())?;
                    Ok(ReadableStreamObjects::from_class(objects_class))
                })
            }

            fn close_steps(
                &self,
                ctx: &Ctx<'js>,
                objects: ReadableStreamDefaultReaderObjects<'js>,
            ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
                let s = self.state.borrow();
                s.reading.store(false, Ordering::Release);
                let b1 = s.branch_1.get().cloned().expect("branch1 not set");
                let b2 = s.branch_2.get().cloned().expect("branch2 not set");
                let r1_none = s.reason_1.get().is_none();
                let r2_none = s.reason_2.get().is_none();
                let cancel_promise = s.cancel_promise.clone();
                drop(s);

                let mut objects_1 =
                    ReadableStreamObjects::from_class_no_reader(b1).refresh_reader();
                let mut objects_2 =
                    ReadableStreamObjects::from_class_no_reader(b2).refresh_reader();

                if r1_none {
                    objects_1 =
                        ReadableByteStreamController::readable_byte_stream_controller_close(
                            ctx.clone(),
                            objects_1,
                        )?;
                }
                if r2_none {
                    objects_2 =
                        ReadableByteStreamController::readable_byte_stream_controller_close(
                            ctx.clone(),
                            objects_2,
                        )?;
                }
                if !objects_1.controller.pending_pull_intos.is_empty() {
                    ReadableByteStreamController::readable_byte_stream_controller_respond(
                        ctx.clone(),
                        objects_1,
                        0,
                    )?;
                }
                if !objects_2.controller.pending_pull_intos.is_empty() {
                    ReadableByteStreamController::readable_byte_stream_controller_respond(
                        ctx.clone(),
                        objects_2,
                        0,
                    )?;
                }
                if r1_none || r2_none {
                    cancel_promise.resolve_undefined()?;
                }
                Ok(objects)
            }

            fn error_steps(
                &self,
                objects: ReadableStreamDefaultReaderObjects<'js>,
                _: Value<'js>,
            ) -> Result<ReadableStreamDefaultReaderObjects<'js>> {
                self.state.borrow().reading.store(false, Ordering::Release);
                Ok(objects)
            }
        }

        Ok(
            ReadableStreamDefaultReader::readable_stream_default_reader_read(
                &ctx,
                objects.set_reader(OwnedBorrowMut::from_class(current_reader)),
                ReadRequest { state },
            )?
            .clear_reader(),
        )
    }

    fn readable_byte_stream_pull_with_byob_reader(
        ctx: Ctx<'js>,
        state: Class<'js, ByteTeeState<'js>>,
        view: ViewBytes<'js>,
        for_branch_2: bool,
    ) -> Result<ReadableByteStreamObjects<'js, UndefinedReader>> {
        let s = state.borrow();
        let source_stream = s.stream.clone();
        let source_controller = s.controller.clone();
        let reader_rc = s.reader.clone();
        let current_reader = reader_rc.borrow().clone();
        drop(s);

        let mut objects = ReadableStreamObjects::from_class(ReadableStreamClassObjects {
            stream: source_stream,
            controller: source_controller,
            reader: UndefinedReader,
        });

        let current_reader = match current_reader {
            ReadableStreamReaderClass::ReadableStreamDefaultReader(r) => {
                let default_reader = OwnedBorrowMut::from_class(r);
                objects = ReadableStreamDefaultReader::readable_stream_default_reader_release(
                    objects.set_reader(default_reader),
                )?
                .clear_reader();

                let (s, new_reader) =
                    ReadableStreamReaderClass::acquire_readable_stream_byob_reader(
                        ctx.clone(),
                        objects.stream,
                    )?;
                objects.stream = s;
                reader_rc.replace(new_reader.clone().into());
                Self::readable_byte_stream_forward_reader_error(
                    &ctx,
                    state.clone(),
                    new_reader.clone().into(),
                )?;
                new_reader
            },
            ReadableStreamReaderClass::ReadableStreamBYOBReader(r) => r,
        };

        #[derive(Clone, Trace)]
        struct ReadIntoRequest<'js> {
            state: Class<'js, ByteTeeState<'js>>,
            for_branch_2: bool,
        }

        impl<'js> ReadableStreamReadIntoRequest<'js> for ReadIntoRequest<'js> {
            fn chunk_steps(
                &self,
                objects: ReadableStreamBYOBObjects<'js>,
                chunk: Value<'js>,
            ) -> Result<ReadableStreamBYOBObjects<'js>> {
                let ctx = chunk.ctx().clone();
                let constructor_uint8array = objects
                    .controller
                    .array_constructor_primordials
                    .constructor_uint8array
                    .clone();
                let function_array_buffer_is_view =
                    objects.controller.function_array_buffer_is_view.clone();
                let chunk =
                    ViewBytes::from_value(&ctx, &function_array_buffer_is_view, Some(&chunk))?;

                let objects_class = objects.into_inner();
                let state = self.state.clone();
                let for_branch_2 = self.for_branch_2;

                let f = {
                    let ctx = ctx.clone();
                    let objects_class = objects_class.clone();
                    move || -> Result<()> {
                        let s = state.borrow();
                        s.read_again_for_branch_1.store(false, Ordering::Release);
                        s.read_again_for_branch_2.store(false, Ordering::Release);
                        let b1 = s.branch_1.get().cloned().expect("branch1 not set");
                        let b2 = s.branch_2.get().cloned().expect("branch2 not set");
                        let r1_none = s.reason_1.get().is_none();
                        let r2_none = s.reason_2.get().is_none();
                        let reading = s.reading.clone();
                        let ra1 = s.read_again_for_branch_1.clone();
                        let ra2 = s.read_again_for_branch_2.clone();
                        let cancel_promise = s.cancel_promise.clone();
                        drop(s);

                        let (byob_canceled, other_canceled, byob_class, other_class) =
                            if for_branch_2 {
                                (!r2_none, !r1_none, b2, b1)
                            } else {
                                (!r1_none, !r2_none, b1, b2)
                            };

                        if !other_canceled {
                            match clone_as_uint8_array(
                                ctx.clone(),
                                &constructor_uint8array,
                                &function_array_buffer_is_view,
                                chunk.clone(),
                            ) {
                                Err(Error::Exception) => {
                                    let err = ctx.catch();
                                    let byob_objects =
                                        ReadableStreamObjects::from_class_no_reader(byob_class)
                                            .refresh_reader();
                                    ReadableByteStreamController::readable_byte_stream_controller_error(
                                        byob_objects,
                                        err.clone(),
                                    )?;
                                    let other_objects =
                                        ReadableStreamObjects::from_class_no_reader(other_class)
                                            .refresh_reader();
                                    ReadableByteStreamController::readable_byte_stream_controller_error(
                                        other_objects,
                                        err.clone(),
                                    )?;
                                    let (promise, _) = ReadableStream::readable_stream_cancel(
                                        ctx,
                                        ReadableStreamObjects::from_class(objects_class),
                                        err,
                                    )?;
                                    cancel_promise.resolve(promise)?;
                                    return Ok(());
                                },
                                Ok(cloned_chunk) => {
                                    if !byob_canceled {
                                        let byob_objects =
                                            ReadableStreamObjects::from_class_no_reader(byob_class)
                                                .refresh_reader();
                                        ReadableByteStreamController::readable_byte_stream_controller_respond_with_new_view(
                                            ctx.clone(),
                                            byob_objects,
                                            chunk,
                                        )?;
                                    }
                                    let other_objects =
                                        ReadableStreamObjects::from_class_no_reader(other_class)
                                            .refresh_reader();
                                    ReadableByteStreamController::readable_byte_stream_controller_enqueue(
                                        &ctx,
                                        other_objects,
                                        cloned_chunk,
                                    )?;
                                },
                                Err(err) => return Err(err),
                            }
                        } else if !byob_canceled {
                            let byob_objects =
                                ReadableStreamObjects::from_class_no_reader(byob_class)
                                    .refresh_reader();
                            ReadableByteStreamController::readable_byte_stream_controller_respond_with_new_view(
                                ctx.clone(),
                                byob_objects,
                                chunk,
                            )?;
                        }

                        reading.store(false, Ordering::Release);

                        if ra1.load(Ordering::Acquire) {
                            byte_tee_pull_algorithm(ctx, state, false)?;
                        } else if ra2.load(Ordering::Acquire) {
                            byte_tee_pull_algorithm(ctx, state, true)?;
                        }

                        Ok(())
                    }
                };

                let () = Function::new(ctx, OnceFn::new(f))?.defer(())?;
                Ok(ReadableStreamObjects::from_class(objects_class))
            }

            fn close_steps(
                &self,
                objects: ReadableStreamBYOBObjects<'js>,
                chunk: Value<'js>,
            ) -> Result<ReadableStreamBYOBObjects<'js>> {
                let ctx = chunk.ctx().clone();
                let s = self.state.borrow();
                s.reading.store(false, Ordering::Release);
                let b1 = s.branch_1.get().cloned().expect("branch1 not set");
                let b2 = s.branch_2.get().cloned().expect("branch2 not set");
                let r1_none = s.reason_1.get().is_none();
                let r2_none = s.reason_2.get().is_none();
                let cancel_promise = s.cancel_promise.clone();
                drop(s);

                let (byob_canceled, other_canceled, byob_class, other_class) = if self.for_branch_2
                {
                    (!r2_none, !r1_none, b2, b1)
                } else {
                    (!r1_none, !r2_none, b1, b2)
                };

                if !byob_canceled {
                    let byob_objects =
                        ReadableStreamObjects::from_class_no_reader(byob_class.clone())
                            .refresh_reader();
                    ReadableByteStreamController::readable_byte_stream_controller_close(
                        ctx.clone(),
                        byob_objects,
                    )?;
                }
                if !other_canceled {
                    let other_objects =
                        ReadableStreamObjects::from_class_no_reader(other_class.clone())
                            .refresh_reader();
                    ReadableByteStreamController::readable_byte_stream_controller_close(
                        ctx.clone(),
                        other_objects,
                    )?;
                }

                if !chunk.is_undefined() {
                    let chunk = ViewBytes::from_value(
                        &ctx,
                        &objects.controller.function_array_buffer_is_view,
                        Some(&chunk),
                    )?;

                    if !byob_canceled {
                        let byob_objects = ReadableStreamObjects::from_class_no_reader(byob_class)
                            .refresh_reader();
                        ReadableByteStreamController::readable_byte_stream_controller_respond_with_new_view(
                            ctx.clone(),
                            byob_objects,
                            chunk,
                        )?;
                    }

                    let other_objects =
                        ReadableStreamObjects::from_class_no_reader(other_class).refresh_reader();
                    if !other_canceled && !other_objects.controller.pending_pull_intos.is_empty() {
                        ReadableByteStreamController::readable_byte_stream_controller_respond(
                            ctx.clone(),
                            other_objects,
                            0,
                        )?;
                    }
                }

                if !byob_canceled || !other_canceled {
                    cancel_promise.resolve_undefined()?;
                }

                Ok(objects)
            }

            fn error_steps(
                &self,
                objects: ReadableStreamBYOBObjects<'js>,
                _: Value<'js>,
            ) -> Result<ReadableStreamBYOBObjects<'js>> {
                self.state.borrow().reading.store(false, Ordering::Release);
                Ok(objects)
            }
        }

        Ok(ReadableStreamBYOBReader::readable_stream_byob_reader_read(
            &ctx,
            objects.set_reader(OwnedBorrowMut::from_class(current_reader)),
            view,
            1,
            ReadIntoRequest {
                state,
                for_branch_2,
            },
        )?
        .clear_reader())
    }
}

/// State for byte-stream `tee()` operation, stored in a Class for GC tracing.
#[rquickjs::class]
pub(crate) struct ByteTeeState<'js> {
    pub(super) stream: ReadableStreamClass<'js>,
    pub(super) controller: ReadableByteStreamControllerClass<'js>,
    pub(super) reader: Rc<RefCell<ReadableStreamReaderClass<'js>>>,
    pub(super) reading: Rc<AtomicBool>,
    pub(super) read_again_for_branch_1: Rc<AtomicBool>,
    pub(super) read_again_for_branch_2: Rc<AtomicBool>,
    pub(super) reason_1: Rc<OnceCell<Value<'js>>>,
    pub(super) reason_2: Rc<OnceCell<Value<'js>>>,
    pub(super) branch_1: Rc<
        OnceCell<
            ReadableStreamClassObjects<
                'js,
                ReadableByteStreamControllerOwned<'js>,
                UndefinedReader,
            >,
        >,
    >,
    pub(super) branch_2: Rc<
        OnceCell<
            ReadableStreamClassObjects<
                'js,
                ReadableByteStreamControllerOwned<'js>,
                UndefinedReader,
            >,
        >,
    >,
    pub(super) cancel_promise: ResolveablePromise<'js>,
}

unsafe impl<'js> JsLifetime<'js> for ByteTeeState<'js> {
    type Changed<'to> = ByteTeeState<'to>;
}

impl<'js> Trace<'js> for ByteTeeState<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.stream.trace(tracer);
        self.controller.trace(tracer);
        if let Ok(r) = self.reader.try_borrow() {
            r.trace(tracer);
        }
        if let Some(r) = self.reason_1.get() {
            r.trace(tracer);
        }
        if let Some(r) = self.reason_2.get() {
            r.trace(tracer);
        }
        if let Some(b) = self.branch_1.get() {
            b.trace(tracer);
        }
        if let Some(b) = self.branch_2.get() {
            b.trace(tracer);
        }
        self.cancel_promise.trace(tracer);
    }
}

pub(super) fn byte_tee_pull_algorithm<'js>(
    ctx: Ctx<'js>,
    state: Class<'js, ByteTeeState<'js>>,
    for_branch_2: bool,
) -> Result<Promise<'js>> {
    let s = state.borrow();
    let promise_resolved = s
        .stream
        .borrow()
        .promise_primordials
        .promise_resolved_with_undefined
        .clone();
    if s.reading.swap(true, Ordering::AcqRel) {
        if for_branch_2 {
            s.read_again_for_branch_2.store(true, Ordering::Release);
        } else {
            s.read_again_for_branch_1.store(true, Ordering::Release);
        }
        return Ok(promise_resolved);
    }

    let branch = if for_branch_2 {
        s.branch_2.get().cloned().expect("branch2 not set")
    } else {
        s.branch_1.get().cloned().expect("branch1 not set")
    };
    drop(s);

    let (byob_request, _) =
        ReadableByteStreamController::readable_byte_stream_controller_get_byob_request(
            ctx.clone(),
            OwnedBorrowMut::from_class(branch.controller),
        )?;

    match byob_request.0 {
        None => {
            ReadableStream::readable_byte_stream_pull_with_default_reader(ctx, state)?;
        },
        Some(byob_request) => {
            let view = byob_request.borrow().view.clone().expect(
                "ReadableByteStream tee pull algorithm called with invalidated byobRequest",
            );
            ReadableStream::readable_byte_stream_pull_with_byob_reader(
                ctx,
                state,
                view,
                for_branch_2,
            )?;
        },
    }

    Ok(promise_resolved)
}

pub(super) fn byte_tee_cancel_algorithm<'js>(
    ctx: Ctx<'js>,
    state: Class<'js, ByteTeeState<'js>>,
    reason: Value<'js>,
    branch: usize,
) -> Result<Promise<'js>> {
    let s = state.borrow();
    let current_reader = s.reader.borrow().clone();
    let stream = s.stream.clone();
    let controller = s.controller.clone();
    let reason_1 = s.reason_1.clone();
    let reason_2 = s.reason_2.clone();
    let cancel_promise = s.cancel_promise.clone();
    drop(s);

    let objects = ReadableStreamObjects {
        stream: OwnedBorrowMut::from_class(stream),
        controller: OwnedBorrowMut::from_class(controller),
        reader: ReadableStreamReaderOwned::from_class(current_reader),
    };
    ReadableStream::tee_cancel_algorithm_impl(
        ctx,
        objects,
        [&reason_1, &reason_2],
        cancel_promise,
        reason,
        branch,
    )
}

fn clone_as_uint8_array<'js>(
    ctx: Ctx<'js>,
    constructor_uint8array: &Constructor<'js>,
    function_array_buffer_is_view: &Function<'js>,
    chunk: ViewBytes<'js>,
) -> Result<ViewBytes<'js>> {
    let (buffer, byte_length, byte_offset) = chunk.get_array_buffer()?;

    // Let buffer be ? CloneArrayBuffer(O.[[ViewedArrayBuffer]], O.[[ByteOffset]], O.[[ByteLength]], %ArrayBuffer%).
    let buffer = ArrayBuffer::new_copy(
        ctx.clone(),
        &buffer
            .as_bytes()
            .expect("CloneAsUInt8Array called on detached buffer")
            [byte_offset..byte_offset + byte_length],
    )?;

    // Let array be ! Construct(%Uint8Array%, « buffer »).
    // Return array.
    ViewBytes::from_value(
        &ctx,
        function_array_buffer_is_view,
        Some(&constructor_uint8array.construct((buffer,))?),
    )
}
