use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use lexe_utils::primordials::Primordial;
use rquickjs::{
    atom::PredefinedAtom,
    class::{Trace, Tracer},
    function::Constructor,
    prelude::{IntoArg, OnceFn, Opt, This},
    promise::PromiseState,
    Class, Ctx, Error, FromJs, Function, IntoJs, JsLifetime, Promise, Result, Value,
};

pub fn promise_rejected_with<'js>(
    primordials: &PromisePrimordials<'js>,
    value: Value<'js>,
) -> Result<Promise<'js>> {
    primordials
        .promise_reject
        .call((This(primordials.promise_constructor.clone()), value))
}

pub fn promise_rejected_catch<'js>(
    ctx: &Ctx<'js>,
    promise_primordials: &PromisePrimordials<'js>,
) -> Result<Promise<'js>> {
    promise_rejected_with(promise_primordials, ctx.catch())
}

pub fn promise_rejected_with_constructor<'js, T: From<Error>>(
    constructor: &Constructor<'js>,
    promise_primordials: &PromisePrimordials<'js>,
    msg: &str,
) -> std::result::Result<Promise<'js>, T> {
    let e: Value = constructor.call((msg,))?;
    Ok(promise_rejected_with(promise_primordials, e)?)
}

pub fn promise_resolved_with<'js>(
    ctx: &Ctx<'js>,
    primordials: &PromisePrimordials<'js>,
    value: Result<Value<'js>>,
) -> Result<Promise<'js>> {
    match value {
        Ok(value) => primordials
            .promise_resolve
            .call((This(primordials.promise_constructor.clone()), value)),
        Err(Error::Exception) => primordials
            .promise_reject
            .call((This(primordials.promise_constructor.clone()), ctx.catch())),
        Err(err) => Err(err),
    }
}

#[derive(JsLifetime, Clone)]
pub struct PromisePrimordials<'js> {
    pub promise_constructor: Constructor<'js>,
    pub promise_resolve: Function<'js>,
    pub promise_reject: Function<'js>,
    pub promise_all: Function<'js>,
    pub promise_resolved_with_undefined: Promise<'js>,
    pub function_bind: Function<'js>,
}

impl<'js> Trace<'js> for PromisePrimordials<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.promise_constructor.trace(tracer);
        self.promise_resolve.trace(tracer);
        self.promise_reject.trace(tracer);
        self.promise_all.trace(tracer);
        self.promise_resolved_with_undefined.trace(tracer);
        self.function_bind.trace(tracer);
    }
}

impl<'js> Primordial<'js> for PromisePrimordials<'js> {
    fn new(ctx: &Ctx<'js>) -> Result<Self>
    where
        Self: Sized,
    {
        let promise_constructor: Constructor<'js> = ctx.globals().get(PredefinedAtom::Promise)?;
        let promise_resolve: Function<'js> = promise_constructor.get("resolve")?;
        let promise_reject: Function<'js> = promise_constructor.get("reject")?;
        let promise_all: Function<'js> = promise_constructor.get("all")?;
        let function_bind: Function<'js> = promise_resolve.get("bind")?;

        let promise_resolved_with_undefined = promise_resolve.call((
            This(promise_constructor.clone()),
            Value::new_undefined(ctx.clone()),
        ))?;

        Ok(Self {
            promise_constructor,
            promise_resolve,
            promise_reject,
            promise_all,
            promise_resolved_with_undefined,
            function_bind,
        })
    }
}

// https://webidl.spec.whatwg.org/#dfn-perform-steps-once-promise-is-settled
pub fn upon_promise<'js, Input: FromJs<'js> + 'js, Output: IntoJs<'js> + 'js>(
    ctx: Ctx<'js>,
    promise: Promise<'js>,
    then: impl FnOnce(Ctx<'js>, std::result::Result<Input, Value<'js>>) -> Result<Output> + 'js,
) -> Result<Promise<'js>> {
    let then = Rc::new(Cell::new(Some(then)));
    let then2 = then.clone();
    promise.then()?.call((
        This(promise.clone()),
        Function::new(
            ctx.clone(),
            OnceFn::new(move |ctx, input| {
                then.take()
                    .expect("Promise.then should only call either resolve or reject")(
                    ctx,
                    Ok(input),
                )
            }),
        ),
        Function::new(
            ctx,
            OnceFn::new(move |ctx, e: Value<'js>| {
                then2
                    .take()
                    .expect("Promise.then should only call either resolve or reject")(
                    ctx, Err(e)
                )
            }),
        ),
    ))
}

pub fn upon_promise_fulfilment<'js, Input: FromJs<'js> + 'js, Output: IntoJs<'js> + 'js>(
    ctx: Ctx<'js>,
    promise: Promise<'js>,
    then: impl FnOnce(Ctx<'js>, Input) -> Result<Output> + 'js,
) -> Result<Promise<'js>> {
    promise.then()?.call((
        This(promise.clone()),
        Function::new(ctx.clone(), OnceFn::new(then)),
    ))
}

#[derive(Debug, JsLifetime, Clone)]
pub struct ResolveablePromise<'js> {
    pub promise: Promise<'js>,
    resolve: Option<Function<'js>>,
    reject: Option<Function<'js>>,
}

impl<'js> ResolveablePromise<'js> {
    pub fn new(ctx: &Ctx<'js>) -> Result<Self> {
        let (promise, resolve, reject) = Promise::new(ctx)?;
        Ok(Self {
            promise,
            resolve: Some(resolve),
            reject: Some(reject),
        })
    }

    pub fn resolved_with_undefined(primordials: &PromisePrimordials<'js>) -> Self {
        Self {
            promise: primordials.promise_resolved_with_undefined.clone(),
            resolve: None,
            reject: None,
        }
    }

    pub fn rejected_with(primordials: &PromisePrimordials<'js>, error: Value<'js>) -> Result<Self> {
        Ok(Self {
            promise: promise_rejected_with(primordials, error)?,
            resolve: None,
            reject: None,
        })
    }

    pub fn rejected_with_constructor(
        primordials: &PromisePrimordials<'js>,
        constructor: &Constructor<'js>,
        msg: &str,
    ) -> Result<Self> {
        Ok(Self {
            promise: promise_rejected_with_constructor::<rquickjs::Error>(
                constructor,
                primordials,
                msg,
            )?,
            resolve: None,
            reject: None,
        })
    }

    pub fn resolve(&self, value: impl IntoArg<'js>) -> Result<()> {
        if let Some(resolve) = &self.resolve {
            let () = resolve.call((value,))?;
        }
        Ok(())
    }

    pub fn resolve_undefined(&self) -> Result<()> {
        if let Some(resolve) = &self.resolve {
            let () = resolve.call((rquickjs::Undefined,))?;
        }
        Ok(())
    }

    pub fn reject(&self, value: impl IntoArg<'js>) -> Result<()> {
        if let Some(reject) = &self.reject {
            let () = reject.call((value,))?;
        }
        Ok(())
    }

    pub fn reject_with_constructor(&self, constructor: &Constructor<'js>, msg: &str) -> Result<()> {
        if let Some(reject) = &self.reject {
            let e: Value = constructor.call((msg,))?;
            let () = reject.call((e,))?;
        }
        Ok(())
    }

    pub fn is_pending(&self) -> bool {
        self.promise.state() == PromiseState::Pending
    }

    pub fn set_is_handled(&self) -> Result<()> {
        self.promise.catch()?.call((
            This(self.promise.clone()),
            Function::new(self.promise.ctx().clone(), || {}),
        ))
    }
}

impl<'js> Trace<'js> for ResolveablePromise<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.promise.trace(tracer);
        self.resolve.trace(tracer);
        self.reject.trace(tracer);
    }
}

pub trait TracedReaction<'js>: Trace<'js> {
    fn call(
        self: Box<Self>,
        ctx: Ctx<'js>,
        result: std::result::Result<Value<'js>, Value<'js>>,
    ) -> Result<Value<'js>>;
}

pub trait TracedCallback<'js>: Trace<'js> {
    fn call(self: Box<Self>, ctx: Ctx<'js>, arg: Value<'js>) -> Result<Value<'js>>;
}

#[rquickjs::class]
pub(crate) struct TracedReactionCell<'js> {
    inner: RefCell<Option<Box<dyn TracedReaction<'js> + 'js>>>,
}

unsafe impl<'js> JsLifetime<'js> for TracedReactionCell<'js> {
    type Changed<'to> = TracedReactionCell<'to>;
}

impl<'js> Trace<'js> for TracedReactionCell<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        if let Ok(inner) = self.inner.try_borrow() {
            if let Some(cb) = inner.as_ref() {
                cb.trace(tracer);
            }
        }
    }
}

#[rquickjs::class]
pub(crate) struct TracedCallbackCell<'js> {
    inner: RefCell<Option<Box<dyn TracedCallback<'js> + 'js>>>,
}

unsafe impl<'js> JsLifetime<'js> for TracedCallbackCell<'js> {
    type Changed<'to> = TracedCallbackCell<'to>;
}

impl<'js> Trace<'js> for TracedCallbackCell<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        if let Ok(inner) = self.inner.try_borrow() {
            if let Some(cb) = inner.as_ref() {
                cb.trace(tracer);
            }
        }
    }
}

/// Bind a GC-traced state cell onto a stateless Rust JS function via `Function.prototype.bind`
/// so QuickJS's `JSBoundFunction` marks captured JS objects and avoids `JS_FreeRuntime` leaks.
pub fn make_traced_once_fn<'js, C: TracedCallback<'js> + 'js>(
    ctx: &Ctx<'js>,
    cb: C,
) -> Result<Function<'js>> {
    let cell = Class::instance(
        ctx.clone(),
        TracedCallbackCell {
            inner: RefCell::new(Some(Box::new(cb))),
        },
    )?;
    let invoke = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>,
         cell: Class<'js, TracedCallbackCell<'js>>,
         arg: Opt<Value<'js>>|
         -> Result<Value<'js>> {
            let taken = cell.borrow().inner.borrow_mut().take();
            match taken {
                Some(cb) => {
                    let v = arg.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                    cb.call(ctx, v)
                },
                None => Ok(Value::new_undefined(ctx)),
            }
        },
    )?;
    let bind = PromisePrimordials::get(ctx)?.function_bind.clone();
    bind.call((This(invoke), Value::new_undefined(ctx.clone()), cell))
}

pub fn upon_promise_traced<'js, C: TracedReaction<'js> + 'js>(
    ctx: &Ctx<'js>,
    promise: Promise<'js>,
    cb: C,
) -> Result<Promise<'js>> {
    let cell = Class::instance(
        ctx.clone(),
        TracedReactionCell {
            inner: RefCell::new(Some(Box::new(cb))),
        },
    )?;
    let invoke = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>,
         cell: Class<'js, TracedReactionCell<'js>>,
         is_reject: bool,
         arg: Opt<Value<'js>>|
         -> Result<Value<'js>> {
            let taken = cell.borrow().inner.borrow_mut().take();
            match taken {
                Some(cb) => {
                    let v = arg.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                    let res = if is_reject { Err(v) } else { Ok(v) };
                    cb.call(ctx, res)
                },
                None => Ok(Value::new_undefined(ctx)),
            }
        },
    )?;
    let bind = PromisePrimordials::get(ctx)?.function_bind.clone();
    let on_fulfill: Function<'js> = bind.call((
        This(invoke.clone()),
        Value::new_undefined(ctx.clone()),
        cell.clone(),
        false,
    ))?;
    let on_reject: Function<'js> =
        bind.call((This(invoke), Value::new_undefined(ctx.clone()), cell, true))?;
    promise.then()?.call((This(promise), on_fulfill, on_reject))
}

pub fn upon_promise_fulfilment_traced<'js, C: TracedCallback<'js> + 'js>(
    ctx: &Ctx<'js>,
    promise: Promise<'js>,
    cb: C,
) -> Result<Promise<'js>> {
    let on_fulfill = make_traced_once_fn(ctx, cb)?;
    promise.then()?.call((This(promise), on_fulfill))
}

pub fn on_promise_rejected_traced<'js, C: TracedCallback<'js> + 'js>(
    ctx: &Ctx<'js>,
    promise: Promise<'js>,
    cb: C,
) -> Result<()> {
    let on_reject = make_traced_once_fn(ctx, cb)?;
    promise.catch()?.call((This(promise), on_reject))
}

pub fn with_promise_result<'js>(
    ctx: &Ctx<'js>,
    f: impl FnOnce() -> Result<Promise<'js>>,
) -> Result<Promise<'js>> {
    match f() {
        Ok(value) => Ok(value),
        Err(Error::Exception) => {
            // Clone and drop the userdata guard before calling Promise.reject so
            // host_promise_rejection_tracker can safely borrow ctx userdata.
            let primordials = PromisePrimordials::get(ctx)?.clone();
            promise_rejected_catch(ctx, &primordials)
        },
        Err(Error::FromJs { from, to, message }) => {
            let primordials = PromisePrimordials::get(ctx)?.clone();
            let type_error = lexe_utils::primordials::BasePrimordials::get(ctx)?
                .constructor_type_error
                .clone();
            let msg = match message {
                Some(m) => format!("Error converting from js '{from}' into type '{to}': {m}"),
                None => format!("Error converting from js '{from}' into type '{to}'"),
            };
            let err_val: Value<'js> = type_error.call((msg,))?;
            promise_rejected_with(&primordials, err_val)
        },
        Err(err) => Err(err),
    }
}
