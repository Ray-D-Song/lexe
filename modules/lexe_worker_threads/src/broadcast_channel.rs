// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! HTML / Node `BroadcastChannel` implementation across isolates and within a realm.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, OnceLock, RwLock,
};

use lexe_events::{Emitter, EventEmitter, EventList, MessageEvent};
use lexe_exceptions::{DOMException, DOMExceptionName};
use lexe_utils::clone_wire::{deserialize, serialize, CloneWirePayload};
use lexe_utils::event_loop::{self, wait_until_io_unref, EventLoopRef};
use rquickjs::{
    atom::PredefinedAtom,
    class::{Trace, Tracer},
    function::Opt,
    prelude::This,
    qjs, Class, Coerced, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime, Null, Object,
    Result, Value,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

struct ChannelEntry {
    name: String,
    tx: UnboundedSender<CloneWirePayload>,
}

static NEXT_CHANNEL_ID: AtomicU64 = AtomicU64::new(1);
static BROADCAST_HUB: OnceLock<Mutex<HashMap<u64, ChannelEntry>>> = OnceLock::new();

fn hub() -> &'static Mutex<HashMap<u64, ChannelEntry>> {
    BROADCAST_HUB.get_or_init(|| Mutex::new(HashMap::new()))
}

#[rquickjs::class]
#[derive(Clone)]
pub struct BroadcastChannel<'js> {
    id: u64,
    name: String,
    closed: Arc<AtomicBool>,
    emitter: EventEmitter<'js>,
    loop_ref: Arc<EventLoopRef>,
    on_message: Option<Function<'js>>,
    on_message_wrapper: Option<Function<'js>>,
    on_message_error: Option<Function<'js>>,
    on_message_error_wrapper: Option<Function<'js>>,
}

unsafe impl<'js> JsLifetime<'js> for BroadcastChannel<'js> {
    type Changed<'to> = BroadcastChannel<'to>;
}

impl<'js> Trace<'js> for BroadcastChannel<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
        if let Some(ref f) = self.on_message {
            tracer.mark(f);
        }
        if let Some(ref f) = self.on_message_wrapper {
            tracer.mark(f);
        }
        if let Some(ref f) = self.on_message_error {
            tracer.mark(f);
        }
        if let Some(ref f) = self.on_message_error_wrapper {
            tracer.mark(f);
        }
    }
}

impl<'js> Emitter<'js> for BroadcastChannel<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

impl<'js> BroadcastChannel<'js> {
    fn dispatch_wire(ctx: &Ctx<'js>, this: &Class<'js, Self>, wire: CloneWirePayload) {
        if this.borrow().closed.load(Ordering::Relaxed) {
            return;
        }
        let (ev_type, payload_val) = match deserialize(ctx, &wire) {
            Ok(val) => ("message", val),
            Err(_) => ("messageerror", Null.into_value(ctx.clone())),
        };
        let Ok(ev) = MessageEvent::with_data(ctx, ev_type, payload_val) else {
            return;
        };
        let Ok(ev_cls) = Class::instance(ctx.clone(), ev) else {
            return;
        };
        if let Some(this_obj) = this.as_object() {
            if let Ok(dispatch) = this_obj.get::<_, Function>("dispatchEvent") {
                let _: Result<Value> = dispatch.call((This(this.clone().into_value()), ev_cls));
            }
        }
    }

    pub fn install_prototype(ctx: &Ctx<'js>, globals: &Object<'js>) -> Result<()> {
        Self::add_event_target_prototype(ctx)?;

        if let Some(proto) = Class::<Self>::prototype(ctx)? {
            if let Some(et_proto) = globals
                .get::<_, Option<Object>>("EventTarget")?
                .and_then(|ctor| ctor.get::<_, Option<Object>>("prototype").ok().flatten())
            {
                let _ = proto.set_prototype(Some(&et_proto));
            }
        }
        if let Some(ctor) = globals.get::<_, Option<Object>>("BroadcastChannel")? {
            if let Some(et_ctor) = globals.get::<_, Option<Object>>("EventTarget")? {
                let _ = ctor.set_prototype(Some(&et_ctor));
            }
        }
        Ok(())
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> BroadcastChannel<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, name: Opt<Value<'js>>) -> Result<Class<'js, Self>> {
        let Some(name_val) = name.0 else {
            return Err(Exception::throw_type(
                &ctx,
                "Failed to construct 'BroadcastChannel': 1 argument required, but only 0 present.",
            ));
        };
        let name = Coerced::<String>::from_js(&ctx, name_val)?.0;
        let id = NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed);
        let closed = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = unbounded_channel::<CloneWirePayload>();

        {
            let mut map = hub().lock().unwrap();
            map.insert(
                id,
                ChannelEntry {
                    name: name.clone(),
                    tx,
                },
            );
        }

        let rt_ptr = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let loop_ref = EventLoopRef::new_referenced(event_loop::rt_key_from_ptr(rt_ptr as *mut _));

        let instance = Class::instance(
            ctx.clone(),
            Self {
                id,
                name,
                closed: Arc::clone(&closed),
                emitter: EventEmitter::new(),
                loop_ref: Arc::clone(&loop_ref),
                on_message: None,
                on_message_wrapper: None,
                on_message_error: None,
                on_message_error_wrapper: None,
            },
        )?;

        let weak_inst = instance.clone();
        let spawn_ctx = ctx.clone();
        ctx.spawn(async move {
            loop {
                tokio::select! {
                    maybe = rx.recv() => {
                        let Some(wire) = maybe else {
                            break;
                        };
                        Self::dispatch_wire(&spawn_ctx, &weak_inst, wire);
                    }
                    _ = wait_until_io_unref(Arc::clone(&loop_ref)) => {
                        while let Ok(wire) = rx.try_recv() {
                            Self::dispatch_wire(&spawn_ctx, &weak_inst, wire);
                        }
                        break;
                    }
                }
            }
            hub().lock().unwrap().remove(&id);
            loop_ref.unreference();
        });

        Ok(instance)
    }

    #[qjs(get)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    #[qjs(get, rename = "onmessage")]
    pub fn get_onmessage(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match &self.on_message {
            Some(f) => Ok(f.clone().into_value()),
            None => Null.into_js(&ctx),
        }
    }

    #[qjs(set, rename = "onmessage")]
    pub fn set_onmessage(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        val: Value<'js>,
    ) -> Result<()> {
        if let Some(func) = val.into_function() {
            let need_register = {
                let mut borrow = this.0.borrow_mut();
                borrow.on_message = Some(func);
                borrow.on_message_wrapper.is_none()
            };
            if need_register {
                let wrapper = Function::new(
                    ctx.clone(),
                    move |_ctx: Ctx<'js>,
                          ev_this: This<Value<'js>>,
                          ev: Value<'js>|
                          -> Result<()> {
                        if let Ok(ch) = Class::<BroadcastChannel>::from_value(&ev_this.0) {
                            let handler = ch.borrow().on_message.clone();
                            if let Some(h) = handler {
                                let _: Value = h.call((This(ev_this.0), ev))?;
                            }
                        }
                        Ok(())
                    },
                )?;
                this.0.borrow_mut().on_message_wrapper = Some(wrapper.clone());
                Self::add_event_listener_str(this.0, &ctx, "message", wrapper, false, false)?;
            }
        } else {
            let old = {
                let mut borrow = this.0.borrow_mut();
                borrow.on_message = None;
                borrow.on_message_wrapper.take()
            };
            if let Some(w) = old {
                Self::remove_event_listener_str(this.0, &ctx, "message", w)?;
            }
        }
        Ok(())
    }

    #[qjs(get, rename = "onmessageerror")]
    pub fn get_onmessageerror(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match &self.on_message_error {
            Some(f) => Ok(f.clone().into_value()),
            None => Null.into_js(&ctx),
        }
    }

    #[qjs(set, rename = "onmessageerror")]
    pub fn set_onmessageerror(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        val: Value<'js>,
    ) -> Result<()> {
        if let Some(func) = val.into_function() {
            let need_register = {
                let mut borrow = this.0.borrow_mut();
                borrow.on_message_error = Some(func);
                borrow.on_message_error_wrapper.is_none()
            };
            if need_register {
                let wrapper = Function::new(
                    ctx.clone(),
                    move |_ctx: Ctx<'js>,
                          ev_this: This<Value<'js>>,
                          ev: Value<'js>|
                          -> Result<()> {
                        if let Ok(ch) = Class::<BroadcastChannel>::from_value(&ev_this.0) {
                            let handler = ch.borrow().on_message_error.clone();
                            if let Some(h) = handler {
                                let _: Value = h.call((This(ev_this.0), ev))?;
                            }
                        }
                        Ok(())
                    },
                )?;
                this.0.borrow_mut().on_message_error_wrapper = Some(wrapper.clone());
                Self::add_event_listener_str(this.0, &ctx, "messageerror", wrapper, false, false)?;
            }
        } else {
            let old = {
                let mut borrow = this.0.borrow_mut();
                borrow.on_message_error = None;
                borrow.on_message_error_wrapper.take()
            };
            if let Some(w) = old {
                Self::remove_event_listener_str(this.0, &ctx, "messageerror", w)?;
            }
        }
        Ok(())
    }

    pub fn post_message(&self, ctx: Ctx<'js>, message: Opt<Value<'js>>) -> Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            let err = DOMException::new_with_name(
                &ctx,
                DOMExceptionName::InvalidStateError,
                "BroadcastChannel is closed".to_string(),
            )?;
            return Err(ctx.throw(err.into_js(&ctx)?));
        }
        let msg = message
            .0
            .unwrap_or_else(|| rquickjs::Undefined.into_value(ctx.clone()));
        let wire = serialize(&ctx, msg, Opt(None))?;
        let targets: Vec<UnboundedSender<CloneWirePayload>> = {
            let map = hub().lock().unwrap();
            map.iter()
                .filter(|(&cid, entry)| cid != self.id && entry.name == self.name)
                .map(|(_, entry)| entry.tx.clone())
                .collect()
        };
        for tx in targets {
            let _ = tx.send(wire.clone());
        }
        Ok(())
    }

    pub fn close(&self) {
        if self.closed.swap(true, Ordering::Relaxed) {
            return;
        }
        hub().lock().unwrap().remove(&self.id);
        self.loop_ref.unreference();
    }

    #[qjs(rename = "ref")]
    pub fn js_ref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        if !this.0.borrow().closed.load(Ordering::Relaxed) {
            this.0.borrow().loop_ref.reference();
        }
        this.0
    }

    pub fn unref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.0.borrow().loop_ref.unreference();
        this.0
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(BroadcastChannel)
    }
}
