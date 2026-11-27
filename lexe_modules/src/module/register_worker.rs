// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Worker-thread async customization hooks backing `module.register()`.
//!
//! Spawns a dedicated OS worker VM via a runner registered at binary startup
//! (mirroring `lexe_worker_threads::set_worker_isolate_runner`) so `lexe_modules`
//! has no circular dependency on `lexe_core` / `Vm`, while supporting full
//! `initialize(data)`, `transferList` (`MessagePort` via `clone_wire`), async
//! `resolve`/`load`, and bidirectional `nextResolve`/`nextLoad` chaining.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{mpsc as std_mpsc, OnceLock},
    thread::JoinHandle,
};

use lexe_utils::clone_wire::{serialize, CloneWirePayload};
use rquickjs::{
    function::{Constructor, Opt, Rest},
    Array, ArrayBuffer, Ctx, Exception, Function, Object, Result, Value,
};
use tokio::sync::mpsc as tokio_mpsc;
use url::Url;

use super::{Hook, ModuleHookState};

#[derive(Debug, Clone)]
pub enum HookSourceData {
    String(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct HookResolveOutput {
    pub url: Option<String>,
    pub format: Option<String>,
    pub short_circuit: bool,
}

#[derive(Debug, Clone)]
pub struct HookLoadOutput {
    pub format: Option<String>,
    pub short_circuit: bool,
    pub source: Option<HookSourceData>,
}

#[derive(Debug)]
pub enum NextReply {
    Resolve(std::result::Result<HookResolveOutput, String>),
    Load(std::result::Result<HookLoadOutput, String>),
}

#[derive(Debug)]
pub enum WorkerRpcMsg {
    CallNextResolve {
        specifier: String,
        parent_url: Option<String>,
        conditions: Option<Vec<String>>,
        import_attributes: Option<Vec<(String, String)>>,
        reply: std_mpsc::SyncSender<NextReply>,
    },
    CallNextLoad {
        url: String,
        format: Option<String>,
        import_attributes: Option<Vec<(String, String)>>,
        reply: std_mpsc::SyncSender<NextReply>,
    },
    ResolveDone(std::result::Result<HookResolveOutput, String>),
    LoadDone(std::result::Result<HookLoadOutput, String>),
}

#[derive(Debug, Clone)]
pub struct RegisterOutcome {
    pub hook_id: u64,
    pub has_resolve: bool,
    pub has_load: bool,
}

pub enum HooksRequest {
    Register {
        hook_url: String,
        data_wire: CloneWirePayload,
        reply: std_mpsc::SyncSender<std::result::Result<RegisterOutcome, String>>,
    },
    Resolve {
        hook_id: u64,
        specifier: String,
        parent_url: String,
        conditions: Vec<String>,
        import_attributes: Vec<(String, String)>,
        rpc_tx: std_mpsc::SyncSender<WorkerRpcMsg>,
    },
    Load {
        hook_id: u64,
        url: String,
        format: Option<String>,
        import_attributes: Vec<(String, String)>,
        rpc_tx: std_mpsc::SyncSender<WorkerRpcMsg>,
    },
    Shutdown,
}

pub struct ModuleHooksWorkerStart {
    pub from_main: tokio_mpsc::UnboundedReceiver<HooksRequest>,
    pub ready_tx: std_mpsc::SyncSender<std::result::Result<(), String>>,
}

pub type ModuleHooksWorkerRunner = fn(ModuleHooksWorkerStart);

static HOOKS_WORKER_RUNNER: OnceLock<ModuleHooksWorkerRunner> = OnceLock::new();

pub fn set_module_hooks_worker_runner(runner: ModuleHooksWorkerRunner) {
    let _ = HOOKS_WORKER_RUNNER.set(runner);
}

#[derive(Clone)]
pub struct HooksWorkerClient {
    tx: tokio_mpsc::UnboundedSender<HooksRequest>,
    join: Rc<RefCell<Option<JoinHandle<()>>>>,
}

impl Drop for HooksWorkerClient {
    fn drop(&mut self) {
        if Rc::strong_count(&self.join) == 1 {
            self.shutdown();
        }
    }
}

impl HooksWorkerClient {
    pub fn start() -> std::result::Result<Self, String> {
        let runner = HOOKS_WORKER_RUNNER.get().copied().ok_or_else(|| {
            "Module customization hooks worker runner is not initialized".to_string()
        })?;
        let (tx, rx) = tokio_mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std_mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name("lexe-module-hooks".to_string())
            .spawn(move || {
                runner(ModuleHooksWorkerStart {
                    from_main: rx,
                    ready_tx,
                });
            })
            .map_err(|e| format!("Failed to spawn module hooks worker: {e}"))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                join: Rc::new(RefCell::new(Some(handle))),
            }),
            Ok(Err(err)) => {
                let _ = handle.join();
                Err(err)
            },
            Err(_) => {
                let _ = handle.join();
                Err("Module hooks worker exited before initialization".to_string())
            },
        }
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(HooksRequest::Shutdown);
        if let Some(handle) = self.join.borrow_mut().take() {
            let _ = handle.join();
        }
    }
}

fn ensure_worker_client(ctx: &Ctx<'_>) -> Result<HooksWorkerClient> {
    let binding = ctx
        .userdata::<RefCell<ModuleHookState>>()
        .ok_or_else(|| Exception::throw_reference(ctx, "ModuleHookState missing"))?;
    if let Some(client) = binding.borrow().worker.clone() {
        return Ok(client);
    }
    let client = HooksWorkerClient::start().map_err(|e| Exception::throw_message(ctx, &e))?;
    binding.borrow_mut().worker = Some(client.clone());
    Ok(client)
}

fn throw_code(ctx: &Ctx<'_>, ctor_name: &str, code: &str, message: &str) -> rquickjs::Error {
    if let Ok(ctor) = ctx.globals().get::<_, Constructor>(ctor_name) {
        if let Ok(err) = ctor.construct::<_, Object>((message,)) {
            let _ = err.set("code", code);
            return ctx.throw(err.into_value());
        }
    }
    Exception::throw_type(ctx, message)
}

fn js_value_to_url_string(ctx: &Ctx<'_>, val: &Value<'_>, role: &str) -> Result<String> {
    if let Some(s) = val.as_string() {
        return s.to_string();
    }
    if let Some(obj) = val.as_object() {
        if let Ok(href) = obj.get::<_, String>("href") {
            return Ok(href);
        }
    }
    Err(throw_code(
        ctx,
        "TypeError",
        "ERR_INVALID_ARG_TYPE",
        &format!("The \"{role}\" argument must be a string or URL instance"),
    ))
}

fn resolve_specifier_against_parent(
    ctx: &Ctx<'_>,
    specifier: &str,
    parent_url: &str,
) -> Result<String> {
    if let Ok(u) = Url::parse(specifier) {
        return Ok(u.to_string());
    }
    if specifier.starts_with('/') {
        let base = Url::parse(parent_url).map_err(|_| {
            throw_code(
                ctx,
                "TypeError",
                "ERR_UNSUPPORTED_RESOLVE_REQUEST",
                &format!(
                    "Failed to resolve module specifier \"{specifier}\" from \"{parent_url}\": Invalid relative URL or base scheme is not hierarchical."
                ),
            )
        })?;
        return base.join(specifier).map(|u| u.to_string()).map_err(|_| {
            throw_code(
                ctx,
                "TypeError",
                "ERR_UNSUPPORTED_RESOLVE_REQUEST",
                &format!(
                    "Failed to resolve module specifier \"{specifier}\" from \"{parent_url}\": Invalid relative URL or base scheme is not hierarchical."
                ),
            )
        });
    }
    if specifier.starts_with("./") || specifier.starts_with("../") {
        let base = Url::parse(parent_url).map_err(|_| {
            throw_code(
                ctx,
                "TypeError",
                "ERR_UNSUPPORTED_RESOLVE_REQUEST",
                &format!(
                    "Failed to resolve module specifier \"{specifier}\" from \"{parent_url}\": Invalid relative URL or base scheme is not hierarchical."
                ),
            )
        })?;
        if base.cannot_be_a_base() {
            return Err(throw_code(
                ctx,
                "TypeError",
                "ERR_UNSUPPORTED_RESOLVE_REQUEST",
                &format!(
                    "Failed to resolve module specifier \"{specifier}\" from \"{parent_url}\": Invalid relative URL or base scheme is not hierarchical."
                ),
            ));
        }
        return base.join(specifier).map(|u| u.to_string()).map_err(|_| {
            throw_code(
                ctx,
                "TypeError",
                "ERR_UNSUPPORTED_RESOLVE_REQUEST",
                &format!(
                    "Failed to resolve module specifier \"{specifier}\" from \"{parent_url}\": Invalid relative URL or base scheme is not hierarchical."
                ),
            )
        });
    }

    // Bare specifier: try PackageResolver against parent path; if parent isn't hierarchical, throw ERR_UNSUPPORTED_RESOLVE_REQUEST
    let base = Url::parse(parent_url).map_err(|_| {
        throw_code(
            ctx,
            "TypeError",
            "ERR_UNSUPPORTED_RESOLVE_REQUEST",
            &format!(
                "Failed to resolve module specifier \"{specifier}\" from \"{parent_url}\": Invalid relative URL or base scheme is not hierarchical."
            ),
        )
    })?;
    if base.cannot_be_a_base() || base.scheme() != "file" {
        return Err(throw_code(
            ctx,
            "TypeError",
            "ERR_UNSUPPORTED_RESOLVE_REQUEST",
            &format!(
                "Failed to resolve module specifier \"{specifier}\" from \"{parent_url}\": Invalid relative URL or base scheme is not hierarchical."
            ),
        ));
    }
    let parent_path = super::resolver::file_url_to_path_and_query(parent_url)
        .map(|(p, _)| p)
        .unwrap_or_else(|| parent_url.to_string());
    let resolved = super::resolver::default_esm_resolve(ctx, specifier, &parent_path)?;
    Ok(super::resolver::path_or_url_to_file_url(&resolved))
}

/// JS `module.register(specifier[, parentURL][, options])`.
pub fn register<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    let spec_val = args
        .0
        .first()
        .cloned()
        .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    let specifier = if spec_val.is_undefined() {
        "undefined".to_string()
    } else if let Some(s) = spec_val.as_string() {
        s.to_string()?
    } else if let Some(obj) = spec_val.as_object() {
        if let Ok(href) = obj.get::<_, String>("href") {
            href
        } else {
            return Err(throw_code(
                &ctx,
                "TypeError",
                "ERR_UNSUPPORTED_RESOLVE_REQUEST",
                "Failed to resolve module specifier from \"data:\": Invalid relative URL or base scheme is not hierarchical.",
            ));
        }
    } else {
        format!("{spec_val:?}")
    };

    let mut parent_url_opt: Option<String> = None;
    let mut options_val: Option<Object<'js>> = None;

    if let Some(arg2) = args.0.get(1) {
        if !arg2.is_undefined() && !arg2.is_null() {
            if let Some(s) = arg2.as_string() {
                parent_url_opt = Some(s.to_string()?);
            } else if let Some(obj) = arg2.as_object() {
                if let Ok(href) = obj.get::<_, String>("href") {
                    parent_url_opt = Some(href);
                } else {
                    options_val = Some(obj.clone());
                }
            } else {
                return Err(throw_code(
                    &ctx,
                    "TypeError",
                    "ERR_UNSUPPORTED_RESOLVE_REQUEST",
                    &format!(
                        "Failed to resolve module specifier \"{specifier}\": Invalid relative URL or base scheme is not hierarchical."
                    ),
                ));
            }
        }
    }
    if let Some(arg3) = args.0.get(2) {
        if !arg3.is_undefined() && !arg3.is_null() {
            let obj = arg3.as_object().ok_or_else(|| {
                throw_code(
                    &ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options\" argument must be of type object",
                )
            })?;
            options_val = Some(obj.clone());
        }
    }

    if let Some(ref opts) = options_val {
        if parent_url_opt.is_none() {
            if let Ok(pu) = opts.get::<_, Value>("parentURL") {
                if !pu.is_undefined() && !pu.is_null() {
                    parent_url_opt = Some(js_value_to_url_string(&ctx, &pu, "options.parentURL")?);
                }
            }
        }
    }

    let parent_url = parent_url_opt.unwrap_or_else(|| "data:".to_string());
    let hook_url = resolve_specifier_against_parent(&ctx, &specifier, &parent_url)?;

    let (data_val, transfer_val) = if let Some(ref opts) = options_val {
        let data = opts
            .get::<_, Value>("data")
            .unwrap_or_else(|_| Value::new_undefined(ctx.clone()));
        let transfer = opts.get::<_, Value>("transferList").ok();
        (data, transfer)
    } else {
        (Value::new_undefined(ctx.clone()), None)
    };

    let data_wire = if let Some(tv) = transfer_val.filter(|v| !v.is_undefined() && !v.is_null()) {
        let transfer_opts = Object::new(ctx.clone())?;
        transfer_opts.set("transfer", tv)?;
        serialize(&ctx, data_val, Opt(Some(transfer_opts)))?
    } else {
        serialize(&ctx, data_val, Opt(None))?
    };

    let client = ensure_worker_client(&ctx)?;
    let (reply_tx, reply_rx) = std_mpsc::sync_channel(1);
    client
        .tx
        .send(HooksRequest::Register {
            hook_url,
            data_wire,
            reply: reply_tx,
        })
        .map_err(|_| Exception::throw_message(&ctx, "Module hooks worker channel closed"))?;

    let outcome = reply_rx
        .recv()
        .map_err(|_| {
            Exception::throw_message(&ctx, "Module hooks worker terminated during register")
        })?
        .map_err(|e| Exception::throw_message(&ctx, &e))?;

    let hook_id = outcome.hook_id;
    let resolve = if outcome.has_resolve {
        let c = client.clone();
        Some(Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  spec: String,
                  context: Object<'js>,
                  next_resolve: Function<'js>|
                  -> Result<Object<'js>> {
                invoke_worker_resolve(&ctx, &c, hook_id, spec, context, next_resolve)
            },
        )?)
    } else {
        None
    };

    let load = if outcome.has_load {
        let c = client.clone();
        Some(Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  url: String,
                  context: Object<'js>,
                  next_load: Function<'js>|
                  -> Result<Object<'js>> {
                invoke_worker_load(&ctx, &c, hook_id, url, context, next_load)
            },
        )?)
    } else {
        None
    };

    let binding = ctx
        .userdata::<RefCell<ModuleHookState>>()
        .ok_or_else(|| Exception::throw_reference(&ctx, "ModuleHookState missing"))?;
    let mut state = binding.borrow_mut();
    let hooks_len = state.hooks.len();
    if state.hook_ids.len() != hooks_len {
        state.hook_ids.resize(hooks_len, 0);
    }
    state.hook_ids.insert(0, 0);
    state.hooks.insert(0, Hook { resolve, load });

    Ok(())
}

fn read_string_vec(arr_val: Value<'_>) -> Vec<String> {
    let Some(arr) = arr_val.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for v in arr.iter::<Value>().flatten() {
        if let Some(s) = v.as_string().and_then(|s| s.to_string().ok()) {
            out.push(s);
        }
    }
    out
}

fn read_attrs_vec(val: Value<'_>) -> Vec<(String, String)> {
    let Some(obj) = val.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in obj.props::<String, String>().flatten() {
        out.push((k, v));
    }
    out
}

fn format_caught_error(ctx: &Ctx<'_>, err: rquickjs::Error) -> String {
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

fn parse_resolve_obj(obj: &Object<'_>) -> Result<HookResolveOutput> {
    let url = obj.get::<_, Option<String>>("url")?;
    let format = obj.get::<_, Option<String>>("format")?;
    let short_circuit = obj.get::<_, Option<bool>>("shortCircuit")?.unwrap_or(false);
    Ok(HookResolveOutput {
        url,
        format,
        short_circuit,
    })
}

fn build_resolve_obj<'js>(ctx: &Ctx<'js>, out: HookResolveOutput) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    if let Some(url) = out.url {
        obj.set("url", url)?;
    }
    if let Some(fmt) = out.format {
        obj.set("format", fmt)?;
    }
    obj.set("shortCircuit", out.short_circuit)?;
    Ok(obj)
}

fn parse_load_obj<'js>(ctx: &Ctx<'js>, obj: &Object<'js>) -> Result<HookLoadOutput> {
    let format = obj.get::<_, Option<String>>("format")?;
    let short_circuit = obj.get::<_, Option<bool>>("shortCircuit")?.unwrap_or(false);
    let source = if let Ok(src_val) = obj.get::<_, Value>("source") {
        if src_val.is_undefined() || src_val.is_null() {
            None
        } else if let Some(s) = src_val.as_string() {
            Some(HookSourceData::String(s.to_string()?))
        } else {
            let bytes = lexe_utils::bytes::ObjectBytes::from(ctx, &src_val)?;
            Some(HookSourceData::Bytes(bytes.as_bytes(ctx)?.to_vec()))
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

fn build_load_obj<'js>(ctx: &Ctx<'js>, out: HookLoadOutput) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    if let Some(fmt) = out.format {
        obj.set("format", fmt)?;
    }
    obj.set("shortCircuit", out.short_circuit)?;
    if let Some(src) = out.source {
        match src {
            HookSourceData::String(s) => obj.set("source", s)?,
            HookSourceData::Bytes(b) => {
                let ab = ArrayBuffer::new_copy(ctx.clone(), &b)?;
                obj.set("source", ab)?;
            },
        }
    }
    Ok(obj)
}

fn invoke_worker_resolve<'js>(
    ctx: &Ctx<'js>,
    client: &HooksWorkerClient,
    hook_id: u64,
    specifier: String,
    context: Object<'js>,
    next_resolve: Function<'js>,
) -> Result<Object<'js>> {
    let parent_url = context
        .get::<_, Option<String>>("parentURL")?
        .unwrap_or_default();
    let conditions = context
        .get::<_, Value>("conditions")
        .map(read_string_vec)
        .unwrap_or_default();
    let import_attributes = context
        .get::<_, Value>("importAttributes")
        .map(read_attrs_vec)
        .unwrap_or_default();

    let (rpc_tx, rpc_rx) = std_mpsc::sync_channel(16);
    client
        .tx
        .send(HooksRequest::Resolve {
            hook_id,
            specifier,
            parent_url,
            conditions,
            import_attributes,
            rpc_tx,
        })
        .map_err(|_| Exception::throw_message(ctx, "Module hooks worker closed"))?;

    while let Ok(msg) = rpc_rx.recv() {
        match msg {
            WorkerRpcMsg::CallNextResolve {
                specifier,
                parent_url,
                conditions,
                import_attributes,
                reply,
            } => {
                let next_ctx = Object::new(ctx.clone())?;
                if let Some(pu) = parent_url {
                    next_ctx.set("parentURL", pu)?;
                }
                if let Some(conds) = conditions {
                    let arr = Array::new(ctx.clone())?;
                    for (i, c) in conds.iter().enumerate() {
                        arr.set(i, c.as_str())?;
                    }
                    next_ctx.set("conditions", arr)?;
                }
                if let Some(attrs) = import_attributes {
                    let aobj = Object::new(ctx.clone())?;
                    for (k, v) in attrs {
                        aobj.set(k, v)?;
                    }
                    next_ctx.set("importAttributes", aobj)?;
                }
                let res = match next_resolve.call::<_, Object>((specifier, next_ctx)) {
                    Ok(obj) => parse_resolve_obj(&obj).map_err(|e| format_caught_error(ctx, e)),
                    Err(e) => Err(format_caught_error(ctx, e)),
                };
                let _ = reply.send(NextReply::Resolve(res));
            },
            WorkerRpcMsg::ResolveDone(Ok(out)) => return build_resolve_obj(ctx, out),
            WorkerRpcMsg::ResolveDone(Err(msg)) => return Err(Exception::throw_message(ctx, &msg)),
            _ => {},
        }
    }

    Err(Exception::throw_message(
        ctx,
        "Module hooks worker exited during resolve",
    ))
}

fn invoke_worker_load<'js>(
    ctx: &Ctx<'js>,
    client: &HooksWorkerClient,
    hook_id: u64,
    url: String,
    context: Object<'js>,
    next_load: Function<'js>,
) -> Result<Object<'js>> {
    let format = context.get::<_, Option<String>>("format")?;
    let import_attributes = context
        .get::<_, Value>("importAttributes")
        .map(read_attrs_vec)
        .unwrap_or_default();

    let (rpc_tx, rpc_rx) = std_mpsc::sync_channel(16);
    client
        .tx
        .send(HooksRequest::Load {
            hook_id,
            url,
            format,
            import_attributes,
            rpc_tx,
        })
        .map_err(|_| Exception::throw_message(ctx, "Module hooks worker closed"))?;

    while let Ok(msg) = rpc_rx.recv() {
        match msg {
            WorkerRpcMsg::CallNextLoad {
                url,
                format,
                import_attributes,
                reply,
            } => {
                let next_ctx = Object::new(ctx.clone())?;
                if let Some(fmt) = format {
                    next_ctx.set("format", fmt)?;
                }
                if let Some(attrs) = import_attributes {
                    let aobj = Object::new(ctx.clone())?;
                    for (k, v) in attrs {
                        aobj.set(k, v)?;
                    }
                    next_ctx.set("importAttributes", aobj)?;
                }
                let res = match next_load.call::<_, Object>((url, next_ctx)) {
                    Ok(obj) => parse_load_obj(ctx, &obj).map_err(|e| format_caught_error(ctx, e)),
                    Err(e) => Err(format_caught_error(ctx, e)),
                };
                let _ = reply.send(NextReply::Load(res));
            },
            WorkerRpcMsg::LoadDone(Ok(out)) => return build_load_obj(ctx, out),
            WorkerRpcMsg::LoadDone(Err(msg)) => return Err(Exception::throw_message(ctx, &msg)),
            _ => {},
        }
    }

    Err(Exception::throw_message(
        ctx,
        "Module hooks worker exited during load",
    ))
}
