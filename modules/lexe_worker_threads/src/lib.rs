// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

mod broadcast_channel;
mod messages;
mod port_hub;
mod thread_state;

pub use broadcast_channel::BroadcastChannel;
pub use messages::{MainToWorker, WorkerEnvMode, WorkerResourceLimits, WorkerToMain};
pub use thread_state::set_worker_thread_id;

use std::cell::RefCell;
use std::collections::HashMap;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
};

use lexe_context::CtxExtension;
use lexe_utils::{
    bytes::ObjectBytes,
    clone_wire::{deserialize, serialize, CloneWirePayload},
    event_loop::{self, wait_until_io_unref, EventLoopRef},
    module::{export_default, ModuleInfo},
    result::ResultExt,
};
use rquickjs::{
    function::Opt,
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, This},
    Class, Coerced, Ctx, Exception, Function, IntoJs, Null, Object, Result, TypedArray, Undefined,
    Value,
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
    pub env_mode: WorkerEnvMode,
    pub piped_stdin: bool,
    pub piped_stdout: bool,
    pub piped_stderr: bool,
    pub resource_limits: WorkerResourceLimits,
    pub to_main: UnboundedSender<WorkerToMain>,
    pub from_main: UnboundedReceiver<MainToWorker>,
    pub thread_inbox: UnboundedReceiver<(u64, CloneWirePayload)>,
}

type ThreadInboxItem = (u64, CloneWirePayload);
type ThreadInboxRxSlot = Arc<Mutex<Option<UnboundedReceiver<ThreadInboxItem>>>>;

#[derive(Clone)]
struct ThreadHubEntry {
    tx: UnboundedSender<ThreadInboxItem>,
    rx: ThreadInboxRxSlot,
    pumping: Arc<AtomicBool>,
}

type ThreadHubMap = HashMap<u64, ThreadHubEntry>;

static THREAD_HUB: OnceLock<Mutex<ThreadHubMap>> = OnceLock::new();

fn thread_hub() -> &'static Mutex<ThreadHubMap> {
    THREAD_HUB.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn unregister_thread_inbox(thread_id: u64) {
    thread_hub().lock().unwrap().remove(&thread_id);
}

pub fn attach_thread_inbox<'js>(
    _ctx: &Ctx<'js>,
    thread_id: u64,
    rx: UnboundedReceiver<(u64, CloneWirePayload)>,
) {
    if let Some(entry) = thread_hub().lock().unwrap().get(&thread_id) {
        *entry.rx.lock().unwrap() = Some(rx);
    }
}

fn start_thread_inbox_pump(ctx: &Ctx<'_>) {
    let thread_id = worker_thread_id();
    let Some(entry) = thread_hub().lock().unwrap().get(&thread_id).cloned() else {
        return;
    };
    if entry.pumping.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(mut rx) = entry.rx.lock().unwrap().take() else {
        entry.pumping.store(false, Ordering::Release);
        return;
    };

    let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let loop_ref = EventLoopRef::new_unreferenced(event_loop::rt_key_from_ptr(rt_ptr as *mut _));
    let ctx_clone = ctx.clone();

    ctx.spawn(async move {
        let mut channel_closed = false;
        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    let Some((source_id, wire)) = maybe else {
                        channel_closed = true;
                        break;
                    };
                    dispatch_thread_message(&ctx_clone, source_id, &wire);
                }
                _ = wait_until_io_unref(Arc::clone(&loop_ref)) => {
                    while let Ok((source_id, wire)) = rx.try_recv() {
                        dispatch_thread_message(&ctx_clone, source_id, &wire);
                    }
                    break;
                }
            }
        }
        if !channel_closed {
            *entry.rx.lock().unwrap() = Some(rx);
        }
        entry.pumping.store(false, Ordering::Release);
    });
}

fn dispatch_thread_message(ctx: &Ctx<'_>, source_id: u64, wire: &CloneWirePayload) {
    let Ok(val) = deserialize(ctx, wire) else {
        return;
    };
    let Ok(proc_obj) = ctx.globals().get::<_, Object>("process") else {
        return;
    };
    let Ok(emit_fn) = proc_obj.get::<_, Function>("emit") else {
        return;
    };
    let _ = emit_fn.call::<_, Value>((This(proc_obj), "workerMessage", val, source_id as f64));
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
    to_worker: UnboundedSender<MainToWorker>,
    registry_id: u64,
}

static WORKER_REGISTRY: OnceLock<WorkerRegistry> = OnceLock::new();

fn registry() -> &'static WorkerRegistry {
    WORKER_REGISTRY.get_or_init(|| WorkerRegistry {
        workers: Mutex::new(Vec::new()),
    })
}

struct WorkerState {
    to_worker: UnboundedSender<MainToWorker>,
    _loop_ref: Arc<EventLoopRef>,
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
    let worker_transfer = options.get::<_, Object>("lexeWorkerDataTransfer").ok();

    let env_mode = if options.get::<_, bool>("lexeEnvShare").unwrap_or(false) {
        WorkerEnvMode::Shared(lexe_utils::env_store::current_env_map(&ctx))
    } else if let Ok(env_obj) = options.get::<_, Object>("lexeEnvMap") {
        let mut map = HashMap::new();
        for key in env_obj.keys::<String>().flatten() {
            if let Ok(val) = env_obj.get::<&str, Coerced<String>>(key.as_str()) {
                map.insert(key, val.0);
            }
        }
        WorkerEnvMode::Snapshot(map)
    } else {
        WorkerEnvMode::Snapshot(lexe_utils::env_store::snapshot_env_map(&ctx))
    };

    let piped_stdin = options.get::<_, bool>("stdin").unwrap_or(false);
    let piped_stdout = options.get::<_, bool>("stdout").unwrap_or(false);
    let piped_stderr = options.get::<_, bool>("stderr").unwrap_or(false);

    let resource_limits = if let Ok(rl_obj) = options.get::<_, Object>("lexeResourceLimits") {
        WorkerResourceLimits {
            max_young_generation_size_mb: rl_obj
                .get::<_, Option<f64>>("maxYoungGenerationSizeMb")
                .ok()
                .flatten(),
            max_old_generation_size_mb: rl_obj
                .get::<_, Option<f64>>("maxOldGenerationSizeMb")
                .ok()
                .flatten(),
            code_range_size_mb: rl_obj
                .get::<_, Option<f64>>("codeRangeSizeMb")
                .ok()
                .flatten(),
            stack_size_mb: rl_obj.get::<_, Option<f64>>("stackSizeMb").ok().flatten(),
        }
    } else {
        WorkerResourceLimits::default()
    };

    let worker_wire = serialize(&ctx, worker_data, Opt(worker_transfer))?;
    let env_wire = serialize(&ctx, env_snapshot, Opt(None))?;

    let thread_id = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
    register_event_sink(&ctx, thread_id, on_event.clone());
    let (to_main_tx, mut to_main_rx) = unbounded_channel::<WorkerToMain>();
    let (to_worker_tx, to_worker_rx) = unbounded_channel::<MainToWorker>();
    let (thread_inbox_tx, thread_inbox_rx) = unbounded_channel::<(u64, CloneWirePayload)>();
    thread_hub().lock().unwrap().insert(
        thread_id,
        ThreadHubEntry {
            tx: thread_inbox_tx,
            rx: Arc::new(Mutex::new(None)),
            pumping: Arc::new(AtomicBool::new(false)),
        },
    );

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
        _loop_ref: loop_ref.clone(),
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
                    env_mode,
                    piped_stdin,
                    piped_stdout,
                    piped_stderr,
                    resource_limits,
                    to_main: to_main_tx,
                    from_main: to_worker_rx,
                    thread_inbox: thread_inbox_rx,
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
            unregister_thread_inbox(state_term.registry_id);
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

    let state_stdin = state.clone();
    out.set(
        "writeStdin",
        Func::from(move |ctx: Ctx<'js>, chunk: Value<'js>| {
            let bytes = extract_chunk_bytes(&ctx, &chunk)?;
            let _ = state_stdin.to_worker.send(MainToWorker::StdinData(bytes));
            Ok::<(), rquickjs::Error>(())
        }),
    )?;

    let state_stdin_end = state.clone();
    out.set(
        "endStdin",
        Func::from(move || {
            let _ = state_stdin_end.to_worker.send(MainToWorker::StdinEnd);
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
        WorkerToMain::Stdout(bytes) => {
            if let Ok(arr) = TypedArray::<u8>::new_copy(ctx.clone(), &bytes) {
                let _ = dispatch_worker_event_js(
                    ctx.clone(),
                    thread_id,
                    "stdout".to_string(),
                    Opt(Some(arr.into_value())),
                );
            }
            None
        },
        WorkerToMain::Stderr(bytes) => {
            if let Ok(arr) = TypedArray::<u8>::new_copy(ctx.clone(), &bytes) {
                let _ = dispatch_worker_event_js(
                    ctx.clone(),
                    thread_id,
                    "stderr".to_string(),
                    Opt(Some(arr.into_value())),
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
        WorkerToMain::Exit { code, signal } => {
            unregister_thread_inbox(thread_id);
            Some((code, signal))
        },
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

fn extract_chunk_bytes<'js>(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<Vec<u8>> {
    if let Some(s) = val.as_string() {
        return Ok(s.to_string()?.into_bytes());
    }
    if let Some(obj) = val.as_object() {
        if let Some(ob) = ObjectBytes::from_array_buffer(obj)? {
            return Ok(ob.as_bytes(ctx)?.to_vec());
        }
    }
    Ok(val.get::<Coerced<String>>()?.0.into_bytes())
}

pub fn worker_write_stdout_hook<'js>(ctx: Ctx<'js>, chunk: Value<'js>) -> Result<()> {
    let bytes = extract_chunk_bytes(&ctx, &chunk)?;
    if let Some(tx) = PARENT_TO_MAIN.with(|c| c.borrow().clone()) {
        let _ = tx.send(WorkerToMain::Stdout(bytes));
    }
    Ok(())
}

pub fn worker_write_stderr_hook<'js>(ctx: Ctx<'js>, chunk: Value<'js>) -> Result<()> {
    let bytes = extract_chunk_bytes(&ctx, &chunk)?;
    if let Some(tx) = PARENT_TO_MAIN.with(|c| c.borrow().clone()) {
        let _ = tx.send(WorkerToMain::Stderr(bytes));
    }
    Ok(())
}

fn post_message_to_thread_js<'js>(
    ctx: Ctx<'js>,
    target_id_f: f64,
    value: Value<'js>,
    transfer: Opt<Value<'js>>,
) -> Result<()> {
    let target_id = target_id_f as u64;
    let source_id = worker_thread_id();
    let entry_opt = thread_hub().lock().unwrap().get(&target_id).cloned();
    let Some(entry) = entry_opt else {
        let err: Object = ctx
            .globals()
            .get::<_, Function>("Error")?
            .call(("Target thread could not be reached",))?;
        err.set("code", "ERR_WORKER_MESSAGING_FAILED")?;
        return Err(ctx.throw(err.into_value()));
    };
    let wire = serialize(&ctx, value, transfer_options(transfer))?;
    if entry.tx.send((source_id, wire)).is_err() {
        unregister_thread_inbox(target_id);
        let err: Object = ctx
            .globals()
            .get::<_, Function>("Error")?
            .call(("Target thread could not be reached",))?;
        err.set("code", "ERR_WORKER_MESSAGING_FAILED")?;
        return Err(ctx.throw(err.into_value()));
    }
    Ok(())
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
    lexe_events::init(ctx)?;
    lexe_exceptions::init(ctx)?;
    port_hub::register_codecs();

    let globals = ctx.globals();
    if !globals.contains_key("BroadcastChannel")? {
        Class::<BroadcastChannel>::define(&globals)?;
        BroadcastChannel::install_prototype(ctx, &globals)?;
    }

    globals.set("__lexePortCreatePair", Func::from(port_hub::create_pair))?;
    globals.set(
        "__lexePortPeer",
        Func::from(|id: f64| port_hub::peer_id(id as u64).map(|p| p as f64)),
    )?;
    globals.set(
        "__lexePortClose",
        Func::from(|id: f64| port_hub::close_port(id as u64)),
    )?;
    globals.set("__lexePortSend", Func::from(port_hub::send_to_peer_js))?;
    globals.set("__lexePortAttach", Func::from(port_hub::attach_port_js))?;
    globals.set("__lexePortDetachSink", Func::from(port_hub::detach_sink_js))?;
    globals.set(
        "__lexeWorkerPostToThread",
        Func::from(post_message_to_thread_js),
    )?;
    globals.set(
        "__lexeWorkerStartThreadInbox",
        Func::from(|ctx: Ctx<'_>| start_thread_inbox_pump(&ctx)),
    )?;

    if worker_is_main_thread() && !globals.contains_key("__lexeWorkerMainInboxInstalled")? {
        globals.set("__lexeWorkerMainInboxInstalled", true)?;
        let (tx, rx) = unbounded_channel::<(u64, CloneWirePayload)>();
        thread_hub().lock().unwrap().insert(
            0,
            ThreadHubEntry {
                tx,
                rx: Arc::new(Mutex::new(Some(rx))),
                pumping: Arc::new(AtomicBool::new(false)),
            },
        );
    }

    ctx.eval::<(), _>(
        r#"(function() {
  if (globalThis.__lexeWorkerMsgListenerHooked) return;
  if (typeof process === "undefined" || typeof process.on !== "function") return;
  globalThis.__lexeWorkerMsgListenerHooked = true;
  const msgRef = typeof globalThis.__lexeCreateLoopRefController === "function"
    ? globalThis.__lexeCreateLoopRefController()
    : { ref() {}, unref() {} };
  const sync = () => {
    const count = typeof process.listenerCount === "function" ? process.listenerCount("workerMessage") : 0;
    if (count > 0) {
      msgRef.ref();
      globalThis.__lexeWorkerStartThreadInbox?.();
    } else {
      msgRef.unref();
    }
  };
  process.on("newListener", (ev) => {
    if (ev === "workerMessage") {
      msgRef.ref();
      globalThis.__lexeWorkerStartThreadInbox?.();
    }
  });
  process.on("removeListener", (ev) => {
    if (ev === "workerMessage") queueMicrotask(sync);
  });
  sync();
})();"#,
    )?;

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
