// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::env;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;

use lexe_context::CtxExtension;
use lexe_events::Emitter;
use lexe_utils::{event_loop::EventLoopRef, object::ObjectExt, result::ResultExt};
use rquickjs::{convert::Coerced, prelude::Rest, Class, Ctx, Exception, IntoJs, Result, Value};
use tokio::net::UnixStream;

use crate::ipc_channel;
use crate::{ChildProcess, DefaultReadableStream, DefaultWritableStream};

const IPC_FD: i32 = 3;

pub fn fork<'js>(
    ctx: Ctx<'js>,
    module_path: String,
    args_and_opts: Rest<Value<'js>>,
) -> Result<Class<'js, ChildProcess<'js>>> {
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

struct ForkOpts {
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
    exec_path: Option<String>,
    silent: bool,
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
    };

    if let Some(first) = args_and_opts.first() {
        if first.is_array() {
            let arr = first.clone().into_array().or_throw(ctx)?;
            for item in arr.iter::<Value>() {
                let item = item?;
                let s = item
                    .as_string()
                    .or_throw_msg(ctx, "fork argv must be strings")?;
                exec_args.push(s.to_string()?);
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

impl<'js> ChildProcess<'js> {
    pub(crate) fn new_fork_ipc(
        ctx: Ctx<'js>,
        pid: u32,
        stream: UnixStream,
        command: String,
        args: Option<Vec<String>>,
        opts: ForkOpts,
    ) -> Result<Class<'js, ChildProcess<'js>>> {
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);

        let instance_inner = Self {
            emitter: lexe_events::EventEmitter::new(),
            command: command.clone(),
            args,
            pid: Some(pid),
            kill_tx: None,
            loop_ref: loop_ref.clone(),
            ipc_outbound: None,
        };

        let stdout_instance = crate::DefaultReadableStream::new(ctx.clone())?;
        let stderr_instance = crate::DefaultReadableStream::new(ctx.clone())?;
        let stdin_instance = crate::DefaultWritableStream::new(ctx.clone())?;

        let instance = Class::instance(ctx.clone(), instance_inner)?;
        instance.set("stderr", stderr_instance)?;
        instance.set("stdout", stdout_instance)?;
        instance.set("stdin", stdin_instance)?;

        ipc_channel::attach_ipc_to_child(&ctx, instance.clone(), stream, loop_ref)?;

        if !opts.silent {
            // inherit stdio — nothing to pipe
        }

        let ctx2 = ctx.clone();
        let inst = instance.clone();
        ctx.spawn_exit_simple(async move {
            let mut status: libc::c_int = 0;
            // Never block the JS event loop on the child: a blocking `waitpid`
            // here freezes every other task (IPC reads, timers), which made
            // messages look lost and hung the child's exit event.
            let waited = loop {
                let r = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
                if r != 0 {
                    // >0: reaped, -1: already gone / error. Either way, stop.
                    break r;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            };
            if waited >= 0 {
                let code = if libc::WIFEXITED(status) {
                    libc::WEXITSTATUS(status)
                } else {
                    1
                };
                ChildProcess::emit_str(
                    inst,
                    &ctx2,
                    "exit",
                    vec![
                        (code as i32).into_js(&ctx2)?,
                        rquickjs::Null.into_js(&ctx2)?,
                    ],
                    false,
                )?;
            }
            Ok(())
        });

        Ok(instance)
    }
}
