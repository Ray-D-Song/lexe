// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::{
    io::{self, Read, Write},
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use lexe_utils::signals::{kill_process_raw, signal_str_from_i32};
use rquickjs::{
    prelude::{Opt, Rest},
    Ctx, Exception, IntoJs, Null, Object, Result, Value,
};

use crate::options::{
    build_command, build_sync_output_array, dup_fd_stdio, io_error_to_errno, make_errno_error,
    make_null, parse_cmd_args_and_opts, ParsedOptions, StdioSpec,
};

enum SyncMode {
    Spawn,
    Exec,
    ExecFile,
}

pub fn spawn_sync<'js>(
    ctx: Ctx<'js>,
    cmd: Value<'js>,
    rest: Rest<Value<'js>>,
) -> Result<Object<'js>> {
    let args_val = rest.first().cloned();
    let opts_val = rest.get(1).cloned();
    let (raw_cmd, raw_args, opts) = parse_cmd_args_and_opts(
        &ctx,
        cmd,
        args_val,
        opts_val,
        false,
        vec![StdioSpec::Pipe, StdioSpec::Pipe, StdioSpec::Pipe],
        false,
    )?;
    run_sync(&ctx, raw_cmd, raw_args, opts, SyncMode::Spawn).map(|res| res.obj)
}

pub fn exec_sync<'js>(
    ctx: Ctx<'js>,
    cmd: Value<'js>,
    opts_val: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let (raw_cmd, raw_args, opts) = parse_cmd_args_and_opts(
        &ctx,
        cmd,
        opts_val.0,
        None,
        true,
        vec![StdioSpec::Pipe, StdioSpec::Pipe, StdioSpec::Inherit],
        false,
    )?;
    let out = run_sync(&ctx, raw_cmd, raw_args, opts, SyncMode::Exec)?;
    if let Some(err_val) = out.throw_err {
        return Err(ctx.throw(err_val));
    }
    Ok(out.stdout_val)
}

pub fn exec_file_sync<'js>(
    ctx: Ctx<'js>,
    file: Value<'js>,
    rest: Rest<Value<'js>>,
) -> Result<Value<'js>> {
    let args_val = rest.first().cloned();
    let opts_val = rest.get(1).cloned();
    let (raw_cmd, raw_args, opts) = parse_cmd_args_and_opts(
        &ctx,
        file,
        args_val,
        opts_val,
        false,
        vec![StdioSpec::Pipe, StdioSpec::Pipe, StdioSpec::Inherit],
        false,
    )?;
    let out = run_sync(&ctx, raw_cmd, raw_args, opts, SyncMode::ExecFile)?;
    if let Some(err_val) = out.throw_err {
        return Err(ctx.throw(err_val));
    }
    Ok(out.stdout_val)
}

struct SyncResult<'js> {
    obj: Object<'js>,
    stdout_val: Value<'js>,
    throw_err: Option<Value<'js>>,
}

fn spec_to_sync_stdio(ctx: &Ctx<'_>, spec: &StdioSpec, fallback_pipe: bool) -> Result<Stdio> {
    match spec {
        StdioSpec::Pipe | StdioSpec::Ipc => Ok(Stdio::piped()),
        StdioSpec::Ignore => Ok(Stdio::null()),
        StdioSpec::Inherit => Ok(if fallback_pipe {
            Stdio::piped()
        } else {
            Stdio::inherit()
        }),
        StdioSpec::Fd(fd) => dup_fd_stdio(ctx, *fd),
    }
}

fn read_pipe_capped(
    mut reader: impl Read,
    max_buffer: usize,
    overflow_flag: &AtomicBool,
    pid: Option<u32>,
    kill_signal: i32,
) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if out.len().saturating_add(n) > max_buffer {
                    overflow_flag.store(true, Ordering::SeqCst);
                    if let Some(p) = pid {
                        send_signal_raw(p, kill_signal);
                    }
                    break;
                }
                out.extend_from_slice(&buf[..n]);
            },
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

#[cfg(unix)]
fn send_signal_raw(pid: u32, signal: i32) {
    let _ = kill_process_raw(pid, signal);
}

#[cfg(not(unix))]
fn send_signal_raw(pid: u32, signal: i32) {
    let _ = lexe_utils::signals::kill_process_raw(pid, signal);
}

fn kill_child(child: &mut Child, pid: u32, kill_signal: i32) {
    #[cfg(unix)]
    {
        if kill_signal > 0 {
            let _ = kill_process_raw(pid, kill_signal);
        } else {
            let _ = child.kill();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, kill_signal);
        let _ = child.kill();
    }
}

fn run_sync<'js>(
    ctx: &Ctx<'js>,
    raw_cmd: String,
    raw_args: Option<Vec<String>>,
    opts: ParsedOptions<'js>,
    mode: SyncMode,
) -> Result<SyncResult<'js>> {
    lexe_utils::permissions::ensure_child_process(ctx)?;
    let is_exec_variant = matches!(mode, SyncMode::Exec | SyncMode::ExecFile);
    let capture_implicit_stderr = is_exec_variant && !opts.stdio_explicit;
    let stdin_spec = opts.stdio.first().unwrap_or(&StdioSpec::Pipe);
    let stdout_spec = opts.stdio.get(1).unwrap_or(&StdioSpec::Pipe);
    let stderr_spec = opts.stdio.get(2).unwrap_or(&StdioSpec::Pipe);

    let (mut command, exec_cmd, spawnargs) = build_command(raw_cmd.clone(), raw_args, &opts);

    if opts.input.is_some()
        || !matches!(
            stdin_spec,
            StdioSpec::Ignore | StdioSpec::Inherit | StdioSpec::Fd(_)
        )
    {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(spec_to_sync_stdio(ctx, stdin_spec, false)?);
    }

    let pipe_stdout = matches!(stdout_spec, StdioSpec::Pipe | StdioSpec::Ipc);
    let pipe_stderr =
        matches!(stderr_spec, StdioSpec::Pipe | StdioSpec::Ipc) || capture_implicit_stderr;

    command.stdout(spec_to_sync_stdio(ctx, stdout_spec, false)?);
    command.stderr(spec_to_sync_stdio(
        ctx,
        stderr_spec,
        capture_implicit_stderr,
    )?);

    let syscall_name = format!("spawnSync {exec_cmd}");

    let spawn_res = command.spawn();
    let mut child = match spawn_res {
        Ok(c) => c,
        Err(err) => {
            let (code, errno) = io_error_to_errno(&err);
            let msg = format!("{syscall_name} {code}");
            let err_val = make_errno_error(ctx, code, errno, &syscall_name, Some(&exec_cmd), &msg)?;

            let obj = Object::new(ctx.clone())?;
            obj.set("pid", 0)?;
            obj.set("output", make_null(ctx))?;
            obj.set("stdout", make_null(ctx))?;
            obj.set("stderr", make_null(ctx))?;
            obj.set("status", make_null(ctx))?;
            obj.set("signal", make_null(ctx))?;
            obj.set("error", err_val.clone())?;

            let throw_err = if is_exec_variant { Some(err_val) } else { None };
            return Ok(SyncResult {
                obj,
                stdout_val: make_null(ctx),
                throw_err,
            });
        },
    };

    let pid = child.id();

    if let Some(input_bytes) = opts.input {
        if let Some(mut stdin_pipe) = child.stdin.take() {
            thread::spawn(move || {
                let _ = stdin_pipe.write_all(&input_bytes);
                let _ = stdin_pipe.flush();
            });
        }
    } else {
        drop(child.stdin.take());
    }

    let overflow = Arc::new(AtomicBool::new(false));
    let max_buffer = opts.max_buffer;
    let kill_signal = opts.kill_signal;

    let stdout_handle = if pipe_stdout {
        child.stdout.take().map(|r| {
            let ov = overflow.clone();
            thread::spawn(move || read_pipe_capped(r, max_buffer, &ov, Some(pid), kill_signal))
        })
    } else {
        None
    };

    let stderr_handle = if pipe_stderr {
        child.stderr.take().map(|r| {
            let ov = overflow.clone();
            thread::spawn(move || read_pipe_capped(r, max_buffer, &ov, Some(pid), kill_signal))
        })
    } else {
        None
    };

    let mut timed_out = false;
    let deadline = opts
        .timeout_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if let Some(dl) = deadline {
                    if Instant::now() >= dl {
                        timed_out = true;
                        kill_child(&mut child, pid, kill_signal);
                        break child.wait().ok();
                    }
                }
                thread::sleep(Duration::from_millis(5));
            },
            Err(_) => break None,
        }
    };

    let stdout_bytes = if let Some(h) = stdout_handle {
        h.join()
            .ok()
            .and_then(|r| r.ok())
            .or_else(|| Some(Vec::new()))
    } else {
        None
    };
    let stderr_bytes = if let Some(h) = stderr_handle {
        h.join()
            .ok()
            .and_then(|r| r.ok())
            .or_else(|| Some(Vec::new()))
    } else {
        None
    };

    if capture_implicit_stderr {
        if let Some(ref b) = stderr_bytes {
            if !b.is_empty() {
                let _ = io::stderr().write_all(b);
            }
        }
    }

    let (status_js, signal_js, exit_code_num, signal_name_opt) = match exit_status {
        Some(st) => {
            #[cfg(unix)]
            {
                if let Some(sig) = st.signal() {
                    let sig_str = signal_str_from_i32(sig)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("SIG{sig}"));
                    (
                        Null.into_value(ctx.clone()),
                        sig_str.clone().into_js(ctx)?,
                        None,
                        Some(sig_str),
                    )
                } else {
                    let code = st.code().unwrap_or(0);
                    (
                        code.into_js(ctx)?,
                        Null.into_value(ctx.clone()),
                        Some(code),
                        None,
                    )
                }
            }
            #[cfg(not(unix))]
            {
                let code = st.code().unwrap_or(0);
                (
                    code.into_js(ctx)?,
                    Null.into_value(ctx.clone()),
                    Some(code),
                    None::<String>,
                )
            }
        },
        None => (
            Null.into_value(ctx.clone()),
            Null.into_value(ctx.clone()),
            None,
            None,
        ),
    };

    let enc = opts.encoding.as_deref();
    let (output_arr, stdout_js, stderr_js) =
        build_sync_output_array(ctx, None, stdout_bytes, stderr_bytes.clone(), enc)?;

    let error_val = if overflow.load(Ordering::SeqCst) {
        let msg = match mode {
            SyncMode::Spawn => format!("{syscall_name} ENOBUFS"),
            _ => "maxBuffer length exceeded".to_string(),
        };
        #[cfg(unix)]
        let errno = -libc::ENOBUFS;
        #[cfg(not(unix))]
        let errno = -105;
        Some(make_errno_error(
            ctx,
            "ENOBUFS",
            errno,
            &syscall_name,
            None,
            &msg,
        )?)
    } else if timed_out {
        #[cfg(unix)]
        let errno = -libc::ETIMEDOUT;
        #[cfg(not(unix))]
        let errno = -60;
        Some(make_errno_error(
            ctx,
            "ETIMEDOUT",
            errno,
            &syscall_name,
            None,
            &format!("{syscall_name} ETIMEDOUT"),
        )?)
    } else {
        None
    };

    let obj = Object::new(ctx.clone())?;
    obj.set("pid", pid)?;
    obj.set("output", output_arr.clone())?;
    obj.set("stdout", stdout_js.clone())?;
    obj.set("stderr", stderr_js.clone())?;
    obj.set("status", status_js.clone())?;
    obj.set("signal", signal_js.clone())?;
    if let Some(ref e) = error_val {
        obj.set("error", e.clone())?;
    }

    let throw_err = if is_exec_variant
        && (error_val.is_some() || exit_code_num != Some(0) || signal_name_opt.is_some())
    {
        let err_text = if let Some(ref e) = error_val {
            e.as_object()
                .and_then(|o| o.get::<_, String>("message").ok())
                .unwrap_or_else(|| format!("Command failed: {raw_cmd}"))
        } else {
            let stderr_preview = stderr_bytes
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            format!(
                "Command failed: {}\n{}",
                spawnargs.join(" "),
                stderr_preview
            )
        };

        let ex = Exception::from_message(ctx.clone(), &err_text)?;
        let eobj = ex.as_object();
        eobj.set("status", status_js)?;
        eobj.set("signal", signal_js)?;
        eobj.set("output", output_arr)?;
        eobj.set("pid", pid)?;
        eobj.set("stdout", stdout_js.clone())?;
        eobj.set("stderr", stderr_js)?;
        if let Some(ref inner_err) = error_val {
            if let Some(io_obj) = inner_err.as_object() {
                if let Ok(c) = io_obj.get::<_, Value>("code") {
                    eobj.set("code", c)?;
                }
                if let Ok(en) = io_obj.get::<_, Value>("errno") {
                    eobj.set("errno", en)?;
                }
                if let Ok(sc) = io_obj.get::<_, Value>("syscall") {
                    eobj.set("syscall", sc)?;
                }
            }
            eobj.set("error", inner_err.clone())?;
        }
        Some(ex.into())
    } else {
        None
    };

    Ok(SyncResult {
        obj,
        stdout_val: stdout_js,
        throw_err,
    })
}
