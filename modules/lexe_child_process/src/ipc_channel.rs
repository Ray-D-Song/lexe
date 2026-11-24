// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use lexe_context::CtxExtension;
use lexe_events::Emitter;
use lexe_utils::{
    clone_wire::{deserialize, serialize, CloneWirePayload},
    fork_ipc::{self, FRAME_MESSAGE},
};
use rquickjs::{function::Opt, function::This, Class, Ctx, Function, Result, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::ChildProcess;

static CHILD_IPC_TX: std::sync::OnceLock<UnboundedSender<CloneWirePayload>> =
    std::sync::OnceLock::new();

pub fn install_child_process_ipc(ctx: &Ctx<'_>, stream: UnixStream) -> Result<()> {
    let (dispatch_tx, mut dispatch_rx) = unbounded_channel::<CloneWirePayload>();
    let (writer_tx, mut writer_rx) = unbounded_channel::<CloneWirePayload>();
    let _ = CHILD_IPC_TX.set(writer_tx);

    let (mut read_half, mut write_half) = stream.into_split();

    ctx.spawn_exit_simple(async move {
        loop {
            match fork_ipc::read_frame_async(&mut read_half).await {
                Ok((FRAME_MESSAGE, wire)) => {
                    let _ = dispatch_tx.send(wire);
                },
                Ok((fork_ipc::FRAME_EXIT, _)) | Err(_) => break,
                Ok(_) => {},
            }
        }
        Ok(())
    });

    ctx.spawn_exit_simple(async move {
        while let Some(wire) = writer_rx.recv().await {
            if fork_ipc::write_frame_async(&mut write_half, FRAME_MESSAGE, &wire)
                .await
                .is_err()
            {
                break;
            }
        }
        Ok(())
    });

    let ctx_dispatch = ctx.clone();
    ctx.spawn_exit_simple(async move {
        while let Some(wire) = dispatch_rx.recv().await {
            dispatch_process_message(&ctx_dispatch, wire)?;
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
    Ok(())
}

fn dispatch_process_message(ctx: &Ctx<'_>, wire: CloneWirePayload) -> Result<()> {
    let value = deserialize(ctx, &wire)?;
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
fn child_process_send<'js>(
    ctx: Ctx<'js>,
    message: Value<'js>,
    _transfer: Opt<Value<'js>>,
) -> Result<bool> {
    let Some(tx) = CHILD_IPC_TX.get() else {
        return Ok(false);
    };
    let wire = serialize(&ctx, message, Opt(None))?;
    Ok(tx.send(wire).is_ok())
}

pub fn attach_ipc_to_child<'js>(
    ctx: &Ctx<'js>,
    instance: Class<'js, ChildProcess<'js>>,
    stream: UnixStream,
    loop_ref: Arc<lexe_utils::event_loop::EventLoopRef>,
) -> Result<()> {
    let (outbound, mut outbound_rx) = unbounded_channel::<CloneWirePayload>();
    let (mut read_half, mut write_half) = stream.into_split();

    instance.set("connected", true)?;

    let ctx2 = ctx.clone();
    let inst = instance.clone();
    ctx.spawn_exit_simple(async move {
        loop {
            match fork_ipc::read_frame_async(&mut read_half).await {
                Ok((FRAME_MESSAGE, wire)) => {
                    emit_ipc_message(inst.clone(), &ctx2, wire)?;
                },
                Ok((fork_ipc::FRAME_EXIT, _)) | Err(_) => break,
                Ok(_) => {},
            }
        }
        loop_ref.unreference();
        Ok(())
    });

    ctx.spawn_exit_simple(async move {
        while let Some(wire) = outbound_rx.recv().await {
            if fork_ipc::write_frame_async(&mut write_half, FRAME_MESSAGE, &wire)
                .await
                .is_err()
            {
                break;
            }
        }
        Ok(())
    });

    instance.borrow_mut().ipc_outbound = Some(outbound);
    Ok(())
}

fn emit_ipc_message<'js>(
    instance: Class<'js, ChildProcess<'js>>,
    ctx: &Ctx<'js>,
    wire: CloneWirePayload,
) -> Result<()> {
    let value = deserialize(ctx, &wire)?;
    ChildProcess::emit_str(instance, ctx, "message", vec![value], false)
}

pub fn child_send<'js>(
    this: This<Class<'js, ChildProcess<'js>>>,
    ctx: Ctx<'js>,
    message: Value<'js>,
    _transfer: Opt<Value<'js>>,
) -> Result<bool> {
    let outbound = this.borrow().ipc_outbound.clone();
    let Some(tx) = outbound else {
        return Err(rquickjs::Exception::throw_type(
            &ctx,
            "IPC channel is not available",
        ));
    };
    let wire = serialize(&ctx, message, Opt(None))?;
    Ok(tx.send(wire).is_ok())
}
