// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};

use lexe_context::CtxExtension;
use lexe_events::Emitter;
use lexe_utils::{
    clone_wire::{deserialize, serialize, CloneWirePayload},
    fork_ipc::{self, FRAME_MESSAGE},
};
use rquickjs::{
    function::{Opt, Rest, This},
    Class, Ctx, Exception, Function, Null, Result, Value,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::options::{throw_coded, SerializationMode};
use crate::ChildProcess;

#[derive(Debug)]
pub enum IpcPacket {
    Clone(CloneWirePayload),
    Json(String),
}

static CHILD_IPC_TX: OnceLock<Mutex<Option<UnboundedSender<IpcPacket>>>> = OnceLock::new();
static CHILD_SERIALIZATION: OnceLock<SerializationMode> = OnceLock::new();
static CHILD_CONNECTED: AtomicBool = AtomicBool::new(false);

fn current_child_serialization() -> SerializationMode {
    *CHILD_SERIALIZATION.get_or_init(|| {
        match std::env::var("LEXE_FORK_SERIALIZATION").ok().as_deref() {
            Some("json") => SerializationMode::Json,
            _ => SerializationMode::Advanced,
        }
    })
}

pub fn install_child_process_ipc(ctx: &Ctx<'_>, stream: UnixStream) -> Result<()> {
    let mode = current_child_serialization();
    let (dispatch_tx, mut dispatch_rx) = unbounded_channel::<IpcPacket>();
    let (writer_tx, mut writer_rx) = unbounded_channel::<IpcPacket>();
    let tx_slot = CHILD_IPC_TX.get_or_init(|| Mutex::new(None));
    *tx_slot.lock().unwrap() = Some(writer_tx);
    CHILD_CONNECTED.store(true, Ordering::SeqCst);

    let (read_half, mut write_half) = stream.into_split();

    let ctx_disc = ctx.clone();
    ctx.spawn_exit_simple(async move {
        match mode {
            SerializationMode::Advanced => {
                let mut reader = read_half;
                loop {
                    match fork_ipc::read_frame_async(&mut reader).await {
                        Ok((FRAME_MESSAGE, wire)) => {
                            let _ = dispatch_tx.send(IpcPacket::Clone(wire));
                        },
                        Ok((fork_ipc::FRAME_EXIT, _)) | Err(_) => break,
                        Ok(_) => {},
                    }
                }
            },
            SerializationMode::Json => {
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_string();
                            if !trimmed.is_empty() {
                                let _ = dispatch_tx.send(IpcPacket::Json(trimmed));
                            }
                        },
                    }
                }
            },
        }
        let _ = emit_process_disconnect(&ctx_disc);
        Ok(())
    });

    ctx.spawn_exit_simple(async move {
        while let Some(pkt) = writer_rx.recv().await {
            let res = match pkt {
                IpcPacket::Clone(wire) => {
                    fork_ipc::write_frame_async(&mut write_half, FRAME_MESSAGE, &wire).await
                },
                IpcPacket::Json(line) => {
                    let mut buf = line.into_bytes();
                    buf.push(b'\n');
                    let r = write_half.write_all(&buf).await;
                    if r.is_ok() {
                        write_half.flush().await
                    } else {
                        r
                    }
                },
            };
            if res.is_err() {
                break;
            }
        }
        let _ = write_half.shutdown().await;
        Ok(())
    });

    let ctx_dispatch = ctx.clone();
    ctx.spawn_exit_simple(async move {
        while let Some(pkt) = dispatch_rx.recv().await {
            dispatch_process_message(&ctx_dispatch, pkt)?;
        }
        Ok(())
    });

    let globals = ctx.globals();
    // The fork child installs IPC before any module runs, so the `process`
    // global may not exist yet; creating it here keeps `get::<Object>` from
    // failing with "converting from js 'undefined' into type 'object'".
    let process: rquickjs::Object = match globals.get::<_, Value>("process")? {
        v if v.is_object() => v.as_object().cloned().expect("checked object"),
        _ => {
            let obj = rquickjs::Object::new(ctx.clone())?;
            globals.set("process", obj.clone())?;
            obj
        },
    };
    process.set("connected", true)?;
    process.set(
        "send",
        Function::new(ctx.clone(), child_process_send)?.with_name("send")?,
    )?;
    process.set(
        "disconnect",
        Function::new(ctx.clone(), child_process_disconnect)?.with_name("disconnect")?,
    )?;
    Ok(())
}

fn emit_process_disconnect(ctx: &Ctx<'_>) -> Result<()> {
    if !CHILD_CONNECTED.swap(false, Ordering::SeqCst) {
        return Ok(());
    }
    if let Ok(process) = ctx.globals().get::<_, rquickjs::Object>("process") {
        process.set("connected", false)?;
        if let Ok(emit) = process.get::<_, Function>("emit") {
            let mut args = rquickjs::function::Args::new(ctx.clone(), 1);
            args.this(process)?;
            args.push_arg("disconnect")?;
            let _ = emit.call_arg::<()>(args);
        }
    }
    Ok(())
}

fn dispatch_process_message(ctx: &Ctx<'_>, pkt: IpcPacket) -> Result<()> {
    let value = decode_packet(ctx, pkt)?;
    let process: rquickjs::Object = ctx.globals().get("process")?;
    let emit: Function = process.get("emit")?;
    // `Function::call` leaves `this` undefined, but the EventEmitter method
    // reads its listener registry from `this`.
    let mut args = rquickjs::function::Args::new(ctx.clone(), 2);
    args.this(process)?;
    args.push_arg("message")?;
    args.push_arg(value)?;
    emit.call_arg::<()>(args)?;
    Ok(())
}

fn encode_packet<'js>(
    ctx: &Ctx<'js>,
    mode: SerializationMode,
    message: Value<'js>,
) -> Result<IpcPacket> {
    match mode {
        SerializationMode::Advanced => {
            let wire = serialize(ctx, message, Opt(None))?;
            Ok(IpcPacket::Clone(wire))
        },
        SerializationMode::Json => {
            if message.is_undefined() {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_MISSING_ARGS",
                    "The \"message\" argument must be specified",
                ));
            }
            let Some(json_str) = ctx.json_stringify(message)? else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "Message cannot be serialized to JSON",
                ));
            };
            Ok(IpcPacket::Json(json_str.to_string()?))
        },
    }
}

fn decode_packet<'js>(ctx: &Ctx<'js>, pkt: IpcPacket) -> Result<Value<'js>> {
    match pkt {
        IpcPacket::Clone(wire) => deserialize(ctx, &wire),
        IpcPacket::Json(text) => ctx.json_parse(text.as_str()),
    }
}

fn child_process_send<'js>(
    ctx: Ctx<'js>,
    message: Value<'js>,
    rest: Rest<Value<'js>>,
) -> Result<bool> {
    let cb = rest.0.into_iter().rev().find_map(|v| v.into_function());
    let tx_opt = CHILD_IPC_TX
        .get()
        .and_then(|m| m.lock().unwrap().as_ref().cloned());
    let Some(tx) = tx_opt else {
        if let Some(cb) = cb {
            let ex = Exception::from_message(ctx.clone(), "IPC channel is closed")?;
            let _ = cb.call::<_, ()>((ex,));
        }
        return Ok(false);
    };
    let mode = current_child_serialization();
    let pkt = encode_packet(&ctx, mode, message)?;
    let ok = tx.send(pkt).is_ok();
    if let Some(cb) = cb {
        if ok {
            let _ = cb.call::<_, ()>((Null.into_value(ctx),));
        } else {
            let ex = Exception::from_message(ctx.clone(), "IPC channel is closed")?;
            let _ = cb.call::<_, ()>((ex,));
        }
    }
    Ok(ok)
}

fn child_process_disconnect<'js>(ctx: Ctx<'js>) -> Result<()> {
    if let Some(slot) = CHILD_IPC_TX.get() {
        slot.lock().unwrap().take();
    }
    emit_process_disconnect(&ctx)
}

pub fn attach_ipc_to_child<'js>(
    ctx: &Ctx<'js>,
    instance: Class<'js, ChildProcess<'js>>,
    stream: UnixStream,
    ipc_loop_ref: Arc<lexe_utils::event_loop::EventLoopRef>,
    serialization: SerializationMode,
) -> Result<()> {
    let (outbound, mut outbound_rx) = unbounded_channel::<IpcPacket>();
    let (read_half, mut write_half) = stream.into_split();

    {
        let mut borrow = instance.borrow_mut();
        borrow.connected = true;
        borrow.serialization = serialization;
        borrow.ipc_outbound = Some(outbound);
        borrow.ipc_loop_ref = Some(ipc_loop_ref.clone());
    }

    let ctx2 = ctx.clone();
    let inst = instance.clone();
    ctx.spawn_exit_simple(async move {
        match serialization {
            SerializationMode::Advanced => {
                let mut reader = read_half;
                loop {
                    match fork_ipc::read_frame_async(&mut reader).await {
                        Ok((FRAME_MESSAGE, wire)) => {
                            emit_ipc_message(inst.clone(), &ctx2, IpcPacket::Clone(wire))?;
                        },
                        Ok((fork_ipc::FRAME_EXIT, _)) | Err(_) => break,
                        Ok(_) => {},
                    }
                }
            },
            SerializationMode::Json => {
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_string();
                            if !trimmed.is_empty() {
                                emit_ipc_message(inst.clone(), &ctx2, IpcPacket::Json(trimmed))?;
                            }
                        },
                    }
                }
            },
        }
        ipc_loop_ref.unreference();
        let was_connected = {
            let mut b = inst.borrow_mut();
            let prev = b.connected;
            b.connected = false;
            b.ipc_outbound = None;
            prev
        };
        if was_connected {
            let _ = ChildProcess::emit_str(inst, &ctx2, "disconnect", vec![], false);
        }
        Ok(())
    });

    ctx.spawn_exit_simple(async move {
        while let Some(pkt) = outbound_rx.recv().await {
            let res = match pkt {
                IpcPacket::Clone(wire) => {
                    fork_ipc::write_frame_async(&mut write_half, FRAME_MESSAGE, &wire).await
                },
                IpcPacket::Json(line) => {
                    let mut buf = line.into_bytes();
                    buf.push(b'\n');
                    let r = write_half.write_all(&buf).await;
                    if r.is_ok() {
                        write_half.flush().await
                    } else {
                        r
                    }
                },
            };
            if res.is_err() {
                break;
            }
        }
        let _ = write_half.shutdown().await;
        Ok(())
    });

    Ok(())
}

fn emit_ipc_message<'js>(
    instance: Class<'js, ChildProcess<'js>>,
    ctx: &Ctx<'js>,
    pkt: IpcPacket,
) -> Result<()> {
    let value = decode_packet(ctx, pkt)?;
    ChildProcess::emit_str(instance, ctx, "message", vec![value], false)
}

pub fn child_send<'js>(
    this: This<Class<'js, ChildProcess<'js>>>,
    ctx: Ctx<'js>,
    message: Value<'js>,
    rest: Rest<Value<'js>>,
) -> Result<bool> {
    let cb = rest.0.into_iter().rev().find_map(|v| v.into_function());
    let (outbound, mode, connected) = {
        let borrow = this.borrow();
        (
            borrow.ipc_outbound.clone(),
            borrow.serialization,
            borrow.connected,
        )
    };
    let Some(tx) = outbound.filter(|_| connected) else {
        let err = throw_coded(&ctx, false, "ERR_IPC_CHANNEL_CLOSED", "Channel closed");
        if let Some(cb) = cb {
            let caught = ctx.catch();
            let _ = cb.call::<_, ()>((caught,));
            return Ok(false);
        }
        return Err(err);
    };
    let pkt = encode_packet(&ctx, mode, message)?;
    let ok = tx.send(pkt).is_ok();
    if let Some(cb) = cb {
        if ok {
            let _ = cb.call::<_, ()>((Null.into_value(ctx),));
        } else {
            let ex = Exception::from_message(ctx.clone(), "Channel closed")?;
            let _ = cb.call::<_, ()>((ex,));
        }
    }
    Ok(ok)
}

pub fn child_disconnect<'js>(
    this: This<Class<'js, ChildProcess<'js>>>,
    ctx: Ctx<'js>,
) -> Result<()> {
    let was_connected = {
        let mut borrow = this.borrow_mut();
        let prev = borrow.connected;
        borrow.connected = false;
        borrow.ipc_outbound = None;
        if let Some(ipc_ref) = &borrow.ipc_loop_ref {
            ipc_ref.unreference();
        }
        prev
    };
    if was_connected {
        ChildProcess::emit_str(this.0, &ctx, "disconnect", vec![], false)?;
    }
    Ok(())
}
