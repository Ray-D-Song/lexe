// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Cross-isolate `MessagePort` endpoint hub and structured-clone codecs.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};

use lexe_utils::clone_wire::{
    deserialize, register_native_codec, register_native_transfer_codec, serialize,
    CloneWirePayload, NativeCloneCodec, NativeTransferCodec,
};
use lexe_utils::event_loop::{self, wait_until_io_unref, EventLoopRef};
use rquickjs::{
    function::Opt, prelude::This, qjs, Array, Ctx, Exception, Function, Object, Result, Symbol,
    Value,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

struct PortEndpoint {
    peer: Option<u64>,
    closed: bool,
    pending: VecDeque<CloneWirePayload>,
    sink: Option<UnboundedSender<CloneWirePayload>>,
}

static NEXT_PORT_ID: AtomicU64 = AtomicU64::new(1);
static PORT_HUB: OnceLock<Mutex<HashMap<u64, PortEndpoint>>> = OnceLock::new();

fn hub() -> &'static Mutex<HashMap<u64, PortEndpoint>> {
    PORT_HUB.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
pub struct TransferredPortHandle {
    pub port_id: u64,
}

pub fn create_pair(ctx: Ctx<'_>) -> Result<Array<'_>> {
    let id1 = NEXT_PORT_ID.fetch_add(1, Ordering::Relaxed);
    let id2 = NEXT_PORT_ID.fetch_add(1, Ordering::Relaxed);
    {
        let mut map = hub().lock().unwrap();
        map.insert(
            id1,
            PortEndpoint {
                peer: Some(id2),
                closed: false,
                pending: VecDeque::new(),
                sink: None,
            },
        );
        map.insert(
            id2,
            PortEndpoint {
                peer: Some(id1),
                closed: false,
                pending: VecDeque::new(),
                sink: None,
            },
        );
    }
    let arr = Array::new(ctx)?;
    arr.set(0, id1 as f64)?;
    arr.set(1, id2 as f64)?;
    Ok(arr)
}

pub fn peer_id(port_id: u64) -> Option<u64> {
    let map = hub().lock().unwrap();
    map.get(&port_id).and_then(|e| e.peer)
}

pub fn close_port(port_id: u64) {
    let mut map = hub().lock().unwrap();
    if let Some(ep) = map.get_mut(&port_id) {
        ep.closed = true;
        ep.sink = None;
        ep.pending.clear();
    }
}

/// Detach an isolate sink without closing the underlying endpoint (for transfer).
pub fn detach_sink(port_id: u64) {
    let mut map = hub().lock().unwrap();
    if let Some(ep) = map.get_mut(&port_id) {
        ep.sink = None;
    }
}

pub fn detach_sink_js(id_f: f64) {
    detach_sink(id_f as u64);
}

pub fn send_to_peer_js<'js>(
    ctx: Ctx<'js>,
    id_f: f64,
    data: Value<'js>,
    transfer: Opt<Value<'js>>,
) -> Result<()> {
    send_to_peer(ctx, id_f as u64, data, transfer)
}

pub fn send_to_peer<'js>(
    ctx: Ctx<'js>,
    port_id: u64,
    data: Value<'js>,
    transfer: Opt<Value<'js>>,
) -> Result<()> {
    let transfer_obj = match transfer.0 {
        Some(v) if !v.is_undefined() && !v.is_null() => Opt(v.into_object()),
        _ => Opt(None),
    };
    let payload = serialize(&ctx, data, transfer_obj)?;
    let mut map = hub().lock().unwrap();
    let Some(peer) = map.get(&port_id).and_then(|e| e.peer) else {
        return Ok(());
    };
    let Some(target) = map.get_mut(&peer) else {
        return Ok(());
    };
    if target.closed {
        return Ok(());
    }
    if let Some(ref tx) = target.sink {
        match tx.send(payload) {
            Ok(()) => return Ok(()),
            Err(send_err) => {
                target.pending.push_back(send_err.0);
                return Ok(());
            },
        }
    }
    target.pending.push_back(payload);
    Ok(())
}

/// Attach a pump loop for `port_id` on the current isolate if not already attached.
pub fn attach_port_js<'js>(ctx: Ctx<'js>, id_f: f64, on_message: Function<'js>) -> Result<()> {
    attach_port(&ctx, id_f as u64, on_message)
}

pub fn attach_port<'js>(ctx: &Ctx<'js>, port_id: u64, on_message: Function<'js>) -> Result<()> {
    let (tx, mut rx) = unbounded_channel::<CloneWirePayload>();
    let pending: Vec<CloneWirePayload> = {
        let mut map = hub().lock().unwrap();
        let Some(ep) = map.get_mut(&port_id) else {
            return Ok(());
        };
        if ep.closed {
            return Ok(());
        }
        if ep.sink.is_some() {
            return Ok(());
        }
        ep.sink = Some(tx);
        ep.pending.drain(..).collect()
    };

    let rt_ptr = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let loop_ref = EventLoopRef::new_unreferenced(event_loop::rt_key_from_ptr(rt_ptr as *mut _));
    let ctx_clone = ctx.clone();

    ctx.spawn(async move {
        for wire in pending {
            if let Ok(val) = deserialize(&ctx_clone, &wire) {
                let _ = on_message.call::<_, ()>((val,));
            }
        }
        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    let Some(wire) = maybe else {
                        break;
                    };
                    if let Ok(val) = deserialize(&ctx_clone, &wire) {
                        let _ = on_message.call::<_, ()>((val,));
                    }
                }
                _ = wait_until_io_unref(Arc::clone(&loop_ref)) => {
                    while let Ok(wire) = rx.try_recv() {
                        if let Ok(val) = deserialize(&ctx_clone, &wire) {
                            let _ = on_message.call::<_, ()>((val,));
                        }
                    }
                    break;
                }
            }
        }
        detach_sink(port_id);
    });

    Ok(())
}

fn port_id_symbol<'js>(ctx: &Ctx<'js>) -> Result<Symbol<'js>> {
    Symbol::new_global(ctx.clone(), "lexe.internal.messagePortId")
}

fn port_transfer_symbol<'js>(ctx: &Ctx<'js>) -> Result<Symbol<'js>> {
    Symbol::new_global(ctx.clone(), "lexe.internal.messagePortTransfer")
}

fn ensure_peer_attached_js<'js>(ctx: &Ctx<'js>) -> Result<Option<Function<'js>>> {
    ctx.globals().get("__lexePortEnsureLocalAttached")
}

fn build_port_in_ctx<'js>(ctx: &Ctx<'js>, port_id: u64) -> Result<Value<'js>> {
    if !ctx.globals().contains_key("__lexePortRebuildFromId")? {
        let _: Value = ctx.eval("require('node:worker_threads')")?;
    }
    let factory: Function<'js> = ctx.globals().get("__lexePortRebuildFromId")?;
    factory.call((port_id as f64,))
}

fn extract_port_handle<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    for_wire: bool,
) -> Result<Option<Arc<dyn Any + Send + Sync>>> {
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };
    let sym = port_id_symbol(ctx)?;
    let Ok(Some(id_f)) = obj.get::<_, Option<f64>>(sym) else {
        return Ok(None);
    };
    let port_id = id_f as u64;
    if !for_wire {
        return Ok(Some(Arc::new(TransferredPortHandle { port_id })));
    }
    // Detach the port in this isolate so the transferred port owns the endpoint.
    let transfer_sym = port_transfer_symbol(ctx)?;
    if let Ok(Some(detach_fn)) = obj.get::<_, Option<Function>>(transfer_sym) {
        let _: Value = detach_fn.call((This(obj.clone()),))?;
    }
    detach_sink(port_id);
    if let Some(peer) = peer_id(port_id) {
        if let Some(ensure_fn) = ensure_peer_attached_js(ctx)? {
            let _: Value = ensure_fn.call((peer as f64,))?;
        }
    }
    Ok(Some(Arc::new(TransferredPortHandle { port_id })))
}

fn rebuild_port_value<'js>(
    ctx: &Ctx<'js>,
    handle: &Arc<dyn Any + Send + Sync>,
) -> Result<Option<Value<'js>>> {
    let Some(port) = handle.downcast_ref::<TransferredPortHandle>() else {
        return Ok(None);
    };
    Ok(Some(build_port_in_ctx(ctx, port.port_id)?))
}

fn transfer_port<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<Option<Value<'js>>> {
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };
    let sym = port_id_symbol(ctx)?;
    let Ok(Some(id_f)) = obj.get::<_, Option<f64>>(sym) else {
        return Ok(None);
    };
    let port_id = id_f as u64;
    let transfer_sym = port_transfer_symbol(ctx)?;
    let detach_fn: Function = obj
        .get(transfer_sym)
        .map_err(|_| Exception::throw_type(ctx, "MessagePort has already been detached"))?;
    let already_detached: bool = detach_fn.call((This(obj.clone()),))?;
    if already_detached {
        let dom_ctor: rquickjs::function::Constructor = ctx.globals().get("DOMException")?;
        let err: Object =
            dom_ctor.construct(("MessagePort has already been detached", "DataCloneError"))?;
        return Err(ctx.throw(err.into_value()));
    }
    detach_sink(port_id);
    Ok(Some(build_port_in_ctx(ctx, port_id)?))
}

pub fn register_codecs() {
    register_native_codec(NativeCloneCodec {
        name: "MessagePort",
        transfer_only: true,
        try_extract: extract_port_handle,
        try_rebuild: rebuild_port_value,
    });
    register_native_transfer_codec(NativeTransferCodec {
        name: "MessagePort",
        try_transfer: transfer_port,
    });
}
