// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::uninlined_format_args)]

#[cfg(unix)]
mod fork;
#[cfg(unix)]
mod ipc_channel;

#[cfg(unix)]
pub use fork::fork;
#[cfg(unix)]
pub use ipc_channel::install_child_process_ipc;

#[cfg(windows)]
use std::os::windows::{
    io::{FromRawHandle, RawHandle},
    process::CommandExt,
};
#[cfg(unix)]
use std::os::{
    fd::FromRawFd,
    unix::process::{CommandExt, ExitStatusExt},
};
use std::{
    collections::HashMap,
    io::Result as IoResult,
    process::{Command as StdCommand, Stdio},
    sync::{Arc, RwLock},
};

use lexe_context::CtxExtension;
use lexe_events::{EmitError, Emitter, EventEmitter, EventList};
use lexe_stream::{
    readable::{DefaultReadableStream, ReadableStream},
    writable::{DefaultWritableStream, WritableStream},
};
use lexe_utils::{
    clone_wire::CloneWirePayload,
    event_loop::EventLoopRef,
    module::{export_default, ModuleInfo},
    object::ObjectExt,
    result::ResultExt,
};
use rquickjs::{
    class::{Trace, Tracer},
    convert::Coerced,
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, Opt, Rest, This},
    Class, Ctx, Error, Exception, IntoJs, Result, Value,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::{
    io::AsyncRead,
    process::{Child, Command},
    sync::{
        broadcast::{channel as broadcast_channel, Receiver, Sender},
        oneshot::Receiver as OneshotReceiver,
    },
};

#[cfg(unix)]
use lexe_utils::signals::{kill, signal_str_from_i32};

#[cfg(not(unix))]
use lexe_utils::signals::{kill_process_raw, parse_signal};

#[allow(unused_variables)]
fn prepare_shell_args(
    shell: &str,
    windows_verbatim_arguments: &mut bool,
    cmd: String,
    command_args: Option<Vec<String>>,
) -> Vec<String> {
    let mut string_args = cmd;

    #[cfg(windows)]
    let shell_is_cmd = shell.ends_with("cmd") || shell.ends_with("cmd.exe");

    #[cfg(windows)]
    {
        if shell_is_cmd {
            *windows_verbatim_arguments = true;
            string_args.insert(0, '"');
        }
    }

    if let Some(command_args) = command_args {
        //reserve at least arg length +1
        let total_length = command_args.iter().map(|s| s.len() + 1).sum();
        string_args.reserve(total_length);
        string_args.push(' ');

        for arg in command_args.iter() {
            string_args.push_str(arg);
            string_args.push(' ');
        }
    } else {
        string_args.push(' ');
    }

    #[cfg(windows)]
    {
        if shell_is_cmd {
            string_args.push('"');
            return vec![
                String::from("/d"),
                String::from("/s"),
                String::from("/c"),
                string_args,
            ];
        }
    }

    vec!["-c".into(), string_args]
}

#[allow(dead_code)]
#[rquickjs::class]
#[derive(rquickjs::JsLifetime)]
pub struct ChildProcess<'js> {
    emitter: EventEmitter<'js>,
    args: Option<Vec<String>>,
    command: String,
    kill_tx: Option<Sender<()>>,
    pid: Option<u32>,
    loop_ref: Arc<EventLoopRef>,
    ipc_outbound: Option<UnboundedSender<CloneWirePayload>>,
}

impl<'js> Trace<'js> for ChildProcess<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

#[derive(Clone)]
enum StdioEnum {
    Piped,
    Ignore,
    Inherit,
    Fd(i32),
}
impl StdioEnum {
    fn to_stdio(&self) -> Stdio {
        match self {
            StdioEnum::Piped => Stdio::piped(),
            StdioEnum::Ignore => Stdio::null(),
            StdioEnum::Inherit => Stdio::inherit(),
            StdioEnum::Fd(id) => {
                #[cfg(unix)]
                unsafe {
                    Stdio::from_raw_fd(*id)
                }
                #[cfg(windows)]
                unsafe {
                    Stdio::from_raw_handle(*id as RawHandle)
                }
            },
        }
    }
}

#[rquickjs::methods]
impl<'js> ChildProcess<'js> {
    #[qjs(get)]
    fn pid(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.pid.into_js(&ctx)
    }

    #[allow(unused_variables)]
    fn kill(&mut self, ctx: Ctx<'js>, signal: Opt<Value<'js>>) -> Result<bool> {
        if let Some(pid) = self.pid {
            #[cfg(unix)]
            {
                return kill(&ctx, pid, signal);
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
                    return Ok(tx.send(()).is_ok());
                }
            }
        }

        Ok(false)
    }

    #[qjs(rename = "ref")]
    fn js_ref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.reference();
        this.0
    }

    #[qjs(rename = "unref")]
    fn js_unref(this: This<Class<'js, Self>>) -> Class<'js, Self> {
        this.borrow().loop_ref.unreference();
        this.0
    }

    fn send(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        message: Value<'js>,
        transfer: Opt<Value<'js>>,
    ) -> Result<bool> {
        #[cfg(unix)]
        return ipc_channel::child_send(this, ctx, message, transfer);
        #[cfg(not(unix))]
        {
            let _ = (this, ctx, message, transfer);
            Ok(false)
        }
    }
}

impl<'js> ChildProcess<'js> {
    fn new(
        ctx: Ctx<'js>,
        command: String,
        args: Option<Vec<String>>,
        child: IoResult<Child>,
    ) -> Result<Class<'js, Self>> {
        let (kill_tx, kill_rx) = broadcast_channel::<()>(1);

        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced(rt_key);

        let instance = Self {
            emitter: EventEmitter::new(),
            command: command.clone(),
            args,
            pid: None,
            kill_tx: Some(kill_tx),
            loop_ref,
            ipc_outbound: None,
        };

        let stdout_instance = DefaultReadableStream::new(ctx.clone())?;
        let stderr_instance = DefaultReadableStream::new(ctx.clone())?;
        let stdin_instance = DefaultWritableStream::new(ctx.clone())?;

        let instance = Class::instance(ctx.clone(), instance)?;
        let instance2 = instance.clone();
        let instance3 = instance.clone();
        let instance4 = instance.clone();

        instance.set("stderr", stderr_instance.clone())?;
        instance.set("stdout", stdout_instance.clone())?;
        instance.set("stdin", stdin_instance.clone())?;

        match child {
            Ok(mut child) => {
                instance2.borrow_mut().pid = child.id();

                if let Some(child_stdin) = child.stdin.take() {
                    DefaultWritableStream::process(stdin_instance.clone(), &ctx, child_stdin)?;
                };

                let stdout_join_receiver =
                    create_output(&ctx, child.stdout.take(), stdout_instance.clone())?;

                let stderr_join_receiver =
                    create_output(&ctx, child.stderr.take(), stderr_instance.clone())?;

                let ctx2 = ctx.clone();
                let ctx3 = ctx.clone();
                let wait_loop_ref = instance2.borrow().loop_ref.clone();

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
                        )
                        .await?;

                        if matches!(outcome, WaitOutcome::Detached) {
                            return Ok::<_, Error>(());
                        }

                        let code = match exit_code {
                            Some(c) => c.into_js(&ctx3)?,
                            None => rquickjs::Null.into_value(ctx3.clone()),
                        };
                        let signal;
                        #[cfg(unix)]
                        {
                            if let Some(s) = exit_signal {
                                signal = signal_str_from_i32(s).into_js(&ctx3)?;
                            } else {
                                signal = rquickjs::Null.into_value(ctx3.clone());
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            signal = "SIGKILL".into_js(&ctx3)?;
                        }

                        ChildProcess::emit_str(
                            instance2.clone(),
                            &ctx3,
                            "exit",
                            vec![code.clone(), signal.clone()],
                            false,
                        )?;

                        if let Some(stderr_join_receiver) = stderr_join_receiver {
                            // Ok if the sender drops. Buffered output without a
                            // `data` listener parks in the stream loop, so bound
                            // the wait: the child is already gone and Node emits
                            // `close` regardless.
                            let _ = tokio::time::timeout(
                                tokio::time::Duration::from_millis(50),
                                stderr_join_receiver,
                            )
                            .await;
                        }
                        if let Some(stdout_join_receiver) = stdout_join_receiver {
                            let _ = tokio::time::timeout(
                                tokio::time::Duration::from_millis(50),
                                stdout_join_receiver,
                            )
                            .await;
                        }

                        WritableStream::end(This(stdin_instance));

                        ReadableStream::drain(stdout_instance.clone(), &ctx3)?;
                        ReadableStream::drain(stderr_instance.clone(), &ctx3)?;

                        ChildProcess::emit_str(
                            instance2.clone(),
                            &ctx3,
                            "close",
                            vec![code, signal],
                            false,
                        )?;

                        // The child is reaped: its stdio read loops have nothing
                        // left to deliver. Leaving them parked keeps a spawned
                        // task pending forever, so a natural exit can never
                        // observe an idle loop.
                        ReadableStream::destroy(This(stdout_instance), Opt(None));
                        ReadableStream::destroy(This(stderr_instance), Opt(None));

                        // The child handle is done; Node releases it before the
                        // loop can go idle, so stop keeping the process alive.
                        instance2.borrow().loop_ref.unreference();

                        Ok::<_, Error>(())
                    };

                    spawn_proc
                        .await
                        .emit_error("child_process", &ctx2, instance4)?;

                    Ok(())
                })?;
            },
            Err(err) => {
                let ctx3 = ctx.clone();

                let err_message = format!("Child process failed to spawn \"{}\". {}", command, err);

                ctx.spawn_exit(async move {
                    if !instance3.borrow().emitter.has_listener_str("error") {
                        return Err(Exception::throw_message(&ctx3, &err_message));
                    }

                    let ex = Exception::from_message(ctx3.clone(), &err_message)?;
                    ChildProcess::emit_str(instance3, &ctx3, "error", vec![ex.into()], false)?;
                    Ok(())
                })?;
            },
        }
        Ok(instance)
    }
}

enum WaitOutcome {
    Exited,
    Detached,
}

async fn wait_for_process(
    mut child: Child,
    ctx: &Ctx<'_>,
    mut kill_rx: Receiver<()>,
    exit_code: &mut Option<i32>,
    _exit_signal: &mut Option<i32>,
    loop_ref: Arc<EventLoopRef>,
) -> Result<WaitOutcome> {
    #[cfg(not(unix))]
    let mut was_killed = false;
    loop {
        if !lexe_utils::event_loop::keeps_event_loop_alive(&loop_ref) {
            std::mem::forget(child);
            return Ok(WaitOutcome::Detached);
        }

        tokio::select! {
            status = child.wait() => {
                let exit_status = status.or_throw(ctx)?;

                #[cfg(unix)]
                {
                    if let Some(sig) = exit_status.signal() {
                        _exit_signal.replace(sig);
                        // code is null when terminated by signal
                    } else {
                        exit_code.replace(exit_status.code().unwrap_or_default());
                    }
                }
                #[cfg(not(unix))]
                {
                    if !was_killed {
                        exit_code.replace(exit_status.code().unwrap_or_default());
                    }
                }
                break;
            }

            Ok(()) = kill_rx.recv() => {
                #[cfg(not(unix))]
                {
                    was_killed = true;
                }
                child.kill().await.or_throw(ctx)?;
            }

            _ = lexe_utils::event_loop::wait_until_io_unref(loop_ref.clone()) => {
                std::mem::forget(child);
                return Ok(WaitOutcome::Detached);
            }
        }
    }

    Ok(WaitOutcome::Exited)
}

impl<'js> Emitter<'js> for ChildProcess<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

fn spawn<'js>(
    ctx: Ctx<'js>,
    cmd: String,
    args_and_opts: Rest<Value<'js>>,
) -> Result<Class<'js, ChildProcess<'js>>> {
    let args_0 = args_and_opts.first();
    let args_1 = args_and_opts.get(1);

    let mut opts = None;

    if args_1.is_some() {
        opts = args_1.and_then(|o| o.as_object()).map(|o| o.to_owned());
    }

    let mut command_args = if let Some(args_0) = args_0 {
        if args_0.is_array() {
            let args = args_0.clone().into_array().or_throw(&ctx)?;
            let mut args_vec = Vec::with_capacity(args.len());
            for arg in args.iter() {
                let arg: Value = arg?;
                let arg = arg
                    .as_string()
                    .or_throw_msg(&ctx, "argument is not a string")?;
                let arg = arg.to_string()?;
                args_vec.push(arg);
            }
            Some(args_vec)
        } else if args_0.is_object() {
            opts = args_0.as_object().map(|o| o.to_owned());
            None
        } else {
            None
        }
    } else {
        None
    };

    let mut windows_verbatim_arguments = if let Some(opts) = &opts {
        opts.get_optional::<&str, bool>("windowsVerbatimArguments")?
            .unwrap_or_default()
    } else {
        false
    };

    let cmd = if let Some(opts) = &opts {
        if opts
            .get_optional::<&str, bool>("shell")?
            .unwrap_or_default()
        {
            #[cfg(windows)]
            let shell = "cmd.exe".to_string();
            #[cfg(not(windows))]
            let shell = "/bin/sh".to_string();
            command_args = Some(prepare_shell_args(
                &shell,
                &mut windows_verbatim_arguments,
                cmd,
                command_args,
            ));
            shell
        } else if let Some(shell) = opts.get_optional::<&str, String>("shell")? {
            command_args = Some(prepare_shell_args(
                &shell,
                &mut windows_verbatim_arguments,
                cmd,
                command_args,
            ));
            shell
        } else {
            cmd
        }
    } else {
        cmd
    };

    let mut command = StdCommand::new(cmd.clone());
    if let Some(args) = &command_args {
        #[cfg(windows)]
        if windows_verbatim_arguments {
            command.raw_arg(args.join(" "));
        } else {
            command.args(args);
        }
        #[cfg(not(windows))]
        command.args(args);
    }

    let mut stdin = StdioEnum::Piped;
    let mut stdout = StdioEnum::Piped;
    let mut stderr = StdioEnum::Piped;
    let mut detached = false;

    if let Some(opts) = opts {
        #[cfg(unix)]
        {
            if let Some(gid) = opts.get_optional("gid")? {
                command.gid(gid);
            }

            if let Some(uid) = opts.get_optional("uid")? {
                command.uid(uid);
            }
        }

        detached = opts.get_optional("detached")?.unwrap_or(false);

        if let Some(cwd) = opts.get_optional::<_, String>("cwd")? {
            command.current_dir(&cwd);
        }

        if let Some(env) = opts.get_optional::<_, HashMap<String, Coerced<String>>>("env")? {
            let env: HashMap<String, String> = env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            command.env_clear();
            command.envs(env);
        }

        if let Some(stdio) = opts.get_optional::<_, Value<'js>>("stdio")? {
            if let Some(stdio_str) = stdio.as_string() {
                let stdio = str_to_stdio(&ctx, &stdio_str.to_string()?)?;
                stdin = stdio.clone();
                stdout = stdio.clone();
                stderr = stdio;
            } else if let Some(stdio) = stdio.as_array() {
                for (i, item) in stdio.iter::<Value>().enumerate() {
                    let item = item?;
                    let stdio = if item.is_undefined() || item.is_null() {
                        StdioEnum::Piped
                    } else if let Some(std_io_str) = item.as_string() {
                        str_to_stdio(&ctx, &std_io_str.to_string()?)?
                    } else if let Some(fd) = item.as_number() {
                        StdioEnum::Fd(fd as i32)
                    } else {
                        StdioEnum::Piped
                    };
                    match i {
                        0 => stdin = stdio,
                        1 => stdout = stdio,
                        2 => stderr = stdio,
                        _ => {
                            break;
                        },
                    }
                }
            }
        }
    }

    command.stdin(stdin.to_stdio());
    command.stdout(stdout.to_stdio());
    command.stderr(stderr.to_stdio());

    if detached {
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            // DETACHED_PROCESS = 0x00000008
            command.creation_flags(0x00000008);
        }
    }

    //tokio command does not have all std command features stabilized
    let mut command = Command::from(command);

    ChildProcess::new(ctx.clone(), cmd, command_args, command.spawn())
}

fn str_to_stdio(ctx: &Ctx<'_>, input: &str) -> Result<StdioEnum> {
    match input {
        "pipe" => Ok(StdioEnum::Piped),
        "ignore" => Ok(StdioEnum::Ignore),
        "inherit" => Ok(StdioEnum::Inherit),
        _ => Err(Exception::throw_type(
            ctx,
            &format!(
                "Invalid stdio \"{}\". Expected one of: pipe, ignore, inherit",
                input
            ),
        )),
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
        declare.declare("spawn")?;
        declare.declare("fork")?;
        declare.declare("exec")?;
        declare.declare("execFile")?;
        declare.declare("execSync")?;
        declare.declare("execFileSync")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        ChildProcess::add_event_emitter_prototype(ctx)?;
        DefaultWritableStream::add_writable_stream_prototype(ctx)?;
        DefaultWritableStream::add_event_emitter_prototype(ctx)?;
        DefaultReadableStream::add_readable_stream_prototype(ctx)?;
        DefaultReadableStream::add_event_emitter_prototype(ctx)?;

        export_default(ctx, exports, |default| {
            default.set("spawn", Func::from(spawn))?;
            #[cfg(unix)]
            default.set("fork", Func::from(fork))?;
            // Lazy JS helpers that call require("child_process").spawn after the
            // module is fully registered (avoids mid-evaluate require cycles).
            let exec: rquickjs::Function = ctx.eval(
                r#"(function(){
  return function exec(command, options, callback) {
    const { spawn } = require("child_process");
    if (typeof options === "function") { callback = options; options = {}; }
    options = options || {};
    const child = spawn(String(command), {
      shell: options.shell !== false,
      cwd: options.cwd,
      env: options.env,
      windowsHide: options.windowsHide,
    });
    let stdout = "", stderr = "";
    if (child.stdout) child.stdout.on("data", (c) => { stdout += String(c); });
    if (child.stderr) child.stderr.on("data", (c) => { stderr += String(c); });
    child.on("error", (err) => { if (typeof callback === "function") callback(err, stdout, stderr); });
    child.on("close", (code, signal) => {
      if (typeof callback !== "function") return;
      if (code === 0) callback(null, stdout, stderr);
      else {
        const err = new Error("Command failed: " + command + "\n" + stderr);
        err.code = code; err.signal = signal; err.stdout = stdout; err.stderr = stderr;
        callback(err, stdout, stderr);
      }
    });
    return child;
  };
})()"#,
            )?;
            let exec_file: rquickjs::Function = ctx.eval(
                r#"(function(){
  return function execFile(file, args, options, callback) {
    const { spawn } = require("child_process");
    if (typeof args === "function") { callback = args; args = []; options = {}; }
    else if (typeof options === "function") { callback = options; options = {}; }
    args = args || []; options = options || {};
    const child = spawn(file, args, { cwd: options.cwd, env: options.env, windowsHide: options.windowsHide });
    let stdout = "", stderr = "";
    if (child.stdout) child.stdout.on("data", (c) => { stdout += String(c); });
    if (child.stderr) child.stderr.on("data", (c) => { stderr += String(c); });
    child.on("error", (err) => { if (typeof callback === "function") callback(err, stdout, stderr); });
    child.on("close", (code, signal) => {
      if (typeof callback !== "function") return;
      if (code === 0) callback(null, stdout, stderr);
      else {
        const err = new Error("Command failed: " + file);
        err.code = code; err.signal = signal; err.stdout = stdout; err.stderr = stderr;
        callback(err, stdout, stderr);
      }
    });
    return child;
  };
})()"#,
            )?;
            default.set("exec", exec)?;
            default.set("execFile", exec_file)?;
            default.set("execSync", Func::from(exec_sync))?;
            default.set("execFileSync", Func::from(exec_file_sync))?;
            Ok(())
        })?;

        Ok(())
    }
}

fn exec_sync<'js>(
    ctx: Ctx<'js>,
    command: String,
    options: Opt<rquickjs::Object<'js>>,
) -> Result<String> {
    let mut shell = true;
    let mut cwd: Option<String> = None;
    if let Some(opts) = options.0 {
        if let Some(s) = opts.get_optional::<_, bool>("shell")? {
            shell = s;
        }
        cwd = opts.get_optional("cwd")?;
    }
    let mut cmd = if shell {
        #[cfg(windows)]
        {
            let mut c = StdCommand::new("cmd");
            c.arg("/C").arg(&command);
            c
        }
        #[cfg(not(windows))]
        {
            let mut c = StdCommand::new("sh");
            c.arg("-c").arg(&command);
            c
        }
    } else {
        StdCommand::new(&command)
    };
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let output = cmd
        .output()
        .map_err(|e| Exception::throw_message(&ctx, &format!("execSync failed: {e}")))?;
    if !output.status.success() {
        return Err(Exception::throw_message(
            &ctx,
            &format!(
                "Command failed: {}\n{}",
                command,
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn exec_file_sync<'js>(
    ctx: Ctx<'js>,
    file: String,
    args: Opt<Vec<String>>,
    options: Opt<rquickjs::Object<'js>>,
) -> Result<String> {
    let args = args.0.unwrap_or_default();
    let mut cmd = StdCommand::new(&file);
    cmd.args(&args);
    if let Some(opts) = options.0 {
        if let Some(cwd) = opts.get_optional::<_, String>("cwd")? {
            cmd.current_dir(cwd);
        }
    }
    let output = cmd
        .output()
        .map_err(|e| Exception::throw_message(&ctx, &format!("execFileSync failed: {e}")))?;
    if !output.status.success() {
        return Err(Exception::throw_message(
            &ctx,
            &format!(
                "Command failed: {} {}\n{}",
                file,
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

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
