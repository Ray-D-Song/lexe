// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{collections::HashMap, process::Command as StdCommand};

use lexe_buffer::Buffer;
use lexe_utils::{object::ObjectExt, result::ResultExt, signals::parse_signal};
use rquickjs::{
    convert::Coerced, prelude::This, Array, Ctx, Error, Exception, Function, IntoJs, Null, Object,
    Result, Value,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerializationMode {
    Advanced,
    Json,
}

#[derive(Clone, Debug)]
pub enum StdioSpec {
    Pipe,
    Ignore,
    Inherit,
    Fd(i32),
    Ipc,
}

#[derive(Clone)]
pub struct ParsedOptions<'js> {
    pub cwd: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub argv0: Option<String>,
    pub shell: Option<String>,
    pub detached: bool,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub windows_verbatim_arguments: bool,
    pub windows_hide: bool,
    pub timeout_ms: Option<u64>,
    pub kill_signal: i32,
    pub kill_signal_val: Option<Value<'js>>,
    pub max_buffer: usize,
    pub encoding: Option<String>,
    pub serialization: SerializationMode,
    pub signal: Option<Object<'js>>,
    pub stdio: Vec<StdioSpec>,
    pub stdio_explicit: bool,
    pub input: Option<Vec<u8>>,
}

pub fn throw_coded<'js>(ctx: &Ctx<'js>, is_type: bool, code: &str, msg: &str) -> Error {
    if is_type {
        if let Ok(ctor) = ctx.globals().get::<_, rquickjs::Constructor>("TypeError") {
            if let Ok(obj) = ctor.construct::<_, Object>((msg,)) {
                let _ = obj.set("code", code);
                return ctx.throw(obj.into_value());
            }
        }
    }
    if let Ok(ex) = Exception::from_message(ctx.clone(), msg) {
        let _ = ex.as_object().set("code", code);
        return ctx.throw(ex.into());
    }
    Exception::throw_message(ctx, msg)
}

pub fn make_abort_error<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let ex = Exception::from_message(ctx.clone(), "The operation was aborted")?;
    let obj = ex.as_object();
    obj.set("name", "AbortError")?;
    obj.set("code", "ABORT_ERR")?;
    Ok(ex.into())
}

pub fn make_errno_error<'js>(
    ctx: &Ctx<'js>,
    code: &str,
    errno: i32,
    syscall: &str,
    path: Option<&str>,
    msg: &str,
) -> Result<Value<'js>> {
    let ex = Exception::from_message(ctx.clone(), msg)?;
    let obj = ex.as_object();
    obj.set("code", code)?;
    obj.set("errno", errno)?;
    obj.set("syscall", syscall)?;
    if let Some(p) = path {
        obj.set("path", p)?;
    }
    Ok(ex.into())
}

pub fn io_error_to_errno(err: &std::io::Error) -> (&'static str, i32) {
    let raw = err.raw_os_error().map(|e| -e.abs()).unwrap_or(-1);
    let code = match err.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::TimedOut => "ETIMEDOUT",
        _ => match err.raw_os_error() {
            #[cfg(unix)]
            Some(libc::ENOENT) => "ENOENT",
            #[cfg(unix)]
            Some(libc::EACCES) | Some(libc::EPERM) => "EACCES",
            #[cfg(unix)]
            Some(libc::ENOEXEC) => "ENOEXEC",
            #[cfg(unix)]
            Some(libc::ENOTDIR) => "ENOTDIR",
            #[cfg(unix)]
            Some(libc::EBADF) => "EBADF",
            #[cfg(unix)]
            Some(libc::ETIMEDOUT) => "ETIMEDOUT",
            _ => "ERR_SYSTEM_ERROR",
        },
    };
    (code, raw)
}

#[allow(unused_variables)]
pub fn prepare_shell_args(
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

pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        "cmd.exe".to_string()
    }
    #[cfg(not(windows))]
    {
        "/bin/sh".to_string()
    }
}

pub fn parse_cmd_args_and_opts<'js>(
    ctx: &Ctx<'js>,
    cmd_val: Value<'js>,
    args_val: Option<Value<'js>>,
    opts_val: Option<Value<'js>>,
    default_shell_flag: bool,
    default_stdio: Vec<StdioSpec>,
    validate_serialization: bool,
) -> Result<(String, Option<Vec<String>>, ParsedOptions<'js>)> {
    let Some(cmd_str) = cmd_val.as_string() else {
        return Err(throw_coded(
            ctx,
            true,
            "ERR_INVALID_ARG_TYPE",
            "The \"file\" argument must be of type string",
        ));
    };
    let cmd = cmd_str.to_string()?;

    let mut command_args: Option<Vec<String>> = None;
    let mut opts_obj: Option<Object<'js>> = None;

    if let Some(a0) = args_val {
        if a0.is_array() {
            let arr = a0.clone().into_array().or_throw(ctx)?;
            let mut vec = Vec::with_capacity(arr.len());
            for item in arr.iter::<Value>() {
                let item = item?;
                let coerced: Coerced<String> = item.get()?;
                vec.push(coerced.0);
            }
            command_args = Some(vec);
            if let Some(o) = opts_val {
                if !o.is_undefined() && !o.is_null() {
                    opts_obj = o.as_object().cloned();
                    if opts_obj.is_none() {
                        return Err(throw_coded(
                            ctx,
                            true,
                            "ERR_INVALID_ARG_TYPE",
                            "The \"options\" argument must be of type object",
                        ));
                    }
                }
            }
        } else if a0.is_object() {
            opts_obj = a0.as_object().cloned();
        } else if a0.is_undefined() || a0.is_null() {
            if let Some(o) = opts_val {
                if !o.is_undefined() && !o.is_null() {
                    opts_obj = o.as_object().cloned();
                    if opts_obj.is_none() {
                        return Err(throw_coded(
                            ctx,
                            true,
                            "ERR_INVALID_ARG_TYPE",
                            "The \"options\" argument must be of type object",
                        ));
                    }
                }
            }
        } else {
            return Err(throw_coded(
                ctx,
                true,
                "ERR_INVALID_ARG_TYPE",
                "The \"args\" argument must be an instance of Array",
            ));
        }
    }

    let parsed = parse_options_object(
        ctx,
        opts_obj.as_ref(),
        default_shell_flag,
        default_stdio,
        validate_serialization,
    )?;

    Ok((cmd, command_args, parsed))
}

pub fn parse_options_object<'js>(
    ctx: &Ctx<'js>,
    opts: Option<&Object<'js>>,
    default_shell_flag: bool,
    default_stdio: Vec<StdioSpec>,
    validate_serialization: bool,
) -> Result<ParsedOptions<'js>> {
    let mut out = ParsedOptions {
        cwd: None,
        env: None,
        argv0: None,
        shell: if default_shell_flag {
            Some(default_shell())
        } else {
            None
        },
        detached: false,
        uid: None,
        gid: None,
        windows_verbatim_arguments: false,
        windows_hide: false,
        timeout_ms: None,
        kill_signal: parse_signal(None)?,
        kill_signal_val: None,
        max_buffer: 1024 * 1024,
        encoding: None,
        serialization: SerializationMode::Advanced,
        signal: None,
        stdio: default_stdio,
        stdio_explicit: false,
        input: None,
    };

    let Some(opts) = opts else {
        return Ok(out);
    };

    out.cwd = opts.get_optional("cwd")?;
    out.argv0 = opts.get_optional("argv0")?;
    out.detached = opts.get_optional("detached")?.unwrap_or(false);
    out.windows_verbatim_arguments = opts
        .get_optional("windowsVerbatimArguments")?
        .unwrap_or(false);
    out.windows_hide = opts.get_optional("windowsHide")?.unwrap_or(false);

    if let Some(uid_val) = opts.get_optional::<_, Value>("uid")? {
        if !uid_val.is_undefined() && !uid_val.is_null() {
            let Some(n) = uid_val.as_number() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options.uid\" property must be of type number",
                ));
            };
            out.uid = Some(n as u32);
        }
    }

    if let Some(gid_val) = opts.get_optional::<_, Value>("gid")? {
        if !gid_val.is_undefined() && !gid_val.is_null() {
            let Some(n) = gid_val.as_number() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options.gid\" property must be of type number",
                ));
            };
            out.gid = Some(n as u32);
        }
    }

    if let Some(shell_val) = opts.get_optional::<_, Value>("shell")? {
        if !shell_val.is_undefined() {
            if let Some(b) = shell_val.as_bool() {
                out.shell = if b { Some(default_shell()) } else { None };
            } else if let Some(s) = shell_val.as_string() {
                out.shell = Some(s.to_string()?);
            } else if shell_val.is_null() {
                out.shell = None;
            } else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options.shell\" property must be of type boolean or string",
                ));
            }
        }
    }

    if let Some(env) = opts.get_optional::<_, HashMap<String, Coerced<String>>>("env")? {
        out.env = Some(env.into_iter().map(|(k, v)| (k, v.to_string())).collect());
    }

    if let Some(timeout_val) = opts.get_optional::<_, Value>("timeout")? {
        if !timeout_val.is_undefined() && !timeout_val.is_null() {
            let Some(n) = timeout_val.as_number() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options.timeout\" property must be of type number",
                ));
            };
            if !n.is_finite() || n < 0.0 {
                return Err(throw_coded(
                    ctx,
                    false,
                    "ERR_OUT_OF_RANGE",
                    "The value of \"options.timeout\" is out of range",
                ));
            }
            let ms = n as u64;
            if ms > 0 {
                out.timeout_ms = Some(ms);
            }
        }
    }

    if let Some(ks_val) = opts.get_optional::<_, Value>("killSignal")? {
        if !ks_val.is_undefined() {
            out.kill_signal = parse_signal(Some(ks_val.clone()))?;
            out.kill_signal_val = Some(ks_val);
        }
    }

    if let Some(mb_val) = opts.get_optional::<_, Value>("maxBuffer")? {
        if !mb_val.is_undefined() {
            let Some(n) = mb_val.as_number() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options.maxBuffer\" property must be of type number",
                ));
            };
            if n.is_nan() || n < 0.0 {
                return Err(throw_coded(
                    ctx,
                    false,
                    "ERR_OUT_OF_RANGE",
                    "The value of \"options.maxBuffer\" is out of range",
                ));
            }
            out.max_buffer = if n.is_infinite() {
                usize::MAX
            } else {
                n as usize
            };
        }
    }

    if let Some(enc_val) = opts.get_optional::<_, Value>("encoding")? {
        if !enc_val.is_undefined() && !enc_val.is_null() {
            let Some(s) = enc_val.as_string() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options.encoding\" property must be of type string",
                ));
            };
            out.encoding = Some(s.to_string()?);
        }
    }

    if validate_serialization {
        if let Some(ser_val) = opts.get_optional::<_, Value>("serialization")? {
            if !ser_val.is_undefined() {
                let s = ser_val.as_string().and_then(|s| s.to_string().ok());
                match s.as_deref() {
                    Some("advanced") => out.serialization = SerializationMode::Advanced,
                    Some("json") => out.serialization = SerializationMode::Json,
                    _ => {
                        return Err(throw_coded(
                            ctx,
                            true,
                            "ERR_INVALID_ARG_VALUE",
                            "The property 'options.serialization' must be one of: undefined, 'json', 'advanced'",
                        ));
                    },
                }
            }
        }
    }

    if let Some(sig_val) = opts.get_optional::<_, Value>("signal")? {
        if !sig_val.is_undefined() {
            let Some(sig_obj) = sig_val.as_object().cloned() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options.signal\" property must be an instance of AbortSignal",
                ));
            };
            let is_signal = sig_obj
                .get::<_, Value>("aborted")
                .map(|v| v.is_bool())
                .unwrap_or(false)
                && sig_obj
                    .get::<_, Value>("addEventListener")
                    .map(|v| v.is_function())
                    .unwrap_or(false);
            if !is_signal {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options.signal\" property must be an instance of AbortSignal",
                ));
            }
            out.signal = Some(sig_obj);
        }
    }

    if let Some(stdio_val) = opts.get_optional::<_, Value>("stdio")? {
        if !stdio_val.is_undefined() {
            out.stdio_explicit = true;
            out.stdio = parse_stdio(ctx, stdio_val)?;
        }
    }

    if let Some(input_val) = opts.get_optional::<_, Value>("input")? {
        if !input_val.is_undefined() && !input_val.is_null() {
            let bytes = lexe_utils::bytes::ObjectBytes::from(ctx, &input_val)?.into_bytes(ctx)?;
            out.input = Some(bytes);
        }
    }

    Ok(out)
}

pub fn parse_serialization_value(
    ctx: &Ctx<'_>,
    val: Option<Value<'_>>,
) -> Result<SerializationMode> {
    let Some(v) = val else {
        return Ok(SerializationMode::Advanced);
    };
    if v.is_undefined() {
        return Ok(SerializationMode::Advanced);
    }
    let s = v.as_string().and_then(|s| s.to_string().ok());
    match s.as_deref() {
        Some("advanced") => Ok(SerializationMode::Advanced),
        Some("json") => Ok(SerializationMode::Json),
        _ => Err(throw_coded(
            ctx,
            true,
            "ERR_INVALID_ARG_VALUE",
            "The property 'options.serialization' must be one of: undefined, 'json', 'advanced'",
        )),
    }
}

fn parse_stdio_entry<'js>(ctx: &Ctx<'js>, item: Value<'js>) -> Result<StdioSpec> {
    if item.is_undefined() || item.is_null() {
        return Ok(StdioSpec::Pipe);
    }
    if let Some(s) = item.as_string() {
        let name = s.to_string()?;
        return match name.as_str() {
            "pipe" | "overlapped" => Ok(StdioSpec::Pipe),
            "ignore" => Ok(StdioSpec::Ignore),
            "inherit" => Ok(StdioSpec::Inherit),
            "ipc" => Ok(StdioSpec::Ipc),
            _ => Err(throw_coded(
                ctx,
                true,
                "ERR_INVALID_OPT_VALUE",
                &format!(
                    "The value \"{name}\" is invalid for option \"stdio\". Expected pipe, ignore, inherit, overlapped, or ipc"
                ),
            )),
        };
    }
    if let Some(n) = item.as_number() {
        if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
            return Err(throw_coded(
                ctx,
                true,
                "ERR_INVALID_ARG_VALUE",
                "The value of \"stdio\" fd must be a non-negative integer",
            ));
        }
        return Ok(StdioSpec::Fd(n as i32));
    }
    if let Some(obj) = item.as_object() {
        if let Ok(fd_val) = obj.get::<_, Value>("fd") {
            if let Some(n) = fd_val.as_number() {
                if n.is_finite() && n >= 0.0 && n.fract() == 0.0 {
                    return Ok(StdioSpec::Fd(n as i32));
                }
            }
        }
    }
    Err(throw_coded(
        ctx,
        true,
        "ERR_INVALID_ARG_TYPE",
        "Incorrect value for \"stdio\" option",
    ))
}

fn parse_stdio<'js>(ctx: &Ctx<'js>, stdio: Value<'js>) -> Result<Vec<StdioSpec>> {
    if let Some(s) = stdio.as_string() {
        let name = s.to_string()?;
        let spec = match name.as_str() {
            "pipe" | "overlapped" => StdioSpec::Pipe,
            "ignore" => StdioSpec::Ignore,
            "inherit" => StdioSpec::Inherit,
            _ => {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_OPT_VALUE",
                    &format!("Invalid stdio option \"{name}\""),
                ));
            },
        };
        return Ok(vec![spec.clone(), spec.clone(), spec]);
    }
    if let Some(arr) = stdio.as_array() {
        let mut list = Vec::with_capacity(arr.len().max(3));
        let mut ipc_count = 0;
        for item in arr.iter::<Value>() {
            let entry = parse_stdio_entry(ctx, item?)?;
            if matches!(entry, StdioSpec::Ipc) {
                ipc_count += 1;
                if ipc_count > 1 {
                    return Err(throw_coded(
                        ctx,
                        false,
                        "ERR_IPC_ONE_PIPE",
                        "Child process can have only one IPC pipe",
                    ));
                }
            }
            list.push(entry);
        }
        while list.len() < 3 {
            list.push(StdioSpec::Pipe);
        }
        return Ok(list);
    }
    Err(throw_coded(
        ctx,
        true,
        "ERR_INVALID_ARG_TYPE",
        "Incorrect value for \"stdio\" option",
    ))
}

#[allow(unused_variables)]
pub fn build_command(
    raw_cmd: String,
    raw_args: Option<Vec<String>>,
    opts: &ParsedOptions<'_>,
) -> (StdCommand, String, Vec<String>) {
    let mut windows_verbatim_arguments = opts.windows_verbatim_arguments;
    let (exec_cmd, final_args) = if let Some(shell) = &opts.shell {
        let args = prepare_shell_args(shell, &mut windows_verbatim_arguments, raw_cmd, raw_args);
        (shell.clone(), Some(args))
    } else {
        (raw_cmd, raw_args)
    };

    let mut spawnargs = Vec::with_capacity(1 + final_args.as_ref().map_or(0, Vec::len));
    spawnargs.push(exec_cmd.clone());
    if let Some(a) = &final_args {
        spawnargs.extend(a.iter().cloned());
    }

    let mut command = StdCommand::new(&exec_cmd);
    #[cfg(unix)]
    if let Some(argv0) = &opts.argv0 {
        command.arg0(argv0);
    }

    if let Some(args) = &final_args {
        #[cfg(windows)]
        if windows_verbatim_arguments {
            command.raw_arg(args.join(" "));
        } else {
            command.args(args);
        }
        #[cfg(not(windows))]
        command.args(args);
    }

    #[cfg(unix)]
    {
        if let Some(gid) = opts.gid {
            command.gid(gid);
        }
        if let Some(uid) = opts.uid {
            command.uid(uid);
        }
        if opts.detached {
            command.process_group(0);
        }
    }

    #[cfg(windows)]
    {
        let mut flags: u32 = 0;
        if opts.detached {
            // DETACHED_PROCESS
            flags |= 0x0000_0008;
        }
        if opts.windows_hide {
            // CREATE_NO_WINDOW
            flags |= 0x0800_0000;
        }
        if flags != 0 {
            command.creation_flags(flags);
        }
    }

    if let Some(cwd) = &opts.cwd {
        command.current_dir(cwd);
    }

    if let Some(env_map) = &opts.env {
        command.env_clear();
        command.envs(env_map);
    }

    (command, exec_cmd, spawnargs)
}

pub fn format_output_value<'js>(
    ctx: &Ctx<'js>,
    bytes: Vec<u8>,
    encoding: Option<&str>,
) -> Result<Value<'js>> {
    let enc = encoding.map(|e| e.trim().to_ascii_lowercase());
    let buf_val = Buffer(bytes).into_js(ctx)?;
    match enc.as_deref() {
        None | Some("buffer") => Ok(buf_val),
        Some(enc_str) => {
            if let Some(obj) = buf_val.as_object() {
                if let Ok(to_str) = obj.get::<_, Function>("toString") {
                    let s: Value = to_str.call((This(obj.clone()), enc_str))?;
                    return Ok(s);
                }
            }
            Ok(buf_val)
        },
    }
}

pub fn make_null<'js>(ctx: &Ctx<'js>) -> Value<'js> {
    Null.into_value(ctx.clone())
}

pub fn build_sync_output_array<'js>(
    ctx: &Ctx<'js>,
    stdin_slot: Option<Value<'js>>,
    stdout_val: Option<Vec<u8>>,
    stderr_val: Option<Vec<u8>>,
    encoding: Option<&str>,
) -> Result<(Array<'js>, Value<'js>, Value<'js>)> {
    let arr = Array::new(ctx.clone())?;
    arr.set(0, stdin_slot.unwrap_or_else(|| make_null(ctx)))?;
    let stdout_js = match stdout_val {
        Some(b) => format_output_value(ctx, b, encoding)?,
        None => make_null(ctx),
    };
    let stderr_js = match stderr_val {
        Some(b) => format_output_value(ctx, b, encoding)?,
        None => make_null(ctx),
    };
    arr.set(1, stdout_js.clone())?;
    arr.set(2, stderr_js.clone())?;
    Ok((arr, stdout_js, stderr_js))
}

#[cfg(unix)]
pub fn dup_fd_stdio(ctx: &Ctx<'_>, fd: i32) -> Result<std::process::Stdio> {
    use std::os::fd::FromRawFd;
    let duped = unsafe { libc::dup(fd) };
    if duped < 0 {
        let err = std::io::Error::last_os_error();
        let (code, errno) = io_error_to_errno(&err);
        return Err(ctx.throw(make_errno_error(
            ctx,
            code,
            errno,
            "dup",
            None,
            &format!("{code}: bad file descriptor fd={fd}"),
        )?));
    }
    Ok(unsafe { std::process::Stdio::from_raw_fd(duped) })
}

#[cfg(windows)]
pub fn dup_fd_stdio(_ctx: &Ctx<'_>, fd: i32) -> Result<std::process::Stdio> {
    use std::os::windows::io::{FromRawHandle, RawHandle};
    Ok(unsafe { std::process::Stdio::from_raw_handle(fd as isize as RawHandle) })
}

#[cfg(all(not(unix), not(windows)))]
pub fn dup_fd_stdio(ctx: &Ctx<'_>, _fd: i32) -> Result<std::process::Stdio> {
    Err(throw_coded(
        ctx,
        false,
        "ERR_NOT_IMPLEMENTED",
        "Raw fd stdio is not supported on this platform",
    ))
}

pub fn attach_abort_error_helper<'js>(
    ctx: &Ctx<'js>,
    signal: &Object<'js>,
    child_val: Value<'js>,
    kill_sig_val: Option<Value<'js>>,
) -> Result<()> {
    let hook: Function = ctx.eval(
        r#"(function (signal, child, killSig) {
  const onAbort = () => {
    try { child.kill(killSig); } catch {}
    const err = new Error("The operation was aborted");
    err.name = "AbortError";
    err.code = "ABORT_ERR";
    child.emit("error", err);
  };
  signal.addEventListener("abort", onAbort, { once: true });
  child.once("close", () => {
    try { signal.removeEventListener("abort", onAbort); } catch {}
  });
})"#,
    )?;
    let ks = kill_sig_val.unwrap_or_else(|| rquickjs::Undefined.into_value(ctx.clone()));
    hook.call::<_, ()>((signal.clone(), child_val, ks))?;
    Ok(())
}
