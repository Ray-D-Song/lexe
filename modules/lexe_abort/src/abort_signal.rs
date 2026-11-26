// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    any::Any,
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock, RwLock,
    },
};

use lexe_events::{Emitter, EventEmitter, EventList};
use lexe_exceptions::{DOMException, DOMExceptionName};
use lexe_utils::{
    clone_wire::{
        deserialize, register_native_codec, register_native_transfer_codec, serialize,
        CloneWirePayload, NativeCloneCodec, NativeTransferCodec,
    },
    event_loop::{self, wait_until_io_unref, EventLoopRef},
    mc_oneshot,
};
use rquickjs::{
    atom::PredefinedAtom,
    class::{Trace, Tracer},
    function::{OnceFn, Opt},
    prelude::This,
    qjs, Array, Class, Ctx, Error, Exception, Function, JsLifetime, Result, Undefined, Value,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

#[derive(Default)]
struct SharedAbortEntry {
    aborted: bool,
    reason: Option<CloneWirePayload>,
    watchers: Vec<UnboundedSender<Option<CloneWirePayload>>>,
}

static NEXT_ABORT_ID: AtomicU64 = AtomicU64::new(1);
static ABORT_HUB: OnceLock<Mutex<HashMap<u64, SharedAbortEntry>>> = OnceLock::new();

fn abort_hub() -> &'static Mutex<HashMap<u64, SharedAbortEntry>> {
    ABORT_HUB.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
struct TransferredAbortSignalHandle {
    shared_id: u64,
}

#[derive(Clone)]
#[rquickjs::class]
pub struct AbortSignal<'js> {
    emitter: EventEmitter<'js>,
    pub aborted: bool,
    reason: Option<Value<'js>>,
    pub sender: mc_oneshot::Sender<Value<'js>>,
    pub transferable: bool,
    pub detached: bool,
    pub shared_id: Option<u64>,
}

unsafe impl<'js> JsLifetime<'js> for AbortSignal<'js> {
    type Changed<'to> = AbortSignal<'to>;
}

impl<'js> Trace<'js> for AbortSignal<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        if let Some(reason) = &self.reason {
            tracer.mark(reason);
        }
        self.emitter.trace(tracer);
        self.sender.trace(tracer);
    }
}

impl<'js> Emitter<'js> for AbortSignal<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> AbortSignal<'js> {
    #[qjs(constructor)]
    pub fn new() -> Self {
        let (sender, _) = mc_oneshot::channel::<Value<'js>>();
        Self {
            emitter: EventEmitter::new(),
            aborted: false,
            reason: None,
            sender,
            transferable: false,
            detached: false,
            shared_id: None,
        }
    }

    #[qjs(get, rename = "onabort")]
    pub fn get_on_abort(&self) -> Option<Function<'js>> {
        Self::get_listeners_str(self, "abort").first().cloned()
    }

    #[qjs(set, rename = "onabort")]
    pub fn set_on_abort(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        listener: Function<'js>,
    ) -> Result<()> {
        Self::add_event_listener_str(this.0, &ctx, "abort", listener, false, false)?;
        Ok(())
    }

    pub fn remove_on_abort(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        listener: Function<'js>,
    ) -> Result<()> {
        Self::remove_event_listener_str(this.0, &ctx, "abort", listener)?;
        Ok(())
    }

    pub fn throw_if_aborted(&self, ctx: Ctx<'js>) -> Result<()> {
        if self.aborted {
            return Err(ctx.throw(
                self.reason
                    .clone()
                    .unwrap_or_else(|| Undefined.into_value(ctx.clone())),
            ));
        }
        Ok(())
    }

    #[qjs(static)]
    pub fn any(ctx: Ctx<'js>, signals: Array<'js>) -> Result<Class<'js, Self>> {
        let mut new_signal = AbortSignal::new();

        let mut signal_instances = Vec::with_capacity(signals.len());

        for signal in signals.iter() {
            let signal: Value = signal?;
            let signal: Class<AbortSignal> = Class::from_value(&signal)
                .map_err(|_| Exception::throw_type(&ctx, "Value is not an AbortSignal instance"))?;
            let signal_borrow = signal.borrow();
            if signal_borrow.aborted {
                new_signal.aborted = true;
                new_signal.reason.clone_from(&signal_borrow.reason);
                let new_signal = Class::instance(ctx, new_signal)?;
                return Ok(new_signal);
            } else {
                drop(signal_borrow);
                signal_instances.push(signal);
            }
        }

        let new_signal_instance = Class::instance(ctx.clone(), new_signal)?;
        for signal in signal_instances {
            let signal_instance_2 = new_signal_instance.clone();
            Self::add_event_listener_str(
                signal,
                &ctx,
                "abort",
                Function::new(
                    ctx.clone(),
                    OnceFn::from(|ctx, signal| {
                        struct Args<'js>(Ctx<'js>, This<Class<'js, AbortSignal<'js>>>);
                        let Args(ctx, signal) = Args(ctx, signal);
                        let mut borrow = signal_instance_2.borrow_mut();
                        borrow.aborted = true;
                        borrow.reason.clone_from(&signal.borrow().reason);
                        drop(borrow);
                        Self::send_aborted(This(signal_instance_2), ctx)
                    }),
                )?,
                false,
                true,
            )?;
        }

        Ok(new_signal_instance)
    }

    #[qjs(get)]
    pub fn aborted(&self) -> bool {
        self.aborted
    }

    #[qjs(get)]
    pub fn reason(&self) -> Option<Value<'js>> {
        self.reason.clone()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(AbortSignal)
    }

    #[qjs(set, rename = "reason")]
    pub fn set_reason(&mut self, reason: Opt<Value<'js>>) {
        match reason.0 {
            Some(new_reason) if !new_reason.is_undefined() => self.reason.replace(new_reason),
            _ => self.reason.take(),
        };
    }

    #[qjs(skip)]
    pub fn send_aborted(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        let mut borrow = this.borrow_mut();
        borrow.aborted = true;
        let reason = get_reason_or_dom_exception(
            &ctx,
            borrow.reason.as_ref(),
            DOMExceptionName::AbortError,
        )?;
        borrow.reason = Some(reason.clone());
        borrow.sender.send(reason.clone());
        let shared_id = borrow.shared_id;
        drop(borrow);

        if let Some(id) = shared_id {
            let wire_reason = serialize(&ctx, reason, Opt(None)).ok();
            let watchers = {
                let mut map = abort_hub().lock().unwrap();
                if let Some(entry) = map.get_mut(&id) {
                    if !entry.aborted {
                        entry.aborted = true;
                        entry.reason = wire_reason.clone();
                        std::mem::take(&mut entry.watchers)
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            };
            for tx in watchers {
                let _ = tx.send(wire_reason.clone());
            }
        }

        Self::emit_str(this.0, &ctx, "abort", vec![], false)?;
        Ok(())
    }

    #[qjs(static)]
    pub fn abort(ctx: Ctx<'js>, reason: Opt<Value<'js>>) -> Result<Class<'js, Self>> {
        let mut signal = Self::new();
        signal.set_reason(reason);
        let instance = Class::instance(ctx.clone(), signal)?;
        Self::send_aborted(This(instance.clone()), ctx)?;
        Ok(instance)
    }

    #[qjs(static)]
    pub fn timeout(ctx: Ctx<'js>, milliseconds: u64) -> Result<Class<'js, Self>> {
        let timeout_error =
            get_reason_or_dom_exception(&ctx, None, DOMExceptionName::TimeoutError)?;

        let signal = Self::new();
        let signal_instance = Class::instance(ctx.clone(), signal)?;
        let signal_instance2 = signal_instance.clone();

        let cb = Function::new(
            ctx.clone(),
            OnceFn::from(move |ctx| {
                let mut borrow = signal_instance.borrow_mut();
                borrow.set_reason(Opt(Some(timeout_error)));
                drop(borrow);
                Self::send_aborted(This(signal_instance), ctx)?;
                Ok::<_, Error>(())
            }),
        )?;

        #[cfg(feature = "sleep-timers")]
        {
            // AbortSignal.timeout() must not keep the event loop alive on its own.
            lexe_timers::set_timeout_interval(
                &ctx,
                cb,
                milliseconds,
                lexe_utils::provider::ProviderType::Timeout,
                false,
            )?;
        }
        #[cfg(all(not(feature = "sleep-timers"), feature = "sleep-tokio"))]
        {
            use lexe_utils::ctx::CtxExtension;
            ctx.clone().spawn_exit_simple(async move {
                tokio::time::sleep(std::time::Duration::from_millis(milliseconds)).await;
                cb.call::<_, ()>(())?;
                Ok(())
            });
        }
        #[cfg(all(not(feature = "sleep-tokio"), not(feature = "sleep-timers")))]
        {
            compile_error!("Either the `sleep-tokio` or `sleep-timers` feature must be enabled")
        }

        Ok(signal_instance2)
    }
}

fn get_reason_or_dom_exception<'js>(
    ctx: &Ctx<'js>,
    reason: Option<&Value<'js>>,
    name: DOMExceptionName,
) -> Result<Value<'js>> {
    let reason = if let Some(reason) = reason {
        reason.clone()
    } else {
        let ex = DOMException::new_with_name(ctx, name, String::new())?;
        Class::instance(ctx.clone(), ex)?.into_value()
    };
    Ok(reason)
}

fn ensure_shared_id<'js>(ctx: &Ctx<'js>, signal: &Class<'js, AbortSignal<'js>>) -> u64 {
    let mut borrow = signal.borrow_mut();
    if let Some(id) = borrow.shared_id {
        return id;
    }
    let id = NEXT_ABORT_ID.fetch_add(1, Ordering::Relaxed);
    borrow.shared_id = Some(id);
    let aborted = borrow.aborted;
    let reason = borrow
        .reason
        .clone()
        .and_then(|r| serialize(ctx, r, Opt(None)).ok());
    drop(borrow);
    abort_hub().lock().unwrap().insert(
        id,
        SharedAbortEntry {
            aborted,
            reason,
            watchers: Vec::new(),
        },
    );
    id
}

pub fn make_signal_transferable<'js>(
    ctx: &Ctx<'js>,
    signal: Class<'js, AbortSignal<'js>>,
) -> Result<Class<'js, AbortSignal<'js>>> {
    signal.borrow_mut().transferable = true;
    ensure_shared_id(ctx, &signal);
    Ok(signal)
}

fn build_transferred_signal<'js>(ctx: &Ctx<'js>, shared_id: u64) -> Result<Value<'js>> {
    let (already_aborted, reason_wire) = {
        let map = abort_hub().lock().unwrap();
        match map.get(&shared_id) {
            Some(entry) => (entry.aborted, entry.reason.clone()),
            None => (false, None),
        }
    };

    let mut sig = AbortSignal::new();
    sig.transferable = true;
    sig.shared_id = Some(shared_id);

    if already_aborted {
        sig.aborted = true;
        let reason_val = match reason_wire {
            Some(ref wire) => deserialize(ctx, wire)?,
            None => {
                let ex =
                    DOMException::new_with_name(ctx, DOMExceptionName::AbortError, String::new())?;
                Class::instance(ctx.clone(), ex)?.into_value()
            },
        };
        sig.reason = Some(reason_val);
        return Ok(Class::instance(ctx.clone(), sig)?.into_value());
    }

    let instance = Class::instance(ctx.clone(), sig)?;
    let (tx, mut rx) = unbounded_channel::<Option<CloneWirePayload>>();
    {
        let mut map = abort_hub().lock().unwrap();
        let entry = map.entry(shared_id).or_default();
        if entry.aborted {
            let reason_wire = entry.reason.clone();
            drop(map);
            let reason_val = match reason_wire {
                Some(ref wire) => deserialize(ctx, wire)?,
                None => {
                    let ex = DOMException::new_with_name(
                        ctx,
                        DOMExceptionName::AbortError,
                        String::new(),
                    )?;
                    Class::instance(ctx.clone(), ex)?.into_value()
                },
            };
            instance.borrow_mut().set_reason(Opt(Some(reason_val)));
            AbortSignal::send_aborted(This(instance.clone()), ctx.clone())?;
            return Ok(instance.into_value());
        }
        entry.watchers.push(tx);
    }

    let rt_ptr = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let loop_ref = EventLoopRef::new_unreferenced(event_loop::rt_key_from_ptr(rt_ptr as *mut _));
    let ctx_clone = ctx.clone();
    let sig_clone = instance.clone();
    ctx.spawn(async move {
        tokio::select! {
            maybe = rx.recv() => {
                if let Some(reason_opt) = maybe {
                    if !sig_clone.borrow().aborted {
                        if let Some(wire) = reason_opt {
                            if let Ok(val) = deserialize(&ctx_clone, &wire) {
                                sig_clone.borrow_mut().set_reason(Opt(Some(val)));
                            }
                        }
                        let _ = AbortSignal::send_aborted(This(sig_clone), ctx_clone);
                    }
                }
            }
            _ = wait_until_io_unref(Arc::clone(&loop_ref)) => {}
        }
    });

    Ok(instance.into_value())
}

fn throw_data_clone_err<'js>(ctx: &Ctx<'js>, msg: &str) -> Error {
    if let Ok(dom_ctor) = ctx
        .globals()
        .get::<_, rquickjs::function::Constructor>("DOMException")
    {
        if let Ok(obj) = dom_ctor.construct::<_, rquickjs::Object<'js>>((msg, "DataCloneError")) {
            return ctx.throw(obj.into_value());
        }
    }
    Exception::throw_type(ctx, msg)
}

fn extract_abort_signal_handle<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    for_wire: bool,
) -> Result<Option<Arc<dyn Any + Send + Sync>>> {
    let Ok(cls) = Class::<AbortSignal<'js>>::from_value(value) else {
        return Ok(None);
    };
    if !for_wire {
        return Ok(Some(Arc::new(TransferredAbortSignalHandle {
            shared_id: 0,
        })));
    }
    let (transferable, detached) = {
        let borrow = cls.borrow();
        (borrow.transferable, borrow.detached)
    };
    if !transferable || detached {
        return Err(throw_data_clone_err(
            ctx,
            "AbortSignal could not be cloned.",
        ));
    }
    let shared_id = ensure_shared_id(ctx, &cls);
    cls.borrow_mut().detached = true;
    Ok(Some(Arc::new(TransferredAbortSignalHandle { shared_id })))
}

fn rebuild_abort_signal_value<'js>(
    ctx: &Ctx<'js>,
    handle: &Arc<dyn Any + Send + Sync>,
) -> Result<Option<Value<'js>>> {
    let Some(transferred) = handle.downcast_ref::<TransferredAbortSignalHandle>() else {
        return Ok(None);
    };
    Ok(Some(build_transferred_signal(ctx, transferred.shared_id)?))
}

fn transfer_abort_signal<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<Option<Value<'js>>> {
    let Ok(cls) = Class::<AbortSignal<'js>>::from_value(value) else {
        return Ok(None);
    };
    let (transferable, detached) = {
        let borrow = cls.borrow();
        (borrow.transferable, borrow.detached)
    };
    if !transferable {
        return Err(throw_data_clone_err(
            ctx,
            "AbortSignal could not be cloned.",
        ));
    }
    if detached {
        return Err(throw_data_clone_err(
            ctx,
            "AbortSignal has already been detached.",
        ));
    }
    let shared_id = ensure_shared_id(ctx, &cls);
    cls.borrow_mut().detached = true;
    Ok(Some(build_transferred_signal(ctx, shared_id)?))
}

pub fn register_codecs() {
    register_native_codec(NativeCloneCodec {
        name: "AbortSignal",
        transfer_only: true,
        try_extract: extract_abort_signal_handle,
        try_rebuild: rebuild_abort_signal_value,
    });
    register_native_transfer_codec(NativeTransferCodec {
        name: "AbortSignal",
        try_transfer: transfer_abort_signal,
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lexe_test::test_async_with;

    use super::*;

    #[cfg(feature = "sleep-timers")]
    #[tokio::test]
    async fn test_abort_signal() {
        test_async_with(|ctx| {
            crate::init(&ctx).unwrap();
            lexe_timers::init(&ctx).unwrap();
            Box::pin(async move {
                // Keep a referenced timer alive so the timer loop runs while testing unref'd AbortSignal.timeout.
                let keep_alive = lexe_timers::set_timeout_interval(
                    &ctx,
                    Function::new(ctx.clone(), || Ok::<_, Error>(())).unwrap(),
                    500,
                    lexe_utils::provider::ProviderType::Timeout,
                    true,
                )
                .unwrap();

                let signal = AbortSignal::timeout(ctx.clone(), 5).unwrap();
                assert!(!signal.borrow().aborted());

                tokio::time::sleep(Duration::from_millis(50)).await;

                assert!(signal.borrow().aborted());
                let reason = signal.borrow().reason().unwrap();
                let reason = Class::<DOMException>::from_value(&reason).unwrap();
                assert_eq!(reason.borrow().name(), "TimeoutError");

                let clear_timeout: Function = ctx.globals().get("clearTimeout").unwrap();
                clear_timeout.call::<_, ()>((keep_alive,)).unwrap();
                let unref_only = AbortSignal::timeout(ctx.clone(), 10_000).unwrap();
                let _ = unref_only;
                let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
                let rt_key =
                    lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
                assert!(!lexe_utils::event_loop::has_refed_handles(rt_key));
                lexe_timers::discard_all_timers_on_process_exit(&ctx, rt_ptr).unwrap();
            })
        })
        .await;
    }
}
