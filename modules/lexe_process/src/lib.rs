// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use std::sync::OnceLock;

use lexe_events::{Emitter, EventEmitter};
use lexe_hooking::{allocate_hook_resource_id, invoke_async_hook, HookType};
use lexe_utils::provider::ProviderType;
use lexe_utils::signals;

use lexe_utils::primordials::{BasePrimordials, Primordial};
pub use lexe_utils::sysinfo;
use lexe_utils::{
    module::ModuleInfo,
    object::Proxy,
    result::ResultExt,
    sysinfo::{ARCH, PLATFORM},
    time, VERSION,
};
#[cfg(feature = "napi")]
use rquickjs::qjs;
use rquickjs::Exception;
use rquickjs::{
    convert::Coerced,
    function::Args,
    module::{Declarations, Exports, ModuleDef},
    object::{Accessor, Property},
    prelude::{Func, Opt, Rest},
    Array, BigInt, Class, Ctx, Error, Function, IntoJs, Object, Result, Value,
};

pub static EXIT_CODE: AtomicU8 = AtomicU8::new(0);
static EXITING: AtomicBool = AtomicBool::new(false);
static PROCESS_ARGV: OnceLock<Vec<String>> = OnceLock::new();

pub static NO_WARNINGS: AtomicBool = AtomicBool::new(false);
pub static TRACE_WARNINGS: AtomicBool = AtomicBool::new(false);

pub mod lifecycle;

fn process_id() -> u32 {
    #[cfg(target_family = "wasm")]
    {
        1
    }
    #[cfg(not(target_family = "wasm"))]
    {
        std::process::id()
    }
}

pub fn set_argv(argv: Vec<String>) {
    let _ = PROCESS_ARGV.set(argv);
}

pub fn no_warnings() -> bool {
    NO_WARNINGS.load(Ordering::Relaxed)
}

fn emit_warning<'js>(_ctx: Ctx<'js>, message: String, type_or_opts: Opt<Value<'js>>) -> Result<()> {
    if no_warnings() {
        return Ok(());
    }

    let mut warning_type = "Warning".to_string();
    if let Some(value) = type_or_opts.0 {
        if let Some(s) = value.as_string() {
            warning_type = s.to_string()?;
        } else if let Some(obj) = value.as_object() {
            if let Ok(t) = obj.get::<_, String>("type") {
                warning_type = t;
            }
        }
    }

    if TRACE_WARNINGS.load(Ordering::Relaxed) {
        eprintln!(
            "(node:{pid}) [{warning_type}] {message}",
            pid = process_id()
        );
    } else {
        eprintln!(
            "(node:{pid}) [{warning_type}] {message}",
            pid = process_id()
        );
    }
    Ok(())
}

pub fn argv() -> Vec<String> {
    PROCESS_ARGV
        .get()
        .cloned()
        .unwrap_or_else(|| env::args().collect())
}

/// Node-compat identity advertised to ecosystem semver gates.
const NODE_COMPAT_VERSION: &str = "24.3.0";

const EVENT_EMITTER_METHODS: &[&str] = &[
    "on",
    "once",
    "off",
    "emit",
    "addListener",
    "removeListener",
    "prependListener",
    "prependOnceListener",
    "eventNames",
    "listenerCount",
    "listeners",
    "removeAllListeners",
];

const PROCESS_NAMED_EXPORTS: &[&str] = &[
    "env",
    "cwd",
    "chdir",
    "argv0",
    "id",
    "pid",
    "argv",
    "platform",
    "arch",
    "hrtime",
    "release",
    "version",
    "versions",
    "exitCode",
    "exit",
    "kill",
    "nextTick",
    "stdin",
    "stdout",
    "stderr",
    "emitWarning",
];

#[cfg(feature = "napi")]
const PROCESS_NAPI_EXPORTS: &[&str] = &["dlopen"];

#[cfg(unix)]
const PROCESS_UNIX_EXPORTS: &[&str] = &[
    "getuid", "getgid", "geteuid", "getegid", "setuid", "setgid", "seteuid", "setegid",
];

#[cfg(feature = "napi")]
fn dlopen<'js>(
    ctx: Ctx<'js>,
    module: Object<'js>,
    filename: String,
    flags: Opt<u32>,
) -> Result<()> {
    let exports: Object = module.get("exports")?;
    let exports_val = exports.clone().into_value().as_raw();
    let ctx_ptr = ctx.as_raw();
    let flag_bits = flags.0.unwrap_or(0);
    let new_exports = lexe_napi::dlopen::process_dlopen(ctx_ptr, exports_val, &filename, flag_bits)
        .map_err(|e| Exception::throw_message(&ctx, &e))?;
    unsafe {
        let module_val = module.clone().into_value().as_raw();
        qjs::JS_SetPropertyStr(
            ctx_ptr.as_ptr(),
            module_val,
            c"exports".as_ptr(),
            new_exports,
        );
    }
    Ok(())
}

fn next_tick<'js>(ctx: Ctx<'js>, cb: Function<'js>, args: Rest<Value<'js>>) -> Result<()> {
    let uid = allocate_hook_resource_id();
    invoke_async_hook(&ctx, HookType::Init, ProviderType::TickObject, uid)?;

    // Wrap callback so BEFORE/AFTER/destroy run around the deferred tick job.
    // Keep true nextTick scheduling via defer (not Promise).
    let wrap: Function = ctx.eval(
        r#"(function (cb, uid, argList) {
  return function () {
    var inv = globalThis.invokeAsyncHook;
    if (typeof inv === "function") {
      inv("before", "", uid);
      try {
        return cb.apply(null, argList);
      } finally {
        inv("after", "", uid);
        inv("destroy", "TickObject", uid);
      }
    }
    return cb.apply(null, argList);
  };
})"#,
    )?;
    let arg_arr = Array::new(ctx.clone())?;
    for (i, arg) in args.0.into_iter().enumerate() {
        arg_arr.set(i, arg)?;
    }
    let wrapped: Function = wrap.call((cb, uid as f64, arg_arr))?;
    wrapped.defer::<()>(())?;
    Ok(())
}

fn cwd(ctx: Ctx<'_>) -> Result<String> {
    env::current_dir()
        .or_throw(&ctx)
        .map(|path| path.to_string_lossy().to_string())
}

/// Map `std::io::ErrorKind` to a Node-style system error code for `chdir`.
fn io_error_code(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::NotADirectory => "ENOTDIR",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        _ => "UNKNOWN",
    }
}

fn create_chdir_error<'js>(
    ctx: &Ctx<'js>,
    error: std::io::Error,
    path: &str,
) -> Result<Exception<'js>> {
    let code = io_error_code(&error);
    // Stable Node-like format: "ENOENT: <os message>, chdir '<path>'"
    let message = format!("{code}: {error}, chdir '{path}'");
    let exception = Exception::from_message(ctx.clone(), &message)?;
    exception.as_object().set("code", code)?;
    exception.as_object().set("path", path)?;
    exception.as_object().set("syscall", "chdir")?;
    Ok(exception)
}

/// Change the process working directory.
///
/// Accepts a JS `Value` so missing / non-string arguments raise TypeError
/// without implicit `String(value)` coercion (Node `ERR_INVALID_ARG_TYPE`).
fn chdir(ctx: Ctx<'_>, path: Opt<Value<'_>>) -> Result<()> {
    let Some(value) = path.0 else {
        return Err(Exception::throw_type(
            &ctx,
            "The \"directory\" argument must be of type string",
        ));
    };

    let Some(js_string) = value.as_string() else {
        return Err(Exception::throw_type(
            &ctx,
            "The \"directory\" argument must be of type string",
        ));
    };

    let path = js_string.to_string()?;

    match env::set_current_dir(&path) {
        Ok(()) => Ok(()),
        Err(error) => Err(create_chdir_error(&ctx, error, &path)?.throw()),
    }
}

fn hr_time_big_int(ctx: Ctx<'_>) -> Result<BigInt<'_>> {
    let now = time::now_nanos();
    let started = time::origin_nanos();

    let elapsed = now.saturating_sub(started);

    BigInt::from_u64(ctx, elapsed)
}

fn hr_time(ctx: Ctx<'_>) -> Result<Array<'_>> {
    let now = time::now_nanos();
    let started = time::origin_nanos();
    let elapsed = now.saturating_sub(started);

    let seconds = elapsed / 1_000_000_000;
    let remaining_nanos = elapsed % 1_000_000_000;

    let array = Array::new(ctx)?;

    array.set(0, seconds)?;
    array.set(1, remaining_nanos)?;

    Ok(array)
}

fn to_exit_code(ctx: &Ctx<'_>, code: &Value<'_>) -> Result<Option<u8>> {
    if let Ok(code) = code.get::<Coerced<f64>>() {
        let code = code.0;
        let code: u8 = if code.fract() != 0.0 {
            return Err(Exception::throw_range(
                ctx,
                "The value of 'code' must be an integer",
            ));
        } else {
            (code as i32).rem_euclid(256) as u8
        };
        return Ok(Some(code));
    }
    Ok(None)
}

/// Synchronously emit `process` `"exit"` without terminating the process.
pub(crate) fn emit_exit<'js>(ctx: &Ctx<'js>, process: &Object<'js>, code: u8) -> Result<()> {
    let emit: Function = process.get("emit")?;
    let mut args = Args::new(ctx.clone(), 2);
    args.this(process.clone())?;
    args.push_arg("exit")?;
    args.push_arg(code)?;
    emit.call_arg::<()>(args)?;
    Ok(())
}

fn exit(ctx: Ctx<'_>, code: Value<'_>) -> Result<()> {
    let code = match to_exit_code(&ctx, &code)? {
        Some(code) => code,
        None => EXIT_CODE.load(Ordering::Relaxed),
    };

    // Prevent recursive `process.exit()` from exit listeners re-emitting.
    if EXITING.swap(true, Ordering::SeqCst) {
        std::process::exit(code.into());
    }

    let process: Object = ctx.globals().get("process")?;
    let _ = emit_exit(&ctx, &process, code);
    std::process::exit(code.into())
}

fn env_proxy_setter<'js>(
    target: Object<'js>,
    prop: Value<'js>,
    value: Coerced<String>,
) -> Result<bool> {
    target.set(prop, value.to_string())?;
    Ok(true)
}

#[cfg(unix)]
fn getuid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(unix)]
fn getgid() -> u32 {
    unsafe { libc::getgid() }
}

#[cfg(unix)]
fn geteuid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn getegid() -> u32 {
    unsafe { libc::getegid() }
}

#[cfg(unix)]
fn setuid(id: u32) -> i32 {
    unsafe { libc::setuid(id) }
}

#[cfg(unix)]
fn setgid(id: u32) -> i32 {
    unsafe { libc::setgid(id) }
}

#[cfg(unix)]
fn seteuid(id: u32) -> i32 {
    unsafe { libc::seteuid(id) }
}

#[cfg(unix)]
fn setegid(id: u32) -> i32 {
    unsafe { libc::setegid(id) }
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    BasePrimordials::init(ctx)?;

    // Register EventEmitter class/prototype without exposing a global constructor.
    let _ctor = Class::<EventEmitter>::create_constructor(ctx)?
        .expect("Can't create EventEmitter constructor");
    EventEmitter::add_event_emitter_prototype(ctx)?;

    // process is an EventEmitter instance; methods come from the prototype
    // (not copied as own enumerable properties).
    let process_class = Class::instance(ctx.clone(), EventEmitter::new())?;
    let process = Object::from_value(process_class.into_value())?;

    let process_versions = Object::new(ctx.clone())?;
    process_versions.set("lexe", VERSION)?;
    process_versions.set("node", NODE_COMPAT_VERSION)?;
    process_versions.set("sqlite", "3.50.1")?;

    let hr_time = Function::new(ctx.clone(), hr_time)?;
    hr_time.set("bigint", Func::from(hr_time_big_int))?;

    let release = Object::new(ctx.clone())?;
    release.prop("name", Property::from("lexe").enumerable())?;

    let env_map: HashMap<String, String> = env::vars().collect();
    let mut args: Vec<String> = argv();

    if let Some(arg) = args.get(1) {
        if arg == "-e" || arg == "--eval" {
            args.remove(1);
            args.remove(1);
        }
    }

    let env_obj = env_map.into_js(ctx)?;

    let env_proxy = Proxy::with_target(ctx.clone(), env_obj)?;
    env_proxy.setter(Func::from(env_proxy_setter))?;

    process.set("env", env_proxy)?;
    process.set("cwd", Func::from(cwd))?;
    process.set("chdir", Func::from(chdir))?;
    process.set("argv0", args.clone().first().cloned().unwrap_or_default())?;
    // Lexe-legacy name kept for compatibility (ordinary writable data property).
    // Prefer process.pid for Node compatibility.
    process.set("id", process_id())?;
    // Node-standard pid: enumerable, non-writable, configurable.
    process.prop(
        "pid",
        Property::from(process_id()).enumerable().configurable(),
    )?;
    process.set("argv", args)?;
    process.set("platform", PLATFORM)?;
    process.set("arch", ARCH)?;
    process.set("hrtime", hr_time)?;
    process.set("release", release)?;
    process.set("version", format!("v{NODE_COMPAT_VERSION}"))?;
    process.set("versions", process_versions)?;

    process.prop(
        "exitCode",
        Accessor::new(
            |ctx| {
                struct Args<'js>(Ctx<'js>);
                let Args(ctx) = Args(ctx);
                ctx.globals().get::<_, Value>("__exitCode")
            },
            |ctx, code| {
                struct Args<'js>(Ctx<'js>, Value<'js>);
                let Args(ctx, code) = Args(ctx, code);
                if let Some(code) = to_exit_code(&ctx, &code)? {
                    EXIT_CODE.store(code, Ordering::Relaxed);
                }
                ctx.globals().set("__exitCode", code)?;
                Ok::<_, Error>(())
            },
        )
        .configurable()
        .enumerable(),
    )?;
    process.set("exit", Func::from(exit))?;
    process.set("emitWarning", Func::from(emit_warning))?;
    process.set(
        "kill",
        Func::from(|ctx, pid, signal| signals::kill(&ctx, pid, signal)),
    )?;

    #[cfg(unix)]
    {
        process.set("getuid", Func::from(getuid))?;
        process.set("getgid", Func::from(getgid))?;
        process.set("geteuid", Func::from(geteuid))?;
        process.set("getegid", Func::from(getegid))?;
        process.set("setuid", Func::from(setuid))?;
        process.set("setgid", Func::from(setgid))?;
        process.set("seteuid", Func::from(seteuid))?;
        process.set("setegid", Func::from(setegid))?;
    }

    process.set("nextTick", Func::from(next_tick))?;

    #[cfg(feature = "napi")]
    {
        process.set("dlopen", Func::from(dlopen))?;
    }

    // Real TTY/stdio streams (Node-compatible ReadStream / WriteStream).
    process.set("stdin", lexe_tty::ReadStream::new(ctx.clone(), 0)?)?;
    process.set("stdout", lexe_tty::WriteStream::new(ctx.clone(), 1)?)?;
    process.set("stderr", lexe_tty::WriteStream::new(ctx.clone(), 2)?)?;

    globals.set("process", process)?;

    Ok(())
}

pub struct ProcessModule;

impl ModuleDef for ProcessModule {
    fn declare(declare: &Declarations) -> Result<()> {
        for &name in PROCESS_NAMED_EXPORTS {
            declare.declare(name)?;
        }

        #[cfg(feature = "napi")]
        {
            for &name in PROCESS_NAPI_EXPORTS {
                declare.declare(name)?;
            }
        }

        for &name in EVENT_EMITTER_METHODS {
            declare.declare(name)?;
        }

        #[cfg(unix)]
        {
            for &name in PROCESS_UNIX_EXPORTS {
                declare.declare(name)?;
            }
        }

        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let globals = ctx.globals();
        let process: Object = globals.get("process")?;

        // Only export the fixed declared surface. Do not iterate runtime keys —
        // user-added enumerable properties must not break module evaluation.
        for &name in PROCESS_NAMED_EXPORTS {
            let value: Value = process.get(name)?;
            exports.export(name, value)?;
        }
        #[cfg(feature = "napi")]
        {
            for &name in PROCESS_NAPI_EXPORTS {
                let value: Value = process.get(name)?;
                exports.export(name, value)?;
            }
        }
        for &name in EVENT_EMITTER_METHODS {
            let value: Value = process.get(name)?;
            exports.export(name, value)?;
        }
        #[cfg(unix)]
        {
            for &name in PROCESS_UNIX_EXPORTS {
                let value: Value = process.get(name)?;
                exports.export(name, value)?;
            }
        }

        exports.export("default", process)?;

        Ok(())
    }
}

impl From<ProcessModule> for ModuleInfo<ProcessModule> {
    fn from(val: ProcessModule) -> Self {
        ModuleInfo {
            name: "process",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    use super::*;

    #[tokio::test]
    async fn test_hr_time() {
        time::init();
        test_async_with(|ctx| {
            Box::pin(async move {
                init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<ProcessModule>(ctx.clone(), "process")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { hrtime } from 'process';

                        export async function test() {
                            // TODO: Delaying with setTimeout
                            for(let i=0; i < (1<<20); i++){}
                            return hrtime()

                        }
                    "#,
                )
                .await
                .unwrap();
                let result = call_test::<Vec<u32>, _>(&ctx, &module, ()).await;
                assert_eq!(result.len(), 2);
                assert_eq!(result[0], 0);
                assert!(result[1] > 0);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_hr_time_bigint() {
        time::init();
        test_async_with(|ctx| {
            Box::pin(async move {
                init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<ProcessModule>(ctx.clone(), "process")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { hrtime } from 'process';

                        export async function test() {
                            // TODO: Delaying with setTimeout
                            for(let i=0; i < (1<<20); i++){}
                            return hrtime.bigint()

                        }
                    "#,
                )
                .await
                .unwrap();
                let result = call_test::<Coerced<i64>, _>(&ctx, &module, ()).await;
                assert!(result.0 > 0);
            })
        })
        .await;
    }
}
