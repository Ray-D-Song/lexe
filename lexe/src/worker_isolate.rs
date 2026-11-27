// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use lexe_core::modules::module::register_worker::{
    set_module_hooks_worker_runner, HookLoadOutput, HookResolveOutput, HookSourceData,
    HooksRequest, ModuleHooksWorkerStart, NextReply, RegisterOutcome, WorkerRpcMsg,
};
use lexe_core::modules::module_builder::{ModuleBuilder, ModuleBuilderConfig};
use lexe_core::vm::{Vm, VmOptions};
use lexe_utils::clone_wire::{deserialize, CloneWirePayload};
use lexe_worker_threads::{
    attach_thread_inbox, set_worker_isolate_runner, set_worker_thread_id, unregister_thread_inbox,
    worker_write_stderr_hook, worker_write_stdout_hook, MainToWorker, WorkerEnvMode,
    WorkerIsolateStart, WorkerToMain,
};
use rquickjs::{
    prelude::Func, CatchResultExt, CaughtError, Ctx, Function, Object, Promise, TypedArray,
};

use url::Url;

pub fn register() {
    set_worker_isolate_runner(run_worker_isolate);
    set_module_hooks_worker_runner(run_module_hooks_worker);
}

fn run_worker_isolate(start: WorkerIsolateStart) {
    set_worker_thread_id(start.thread_id);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("worker tokio runtime");
    if let Err(err) = rt.block_on(run_worker_isolate_async(start)) {
        eprintln!("worker isolate error: {err}");
    }
}

async fn run_worker_isolate_async(start: WorkerIsolateStart) -> Result<(), String> {
    let mut vm_opts = VmOptions {
        module_builder: ModuleBuilder::with_config(ModuleBuilderConfig {
            sqlite_enabled: false,
        })
        .with_global(lexe_core::modules::embedded::init)
        .with_global(lexe_core::builtins_inspect::init)
        .with_global(lexe_worker_threads::init)
        .with_module(lexe_core::modules::lexe::hex::LexeHexModule)
        .with_module(lexe_core::modules::lexe::qjs::LexeQjsModule)
        .with_module(lexe_core::modules::lexe::util::LexeUtilModule)
        .with_module(lexe_core::modules::lexe::xml::LexeXmlModule),
        ..VmOptions::default()
    };
    if let Some(stack_mb) = start.resource_limits.stack_size_mb {
        if stack_mb > 0.0 && stack_mb.is_finite() {
            vm_opts.max_stack_size = (stack_mb * 1024.0 * 1024.0) as usize;
        }
    }
    let vm = Vm::from_options(vm_opts).await.map_err(|e| e.to_string())?;

    let alive_ref = vm
        .ctx
        .with(|ctx| {
            let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
            lexe_utils::event_loop::EventLoopRef::new_referenced(
                lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void),
            )
        })
        .await;

    let to_main = start.to_main.clone();
    let to_main_init = to_main.clone();
    let mut from_main = start.from_main;
    let thread_id = start.thread_id;
    let env_mode = start.env_mode;
    let piped_stdin = start.piped_stdin;
    let piped_stdout = start.piped_stdout;
    let piped_stderr = start.piped_stderr;
    let resource_limits = start.resource_limits;
    let thread_inbox = start.thread_inbox;

    let init_result: Result<bool, String> = vm
        .ctx
        .async_with(async move |ctx| {
            let to_main = to_main_init;
            lexe_worker_threads::init(&ctx).map_err(|e| e.to_string())?;
            attach_thread_inbox(&ctx, thread_id, thread_inbox);

            let worker_data = deserialize(&ctx, &start.worker_data).map_err(|e| e.to_string())?;
            let env_data = deserialize(&ctx, &start.env_snapshot).map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__lexeWorkerBootstrapData", worker_data)
                .map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__lexeWorkerEnvironmentSnapshot", env_data)
                .map_err(|e| e.to_string())?;

            let rl_obj = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
            rl_obj
                .set(
                    "maxYoungGenerationSizeMb",
                    resource_limits.max_young_generation_size_mb.unwrap_or(16.0),
                )
                .map_err(|e| e.to_string())?;
            rl_obj
                .set(
                    "maxOldGenerationSizeMb",
                    resource_limits.max_old_generation_size_mb.unwrap_or(2048.0),
                )
                .map_err(|e| e.to_string())?;
            rl_obj
                .set(
                    "codeRangeSizeMb",
                    resource_limits.code_range_size_mb.unwrap_or(0.0),
                )
                .map_err(|e| e.to_string())?;
            rl_obj
                .set(
                    "stackSizeMb",
                    resource_limits.stack_size_mb.unwrap_or(4.0),
                )
                .map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__lexeWorkerResourceLimits", rl_obj)
                .map_err(|e| e.to_string())?;

            match env_mode {
                WorkerEnvMode::Shared(shared_map) => {
                    lexe_utils::env_store::replace_env_map(&ctx, shared_map);
                },
                WorkerEnvMode::Snapshot(map) => {
                    lexe_utils::env_store::replace_env_map(
                        &ctx,
                        std::sync::Arc::new(std::sync::RwLock::new(map)),
                    );
                },
            }

            lexe_worker_threads::set_worker_parent_channel(to_main.clone());
            ctx.globals()
                .set(
                    "__lexeWorkerParentPostMessage",
                    Func::from(lexe_worker_threads::worker_parent_post_message_hook),
                )
                .map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__lexeWorkerWriteStdout", Func::from(worker_write_stdout_hook))
                .map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__lexeWorkerWriteStderr", Func::from(worker_write_stderr_hook))
                .map_err(|e| e.to_string())?;

            let stdio_setup = format!(
                r#"
if ({piped_stdout} || {piped_stderr} || {piped_stdin}) {{
  const loadMod = (id) =>
    typeof require === "function" ? require(id) : process.getBuiltinModule(id);
  const streamMod = loadMod("node:stream");
  if ({piped_stdout}) {{
    const outStream = new streamMod.Writable({{
      write(chunk, _enc, cb) {{
        globalThis.__lexeWorkerWriteStdout(chunk);
        cb();
      }},
    }});
    Object.defineProperty(process, "stdout", {{ value: outStream, writable: true, configurable: true }});
  }}
  if ({piped_stderr}) {{
    const errStream = new streamMod.Writable({{
      write(chunk, _enc, cb) {{
        globalThis.__lexeWorkerWriteStderr(chunk);
        cb();
      }},
    }});
    Object.defineProperty(process, "stderr", {{ value: errStream, writable: true, configurable: true }});
  }}
  if ({piped_stdout} || {piped_stderr}) {{
    const consoleMod = loadMod("node:console");
    globalThis.console = new consoleMod.Console({{ stdout: process.stdout, stderr: process.stderr }});
  }}
  if ({piped_stdin}) {{
    const inStream = new streamMod.PassThrough();
    const stdinRef = typeof globalThis.__lexeCreateLoopRefController === "function"
      ? globalThis.__lexeCreateLoopRefController()
      : {{ ref() {{}}, unref() {{}} }};
    stdinRef.ref();
    inStream.once("end", () => queueMicrotask(() => stdinRef.unref()));
    inStream.once("close", () => queueMicrotask(() => stdinRef.unref()));
    globalThis.__lexeWorkerStdinStream = inStream;
    Object.defineProperty(process, "stdin", {{ value: inStream, writable: true, configurable: true }});
  }}
}}
"#,
            );

            if let Some(old_mb) = resource_limits.max_old_generation_size_mb {
                if old_mb > 0.0 && old_mb.is_finite() {
                    let bytes = (old_mb * 1024.0 * 1024.0) as usize;
                    unsafe {
                        let rt = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                        rquickjs::qjs::JS_SetMemoryLimit(rt, bytes as _);
                    }
                }
            }

            let body = if start.eval {
                let c_script = std::ffi::CString::new(start.script.as_str())
                    .map_err(|e| format!("invalid NUL byte in worker script: {e}"))?;
                let is_esm = unsafe {
                    rquickjs::qjs::JS_DetectModule(c_script.as_ptr(), start.script.len() as _)
                };
                if is_esm {
                    let data_url =
                        format!("data:text/javascript;base64,{}", encode_base64(start.script.as_bytes()));
                    format!("await import({data_url:?});")
                } else {
                    start.script.clone()
                }
            } else if start.script.len() >= 5 && start.script[..5].eq_ignore_ascii_case("data:") {
                format!("await import({:?});", start.script)
            } else {
                let script_path = resolve_script(&ctx, &start.script)?;
                let import_url = Url::from_file_path(&script_path)
                    .map_err(|_| format!("Path is not absolute: {script_path}"))?;
                format!("await import({:?});", import_url.as_str())
            };

            let bootstrap = format!(
                r#"
globalThis.workerData = globalThis.__lexeWorkerBootstrapData;
if (typeof globalThis.__lexeSetWorkerData === "function") {{
  globalThis.__lexeSetWorkerData(globalThis.workerData);
}}
delete globalThis.__lexeWorkerBootstrapData;
(function () {{
  const snap = globalThis.__lexeWorkerEnvironmentSnapshot;
  if (snap && typeof snap === "object") {{
    const m = (globalThis.__lexeWorkerEnvBootstrap = new Map());
    for (const [k, v] of Object.entries(snap)) m.set(k, v);
  }}
  delete globalThis.__lexeWorkerEnvironmentSnapshot;
}})();
{stdio_setup}
"#
            );

            if let Err(err) = ctx.eval::<(), _>(bootstrap).catch(&ctx) {
                let message = match err {
                    CaughtError::Exception(exc) => exc.to_string(),
                    CaughtError::Value(val) => format!("Uncaught {val:?}"),
                    CaughtError::Error(e) => e.to_string(),
                };
                let _ = to_main.send(WorkerToMain::Error(message.clone()));
                let _ = to_main.send(WorkerToMain::Exit {
                    code: 1,
                    signal: None,
                });
                return Err(message);
            }

            let mut opts = rquickjs::context::EvalOptions::default();
            opts.promise = true;
            let promise = match ctx.eval_with_options::<Promise, _>(body, opts).catch(&ctx) {
                Ok(p) => p,
                Err(err) => {
                    let message = match err {
                        CaughtError::Exception(exc) => exc.to_string(),
                        CaughtError::Value(val) => format!("Uncaught {val:?}"),
                        CaughtError::Error(e) => e.to_string(),
                    };
                    let _ = to_main.send(WorkerToMain::Error(message.clone()));
                    let _ = to_main.send(WorkerToMain::Exit {
                        code: 1,
                        signal: None,
                    });
                    return Err(message);
                },
            };
            if let Err(err) = promise.into_future::<()>().await.catch(&ctx) {
                let message = match err {
                    CaughtError::Exception(exc) => exc.to_string(),
                    CaughtError::Value(val) => format!("Uncaught {val:?}"),
                    CaughtError::Error(e) => e.to_string(),
                };
                let _ = to_main.send(WorkerToMain::Error(message.clone()));
                let _ = to_main.send(WorkerToMain::Exit {
                    code: 1,
                    signal: None,
                });
                return Err(message);
            }

            let _ = to_main.send(WorkerToMain::Online);

            Ok(start.eval)
        })
        .await;

    let rt_key = alive_ref.rt();
    let idle_watch = Arc::clone(&alive_ref);
    let run_result = match init_result {
        Ok(is_eval) => {
            if is_eval {
                // `eval` workers drain their event loop naturally and exit once
                // all referenced handles (timers, piped stdin, workerMessage) finish.
                alive_ref.unreference();
            }
            let mut has_pending_tasks = true;
            loop {
                if has_pending_tasks {
                    tokio::select! {
                        _ = vm.idle() => {
                            if is_eval && !lexe_utils::event_loop::has_refed_handles(rt_key) {
                                let _ = to_main.send(WorkerToMain::Exit {
                                    code: 0,
                                    signal: None,
                                });
                                break;
                            }
                            has_pending_tasks = false;
                        }
                        ev = from_main.recv() => {
                            if handle_main_event(&vm, &to_main, ev).await {
                                break;
                            }
                        }
                    }
                } else if is_eval && !lexe_utils::event_loop::has_refed_handles(rt_key) {
                    let _ = to_main.send(WorkerToMain::Exit {
                        code: 0,
                        signal: None,
                    });
                    break;
                } else {
                    tokio::select! {
                        _ = lexe_utils::event_loop::wait_until_io_unref(Arc::clone(&idle_watch)), if is_eval => {
                            has_pending_tasks = true;
                        }
                        ev = from_main.recv() => {
                            if handle_main_event(&vm, &to_main, ev).await {
                                break;
                            }
                            has_pending_tasks = true;
                        }
                    }
                }
            }
            if !is_eval {
                alive_ref.unreference();
            }
            drop(alive_ref);
            Ok(())
        },
        Err(e) => {
            alive_ref.unreference();
            drop(alive_ref);
            Err(e)
        },
    };

    unregister_thread_inbox(thread_id);
    let _ = vm.idle().await;
    vm.ctx
        .with(|ctx| unsafe {
            let rt = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
            rquickjs::qjs::JS_SetMemoryLimit(rt, 0);
        })
        .await;
    vm.shutdown().await;
    run_result
}

async fn handle_main_event(
    vm: &Vm,
    to_main: &tokio::sync::mpsc::UnboundedSender<WorkerToMain>,
    ev: Option<MainToWorker>,
) -> bool {
    match ev {
        Some(MainToWorker::Terminate) => {
            let _ = to_main.send(WorkerToMain::Exit {
                code: 0,
                signal: None,
            });
            true
        },
        Some(MainToWorker::Message(wire)) => {
            vm.ctx
                .with(|ctx| {
                    if let Ok(v) = deserialize(&ctx, &wire) {
                        let _ = ctx
                            .globals()
                            .get::<_, Function>("__lexeWorkerDispatchParentMessage")
                            .and_then(|f| f.call::<_, ()>((v,)));
                    }
                })
                .await;
            false
        },
        Some(MainToWorker::EnvPatch { key, value }) => {
            vm.ctx
                .with(|ctx| {
                    if let (Ok(k), Ok(v)) = (deserialize(&ctx, &key), deserialize(&ctx, &value)) {
                        let _ = ctx
                            .globals()
                            .get::<_, Function>("__lexeWorkerApplyEnvPatch")
                            .and_then(|f| f.call::<_, ()>((k, v)));
                    }
                })
                .await;
            false
        },
        Some(MainToWorker::StdinData(bytes)) => {
            vm.ctx
                .with(|ctx| {
                    if let Ok(Some(stream)) = ctx
                        .globals()
                        .get::<_, Option<Object>>("__lexeWorkerStdinStream")
                    {
                        if let Ok(arr) = TypedArray::<u8>::new_copy(ctx.clone(), &bytes) {
                            if let Ok(push_fn) = stream.get::<_, Function>("write") {
                                let _ = push_fn.call::<_, rquickjs::Value>((
                                    rquickjs::prelude::This(stream),
                                    arr.into_value(),
                                ));
                            }
                        }
                    }
                })
                .await;
            false
        },
        Some(MainToWorker::StdinEnd) => {
            vm.ctx
                .with(|ctx| {
                    if let Ok(Some(stream)) = ctx
                        .globals()
                        .get::<_, Option<Object>>("__lexeWorkerStdinStream")
                    {
                        if let Ok(end_fn) = stream.get::<_, Function>("end") {
                            let _ = end_fn
                                .call::<_, rquickjs::Value>((rquickjs::prelude::This(stream),));
                        }
                    }
                })
                .await;
            false
        },
        None => true,
    }
}

fn resolve_script(_ctx: &Ctx<'_>, script: &str) -> Result<String, String> {
    if let Some(path) = script.strip_prefix("file://") {
        return Ok(path.to_string());
    }
    let path = std::path::PathBuf::from(script);
    if path.is_absolute() {
        return Ok(script.to_string());
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    Ok(cwd.join(path).to_string_lossy().into_owned())
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        out.push(TABLE[b0 >> 2] as char);
        out.push(TABLE[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((b1 & 0x0f) << 2) | (b2 >> 6)] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[b2 & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

fn run_module_hooks_worker(start: ModuleHooksWorkerStart) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("module hooks worker tokio runtime");
    rt.block_on(run_module_hooks_worker_async(start));
}

async fn run_module_hooks_worker_async(mut start: ModuleHooksWorkerStart) {
    let vm_opts = VmOptions {
        module_builder: ModuleBuilder::with_config(ModuleBuilderConfig {
            sqlite_enabled: false,
        })
        .with_global(lexe_core::modules::embedded::init)
        .with_global(lexe_core::builtins_inspect::init)
        .with_global(lexe_worker_threads::init)
        .with_module(lexe_core::modules::lexe::hex::LexeHexModule)
        .with_module(lexe_core::modules::lexe::qjs::LexeQjsModule)
        .with_module(lexe_core::modules::lexe::util::LexeUtilModule)
        .with_module(lexe_core::modules::lexe::xml::LexeXmlModule),
        ..VmOptions::default()
    };
    let vm = match Vm::from_options(vm_opts).await {
        Ok(vm) => vm,
        Err(err) => {
            let _ = start.ready_tx.send(Err(err.to_string()));
            return;
        },
    };

    let init_err = vm
        .ctx
        .with(|ctx| {
            lexe_worker_threads::init(&ctx).map_err(|e| e.to_string())?;
            ctx.eval::<(), _>(
                r#"
                globalThis.__lexeHooksRegistry = new Map();
                globalThis.__lexeNextHookId = 1;
                "#,
            )
            .map_err(|e| e.to_string())?;
            Ok::<_, String>(())
        })
        .await;
    if let Err(err) = init_err {
        let _ = start.ready_tx.send(Err(err));
        vm.shutdown().await;
        return;
    }

    let _ = start.ready_tx.send(Ok(()));

    let mut has_pending = true;
    loop {
        tokio::select! {
            _ = vm.idle(), if has_pending => {
                has_pending = false;
            }
            req = start.from_main.recv() => {
                match req {
                    None | Some(HooksRequest::Shutdown) => break,
                    Some(req) => {
                        dispatch_hooks_request(&vm, req).await;
                        has_pending = true;
                    },
                }
            }
        }
    }

    let _ = vm.idle().await;
    vm.shutdown().await;
}

fn worker_err_msg(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    if ctx.has_exception() {
        let exc = ctx.catch();
        if let Some(obj) = exc.as_object() {
            if let Ok(msg) = obj.get::<_, String>("message") {
                return msg;
            }
        }
        return format!("{exc:?}");
    }
    err.to_string()
}

async fn dispatch_hooks_request(vm: &Vm, req: HooksRequest) {
    vm.ctx
        .with(|ctx| match req {
            HooksRequest::Register {
                hook_url,
                data_wire,
                reply,
            } => {
                let c = ctx.clone();
                ctx.spawn(async move {
                    let res = handle_worker_register(&c, hook_url, data_wire).await;
                    let _ = reply.send(res);
                });
            },
            HooksRequest::Resolve {
                hook_id,
                specifier,
                parent_url,
                conditions,
                import_attributes,
                rpc_tx,
            } => {
                let c = ctx.clone();
                ctx.spawn(async move {
                    let res = handle_worker_resolve(
                        &c,
                        hook_id,
                        specifier,
                        parent_url,
                        conditions,
                        import_attributes,
                        rpc_tx.clone(),
                    )
                    .await;
                    let _ = rpc_tx.send(WorkerRpcMsg::ResolveDone(res));
                });
            },
            HooksRequest::Load {
                hook_id,
                url,
                format,
                import_attributes,
                rpc_tx,
            } => {
                let c = ctx.clone();
                ctx.spawn(async move {
                    let res = handle_worker_load(
                        &c,
                        hook_id,
                        url,
                        format,
                        import_attributes,
                        rpc_tx.clone(),
                    )
                    .await;
                    let _ = rpc_tx.send(WorkerRpcMsg::LoadDone(res));
                });
            },
            HooksRequest::Shutdown => {},
        })
        .await;
}

async fn handle_worker_register<'js>(
    ctx: &rquickjs::Ctx<'js>,
    hook_url: String,
    data_wire: CloneWirePayload,
) -> Result<RegisterOutcome, String> {
    let data = deserialize(ctx, &data_wire).map_err(|e| e.to_string())?;
    ctx.globals()
        .set("__lexeHookInitData", data)
        .map_err(|e| worker_err_msg(ctx, e))?;

    let body = format!(
        r#"
        (async () => {{
            const mod = await import({hook_url:?});
            const initData = globalThis.__lexeHookInitData;
            delete globalThis.__lexeHookInitData;
            if (typeof mod.initialize === "function") {{
                await mod.initialize(initData);
            }}
            const id = globalThis.__lexeNextHookId++;
            globalThis.__lexeHooksRegistry.set(id, {{
                resolve: typeof mod.resolve === "function" ? mod.resolve : null,
                load: typeof mod.load === "function" ? mod.load : null,
            }});
            return {{
                hookId: id,
                hasResolve: typeof mod.resolve === "function",
                hasLoad: typeof mod.load === "function",
            }};
        }})()
        "#
    );

    let promise = ctx
        .eval::<Promise<'js>, _>(body)
        .map_err(|e| worker_err_msg(ctx, e))?;
    let out_val: rquickjs::Value<'js> = promise
        .into_future()
        .await
        .map_err(|e| worker_err_msg(ctx, e))?;
    let obj = out_val
        .as_object()
        .ok_or_else(|| "Register script did not return metadata object".to_string())?;
    let hook_id = obj.get::<_, u64>("hookId").map_err(|e| e.to_string())?;
    let has_resolve = obj
        .get::<_, bool>("hasResolve")
        .map_err(|e| e.to_string())?;
    let has_load = obj.get::<_, bool>("hasLoad").map_err(|e| e.to_string())?;
    Ok(RegisterOutcome {
        hook_id,
        has_resolve,
        has_load,
    })
}

async fn handle_worker_resolve<'js>(
    ctx: &rquickjs::Ctx<'js>,
    hook_id: u64,
    specifier: String,
    parent_url: String,
    conditions: Vec<String>,
    import_attributes: Vec<(String, String)>,
    rpc_tx: std::sync::mpsc::SyncSender<WorkerRpcMsg>,
) -> Result<HookResolveOutput, String> {
    let context = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
    context
        .set("parentURL", parent_url.clone())
        .map_err(|e| e.to_string())?;
    let cond_arr = rquickjs::Array::new(ctx.clone()).map_err(|e| e.to_string())?;
    for (i, c) in conditions.iter().enumerate() {
        cond_arr.set(i, c.as_str()).map_err(|e| e.to_string())?;
    }
    context
        .set("conditions", cond_arr)
        .map_err(|e| e.to_string())?;
    let attrs_obj = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
    for (k, v) in import_attributes {
        attrs_obj.set(k, v).map_err(|e| e.to_string())?;
    }
    context
        .set("importAttributes", attrs_obj)
        .map_err(|e| e.to_string())?;

    let rpc_for_next = rpc_tx;
    let next_resolve = Function::new(
        ctx.clone(),
        move |call_ctx: rquickjs::Ctx<'js>,
              next_spec: String,
              next_ctx: rquickjs::function::Opt<Object<'js>>|
              -> rquickjs::Result<rquickjs::Value<'js>> {
            let mut next_parent = None;
            let mut next_conds = None;
            let mut next_attrs = None;
            if let Some(ref nc) = next_ctx.0 {
                if let Ok(Some(p)) = nc.get::<_, Option<String>>("parentURL") {
                    next_parent = Some(p);
                }
                if let Ok(arr) = nc.get::<_, rquickjs::Array>("conditions") {
                    let mut v = Vec::with_capacity(arr.len());
                    for item in arr.iter::<String>().flatten() {
                        v.push(item);
                    }
                    next_conds = Some(v);
                }
                if let Ok(aobj) = nc.get::<_, Object>("importAttributes") {
                    let mut v = Vec::new();
                    for (k, val) in aobj.props::<String, String>().flatten() {
                        v.push((k, val));
                    }
                    next_attrs = Some(v);
                }
            }

            let rpc = rpc_for_next.clone();
            let (promise, resolve, reject) = call_ctx.promise()?;
            let c = call_ctx.clone();
            call_ctx.spawn(async move {
                let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
                if rpc
                    .send(WorkerRpcMsg::CallNextResolve {
                        specifier: next_spec,
                        parent_url: next_parent,
                        conditions: next_conds,
                        import_attributes: next_attrs,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    let err = rquickjs::Exception::throw_message(&c, "Main thread channel closed");
                    let _ = reject.call::<_, ()>((err.to_string(),));
                    return;
                }
                match tokio::task::spawn_blocking(move || reply_rx.recv()).await {
                    Ok(Ok(NextReply::Resolve(Ok(out)))) => {
                        if let Ok(obj) = Object::new(c.clone()) {
                            if let Some(u) = out.url {
                                let _ = obj.set("url", u);
                            }
                            if let Some(f) = out.format {
                                let _ = obj.set("format", f);
                            }
                            let _ = obj.set("shortCircuit", out.short_circuit);
                            let _ = resolve.call::<_, ()>((obj,));
                        }
                    },
                    Ok(Ok(NextReply::Resolve(Err(msg)))) => {
                        if let Ok(err_ctor) = c
                            .globals()
                            .get::<_, rquickjs::function::Constructor>("Error")
                        {
                            if let Ok(err_obj) = err_ctor.construct::<_, Object>((msg,)) {
                                let _ = reject.call::<_, ()>((err_obj,));
                            }
                        }
                    },
                    _ => {
                        if let Ok(err_ctor) = c
                            .globals()
                            .get::<_, rquickjs::function::Constructor>("Error")
                        {
                            if let Ok(err_obj) =
                                err_ctor.construct::<_, Object>(("nextResolve channel failed",))
                            {
                                let _ = reject.call::<_, ()>((err_obj,));
                            }
                        }
                    },
                }
            });
            Ok(promise.into_value())
        },
    )
    .map_err(|e| e.to_string())?;

    let reg: Object = ctx
        .globals()
        .get("__lexeHooksRegistry")
        .map_err(|e| e.to_string())?;
    let entry: Object = reg
        .get::<_, Function>("get")
        .map_err(|e| e.to_string())?
        .call((rquickjs::prelude::This(reg), hook_id))
        .map_err(|e| worker_err_msg(ctx, e))?;
    let resolve_fn: Function = entry.get("resolve").map_err(|e| e.to_string())?;
    let ret: rquickjs::Value<'js> = resolve_fn
        .call((specifier, context, next_resolve))
        .map_err(|e| worker_err_msg(ctx, e))?;
    let awaited = if let Ok(p) = rquickjs::Promise::from_value(ret.clone()) {
        p.into_future::<rquickjs::Value<'js>>()
            .await
            .map_err(|e| worker_err_msg(ctx, e))?
    } else {
        ret
    };
    let obj = awaited
        .as_object()
        .ok_or_else(|| "resolve hook must return an object".to_string())?;
    Ok(HookResolveOutput {
        url: obj
            .get::<_, Option<String>>("url")
            .map_err(|e| e.to_string())?,
        format: obj
            .get::<_, Option<String>>("format")
            .map_err(|e| e.to_string())?,
        short_circuit: obj
            .get::<_, Option<bool>>("shortCircuit")
            .map_err(|e| e.to_string())?
            .unwrap_or(false),
    })
}

async fn handle_worker_load<'js>(
    ctx: &rquickjs::Ctx<'js>,
    hook_id: u64,
    url: String,
    format: Option<String>,
    import_attributes: Vec<(String, String)>,
    rpc_tx: std::sync::mpsc::SyncSender<WorkerRpcMsg>,
) -> Result<HookLoadOutput, String> {
    let context = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
    if let Some(fmt) = format {
        context.set("format", fmt).map_err(|e| e.to_string())?;
    }
    let attrs_obj = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
    for (k, v) in import_attributes {
        attrs_obj.set(k, v).map_err(|e| e.to_string())?;
    }
    context
        .set("importAttributes", attrs_obj)
        .map_err(|e| e.to_string())?;

    let rpc_for_next = rpc_tx;
    let next_load = Function::new(
        ctx.clone(),
        move |call_ctx: rquickjs::Ctx<'js>,
              next_url: String,
              next_ctx: rquickjs::function::Opt<Object<'js>>|
              -> rquickjs::Result<rquickjs::Value<'js>> {
            let mut next_fmt = None;
            let mut next_attrs = None;
            if let Some(ref nc) = next_ctx.0 {
                if let Ok(Some(f)) = nc.get::<_, Option<String>>("format") {
                    next_fmt = Some(f);
                }
                if let Ok(aobj) = nc.get::<_, Object>("importAttributes") {
                    let mut v = Vec::new();
                    for (k, val) in aobj.props::<String, String>().flatten() {
                        v.push((k, val));
                    }
                    next_attrs = Some(v);
                }
            }

            let rpc = rpc_for_next.clone();
            let (promise, resolve, reject) = call_ctx.promise()?;
            let c = call_ctx.clone();
            call_ctx.spawn(async move {
                let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
                if rpc
                    .send(WorkerRpcMsg::CallNextLoad {
                        url: next_url,
                        format: next_fmt,
                        import_attributes: next_attrs,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    if let Ok(err_ctor) = c
                        .globals()
                        .get::<_, rquickjs::function::Constructor>("Error")
                    {
                        if let Ok(err_obj) =
                            err_ctor.construct::<_, Object>(("Main thread channel closed",))
                        {
                            let _ = reject.call::<_, ()>((err_obj,));
                        }
                    }
                    return;
                }
                match tokio::task::spawn_blocking(move || reply_rx.recv()).await {
                    Ok(Ok(NextReply::Load(Ok(out)))) => {
                        if let Ok(obj) = Object::new(c.clone()) {
                            if let Some(f) = out.format {
                                let _ = obj.set("format", f);
                            }
                            let _ = obj.set("shortCircuit", out.short_circuit);
                            if let Some(src) = out.source {
                                match src {
                                    HookSourceData::String(s) => {
                                        let _ = obj.set("source", s);
                                    },
                                    HookSourceData::Bytes(b) => {
                                        if let Ok(ab) =
                                            rquickjs::ArrayBuffer::new_copy(c.clone(), &b)
                                        {
                                            let _ = obj.set("source", ab);
                                        }
                                    },
                                }
                            }
                            let _ = resolve.call::<_, ()>((obj,));
                        }
                    },
                    Ok(Ok(NextReply::Load(Err(msg)))) => {
                        if let Ok(err_ctor) = c
                            .globals()
                            .get::<_, rquickjs::function::Constructor>("Error")
                        {
                            if let Ok(err_obj) = err_ctor.construct::<_, Object>((msg,)) {
                                let _ = reject.call::<_, ()>((err_obj,));
                            }
                        }
                    },
                    _ => {
                        if let Ok(err_ctor) = c
                            .globals()
                            .get::<_, rquickjs::function::Constructor>("Error")
                        {
                            if let Ok(err_obj) =
                                err_ctor.construct::<_, Object>(("nextLoad channel failed",))
                            {
                                let _ = reject.call::<_, ()>((err_obj,));
                            }
                        }
                    },
                }
            });
            Ok(promise.into_value())
        },
    )
    .map_err(|e| e.to_string())?;

    let reg: Object = ctx
        .globals()
        .get("__lexeHooksRegistry")
        .map_err(|e| e.to_string())?;
    let entry: Object = reg
        .get::<_, Function>("get")
        .map_err(|e| e.to_string())?
        .call((rquickjs::prelude::This(reg), hook_id))
        .map_err(|e| worker_err_msg(ctx, e))?;
    let load_fn: Function = entry.get("load").map_err(|e| e.to_string())?;
    let ret: rquickjs::Value<'js> = load_fn
        .call((url, context, next_load))
        .map_err(|e| worker_err_msg(ctx, e))?;
    let awaited = if let Ok(p) = rquickjs::Promise::from_value(ret.clone()) {
        p.into_future::<rquickjs::Value<'js>>()
            .await
            .map_err(|e| worker_err_msg(ctx, e))?
    } else {
        ret
    };
    let obj = awaited
        .as_object()
        .ok_or_else(|| "load hook must return an object".to_string())?;
    let format = obj
        .get::<_, Option<String>>("format")
        .map_err(|e| e.to_string())?;
    let short_circuit = obj
        .get::<_, Option<bool>>("shortCircuit")
        .map_err(|e| e.to_string())?
        .unwrap_or(false);
    let source = if let Ok(src_val) = obj.get::<_, rquickjs::Value>("source") {
        if src_val.is_undefined() || src_val.is_null() {
            None
        } else if let Some(s) = src_val.as_string() {
            Some(HookSourceData::String(
                s.to_string().map_err(|e| e.to_string())?,
            ))
        } else {
            let bytes =
                lexe_utils::bytes::ObjectBytes::from(ctx, &src_val).map_err(|e| e.to_string())?;
            Some(HookSourceData::Bytes(
                bytes.as_bytes(ctx).map_err(|e| e.to_string())?.to_vec(),
            ))
        }
    } else {
        None
    };

    Ok(HookLoadOutput {
        format,
        short_circuit,
        source,
    })
}
