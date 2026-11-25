// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::uninlined_format_args)]

#[cfg(unix)]
mod fork;
#[cfg(unix)]
mod ipc_channel;
mod options;
mod sync;

#[cfg(unix)]
pub use fork::fork;
#[cfg(unix)]
pub use ipc_channel::install_child_process_ipc;

#[cfg(unix)]
use std::os::{
    fd::IntoRawFd,
    unix::net::UnixStream as StdUnixStream,
    unix::process::{CommandExt, ExitStatusExt},
};
use std::{
    process::Stdio,
    sync::{Arc, RwLock},
    time::Duration,
};

use lexe_context::CtxExtension;
use lexe_events::{EmitError, Emitter, EventEmitter, EventKey, EventList};
use lexe_stream::{
    impl_stream_events,
    readable::{DefaultReadableStream, ReadableStream, ReadableStreamInner},
    writable::{DefaultWritableStream, WritableStream, WritableStreamInner},
};
use lexe_utils::{
    event_loop::{keeps_event_loop_alive, should_keep_process_alive, EventLoopRef},
    module::{export_default, ModuleInfo},
    result::ResultExt,
};
use rquickjs::{
    class::{Trace, Tracer},
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, Opt, Rest, This},
    Array, Class, Ctx, Error, Function, IntoJs, JsLifetime, Null, Result, Value,
};
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::{
    io::AsyncRead,
    process::{Child, Command},
    sync::{
        broadcast::{channel as broadcast_channel, Receiver, Sender},
        oneshot::Receiver as OneshotReceiver,
    },
};

use crate::options::{
    attach_abort_error_helper, build_command, dup_fd_stdio, make_abort_error,
    parse_cmd_args_and_opts, SerializationMode, StdioSpec,
};
#[cfg(not(unix))]
use lexe_utils::signals::kill_process_raw;
use lexe_utils::signals::parse_signal;
#[cfg(unix)]
use lexe_utils::signals::{kill, kill_process_raw, signal_str_from_i32};

#[cfg(unix)]
#[rquickjs::class]
pub struct ChildPipe<'js> {
    emitter: EventEmitter<'js>,
    readable: ReadableStreamInner<'js>,
    writable: WritableStreamInner<'js>,
}

#[cfg(unix)]
impl<'js> Trace<'js> for ChildPipe<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
        self.readable.trace(tracer);
        self.writable.trace(tracer);
    }
}

#[cfg(unix)]
unsafe impl<'js> JsLifetime<'js> for ChildPipe<'js> {
    type Changed<'to> = ChildPipe<'to>;
}

#[cfg(unix)]
impl_stream_events!(ChildPipe);

#[cfg(unix)]
impl<'js> Emitter<'js> for ChildPipe<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }

    fn on_event_changed(&mut self, event: EventKey<'js>, added: bool) -> Result<()> {
        self.readable.on_event_changed(event, added)
    }
}

#[cfg(unix)]
impl<'js> ReadableStream<'js> for ChildPipe<'js> {
    fn inner_mut(&mut self) -> &mut ReadableStreamInner<'js> {
        &mut self.readable
    }

    fn inner(&self) -> &ReadableStreamInner<'js> {
        &self.readable
    }
}

#[cfg(unix)]
impl<'js> WritableStream<'js> for ChildPipe<'js> {
    fn inner_mut(&mut self) -> &mut WritableStreamInner<'js> {
        &mut self.writable
    }

    fn inner(&self) -> &WritableStreamInner<'js> {
        &self.writable
    }
}

#[cfg(unix)]
#[rquickjs::methods]
impl<'js> ChildPipe<'js> {
    pub fn destroy(this: This<Class<'js, Self>>, error: Opt<Value<'js>>) -> Class<'js, Self> {
        ReadableStream::destroy(This(this.clone()), Opt(error.0.clone()));
        WritableStream::destroy(This(this.clone()), error);
        this.0
    }
}

#[cfg(unix)]
impl<'js> ChildPipe<'js> {
    fn from_unix_stream(
        ctx: &Ctx<'js>,
        stream: UnixStream,
    ) -> Result<(Class<'js, Self>, OneshotReceiver<bool>)> {
        let emitter = EventEmitter::new();
        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter: emitter.clone(),
                readable: ReadableStreamInner::new(emitter.clone(), false),
                writable: WritableStreamInner::new(emitter, false),
            },
        )?;
        let (reader, writer) = stream.into_split();
        let read_rx = ReadableStream::process(instance.clone(), ctx, reader)?;
        let _write_rx = WritableStream::process(instance.clone(), ctx, writer)?;
        Ok((instance, read_rx))
    }
}

#[allow(dead_code)]
#[rquickjs::class]
#[derive(rquickjs::JsLifetime)]
pub struct ChildProcess<'js> {
    emitter: EventEmitter<'js>,
    spawnfile: String,
    spawnargs: Vec<String>,
    args: Option<Vec<String>>,
    command: String,
    kill_tx: Option<Sender<i32>>,
    pid: Option<u32>,
    killed: bool,
    exit_code: Option<i32>,
    signal_code: Option<String>,
    connected: bool,
    serialization: SerializationMode,
    loop_ref: Arc<EventLoopRef>,
    ipc_loop_ref: Option<Arc<EventLoopRef>>,
    #[cfg(unix)]
    ipc_outbound: Option<tokio::sync::mpsc::UnboundedSender<ipc_channel::IpcPacket>>,
}

impl<'js> Trace<'js> for ChildProcess<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

#[rquickjs::methods]
impl<'js> ChildProcess<'js> {
    #[qjs(constructor)]
    pub fn js_ctor(ctx: Ctx<'js>) -> Result<Class<'js, Self>> {
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        Class::instance(
            ctx,
            Self {
                emitter: EventEmitter::new(),
                spawnfile: String::new(),
                spawnargs: Vec::new(),
                args: None,
                command: String::new(),
                kill_tx: None,
                pid: None,
                killed: false,
                exit_code: None,
                signal_code: None,
                connected: false,
                serialization: SerializationMode::Advanced,
                loop_ref: EventLoopRef::new_unreferenced(rt_key),
                ipc_loop_ref: None,
                #[cfg(unix)]
                ipc_outbound: None,
            },
        )
    }

    #[qjs(get, enumerable)]
    fn pid(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.pid.into_js(&ctx)
    }

    #[qjs(get, enumerable)]
    fn spawnfile(&self) -> String {
        self.spawnfile.clone()
    }

    #[qjs(get, enumerable)]
    fn spawnargs(&self) -> Vec<String> {
        self.spawnargs.clone()
    }

    #[qjs(get, enumerable)]
    fn killed(&self) -> bool {
        self.killed
    }

    #[qjs(get, enumerable, rename = "exitCode")]
    fn exit_code(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match self.exit_code {
            Some(c) => c.into_js(&ctx),
            None => Ok(Null.into_value(ctx)),
        }
    }

    #[qjs(get, enumerable, rename = "signalCode")]
    fn signal_code(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match &self.signal_code {
            Some(s) => s.into_js(&ctx),
            None => Ok(Null.into_value(ctx)),
        }
    }

    #[qjs(get, enumerable)]
    fn connected(&self) -> bool {
        self.connected
    }

    #[allow(unused_variables)]
    fn kill(&mut self, ctx: Ctx<'js>, signal: Opt<Value<'js>>) -> Result<bool> {
        if self.exit_code.is_some() || self.signal_code.is_some() {
            return Ok(false);
        }
        if let Some(pid) = self.pid {
            #[cfg(unix)]
            {
                let sig = parse_signal(signal.0.clone())?;
                let ok = kill(&ctx, pid, signal)?;
                if ok && sig != 0 {
                    self.killed = true;
                }
                return Ok(ok);
            }

            #[cfg(windows)]
            {
                let signal = parse_signal(signal.0)?;
                if signal == 0 {
                    return kill_process_raw(pid, 0)
                        .map(|_| true)
                        .or_else(|_| Ok(false));
                }

                if let Some(tx) = self.kill_tx.take() {
                    let ok = tx.send(signal).is_ok();
                    if ok {
                        self.killed = true;
                    }
                    return Ok(ok);
                }
            }
        }

        Ok(false)
    }

    #[qjs(rename = "ref")]
    fn js_ref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        {
            let borrow = this.borrow();
            borrow.loop_ref.reference();
            if borrow.connected {
                if let Some(ipc_ref) = &borrow.ipc_loop_ref {
                    ipc_ref.reference();
                }
            }
        }
        this.0
    }

    #[qjs(rename = "unref")]
    fn js_unref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        {
            let borrow = this.borrow();
            borrow.loop_ref.unreference();
            if let Some(ipc_ref) = &borrow.ipc_loop_ref {
                ipc_ref.unreference();
            }
        }
        this.0
    }

    fn send(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        message: Value<'js>,
        rest: Rest<Value<'js>>,
    ) -> Result<bool> {
        #[cfg(unix)]
        return ipc_channel::child_send(this, ctx, message, rest);
        #[cfg(not(unix))]
        {
            let _ = (this, ctx, message, rest);
            Ok(false)
        }
    }

    fn disconnect(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        #[cfg(unix)]
        return ipc_channel::child_disconnect(this, ctx);
        #[cfg(not(unix))]
        {
            let _ = (this, ctx);
            Ok(())
        }
    }
}

struct SpawnRuntimeConfig<'js> {
    spawnfile: String,
    spawnargs: Vec<String>,
    command: String,
    args: Option<Vec<String>>,
    timeout_ms: Option<u64>,
    kill_signal: i32,
    kill_signal_val: Option<Value<'js>>,
    serialization: SerializationMode,
    abort_signal: Option<rquickjs::Object<'js>>,
    stdio_specs: Vec<StdioSpec>,
    #[cfg(unix)]
    extra_pipes: Vec<(usize, UnixStream)>,
    #[cfg(unix)]
    ipc_stream: Option<UnixStream>,
}

impl<'js> ChildProcess<'js> {
    fn new_aborted(
        ctx: Ctx<'js>,
        command: String,
        args: Option<Vec<String>>,
        spawnfile: String,
        spawnargs: Vec<String>,
    ) -> Result<Class<'js, Self>> {
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_unreferenced(rt_key);

        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                spawnfile,
                spawnargs,
                command,
                args,
                kill_tx: None,
                pid: None,
                killed: false,
                exit_code: None,
                signal_code: None,
                connected: false,
                serialization: SerializationMode::Advanced,
                loop_ref,
                ipc_loop_ref: None,
                #[cfg(unix)]
                ipc_outbound: None,
            },
        )?;

        let null_val = Null.into_value(ctx.clone());
        instance.set("stdin", null_val.clone())?;
        instance.set("stdout", null_val.clone())?;
        instance.set("stderr", null_val.clone())?;
        let stdio = Array::new(ctx.clone())?;
        stdio.set(0, null_val.clone())?;
        stdio.set(1, null_val.clone())?;
        stdio.set(2, null_val)?;
        instance.set("stdio", stdio)?;

        let inst2 = instance.clone();
        let ctx2 = ctx.clone();
        ctx.spawn_exit_simple(async move {
            let err_val = make_abort_error(&ctx2)?;
            if !inst2.borrow().emitter.has_listener_str("error") {
                return Err(ctx2.throw(err_val));
            }
            ChildProcess::emit_str(inst2, &ctx2, "error", vec![err_val], false)?;
            Ok(())
        });

        Ok(instance)
    }

    fn new_spawned(
        ctx: Ctx<'js>,
        cfg: SpawnRuntimeConfig<'js>,
        child: std::io::Result<Child>,
    ) -> Result<Class<'js, Self>> {
        let (kill_tx, kill_rx) = broadcast_channel::<i32>(4);

        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced_named(rt_key, "ChildProcess");

        let instance = Self {
            emitter: EventEmitter::new(),
            spawnfile: cfg.spawnfile,
            spawnargs: cfg.spawnargs,
            command: cfg.command.clone(),
            args: cfg.args,
            pid: None,
            killed: false,
            exit_code: None,
            signal_code: None,
            connected: false,
            serialization: cfg.serialization,
            kill_tx: Some(kill_tx),
            loop_ref: loop_ref.clone(),
            ipc_loop_ref: None,
            #[cfg(unix)]
            ipc_outbound: None,
        };

        let instance = Class::instance(ctx.clone(), instance)?;
        let instance2 = instance.clone();
        let instance3 = instance.clone();
        let instance4 = instance.clone();

        let null_val = Null.into_value(ctx.clone());
        let pipe_stdin = matches!(cfg.stdio_specs.first(), Some(StdioSpec::Pipe));
        let pipe_stdout = matches!(cfg.stdio_specs.get(1), Some(StdioSpec::Pipe));
        let pipe_stderr = matches!(cfg.stdio_specs.get(2), Some(StdioSpec::Pipe));

        let stdin_instance = if pipe_stdin {
            Some(DefaultWritableStream::new(ctx.clone())?)
        } else {
            None
        };
        let stdout_instance = if pipe_stdout {
            Some(DefaultReadableStream::new(ctx.clone())?)
        } else {
            None
        };
        let stderr_instance = if pipe_stderr {
            Some(DefaultReadableStream::new(ctx.clone())?)
        } else {
            None
        };

        instance.set(
            "stdin",
            stdin_instance
                .as_ref()
                .map(|s| s.clone().into_value())
                .unwrap_or_else(|| null_val.clone()),
        )?;
        instance.set(
            "stdout",
            stdout_instance
                .as_ref()
                .map(|s| s.clone().into_value())
                .unwrap_or_else(|| null_val.clone()),
        )?;
        instance.set(
            "stderr",
            stderr_instance
                .as_ref()
                .map(|s| s.clone().into_value())
                .unwrap_or_else(|| null_val.clone()),
        )?;

        let stdio_arr = Array::new(ctx.clone())?;
        for idx in 0..cfg.stdio_specs.len().max(3) {
            let val = match idx {
                0 => stdin_instance
                    .as_ref()
                    .map(|s| s.clone().into_value())
                    .unwrap_or_else(|| null_val.clone()),
                1 => stdout_instance
                    .as_ref()
                    .map(|s| s.clone().into_value())
                    .unwrap_or_else(|| null_val.clone()),
                2 => stderr_instance
                    .as_ref()
                    .map(|s| s.clone().into_value())
                    .unwrap_or_else(|| null_val.clone()),
                _ => null_val.clone(),
            };
            stdio_arr.set(idx, val)?;
        }

        match child {
            Ok(mut child) => {
                instance2.borrow_mut().pid = child.id();

                if let (Some(stdin_inst), Some(child_stdin)) =
                    (stdin_instance.as_ref(), child.stdin.take())
                {
                    DefaultWritableStream::process(stdin_inst.clone(), &ctx, child_stdin)?;
                }

                let stdout_join_receiver = match (stdout_instance.as_ref(), child.stdout.take()) {
                    (Some(inst), Some(out)) => create_output(&ctx, Some(out), inst.clone())?,
                    _ => None,
                };
                let stderr_join_receiver = match (stderr_instance.as_ref(), child.stderr.take()) {
                    (Some(inst), Some(err)) => create_output(&ctx, Some(err), inst.clone())?,
                    _ => None,
                };

                #[cfg(unix)]
                let mut extra_pipe_handles = Vec::new();
                #[cfg(unix)]
                for (slot_idx, stream) in cfg.extra_pipes {
                    let (pipe, rx) = ChildPipe::from_unix_stream(&ctx, stream)?;
                    stdio_arr.set(slot_idx, pipe.clone())?;
                    extra_pipe_handles.push((pipe, rx));
                }

                #[cfg(unix)]
                if let Some(ipc_stream) = cfg.ipc_stream {
                    let ipc_ref = EventLoopRef::new_referenced(rt_key);
                    ipc_channel::attach_ipc_to_child(
                        &ctx,
                        instance2.clone(),
                        ipc_stream,
                        ipc_ref,
                        cfg.serialization,
                    )?;
                }

                instance.set("stdio", stdio_arr)?;

                if let Some(ref sig_obj) = cfg.abort_signal {
                    attach_abort_error_helper(
                        &ctx,
                        sig_obj,
                        instance.clone().into_value(),
                        cfg.kill_signal_val.clone(),
                    )?;
                }

                let ctx_spawn = ctx.clone();
                let inst_spawn = instance.clone();
                ctx.spawn_exit_simple(async move {
                    let _ = ChildProcess::emit_str(inst_spawn, &ctx_spawn, "spawn", vec![], false);
                    Ok(())
                });

                let ctx2 = ctx.clone();
                let ctx3 = ctx.clone();
                let wait_loop_ref = instance2.borrow().loop_ref.clone();
                let timeout_ms = cfg.timeout_ms;
                let kill_signal = cfg.kill_signal;
                let stdout_inst_clone = stdout_instance.clone();
                let stderr_inst_clone = stderr_instance.clone();
                let stdin_inst_clone = stdin_instance.clone();

                ctx.spawn_exit(async move {
                    let spawn_proc = async move {
                        let mut exit_code = None;
                        let mut exit_signal = None;

                        let outcome = wait_for_process(
                            child,
                            &ctx3,
                            kill_rx,
                            &mut exit_code,
                            &mut exit_signal,
                            wait_loop_ref,
                            timeout_ms,
                            kill_signal,
                        )
                        .await?;

                        if matches!(outcome, WaitOutcome::Detached) {
                            return Ok::<_, Error>(());
                        }

                        if outcome == WaitOutcome::TimedOut {
                            instance2.borrow_mut().killed = true;
                        }

                        let code = match exit_code {
                            Some(c) => {
                                instance2.borrow_mut().exit_code = Some(c);
                                c.into_js(&ctx3)?
                            },
                            None => Null.into_value(ctx3.clone()),
                        };
                        let signal;
                        #[cfg(unix)]
                        {
                            if let Some(s) = exit_signal {
                                let sig_str = signal_str_from_i32(s)
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| format!("SIG{s}"));
                                instance2.borrow_mut().signal_code = Some(sig_str.clone());
                                signal = sig_str.into_js(&ctx3)?;
                            } else {
                                signal = Null.into_value(ctx3.clone());
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            if let Some(s) = exit_signal {
                                let _ = s;
                                instance2.borrow_mut().signal_code = Some("SIGKILL".into());
                                signal = "SIGKILL".into_js(&ctx3)?;
                            } else {
                                signal = Null.into_value(ctx3.clone());
                            }
                        }

                        ChildProcess::emit_str(
                            instance2.clone(),
                            &ctx3,
                            "exit",
                            vec![code.clone(), signal.clone()],
                            false,
                        )?;

                        if let Some(stderr_join_receiver) = stderr_join_receiver {
                            let _ = tokio::time::timeout(
                                Duration::from_millis(50),
                                stderr_join_receiver,
                            )
                            .await;
                        }
                        if let Some(stdout_join_receiver) = stdout_join_receiver {
                            let _ = tokio::time::timeout(
                                Duration::from_millis(50),
                                stdout_join_receiver,
                            )
                            .await;
                        }
                        #[cfg(unix)]
                        for (_pipe, rx) in &mut extra_pipe_handles {
                            let _ = tokio::time::timeout(Duration::from_millis(20), rx).await;
                        }

                        if let Some(stdin_inst) = stdin_inst_clone {
                            WritableStream::end(This(stdin_inst));
                        }
                        if let Some(ref stdout_inst) = stdout_inst_clone {
                            ReadableStream::drain(stdout_inst.clone(), &ctx3)?;
                        }
                        if let Some(ref stderr_inst) = stderr_inst_clone {
                            ReadableStream::drain(stderr_inst.clone(), &ctx3)?;
                        }

                        ChildProcess::emit_str(
                            instance2.clone(),
                            &ctx3,
                            "close",
                            vec![code, signal],
                            false,
                        )?;

                        if let Some(stdout_inst) = stdout_inst_clone {
                            ReadableStream::destroy(This(stdout_inst), Opt(None));
                        }
                        if let Some(stderr_inst) = stderr_inst_clone {
                            ReadableStream::destroy(This(stderr_inst), Opt(None));
                        }
                        #[cfg(unix)]
                        for (pipe, _rx) in extra_pipe_handles {
                            ChildPipe::destroy(This(pipe), Opt(None));
                        }

                        instance2.borrow().loop_ref.unreference();
                        if let Some(ipc_ref) = &instance2.borrow().ipc_loop_ref {
                            ipc_ref.unreference();
                        }

                        Ok::<_, Error>(())
                    };

                    spawn_proc
                        .await
                        .emit_error("child_process", &ctx2, instance4)?;

                    Ok(())
                })?;
            },
            Err(err) => {
                instance.set("stdio", stdio_arr)?;
                let ctx3 = ctx.clone();
                let (err_code, errno) = options::io_error_to_errno(&err);
                let cmd_name = cfg.command;
                let err_message = format!("spawn {cmd_name} {err_code}");

                ctx.spawn_exit(async move {
                    instance3.borrow().loop_ref.unreference();
                    let ex = options::make_errno_error(
                        &ctx3,
                        err_code,
                        errno,
                        &format!("spawn {cmd_name}"),
                        Some(&cmd_name),
                        &err_message,
                    )?;
                    if !instance3.borrow().emitter.has_listener_str("error") {
                        return Err(ctx3.throw(ex));
                    }
                    ChildProcess::emit_str(instance3.clone(), &ctx3, "error", vec![ex], false)?;
                    Ok(())
                })?;
            },
        }
        Ok(instance)
    }
}

#[derive(PartialEq, Eq)]
enum WaitOutcome {
    Exited,
    Detached,
    TimedOut,
}

#[allow(clippy::too_many_arguments)]
async fn wait_for_process(
    mut child: Child,
    ctx: &Ctx<'_>,
    mut kill_rx: Receiver<i32>,
    exit_code: &mut Option<i32>,
    _exit_signal: &mut Option<i32>,
    loop_ref: Arc<EventLoopRef>,
    timeout_ms: Option<u64>,
    default_kill_signal: i32,
) -> Result<WaitOutcome> {
    #[cfg(not(unix))]
    let mut was_killed = false;
    let mut timed_out = false;
    let deadline = timeout_ms.map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms));

    loop {
        if !keeps_event_loop_alive(&loop_ref) {
            tokio::task::yield_now().await;
            if !keeps_event_loop_alive(&loop_ref) && !should_keep_process_alive(loop_ref.rt()) {
                std::mem::forget(child);
                return Ok(WaitOutcome::Detached);
            }
        }

        tokio::select! {
            status = child.wait() => {
                let exit_status = status.or_throw(ctx)?;

                #[cfg(unix)]
                {
                    if let Some(sig) = exit_status.signal() {
                        _exit_signal.replace(sig);
                    } else {
                        exit_code.replace(exit_status.code().unwrap_or_default());
                    }
                }
                #[cfg(not(unix))]
                {
                    if !was_killed {
                        exit_code.replace(exit_status.code().unwrap_or_default());
                    } else {
                        _exit_signal.replace(1);
                    }
                }
                break;
            }

            _ = async {
                if let Some(dl) = deadline {
                    tokio::time::sleep_until(dl).await;
                } else {
                    std::future::pending::<()>().await;
                }
            }, if deadline.is_some() && !timed_out => {
                timed_out = true;
                #[cfg(unix)]
                {
                    if let Some(pid) = child.id() {
                        let _ = kill_process_raw(pid, default_kill_signal);
                    } else {
                        let _ = child.kill().await;
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = default_kill_signal;
                    was_killed = true;
                    let _ = child.kill().await;
                }
            }

            Ok(sig) = kill_rx.recv() => {
                let _ = sig;
                #[cfg(not(unix))]
                {
                    was_killed = true;
                }
                child.kill().await.or_throw(ctx)?;
            }

            _ = lexe_utils::event_loop::wait_until_io_unref(loop_ref.clone()) => {
                tokio::task::yield_now().await;
                if !keeps_event_loop_alive(&loop_ref) && !should_keep_process_alive(loop_ref.rt()) {
                    std::mem::forget(child);
                    return Ok(WaitOutcome::Detached);
                }
            }
        }
    }

    Ok(if timed_out {
        WaitOutcome::TimedOut
    } else {
        WaitOutcome::Exited
    })
}

impl<'js> Emitter<'js> for ChildProcess<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

fn spec_to_async_stdio(ctx: &Ctx<'_>, spec: &StdioSpec) -> Result<Stdio> {
    match spec {
        StdioSpec::Pipe => Ok(Stdio::piped()),
        StdioSpec::Ignore => Ok(Stdio::null()),
        StdioSpec::Inherit => Ok(Stdio::inherit()),
        StdioSpec::Fd(fd) => dup_fd_stdio(ctx, *fd),
        StdioSpec::Ipc => Ok(Stdio::null()),
    }
}

fn spawn<'js>(
    ctx: Ctx<'js>,
    cmd_val: Value<'js>,
    args_and_opts: Rest<Value<'js>>,
) -> Result<Class<'js, ChildProcess<'js>>> {
    lexe_utils::permissions::ensure_child_process(&ctx)?;
    let args_val = args_and_opts.first().cloned();
    let opts_val = args_and_opts.get(1).cloned();
    let (raw_cmd, raw_args, opts) = parse_cmd_args_and_opts(
        &ctx,
        cmd_val,
        args_val,
        opts_val,
        false,
        vec![StdioSpec::Pipe, StdioSpec::Pipe, StdioSpec::Pipe],
        true,
    )?;

    let (mut std_cmd, spawnfile, spawnargs) =
        build_command(raw_cmd.clone(), raw_args.clone(), &opts);

    if let Some(ref sig_obj) = opts.signal {
        if sig_obj.get::<_, bool>("aborted").unwrap_or(false) {
            return ChildProcess::new_aborted(ctx, raw_cmd, raw_args, spawnfile, spawnargs);
        }
    }

    #[cfg(unix)]
    let mut fd_maps: Vec<(i32, i32)> = Vec::new();
    #[cfg(unix)]
    let mut close_in_parent: Vec<i32> = Vec::new();
    #[cfg(unix)]
    let mut extra_pipes: Vec<(usize, UnixStream)> = Vec::new();
    #[cfg(unix)]
    let mut ipc_stream: Option<UnixStream> = None;

    for (idx, spec) in opts.stdio.iter().enumerate() {
        if idx <= 2 && !matches!(spec, StdioSpec::Ipc) {
            let stdio = match spec_to_async_stdio(&ctx, spec) {
                Ok(s) => s,
                Err(e) => {
                    #[cfg(unix)]
                    close_raw_fds(&close_in_parent);
                    return Err(e);
                },
            };
            match idx {
                0 => std_cmd.stdin(stdio),
                1 => std_cmd.stdout(stdio),
                _ => std_cmd.stderr(stdio),
            };
            continue;
        }

        #[cfg(unix)]
        match spec {
            StdioSpec::Pipe => {
                let (parent_sock, child_sock) = match StdUnixStream::pair().or_throw(&ctx) {
                    Ok(p) => p,
                    Err(e) => {
                        close_raw_fds(&close_in_parent);
                        return Err(e);
                    },
                };
                if let Err(e) = parent_sock.set_nonblocking(true).or_throw(&ctx) {
                    close_raw_fds(&close_in_parent);
                    return Err(e);
                }
                let child_raw = child_sock.into_raw_fd();
                close_in_parent.push(child_raw);
                fd_maps.push((idx as i32, child_raw));
                let tokio_parent = UnixStream::from_std(parent_sock).or_throw(&ctx)?;
                extra_pipes.push((idx, tokio_parent));
            },
            StdioSpec::Ipc => {
                let (parent_sock, child_sock) = match StdUnixStream::pair().or_throw(&ctx) {
                    Ok(p) => p,
                    Err(e) => {
                        close_raw_fds(&close_in_parent);
                        return Err(e);
                    },
                };
                if let Err(e) = parent_sock.set_nonblocking(true).or_throw(&ctx) {
                    close_raw_fds(&close_in_parent);
                    return Err(e);
                }
                let child_raw = child_sock.into_raw_fd();
                close_in_parent.push(child_raw);
                fd_maps.push((idx as i32, child_raw));
                std_cmd.env("NODE_CHANNEL_FD", idx.to_string());
                match opts.serialization {
                    SerializationMode::Json => std_cmd.env("LEXE_FORK_SERIALIZATION", "json"),
                    SerializationMode::Advanced => std_cmd.env_remove("LEXE_FORK_SERIALIZATION"),
                };
                let tokio_parent = UnixStream::from_std(parent_sock).or_throw(&ctx)?;
                ipc_stream = Some(tokio_parent);
            },
            StdioSpec::Fd(src_fd) => {
                let duped = unsafe { libc::dup(*src_fd) };
                if duped < 0 {
                    close_raw_fds(&close_in_parent);
                    let err = std::io::Error::last_os_error();
                    let (code, errno) = options::io_error_to_errno(&err);
                    return Err(ctx.throw(options::make_errno_error(
                        &ctx,
                        code,
                        errno,
                        "dup",
                        None,
                        &format!("{code}: bad file descriptor fd={src_fd}"),
                    )?));
                }
                close_in_parent.push(duped);
                fd_maps.push((idx as i32, duped));
            },
            StdioSpec::Ignore | StdioSpec::Inherit => {},
        }
    }

    #[cfg(unix)]
    if !fd_maps.is_empty() {
        let min_safe = fd_maps.iter().map(|(t, _)| *t).max().unwrap_or(2) + 1;
        for entry in &mut fd_maps {
            let (target, src) = *entry;
            if src != target && src < min_safe {
                let tmp = unsafe { libc::fcntl(src, libc::F_DUPFD_CLOEXEC, min_safe) };
                if tmp < 0 {
                    close_raw_fds(&close_in_parent);
                    let err = std::io::Error::last_os_error();
                    let (code, errno) = options::io_error_to_errno(&err);
                    return Err(ctx.throw(options::make_errno_error(
                        &ctx,
                        code,
                        errno,
                        "fcntl",
                        None,
                        &format!("{code}: failed to duplicate fd={src}"),
                    )?));
                }
                entry.1 = tmp;
                close_in_parent.push(tmp);
            }
        }
        unsafe {
            std_cmd.pre_exec(move || {
                for &(target, src) in &fd_maps {
                    if src == target {
                        let flags = libc::fcntl(target, libc::F_GETFD);
                        if flags >= 0 {
                            libc::fcntl(target, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                        }
                    } else if libc::dup2(src, target) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }

    let mut command = Command::from(std_cmd);
    let spawned = command.spawn();

    #[cfg(unix)]
    close_raw_fds(&close_in_parent);

    let cfg = SpawnRuntimeConfig {
        spawnfile,
        spawnargs,
        command: raw_cmd,
        args: raw_args,
        timeout_ms: opts.timeout_ms,
        kill_signal: opts.kill_signal,
        kill_signal_val: opts.kill_signal_val,
        serialization: opts.serialization,
        abort_signal: opts.signal,
        stdio_specs: opts.stdio,
        #[cfg(unix)]
        extra_pipes,
        #[cfg(unix)]
        ipc_stream,
    };

    ChildProcess::new_spawned(ctx, cfg, spawned)
}

#[cfg(unix)]
fn close_raw_fds(fds: &[i32]) {
    for &fd in fds {
        unsafe {
            libc::close(fd);
        }
    }
}

fn create_output<'js, T>(
    ctx: &Ctx<'js>,
    output: Option<T>,
    native_readable_stream: Class<'js, DefaultReadableStream<'js>>,
) -> Result<Option<OneshotReceiver<bool>>>
where
    T: AsyncRead + Unpin + Send + 'static,
{
    if let Some(output) = output {
        let receiver = DefaultReadableStream::process(native_readable_stream, ctx, output)?;
        return Ok(Some(receiver));
    }

    Ok(None)
}

pub struct ChildProcessModule;

impl ModuleDef for ChildProcessModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("ChildProcess")?;
        declare.declare("spawn")?;
        declare.declare("spawnSync")?;
        declare.declare("fork")?;
        declare.declare("exec")?;
        declare.declare("execFile")?;
        declare.declare("execSync")?;
        declare.declare("execFileSync")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let ctor = Class::<ChildProcess>::create_constructor(ctx)?
            .or_throw_msg(ctx, "Failed to create ChildProcess constructor")?;
        ChildProcess::add_event_emitter_prototype(ctx)?;
        DefaultWritableStream::add_writable_stream_prototype(ctx)?;
        DefaultWritableStream::add_event_emitter_prototype(ctx)?;
        DefaultReadableStream::add_readable_stream_prototype(ctx)?;
        DefaultReadableStream::add_event_emitter_prototype(ctx)?;
        #[cfg(unix)]
        {
            ChildPipe::add_event_emitter_prototype(ctx)?;
            ChildPipe::add_readable_stream_prototype(ctx)?;
            ChildPipe::add_writable_stream_prototype(ctx)?;
            if let Some(proto) = Class::<ChildPipe>::prototype(ctx)? {
                proto.set("write", Func::from(ChildPipe::write_flushed))?;
                proto.set("destroy", Func::from(ChildPipe::destroy))?;
            }
        }

        export_default(ctx, exports, |default| {
            default.set("ChildProcess", ctor)?;
            default.set("spawn", Func::from(spawn))?;
            default.set("spawnSync", Func::from(sync::spawn_sync))?;
            #[cfg(unix)]
            default.set("fork", Func::from(fork))?;

            let helpers: rquickjs::Object = ctx.eval(EXEC_HELPERS_JS)?;
            let exec: Function = helpers.get("exec")?;
            let exec_file: Function = helpers.get("execFile")?;
            default.set("exec", exec)?;
            default.set("execFile", exec_file)?;
            default.set("execSync", Func::from(sync::exec_sync))?;
            default.set("execFileSync", Func::from(sync::exec_file_sync))?;
            Ok(())
        })?;

        Ok(())
    }
}

const EXEC_HELPERS_JS: &str = r#"(function(){
  const DEFAULT_MAX_BUFFER = 1024 * 1024;
  const runExec = (isExec, target, args, options, callback) => {
    const { spawn } = require("child_process");
    options = options || {};
    const encoding = options.encoding === undefined ? "utf8" : options.encoding;
    const maxBuffer = options.maxBuffer === undefined ? DEFAULT_MAX_BUFFER : options.maxBuffer;
    const killSignal = options.killSignal === undefined ? "SIGTERM" : options.killSignal;
    const spawnOpts = {
      cwd: options.cwd,
      env: options.env,
      shell: isExec ? (options.shell === false ? false : (options.shell || true)) : options.shell,
      uid: options.uid,
      gid: options.gid,
      argv0: options.argv0,
      timeout: options.timeout,
      killSignal,
      signal: options.signal,
      windowsHide: options.windowsHide,
      windowsVerbatimArguments: options.windowsVerbatimArguments,
    };
    const child = isExec ? spawn(String(target), spawnOpts) : spawn(target, args, spawnOpts);
    const stdoutChunks = [];
    const stderrChunks = [];
    let stdoutLen = 0;
    let stderrLen = 0;
    let exitedError = null;
    let maxBufferErr = null;
    let settled = false;

    const formatBufs = (chunks) => {
      const merged = Buffer.concat(chunks);
      if (encoding === null || encoding === "buffer") return merged;
      return merged.toString(encoding);
    };

    const finish = (err) => {
      if (settled) return;
      settled = true;
      if (typeof callback === "function") {
        callback(err || null, formatBufs(stdoutChunks), formatBufs(stderrChunks));
      }
    };

    const bindPipe = (stream, chunks, isStdout) => {
      if (!stream) return;
      stream.on("data", (c) => {
        const buf = Buffer.isBuffer(c) ? c : Buffer.from(c);
        const nextLen = isStdout ? (stdoutLen += buf.length) : (stderrLen += buf.length);
        if (nextLen > maxBuffer && !maxBufferErr) {
          maxBufferErr = new RangeError((isStdout ? "stdout" : "stderr") + " maxBuffer length exceeded");
          maxBufferErr.code = "ERR_CHILD_PROCESS_STDIO_MAXBUFFER";
          child.kill(killSignal);
          return;
        }
        chunks.push(buf);
      });
    };

    bindPipe(child.stdout, stdoutChunks, true);
    bindPipe(child.stderr, stderrChunks, false);

    child.on("error", (err) => {
      exitedError = err;
      finish(err);
    });
    child.on("close", (code, signal) => {
      if (maxBufferErr) return finish(maxBufferErr);
      if (exitedError) return finish(exitedError);
      if (code === 0 && !signal) return finish(null);
      const label = isExec ? String(target) : String(target);
      const stderrText = isExec ? "\n" + Buffer.concat(stderrChunks).toString() : "";
      const err = new Error("Command failed: " + label + stderrText);
      err.code = code;
      err.killed = child.killed;
      err.signal = signal;
      err.cmd = isExec ? String(target) : [target, ...args].join(" ");
      err.stdout = formatBufs(stdoutChunks);
      err.stderr = formatBufs(stderrChunks);
      finish(err);
    });
    return child;
  };

  return {
    exec(command, options, callback) {
      if (typeof options === "function") { callback = options; options = {}; }
      return runExec(true, command, [], options, callback);
    },
    execFile(file, args, options, callback) {
      if (typeof args === "function") { callback = args; args = []; options = {}; }
      else if (args && !Array.isArray(args)) {
        if (typeof options === "function") callback = options;
        options = args;
        args = [];
      } else if (typeof options === "function") { callback = options; options = {}; }
      return runExec(false, file, args || [], options, callback);
    },
  };
})()"#;

impl From<ChildProcessModule> for ModuleInfo<ChildProcessModule> {
    fn from(val: ChildProcessModule) -> Self {
        ModuleInfo {
            name: "child_process",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_buffer as buffer;
    use lexe_test::{test_async_with, ModuleEvaluator};
    use rquickjs::CatchResultExt;

    #[tokio::test]
    async fn test_spawn() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();

                ModuleEvaluator::eval_rust::<ChildProcessModule>(ctx.clone(), "node:child_process")
                    .await
                    .unwrap();

                let message: String = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                   import {spawn} from "node:child_process";

                    let resolve = null;
                    const deferred = new Promise(res => {
                        resolve = res;
                    });

                    spawn("echo", ["hello"]).stdout.on("data", (data) => {
                        resolve(data.toString().trim())
                    });

                    export default await deferred;

                "#,
                )
                .await
                .catch(&ctx)
                .unwrap()
                .get("default")
                .unwrap();

                assert_eq!(message, "hello");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_spawn_shell() {
        test_async_with(|ctx| {
            Box::pin(async move {
                buffer::init(&ctx).unwrap();

                ModuleEvaluator::eval_rust::<ChildProcessModule>(ctx.clone(), "node:child_process")
                    .await
                    .unwrap();

                let message: String = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    import {spawn} from "node:child_process";

                    let resolve = null;
                    const deferred = new Promise(res => {
                        resolve = res;
                    });

                    spawn("echo", ["hello"], {
                        shell: true
                    }).stdout.on("data", (data) => {
                        resolve(data.toString().trim())
                    });

                    export default await deferred;
                "#,
                )
                .await
                .catch(&ctx)
                .unwrap()
                .get("default")
                .unwrap();

                assert_eq!(message, "hello");
            })
        })
        .await;
    }
}
