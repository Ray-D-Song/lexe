use lexe_utils::{
    option::{Null, Undefined},
    primordials::Primordial,
};
use rquickjs::{
    class::Trace,
    prelude::{Opt, This},
    Class, Ctx, Exception, JsLifetime, Object, Promise, Result, Value,
};

use crate::{
    queuing_strategy::QueuingStrategy,
    readable::stream::{
        algorithms::{CancelAlgorithm, PullAlgorithm, StartAlgorithm},
        ReadableStream, ReadableStreamState,
    },
    utils::promise::{promise_rejected_with, PromisePrimordials, ResolveablePromise},
    writable::{WritableStream, WritableStreamState},
};

use super::{
    controller::{
        self, CancelAlgorithm as TsCancelAlgorithm, FlushAlgorithm, TransformAlgorithm,
        TransformStreamDefaultController, TransformStreamDefaultControllerClass,
    },
    transformer::Transformer,
};

#[rquickjs::class]
#[derive(JsLifetime, Trace)]
pub(crate) struct TransformStream<'js> {
    pub(super) readable: Option<Class<'js, ReadableStream<'js>>>,
    pub(super) writable: Option<Class<'js, WritableStream<'js>>>,
    pub(super) controller: Option<TransformStreamDefaultControllerClass<'js>>,
    pub(super) backpressure: bool,
    pub(super) backpressure_change_promise: Option<ResolveablePromise<'js>>,
}

pub(crate) type TransformStreamClass<'js> = Class<'js, TransformStream<'js>>;

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> TransformStream<'js> {
    #[qjs(constructor)]
    fn new(
        ctx: Ctx<'js>,
        transformer: Opt<Undefined<Object<'js>>>,
        writable_strategy: Opt<Null<Undefined<QueuingStrategy<'js>>>>,
        readable_strategy: Opt<Null<Undefined<QueuingStrategy<'js>>>>,
    ) -> Result<Class<'js, Self>> {
        let transformer_obj = transformer.0.and_then(|u| u.0);
        let transformer_dict = transformer_obj
            .as_ref()
            .map(|obj| Transformer::from_object(obj.clone()))
            .transpose()?
            .unwrap_or_default();

        if transformer_dict.readable_type {
            return Err(Exception::throw_range(
                &ctx,
                "readableType is not supported",
            ));
        }
        if transformer_dict.writable_type {
            return Err(Exception::throw_range(
                &ctx,
                "writableType is not supported",
            ));
        }

        let readable_strategy = readable_strategy.0.and_then(|n| n.0).and_then(|u| u.0);
        let writable_strategy = writable_strategy.0.and_then(|n| n.0).and_then(|u| u.0);

        let writable_size = QueuingStrategy::extract_size_algorithm(writable_strategy.as_ref());
        let writable_hwm = QueuingStrategy::extract_high_water_mark(&ctx, writable_strategy, 1.0)?;
        let readable_size = QueuingStrategy::extract_size_algorithm(readable_strategy.as_ref());
        let readable_hwm = QueuingStrategy::extract_high_water_mark(&ctx, readable_strategy, 0.0)?;

        // Create the TransformStream instance
        let stream_class = Class::instance(
            ctx.clone(),
            Self {
                readable: None,
                writable: None,
                controller: None,
                backpressure: false,
                backpressure_change_promise: None,
            },
        )?;

        // Build controller algorithms
        let transform_algorithm = transformer_dict
            .transform
            .map(|f| TransformAlgorithm::Function {
                f,
                transformer: transformer_obj.clone(),
            })
            .unwrap_or(TransformAlgorithm::Identity);

        let flush_algorithm = transformer_dict
            .flush
            .map(|f| FlushAlgorithm::Function {
                f,
                transformer: transformer_obj.clone(),
            })
            .unwrap_or(FlushAlgorithm::Noop);

        let cancel_algorithm = transformer_dict
            .cancel
            .map(|f| TsCancelAlgorithm::Function {
                f,
                transformer: transformer_obj.clone(),
            })
            .unwrap_or(TsCancelAlgorithm::Noop);

        // Create controller
        let controller_class = Class::instance(
            ctx.clone(),
            TransformStreamDefaultController {
                stream: stream_class.clone(),
                transform_algorithm: Some(transform_algorithm),
                flush_algorithm: Some(flush_algorithm),
                cancel_algorithm: Some(cancel_algorithm),
                finish_promise: None,
            },
        )?;
        stream_class.borrow_mut().controller = Some(controller_class.clone());

        // Start promise shared between readable and writable sides
        let start_promise = ResolveablePromise::new(&ctx)?;

        // --- Create writable side with properly traced algorithm variants ---
        let writable_class = WritableStream::create_for_transform(
            ctx.clone(),
            start_promise.promise.clone(),
            stream_class.clone(),
            controller_class.clone(),
            writable_hwm,
            writable_size,
        )?;

        // --- Create readable side ---
        let pull_algorithm = PullAlgorithm::Transform(stream_class.clone());

        let cancel_algo = CancelAlgorithm::Transform {
            stream: stream_class.clone(),
            controller: controller_class.clone(),
        };

        let readable_objects = ReadableStream::create_readable_stream(
            ctx.clone(),
            StartAlgorithm::Transform(start_promise.promise.clone()),
            pull_algorithm,
            cancel_algo,
            Some(readable_hwm),
            Some(readable_size),
        )?;

        {
            let mut stream = stream_class.borrow_mut();
            stream.readable = Some(readable_objects.stream.clone());
            stream.writable = Some(writable_class);
        }

        // Initial backpressure is always true per InitializeTransformStream
        controller::transform_stream_set_backpressure(&ctx, &stream_class, true)?;

        // Invoke start() if present (synchronous throw propagates out of constructor)
        if let Some(start_fn) = transformer_dict.start {
            let val: Value = start_fn.call((This(transformer_obj), controller_class))?;
            start_promise.resolve(val)?;
        } else {
            start_promise.resolve_undefined()?;
        }

        Ok(stream_class)
    }

    #[qjs(get)]
    fn readable(&self) -> Option<Class<'js, ReadableStream<'js>>> {
        self.readable.clone()
    }

    #[qjs(get)]
    fn writable(&self) -> Option<Class<'js, WritableStream<'js>>> {
        self.writable.clone()
    }
}

// --- Sink algorithms ---

fn clear_backpressure_promise<'js>(stream_class: &TransformStreamClass<'js>) -> Result<()> {
    let mut stream = stream_class.borrow_mut();
    if let Some(ref bp) = stream.backpressure_change_promise {
        bp.resolve_undefined()?;
    }
    stream.backpressure_change_promise = None;
    stream.backpressure = false;
    Ok(())
}

pub(crate) fn sink_write_algorithm<'js>(
    ctx: Ctx<'js>,
    stream_class: &TransformStreamClass<'js>,
    controller_class: &TransformStreamDefaultControllerClass<'js>,
    chunk: Value<'js>,
) -> Result<Promise<'js>> {
    let stream = stream_class.borrow();
    if stream.backpressure {
        let bp_promise = stream
            .backpressure_change_promise
            .as_ref()
            .map(|p| p.promise.clone());
        drop(stream);

        if let Some(bp_promise) = bp_promise {
            let sc = stream_class.clone();
            let cc = controller_class.clone();
            return crate::utils::promise::upon_promise::<Value<'js>, _>(
                ctx.clone(),
                bp_promise,
                move |ctx, _| {
                    let stored_error = {
                        let stream = sc.borrow();
                        stream
                            .writable
                            .as_ref()
                            .and_then(|w| w.borrow().stored_error())
                    };
                    if let Some(err) = stored_error {
                        return Err(ctx.throw(err));
                    }
                    let p = controller::transform_stream_default_controller_perform_transform(
                        ctx.clone(),
                        &sc,
                        &cc,
                        chunk,
                    )?;
                    Ok(p.into_value())
                },
            );
        }
    } else {
        drop(stream);
    }

    let stored_error = {
        let stream = stream_class.borrow();
        stream
            .writable
            .as_ref()
            .and_then(|w| w.borrow().stored_error())
    };
    if let Some(err) = stored_error {
        let primordials = PromisePrimordials::get(&ctx)?.clone();
        return promise_rejected_with(&primordials, err);
    }

    controller::transform_stream_default_controller_perform_transform(
        ctx,
        stream_class,
        controller_class,
        chunk,
    )
}

pub(crate) fn sink_close_algorithm<'js>(
    ctx: Ctx<'js>,
    stream_class: &TransformStreamClass<'js>,
    controller_class: &TransformStreamDefaultControllerClass<'js>,
) -> Result<Promise<'js>> {
    if let Some(ref existing) = controller_class.borrow().finish_promise {
        return Ok(existing.promise.clone());
    }

    let finish_promise = ResolveablePromise::new(&ctx)?;
    controller_class.borrow_mut().finish_promise = Some(finish_promise.clone());

    let flush_promise = controller::perform_flush(ctx.clone(), stream_class, controller_class)?;
    controller_class.borrow_mut().clear_algorithms();

    let sc = stream_class.clone();
    let fp = finish_promise.clone();
    let reaction = crate::utils::promise::upon_promise::<Value<'js>, _>(
        ctx.clone(),
        flush_promise,
        move |ctx, result| {
            clear_backpressure_promise(&sc)?;
            match result {
                Ok(_) => {
                    let (readable_errored, readable_controller) = {
                        let stream = sc.borrow();
                        if let Some(ref readable) = stream.readable {
                            let r = readable.borrow();
                            let err = match r.state {
                                ReadableStreamState::Errored(ref e) => Some(e.clone()),
                                _ => None,
                            };
                            let ctrl = match &r.controller {
                                crate::readable::ReadableStreamControllerClass::ReadableStreamDefaultController(c) => Some(c.clone()),
                                _ => None,
                            };
                            (err, ctrl)
                        } else {
                            (None, None)
                        }
                    };
                    if let Some(e) = readable_errored {
                        fp.reject(e)?;
                    } else {
                        if let Some(c) = readable_controller {
                            crate::readable::readable_stream_default_controller_close_stream(
                                ctx.clone(),
                                c,
                            )?;
                        }
                        fp.resolve_undefined()?;
                    }
                    Ok(Value::new_undefined(ctx))
                },
                Err(r) => {
                    controller::transform_stream_error(ctx.clone(), &sc, r.clone())?;
                    fp.reject(r)?;
                    Ok(Value::new_undefined(ctx))
                },
            }
        },
    )?;
    let _: () = reaction.catch()?.call((
        This(reaction.clone()),
        rquickjs::Function::new(ctx.clone(), || {}),
    ))?;

    Ok(finish_promise.promise)
}

pub(crate) fn sink_abort_algorithm<'js>(
    ctx: Ctx<'js>,
    stream_class: &TransformStreamClass<'js>,
    controller_class: &TransformStreamDefaultControllerClass<'js>,
    reason: Value<'js>,
) -> Result<Promise<'js>> {
    if let Some(ref existing) = controller_class.borrow().finish_promise {
        return Ok(existing.promise.clone());
    }

    let finish_promise = ResolveablePromise::new(&ctx)?;
    controller_class.borrow_mut().finish_promise = Some(finish_promise.clone());

    let cancel_promise = controller::perform_cancel(ctx.clone(), controller_class, reason.clone())?;
    controller_class.borrow_mut().clear_algorithms();

    let sc = stream_class.clone();
    let fp = finish_promise.clone();
    let reaction = crate::utils::promise::upon_promise::<Value<'js>, _>(
        ctx.clone(),
        cancel_promise,
        move |ctx, result| {
            clear_backpressure_promise(&sc)?;
            let (readable_errored, readable_controller) = {
                let stream = sc.borrow();
                if let Some(ref readable) = stream.readable {
                    let r = readable.borrow();
                    let err = match r.state {
                        ReadableStreamState::Errored(ref e) => Some(e.clone()),
                        _ => None,
                    };
                    let ctrl = match &r.controller {
                        crate::readable::ReadableStreamControllerClass::ReadableStreamDefaultController(c) => Some(c.clone()),
                        _ => None,
                    };
                    (err, ctrl)
                } else {
                    (None, None)
                }
            };
            match result {
                Ok(_) => {
                    if let Some(e) = readable_errored {
                        fp.reject(e)?;
                    } else {
                        if let Some(c) = readable_controller {
                            crate::readable::readable_stream_default_controller_error_stream(
                                c, reason,
                            )?;
                        }
                        fp.resolve_undefined()?;
                    }
                },
                Err(r) => {
                    if let Some(c) = readable_controller {
                        crate::readable::readable_stream_default_controller_error_stream(
                            c,
                            r.clone(),
                        )?;
                    }
                    fp.reject(r)?;
                },
            }
            Ok(Value::new_undefined(ctx))
        },
    )?;
    let _: () = reaction.catch()?.call((
        This(reaction.clone()),
        rquickjs::Function::new(ctx.clone(), || {}),
    ))?;

    Ok(finish_promise.promise)
}

// --- Source algorithms ---

pub(crate) fn source_pull_algorithm<'js>(
    ctx: Ctx<'js>,
    stream_class: &TransformStreamClass<'js>,
) -> Result<Promise<'js>> {
    controller::transform_stream_set_backpressure(&ctx, stream_class, false)?;
    let stream = stream_class.borrow();
    Ok(stream
        .backpressure_change_promise
        .as_ref()
        .expect("backpressureChangePromise set by transform_stream_set_backpressure")
        .promise
        .clone())
}

pub(crate) fn source_cancel_algorithm<'js>(
    ctx: Ctx<'js>,
    stream_class: &TransformStreamClass<'js>,
    controller_class: &TransformStreamDefaultControllerClass<'js>,
    reason: Value<'js>,
) -> Result<Promise<'js>> {
    if let Some(ref existing) = controller_class.borrow().finish_promise {
        return Ok(existing.promise.clone());
    }

    let finish_promise = ResolveablePromise::new(&ctx)?;
    controller_class.borrow_mut().finish_promise = Some(finish_promise.clone());

    let cancel_promise = controller::perform_cancel(ctx.clone(), controller_class, reason.clone())?;
    controller_class.borrow_mut().clear_algorithms();

    let sc = stream_class.clone();
    let fp = finish_promise.clone();
    let reaction = crate::utils::promise::upon_promise::<Value<'js>, _>(
        ctx.clone(),
        cancel_promise,
        move |ctx, result| {
            match result {
                Ok(_) => {
                    let writable_errored = {
                        let stream = sc.borrow();
                        stream.writable.as_ref().and_then(|w| {
                            let w = w.borrow();
                            match w.state {
                                WritableStreamState::Errored(ref e) => Some(e.clone()),
                                _ => None,
                            }
                        })
                    };
                    if let Some(e) = writable_errored {
                        clear_backpressure_promise(&sc)?;
                        fp.reject(e)?;
                    } else {
                        controller::transform_stream_error_writable_and_unblock_write(
                            ctx.clone(),
                            &sc,
                            reason,
                        )?;
                        clear_backpressure_promise(&sc)?;
                        fp.resolve_undefined()?;
                    }
                },
                Err(r) => {
                    controller::transform_stream_error_writable_and_unblock_write(
                        ctx.clone(),
                        &sc,
                        r.clone(),
                    )?;
                    clear_backpressure_promise(&sc)?;
                    fp.reject(r)?;
                },
            }
            Ok(Value::new_undefined(ctx))
        },
    )?;
    let _: () = reaction.catch()?.call((
        This(reaction.clone()),
        rquickjs::Function::new(ctx.clone(), || {}),
    ))?;

    Ok(finish_promise.promise)
}
