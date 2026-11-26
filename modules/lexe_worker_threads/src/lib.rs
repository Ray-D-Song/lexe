// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

mod messages;
mod thread_state;

pub use messages::{MainToWorker, WorkerToMain};
pub use thread_state::set_worker_thread_id;

use std::cell::RefCell;
use std::collections::HashMap;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
};

use lexe_context::CtxExtension;
use lexe_utils::{
    clone_wire::{deserialize, serialize, CloneWirePayload},
    event_loop::{self, EventLoopRef},
    module::{export_default, ModuleInfo},
    result::ResultExt,
};
use rquickjs::{
    function::Opt,
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Array, Ctx, Exception, Function, IntoJs, Null, Object, Result, Undefined, Value,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

thread_local! {
    static PARENT_TO_MAIN: RefCell<Option<UnboundedSender<WorkerToMain>>> = const { RefCell::new(None) };
}

static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

use thread_state::{worker_is_main_thread, worker_thread_id};

pub struct WorkerIsolateStart {
    pub thread_id: u64,
    pub script: String,
    /// When set, `script` is source code to evaluate instead of a file path.
    pub eval: bool,
    pub worker_data: CloneWirePayload,
    pub env_snapshot: CloneWirePayload,
    pub to_main: UnboundedSender<WorkerToMain>,
    pub from_main: mpsc::Receiver<MainToWorker>,
}

pub type WorkerIsolateRunner = fn(WorkerIsolateStart);

static WORKER_ISOLATE_RUNNER: OnceLock<WorkerIsolateRunner> = OnceLock::new();

pub fn set_worker_isolate_runner(runner: WorkerIsolateRunner) {
    let _ = WORKER_ISOLATE_RUNNER.set(runner);
}

struct WorkerRegistry {
    workers: Mutex<Vec<WorkerHandleInner>>,
}

struct WorkerHandleInner {
    to_worker: mpsc::Sender<MainToWorker>,
    registry_id: u64,
}

static WORKER_REGISTRY: OnceLock<WorkerRegistry> = OnceLock::new();

fn registry() -> &'static WorkerRegistry {
    WORKER_REGISTRY.get_or_init(|| WorkerRegistry {
        workers: Mutex::new(Vec::new()),
    })
}

struct WorkerState {
    to_worker: mpsc::Sender<MainToWorker>,
    loop_ref: Arc<EventLoopRef>,
    registry_id: u64,
}

fn resolve_worker_filename(ctx: &Ctx<'_>, filename: String) -> Result<String> {
    if filename.len() >= 5 && filename[..5].eq_ignore_ascii_case("data:") {
        return Ok(filename);
    }
    if let Some(path) = filename.strip_prefix("file://") {
        return Ok(path.to_string());
    }
    let path = PathBuf::from(&filename);
    if path.is_absolute() {
        return Ok(filename);
    }
    let cwd = std::env::current_dir().or_throw(ctx)?;
    Ok(cwd.join(path).to_string_lossy().into_owned())
}

fn transfer_options<'js>(transfer: Opt<Value<'js>>) -> Opt<Object<'js>> {
    match transfer.0 {
        None => Opt(None),
        Some(v) if v.is_null() || v.is_undefined() => Opt(None),
        Some(v) => {
            if let Some(obj) = v.as_object() {
                Opt(Some(obj.clone()))
            } else {
                Opt(None)
            }
        },
    }
}

fn emit_worker_exit<'js>(
    ctx: &Ctx<'js>,
    on_event: &Function<'js>,
    code: i32,
    signal: Option<&str>,
) -> Result<()> {
    let exit_obj = Object::new(ctx.clone())?;
    exit_obj.set("code", code)?;
    let signal_val: Value = match signal {
        Some(s) => s.into_js(ctx)?,
        None => Null.into_value(ctx.clone()),
    };
    exit_obj.set("signal", signal_val)?;
    let _ = on_event.call::<_, ()>(("exit", exit_obj));
    Ok(())
}

thread_local! {
    /// JS event sinks keyed by worker thread id.
    ///
    /// Storing a `Persistent` here (instead of capturing the `Function` inside a
    /// `'js` closure handed to JS) lets teardown release every worker root
    /// explicitly; a captured `Function` outlives the context and aborts QuickJS
    /// with `gc_obj_list`.
    static WORKER_EVENT_SINKS: RefCell<HashMap<u64, rquickjs::Persistent<Function<'static>>>> =
        RefCell::new(HashMap::new());
}

fn register_event_sink<'js>(ctx: &Ctx<'js>, thread_id: u64, on_event: Function<'js>) {
    let saved = rquickjs::Persistent::save(ctx, on_event);
    WORKER_EVENT_SINKS.with(|sinks| {
        sinks.borrow_mut().insert(thread_id, saved);
    });
}

fn restore_event_sink<'js>(thread_id: u64, ctx: &Ctx<'js>) -> Option<Function<'js>> {
    WORKER_EVENT_SINKS.with(|sinks| {
        let saved = sinks.borrow().get(&thread_id).cloned()?;
        saved.restore(ctx).ok()
    })
}

fn spawn_worker<'js>(
    ctx: Ctx<'js>,
    filename: String,
    options: Object<'js>,
    on_event: Function<'js>,
) -> Result<Object<'js>> {
    let runner = WORKER_ISOLATE_RUNNER.get().ok_or_else(|| {
        Exception::throw_type(&ctx, "worker_threads.Worker is unavailable in this build")
    })?;

    // `new Worker(code, { eval: true })` runs the string as the worker script
    // instead of resolving it as a file path.
    let is_eval = options
        .get::<_, Value>("eval")
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(false);
    let script = if is_eval {
        filename
    } else {
        resolve_worker_filename(&ctx, filename)?
    };
    let worker_data: Value = options
        .get("workerData")
        .unwrap_or(Undefined.into_value(ctx.clone()));
    let env_snapshot: Value = match options.get::<_, Value>("lexeEnvironmentSnapshot") {
        Ok(v) => v,
        Err(_) => Object::new(ctx.clone())?.into_value(),
    };

    let worker_wire = serialize(&ctx, worker_data, Opt(None))?;
    let env_wire = serialize(&ctx, env_snapshot, Opt(None))?;

    let thread_id = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
    register_event_sink(&ctx, thread_id, on_event.clone());
    let (to_main_tx, mut to_main_rx) = unbounded_channel::<WorkerToMain>();
    let (to_worker_tx, to_worker_rx) = mpsc::channel::<MainToWorker>();

    let registry_id = thread_id;
    registry().workers.lock().unwrap().push(WorkerHandleInner {
        to_worker: to_worker_tx.clone(),
        registry_id,
    });

    let loop_ref = EventLoopRef::new_referenced(event_loop::rt_key_from_ptr(unsafe {
        rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr())
    }
        as *mut std::ffi::c_void));

    let state = Arc::new(WorkerState {
        to_worker: to_worker_tx,
        loop_ref: loop_ref.clone(),
        registry_id,
    });

    thread::Builder::new()
        .name(format!("lexe-worker-{thread_id}"))
        .spawn({
            let script = script.clone();
            move || {
                runner(WorkerIsolateStart {
                    thread_id,
                    script,
                    eval: is_eval,
                    worker_data: worker_wire,
                    env_snapshot: env_wire,
                    to_main: to_main_tx,
                    from_main: to_worker_rx,
                });
            }
        })
        .or_throw(&ctx)?;

    let out = Object::new(ctx.clone())?;
    out.set("threadId", thread_id)?;

    let state_post = state.clone();
    out.set(
        "postMessage",
        Func::from(
            move |ctx: Ctx<'js>, data: Value<'js>, transfer: Opt<Value<'js>>| {
                let wire = serialize(&ctx, data, transfer_options(transfer))?;
                let _ = state_post.to_worker.send(MainToWorker::Message(wire));
                Ok::<(), rquickjs::Error>(())
            },
        ),
    )?;

    let state_term = state.clone();
    out.set(
        "terminate",
        Func::from(move |ctx: Ctx<'js>| {
            let _ = state_term.to_worker.send(MainToWorker::Terminate);
            registry()
                .workers
                .lock()
                .unwrap()
                .retain(|w| w.registry_id != state_term.registry_id);
            if let Ok(exit_obj) = Object::new(ctx.clone()) {
                let _ = exit_obj.set("code", 1i32);
                let _ = exit_obj.set("signal", "SIGTERM");
                let _ = dispatch_worker_event_js(
                    ctx.clone(),
                    state_term.registry_id,
                    "exit".to_string(),
                    Opt(Some(exit_obj.into_value())),
                );
            }
            Ok::<(), rquickjs::Error>(())
        }),
    )?;

    let loop_ref_ref = loop_ref.clone();
    out.set(
        "ref",
        Func::from(move |_ctx: Ctx<'_>| {
            loop_ref_ref.reference();
            Ok::<(), rquickjs::Error>(())
        }),
    )?;

    let loop_ref_unref = loop_ref.clone();
    out.set(
        "unref",
        Func::from(move |_ctx: Ctx<'_>| {
            loop_ref_unref.unreference();
            Ok::<(), rquickjs::Error>(())
        }),
    )?;

    let ctx_events = ctx.clone();
    let loop_ref_stdout = loop_ref.clone();
    let dispatch_id = thread_id;
    ctx.spawn_exit_simple(async move {
        while let Some(ev) = to_main_rx.recv().await {
            if let Some(exit_event) = dispatch_worker_event(&ctx_events, dispatch_id, ev) {
                emit_worker_exit_object(&ctx_events, dispatch_id, exit_event);
                break;
            }
        }
        loop_ref_stdout.unreference();
        Ok(())
    });

    Ok(out)
}

/// Forward one worker event to the JS side dispatch hook. Returns the exit
/// payload when the worker finished.
fn dispatch_worker_event(
    ctx: &Ctx<'_>,
    thread_id: u64,
    ev: WorkerToMain,
) -> Option<(i32, Option<String>)> {
    match ev {
        WorkerToMain::Online => {
            let _ =
                dispatch_worker_event_js(ctx.clone(), thread_id, "online".to_string(), Opt(None));
            None
        },
        WorkerToMain::Message(wire) => {
            if let Ok(value) = deserialize(ctx, &wire) {
                let _ = dispatch_worker_event_js(
                    ctx.clone(),
                    thread_id,
                    "message".to_string(),
                    Opt(Some(value)),
                );
            }
            None
        },
        WorkerToMain::Error(msg) => {
            if let Ok(err) = Object::new(ctx.clone()) {
                let _ = err.set("message", msg);
                let _ = dispatch_worker_event_js(
                    ctx.clone(),
                    thread_id,
                    "error".to_string(),
                    Opt(Some(err.into_value())),
                );
            }
            None
        },
        WorkerToMain::Exit { code, signal } => Some((code, signal)),
    }
}

fn emit_worker_exit_object(ctx: &Ctx<'_>, thread_id: u64, (code, signal): (i32, Option<String>)) {
    let Ok(exit_obj) = Object::new(ctx.clone()) else {
        return;
    };
    let _ = exit_obj.set("code", code);
    let signal_val: Value = match signal {
        Some(s) => match s.into_js(ctx) {
            Ok(v) => v,
            Err(_) => return,
        },
        None => Null.into_value(ctx.clone()),
    };
    let _ = exit_obj.set("signal", signal_val);
    let _ = dispatch_worker_event_js(
        ctx.clone(),
        thread_id,
        "exit".to_string(),
        Opt(Some(exit_obj.into_value())),
    );
}

pub fn set_worker_parent_channel(tx: UnboundedSender<WorkerToMain>) {
    PARENT_TO_MAIN.with(|c| *c.borrow_mut() = Some(tx));
}

/// Native side of the worker event dispatch: restores the sink registered for
/// this thread id and calls it.
fn dispatch_worker_event_js<'js>(
    ctx: Ctx<'js>,
    thread_id: u64,
    event: String,
    arg: Opt<Value<'js>>,
) -> Result<()> {
    let Some(sink) = restore_event_sink(thread_id, &ctx) else {
        return Ok(());
    };
    match arg.0 {
        Some(value) => sink.call::<_, ()>((event, value))?,
        None => sink.call::<_, ()>((event, Undefined.into_value(ctx.clone())))?,
    }
    Ok(())
}

/// Release every worker handle held by the process-wide registry.
///
/// The registry keeps `mpsc::Sender`s alive for the whole process, so a handle
/// that is still registered keeps its worker's channel open. Dropping them
/// before runtime teardown lets the worker read loops finish and release the JS
/// values they captured (otherwise QuickJS aborts with `gc_obj_list`).
pub fn shutdown_workers() {
    if let Some(registry) = WORKER_REGISTRY.get() {
        registry.workers.lock().unwrap().clear();
    }
    // Drop the saved JS sinks: they are the only Rust-side roots for worker
    // objects, and QuickJS asserts none remain when the runtime is freed.
    WORKER_EVENT_SINKS.with(|sinks| sinks.borrow_mut().clear());
}

pub fn worker_parent_post_message_hook<'js>(
    ctx: Ctx<'js>,
    data: Value<'js>,
    transfer: Opt<Value<'js>>,
) -> Result<()> {
    worker_parent_post_message(ctx, data, transfer)
}

fn worker_parent_post_message<'js>(
    ctx: Ctx<'js>,
    data: Value<'js>,
    transfer: Opt<Value<'js>>,
) -> Result<()> {
    let tx = PARENT_TO_MAIN
        .with(|c| c.borrow().clone())
        .ok_or_else(|| Exception::throw_type(&ctx, "worker parent IPC is unavailable"))?;
    let wire = serialize(&ctx, data, transfer_options(transfer))?;
    let _ = tx.send(WorkerToMain::Message(wire));
    Ok(())
}

fn worker_broadcast_env<'js>(ctx: Ctx<'js>, key: Value<'js>, value: Value<'js>) -> Result<()> {
    let key_wire = serialize(&ctx, key, Opt(None))?;
    let value_wire = serialize(&ctx, value, Opt(None))?;
    for worker in registry().workers.lock().unwrap().iter() {
        let _ = worker.to_worker.send(MainToWorker::EnvPatch {
            key: key_wire.clone(),
            value: value_wire.clone(),
        });
    }
    Ok(())
}

pub fn worker_is_main_thread_ctx(_ctx: Ctx<'_>) -> bool {
    worker_is_main_thread()
}

pub fn worker_thread_id_ctx(_ctx: Ctx<'_>) -> u64 {
    worker_thread_id()
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();
    globals.set(
        "__lexeWorkerIsMainThread",
        Func::from(worker_is_main_thread_ctx),
    )?;
    globals.set("__lexeWorkerThreadId", Func::from(worker_thread_id_ctx))?;
    globals.set("__lexeWorkerSpawn", Func::from(spawn_worker))?;
    globals.set(
        "__lexeWorkerDispatchEvent",
        Func::from(dispatch_worker_event_js),
    )?;
    globals.set("__lexeWorkerBroadcastEnv", Func::from(worker_broadcast_env))?;
    // Expose global MessagePort / MessageChannel lazily from node:worker_threads so
    // `globalThis.MessagePort === import("node:worker_threads").MessagePort` while keeping init ordering safe.
    ctx.eval::<(), _>(
        r#"(function() {
  for (const name of ["MessagePort", "MessageChannel"]) {
    if (name in globalThis) continue;
    Object.defineProperty(globalThis, name, {
      configurable: true,
      enumerable: false,
      get() {
        let wt;
        if (typeof process !== "undefined" && typeof process.getBuiltinModule === "function") {
          wt = process.getBuiltinModule("node:worker_threads");
        } else if (typeof require === "function") {
          wt = require("node:worker_threads");
        }
        const val = wt && wt[name];
        if (val !== undefined) {
          Object.defineProperty(globalThis, name, {
            value: val,
            writable: true,
            configurable: true,
            enumerable: false,
          });
        }
        return val;
      },
      set(v) {
        Object.defineProperty(globalThis, name, {
          value: v,
          writable: true,
          configurable: true,
          enumerable: false,
        });
      },
    });
  }
})();"#,
    )?;
    Ok(())
}

pub struct WorkerThreadsModule;

impl ModuleDef for WorkerThreadsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |_default| Ok(()))?;
        Ok(())
    }
}

impl From<WorkerThreadsModule> for ModuleInfo<WorkerThreadsModule> {
    fn from(val: WorkerThreadsModule) -> Self {
        ModuleInfo {
            name: "lexe:worker_threads",
            module: val,
        }
    }
}
