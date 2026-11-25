// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::env;
use std::os::unix::io::IntoRawFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::sync::Arc;

use lexe_context::CtxExtension;
use lexe_events::Emitter;
use lexe_utils::{
    event_loop::{
        keeps_event_loop_alive, should_keep_process_alive, wait_until_io_unref, EventLoopRef,
    },
    object::ObjectExt,
    result::ResultExt,
    signals::signal_str_from_i32,
};
use rquickjs::{
    convert::Coerced, prelude::Rest, Array, Class, Ctx, Exception, IntoJs, Null, Result, Value,
};
use tokio::net::UnixStream;
use tokio::sync::broadcast::Receiver;

use crate::ipc_channel;
use crate::options::{parse_serialization_value, SerializationMode};
use crate::ChildProcess;

const IPC_FD: i32 = 3;

pub fn fork<'js>(
    ctx: Ctx<'js>,
    module_path: String,
    args_and_opts: Rest<Value<'js>>,
) -> Result<Class<'js, ChildProcess<'js>>> {
    lexe_utils::permissions::ensure_child_process(&ctx)?;
    let module_path = resolve_module_path(&ctx, module_path)?;
    let (exec_args, opts) = parse_fork_args(&ctx, args_and_opts)?;

    let (std_parent, std_child) = StdUnixStream::pair().or_throw(&ctx)?;
    std_parent.set_nonblocking(true).or_throw(&ctx)?;
    std_child.set_nonblocking(true).or_throw(&ctx)?;

    let child_socket_fd = std_child.into_raw_fd();
    clear_cloexec(child_socket_fd);

    let exe = opts.exec_path.clone().unwrap_or_else(|| {
        env::current_exe()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });

    let pid = unsafe { libc::fork() };
    if pid < 0 {
        let _ = unsafe { libc::close(child_socket_fd) };
        return Err(Exception::throw_type(
            &ctx,
            &format!("fork failed: {}", std::io::Error::last_os_error()),
        ));
    }

    if pid == 0 {
        drop(std_parent);
        if child_socket_fd != IPC_FD {
            if unsafe { libc::dup2(child_socket_fd, IPC_FD) } < 0 {
                unsafe { libc::_exit(1) };
            }
            let _ = unsafe { libc::close(child_socket_fd) };
        }
        clear_cloexec(IPC_FD);

        env::set_var("LEXE_FORK_CHILD", "1");
        env::set_var("LEXE_FORK_MODULE", &module_path);
        env::set_var("NODE_CHANNEL_FD", IPC_FD.to_string());
        match opts.serialization {
            SerializationMode::Json => env::set_var("LEXE_FORK_SERIALIZATION", "json"),
            SerializationMode::Advanced => env::remove_var("LEXE_FORK_SERIALIZATION"),
        }
        if let Some(cwd) = &opts.cwd {
            let _ = env::set_current_dir(cwd);
        }
        if let Some(env_map) = &opts.env {
            for (k, v) in env_map {
                env::set_var(k, v);
            }
        }

        let err = std::process::Command::new(&exe).exec();
        eprintln!("fork child exec failed: {err}");
        unsafe { libc::_exit(1) };
    }

    let _ = unsafe { libc::close(child_socket_fd) };
    let stream = UnixStream::from_std(std_parent).or_throw(&ctx)?;
    ChildProcess::new_fork_ipc(ctx, pid as u32, stream, module_path, Some(exec_args), opts)
}

pub(crate) struct ForkOpts {
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
    exec_path: Option<String>,
    silent: bool,
    serialization: SerializationMode,
}

fn parse_fork_args<'js>(
    ctx: &Ctx<'js>,
    args_and_opts: Rest<Value<'js>>,
) -> Result<(Vec<String>, ForkOpts)> {
    let mut exec_args = Vec::new();
    let mut opts = ForkOpts {
        cwd: None,
        env: None,
        exec_path: None,
        silent: false,
        serialization: SerializationMode::Advanced,
    };

    if let Some(first) = args_and_opts.first() {
        if first.is_array() {
            let arr = first.clone().into_array().or_throw(ctx)?;
            for item in arr.iter::<Value>() {
                let item = item?;
                let coerced: Coerced<String> = item.get()?;
                exec_args.push(coerced.0);
            }
        }
    }

    let opt_obj = args_and_opts.get(if args_and_opts.first().is_some_and(|v| v.is_array()) {
        1
    } else {
        0
    });
    if let Some(v) = opt_obj {
        if let Some(obj) = v.as_object() {
            opts.cwd = obj.get_optional("cwd")?;
            opts.silent = obj.get_optional("silent")?.unwrap_or(false);
            opts.exec_path = obj.get_optional("execPath")?;
            if let Some(env) = obj.get_optional::<_, HashMap<String, Coerced<String>>>("env")? {
                opts.env = Some(env.into_iter().map(|(k, v)| (k, v.to_string())).collect());
            }
            let ser = obj.get_optional::<_, Value>("serialization")?;
            opts.serialization = parse_serialization_value(ctx, ser)?;
        }
    }

    Ok((exec_args, opts))
}

fn resolve_module_path(ctx: &Ctx<'_>, filename: String) -> Result<String> {
    if let Some(path) = filename.strip_prefix("file://") {
        return Ok(path.to_string());
    }
    let path = PathBuf::from(&filename);
    if path.is_absolute() {
        return Ok(filename);
    }
    let cwd = env::current_dir().or_throw(ctx)?;
    Ok(cwd.join(path).to_string_lossy().into_owned())
}

fn clear_cloexec(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        }
    }
}

async fn wait_for_fork_exit(
    pid: u32,
    mut kill_rx: Receiver<i32>,
    loop_ref: Arc<EventLoopRef>,
) -> Option<libc::c_int> {
    let mut status: libc::c_int = 0;
    loop {
        let r = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if r > 0 {
            return Some(status);
        }
        if r < 0 {
            return None;
        }
        if !keeps_event_loop_alive(&loop_ref) {
            tokio::task::yield_now().await;
            if !keeps_event_loop_alive(&loop_ref) && !should_keep_process_alive(loop_ref.rt()) {
                return None;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {},
            Ok(sig) = kill_rx.recv() => {
                unsafe { libc::kill(pid as libc::pid_t, sig); }
            },
            _ = wait_until_io_unref(loop_ref.clone()) => {
                tokio::task::yield_now().await;
                if !keeps_event_loop_alive(&loop_ref) && !should_keep_process_alive(loop_ref.rt()) {
                    return None;
                }
            }
        }
    }
}

impl<'js> ChildProcess<'js> {
    pub(crate) fn new_fork_ipc(
        ctx: Ctx<'js>,
        pid: u32,
        stream: UnixStream,
        command: String,
        args: Option<Vec<String>>,
        opts: ForkOpts,
    ) -> Result<Class<'js, Self>> {
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced_named(rt_key, "ChildProcess");
        let ipc_loop_ref = EventLoopRef::new_referenced_named(rt_key, "ChildProcessIPC");
        let (kill_tx, kill_rx) = tokio::sync::broadcast::channel::<i32>(1);

        let mut spawnargs = Vec::with_capacity(1 + args.as_ref().map_or(0, Vec::len));
        spawnargs.push(command.clone());
        if let Some(ref a) = args {
            spawnargs.extend(a.iter().cloned());
        }

        let instance_inner = Self {
            emitter: lexe_events::EventEmitter::new(),
            spawnfile: command.clone(),
            spawnargs,
            command: command.clone(),
            args,
            kill_tx: Some(kill_tx),
            pid: Some(pid),
            killed: false,
            exit_code: None,
            signal_code: None,
            connected: false,
            serialization: opts.serialization,
            loop_ref: loop_ref.clone(),
            ipc_loop_ref: Some(ipc_loop_ref.clone()),
            ipc_outbound: None,
        };

        let instance = Class::instance(ctx.clone(), instance_inner)?;
        let null_val = Null.into_value(ctx.clone());
        instance.set("stderr", null_val.clone())?;
        instance.set("stdout", null_val.clone())?;
        instance.set("stdin", null_val.clone())?;
        let stdio = Array::new(ctx.clone())?;
        stdio.set(0, null_val.clone())?;
        stdio.set(1, null_val.clone())?;
        stdio.set(2, null_val.clone())?;
        stdio.set(3, null_val)?;
        instance.set("stdio", stdio)?;

        ipc_channel::attach_ipc_to_child(
            &ctx,
            instance.clone(),
            stream,
            ipc_loop_ref,
            opts.serialization,
        )?;

        let _ = opts.silent;

        let ctx_spawn = ctx.clone();
        let inst_spawn = instance.clone();
        ctx.spawn_exit_simple(async move {
            let _ = ChildProcess::emit_str(inst_spawn, &ctx_spawn, "spawn", vec![], false);
            Ok(())
        });

        let ctx2 = ctx.clone();
        let inst = instance.clone();
        ctx.spawn_exit_simple(async move {
            if let Some(status) = wait_for_fork_exit(pid, kill_rx, loop_ref.clone()).await {
                let (code, signal) = if libc::WIFSIGNALED(status) {
                    let sig = libc::WTERMSIG(status);
                    let sig_str = signal_str_from_i32(sig)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("SIG{sig}"));
                    inst.borrow_mut().signal_code = Some(sig_str.clone());
                    (Null.into_value(ctx2.clone()), sig_str.into_js(&ctx2)?)
                } else {
                    let code = if libc::WIFEXITED(status) {
                        libc::WEXITSTATUS(status)
                    } else {
                        1
                    };
                    inst.borrow_mut().exit_code = Some(code);
                    (code.into_js(&ctx2)?, Null.into_value(ctx2.clone()))
                };
                let _ = ChildProcess::emit_str(
                    inst.clone(),
                    &ctx2,
                    "exit",
                    vec![code.clone(), signal.clone()],
                    false,
                );
                let _ =
                    ChildProcess::emit_str(inst.clone(), &ctx2, "close", vec![code, signal], false);
            }
            loop_ref.unreference();
            Ok(())
        });

        Ok(instance)
    }
}
