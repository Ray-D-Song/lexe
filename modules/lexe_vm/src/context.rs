// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    ffi::c_void,
    mem::MaybeUninit,
    ptr::{self, NonNull},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use lexe_utils::event_loop::{register_runtime_cleanup, rt_key_from_ptr};
use rquickjs::{
    atom::PredefinedAtom,
    function::{Opt, This},
    object::Property,
    qjs, Ctx, Error, Exception, Function, IntoJs, Object, Result, Value,
};

use crate::{
    errors::{
        into_raw_value, make_dynamic_import_missing_error, throw_invalid_arg_type,
        throw_invalid_arg_value, transfer_exception, JsCString, VmPrimordials,
    },
    interrupt,
};

#[derive(Clone, Copy)]
struct SendCtxPtr(*mut qjs::JSContext);
unsafe impl Send for SendCtxPtr {}

type WasmInitFn = fn(&Ctx<'_>) -> Result<()>;
static WASM_INITIALIZER: OnceLock<WasmInitFn> = OnceLock::new();

pub fn set_wasm_initializer(init: WasmInitFn) {
    let _ = WASM_INITIALIZER.set(init);
}

#[derive(Clone)]
pub struct ContextMeta {
    pub raw_ctx: *mut qjs::JSContext,
    pub alive: Arc<AtomicBool>,
    pub microtask_mode_after_eval: bool,
    pub _name: String,
    pub _origin: String,
    pub _is_dont_contextify: bool,
}

unsafe impl Send for ContextMeta {}

struct StoredDynamicImportHook {
    kind: StoredImportHookKind,
}

enum StoredImportHookKind {
    None,
    UseMainContextDefaultLoader,
    Custom {
        callback: qjs::JSValue,
        referencing: qjs::JSValue,
        cb_ctx: *mut qjs::JSContext,
    },
}

unsafe impl Send for StoredDynamicImportHook {}

pub enum DynamicImportSpec<'js> {
    Missing,
    MainContextDefaultLoader,
    Custom {
        callback: Function<'js>,
        referencing: Value<'js>,
    },
}

struct ActiveImportScopeEntry {
    target_ctx: *mut qjs::JSContext,
    hook: StoredDynamicImportHook,
}

struct SyntheticEntry {
    callback: qjs::JSValue,
    module_js: qjs::JSValue,
    cb_ctx: *mut qjs::JSContext,
}

unsafe impl Send for SyntheticEntry {}

pub struct RuntimeVmState {
    main_ctx: SendCtxPtr,
    weak_map: qjs::JSValue,
    wm_get: qjs::JSValue,
    wm_set: qjs::JSValue,
    dont_contextify_sym: qjs::JSValue,
    default_loader_sym: qjs::JSValue,
    main_object_proto: qjs::JSValue,
    default_import_helper: qjs::JSValue,
    promise_unwrap_helper: qjs::JSValue,
    module_unwrap_fn: Option<qjs::JSValue>,
    bypass_default_import: bool,
    next_context_id: u64,
    next_module_id: u64,
    child_contexts: HashMap<u64, ContextMeta>,
    child_ctx_ptrs: HashMap<usize, bool>,
    active_import_hooks: Vec<ActiveImportScopeEntry>,
    named_import_hooks: HashMap<(usize, String), StoredDynamicImportHook>,
    synthetic_callbacks: HashMap<usize, SyntheticEntry>,
}

unsafe impl Send for RuntimeVmState {}

fn runtime_states() -> &'static Mutex<HashMap<usize, RuntimeVmState>> {
    static STATES: OnceLock<Mutex<HashMap<usize, RuntimeVmState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

unsafe fn free_stored_import_hook(rt: *mut qjs::JSRuntime, hook: StoredDynamicImportHook) {
    if let StoredImportHookKind::Custom {
        callback,
        referencing,
        ..
    } = hook.kind
    {
        qjs::JS_FreeValueRT(rt, callback);
        qjs::JS_FreeValueRT(rt, referencing);
    }
}

fn cleanup_runtime_vm_state(rt_ptr: *mut c_void) {
    let rt = rt_ptr as *mut qjs::JSRuntime;
    interrupt::clear_runtime_interrupts(rt);
    unsafe {
        qjs::JS_SetDynamicImportHook(rt, None, ptr::null_mut());
    }

    let removed = runtime_states().lock().unwrap().remove(&(rt as usize));
    let Some(mut state) = removed else {
        return;
    };

    unsafe {
        for entry in state.active_import_hooks.drain(..) {
            free_stored_import_hook(rt, entry.hook);
        }
        for (_, hook) in state.named_import_hooks.drain() {
            free_stored_import_hook(rt, hook);
        }
        for (_, syn) in state.synthetic_callbacks.drain() {
            qjs::JS_FreeValueRT(rt, syn.callback);
            qjs::JS_FreeValueRT(rt, syn.module_js);
        }
        if let Some(unwrap_fn) = state.module_unwrap_fn.take() {
            qjs::JS_FreeValueRT(rt, unwrap_fn);
        }
        qjs::JS_FreeValueRT(rt, state.weak_map);
        qjs::JS_FreeValueRT(rt, state.wm_get);
        qjs::JS_FreeValueRT(rt, state.wm_set);
        qjs::JS_FreeValueRT(rt, state.dont_contextify_sym);
        qjs::JS_FreeValueRT(rt, state.default_loader_sym);
        qjs::JS_FreeValueRT(rt, state.main_object_proto);
        qjs::JS_FreeValueRT(rt, state.default_import_helper);
        qjs::JS_FreeValueRT(rt, state.promise_unwrap_helper);

        for (_, child) in state.child_contexts.drain() {
            child.alive.store(false, Ordering::SeqCst);
            qjs::JS_SetGlobalObject(child.raw_ctx, qjs::JS_UNDEFINED);
            qjs::JS_FreeContext(child.raw_ctx);
        }
    }
}

pub fn ensure_runtime_state(ctx: &Ctx<'_>) -> Result<*mut qjs::JSRuntime> {
    VmPrimordials::init_if_needed(ctx)?;
    let raw_ctx = ctx.as_raw().as_ptr();
    let rt = unsafe { qjs::JS_GetRuntime(raw_ctx) };
    let rt_key = rt as usize;

    {
        let states = runtime_states().lock().unwrap();
        if states.contains_key(&rt_key) {
            return Ok(rt);
        }
    }

    let helpers: Object<'_> = ctx.eval(
        r#"(() => {
            const wm = new WeakMap();
            return {
                wm,
                wmGet: WeakMap.prototype.get,
                wmSet: WeakMap.prototype.set,
                dontContextify: Symbol("vm_context_no_contextify"),
                defaultLoader: Symbol("vm_dynamic_import_main_context_default"),
                objectProto: Object.prototype,
                defaultImport: (spec, attrs) => attrs !== undefined ? import(spec, { with: attrs }) : import(spec),
                promiseUnwrap: (res, unwrapFn) => unwrapFn ? Promise.resolve(res).then(unwrapFn) : Promise.resolve(res),
            };
        })()"#,
    )?;

    let weak_map = into_raw_value(helpers.get::<_, Value>("wm")?);
    let wm_get = into_raw_value(helpers.get::<_, Value>("wmGet")?);
    let wm_set = into_raw_value(helpers.get::<_, Value>("wmSet")?);
    let dont_contextify_sym = into_raw_value(helpers.get::<_, Value>("dontContextify")?);
    let default_loader_sym = into_raw_value(helpers.get::<_, Value>("defaultLoader")?);
    let main_object_proto = into_raw_value(helpers.get::<_, Value>("objectProto")?);
    let default_import_helper = into_raw_value(helpers.get::<_, Value>("defaultImport")?);
    let promise_unwrap_helper = into_raw_value(helpers.get::<_, Value>("promiseUnwrap")?);

    let mut states = runtime_states().lock().unwrap();
    if states.contains_key(&rt_key) {
        unsafe {
            qjs::JS_FreeValueRT(rt, weak_map);
            qjs::JS_FreeValueRT(rt, wm_get);
            qjs::JS_FreeValueRT(rt, wm_set);
            qjs::JS_FreeValueRT(rt, dont_contextify_sym);
            qjs::JS_FreeValueRT(rt, default_loader_sym);
            qjs::JS_FreeValueRT(rt, main_object_proto);
            qjs::JS_FreeValueRT(rt, default_import_helper);
            qjs::JS_FreeValueRT(rt, promise_unwrap_helper);
        }
        return Ok(rt);
    }

    register_runtime_cleanup(rt_key_from_ptr(rt as *mut c_void), cleanup_runtime_vm_state);
    unsafe {
        qjs::JS_SetDynamicImportHook(rt, Some(vm_dynamic_import_hook), rt as *mut c_void);
    }

    states.insert(
        rt_key,
        RuntimeVmState {
            main_ctx: SendCtxPtr(raw_ctx),
            weak_map,
            wm_get,
            wm_set,
            dont_contextify_sym,
            default_loader_sym,
            main_object_proto,
            default_import_helper,
            promise_unwrap_helper,
            module_unwrap_fn: None,
            bypass_default_import: false,
            next_context_id: 1,
            next_module_id: 0,
            child_contexts: HashMap::new(),
            child_ctx_ptrs: HashMap::new(),
            active_import_hooks: Vec::new(),
            named_import_hooks: HashMap::new(),
            synthetic_callbacks: HashMap::new(),
        },
    );
    Ok(rt)
}

pub fn alloc_module_id(ctx: &Ctx<'_>) -> Result<u64> {
    let rt = ensure_runtime_state(ctx)?;
    let mut states = runtime_states().lock().unwrap();
    let state = states.get_mut(&(rt as usize)).ok_or(Error::Unknown)?;
    let id = state.next_module_id;
    state.next_module_id += 1;
    Ok(id)
}

pub fn set_module_unwrap_fn(ctx: &Ctx<'_>, func: Function<'_>) -> Result<()> {
    let rt = ensure_runtime_state(ctx)?;
    let raw = into_raw_value(func.into_value());
    let mut states = runtime_states().lock().unwrap();
    let state = states.get_mut(&(rt as usize)).ok_or(Error::Unknown)?;
    if let Some(prev) = state.module_unwrap_fn.replace(raw) {
        unsafe {
            qjs::JS_FreeValueRT(rt, prev);
        }
    }
    Ok(())
}

pub fn is_dont_contextify_symbol(ctx: &Ctx<'_>, val: &Value<'_>) -> Result<bool> {
    let rt = ensure_runtime_state(ctx)?;
    let states = runtime_states().lock().unwrap();
    let state = states.get(&(rt as usize)).ok_or(Error::Unknown)?;
    let sym = state.dont_contextify_sym;
    unsafe {
        let b = qjs::JS_IsStrictEqual(ctx.as_raw().as_ptr(), val.as_raw(), sym);
        Ok(b)
    }
}

pub fn is_default_loader_symbol(ctx: &Ctx<'_>, val: &Value<'_>) -> Result<bool> {
    let rt = ensure_runtime_state(ctx)?;
    let states = runtime_states().lock().unwrap();
    let state = states.get(&(rt as usize)).ok_or(Error::Unknown)?;
    let sym = state.default_loader_sym;
    unsafe {
        let b = qjs::JS_IsStrictEqual(ctx.as_raw().as_ptr(), val.as_raw(), sym);
        Ok(b)
    }
}

pub fn build_constants<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let rt = ensure_runtime_state(ctx)?;
    let (dont_sym, loader_sym) = {
        let states = runtime_states().lock().unwrap();
        let state = states.get(&(rt as usize)).ok_or(Error::Unknown)?;
        unsafe {
            (
                Value::from_raw(
                    ctx.clone(),
                    qjs::JS_DupValue(ctx.as_raw().as_ptr(), state.dont_contextify_sym),
                ),
                Value::from_raw(
                    ctx.clone(),
                    qjs::JS_DupValue(ctx.as_raw().as_ptr(), state.default_loader_sym),
                ),
            )
        }
    };

    let obj = unsafe {
        let raw = qjs::JS_NewObjectProto(
            ctx.as_raw().as_ptr(),
            into_raw_value(Value::new_null(ctx.clone())),
        );
        Object::from_value(Value::from_raw(ctx.clone(), raw))
    }?;
    obj.prop(
        "USE_MAIN_CONTEXT_DEFAULT_LOADER",
        Property::from(loader_sym).enumerable(),
    )?;
    obj.prop("DONT_CONTEXTIFY", Property::from(dont_sym).enumerable())?;
    let freeze: Function<'_> = ctx.eval("Object.freeze")?;
    let frozen: Object<'_> = freeze.call((obj,))?;
    Ok(frozen)
}

pub fn is_non_function_object(val: &Value<'_>) -> bool {
    val.is_object() && !val.is_null() && !val.is_function()
}

fn lookup_context_id(ctx: &Ctx<'_>, obj: &Value<'_>) -> Result<Option<u64>> {
    let rt = ensure_runtime_state(ctx)?;
    let (wm, wm_get) = {
        let states = runtime_states().lock().unwrap();
        let state = states.get(&(rt as usize)).ok_or(Error::Unknown)?;
        (state.weak_map, state.wm_get)
    };
    let mut arg = obj.as_raw();
    let res = unsafe { qjs::JS_Call(ctx.as_raw().as_ptr(), wm_get, wm, 1, &mut arg) };
    let val = unsafe { Value::from_raw(ctx.clone(), res) };
    if val.is_undefined() {
        return Ok(None);
    }
    if let Some(num) = val.as_number() {
        if num >= 1.0 && num.is_finite() {
            return Ok(Some(num as u64));
        }
    }
    Ok(None)
}

fn store_context_id(ctx: &Ctx<'_>, obj: &Value<'_>, id: u64) -> Result<()> {
    let rt = ensure_runtime_state(ctx)?;
    let (wm, wm_set) = {
        let states = runtime_states().lock().unwrap();
        let state = states.get(&(rt as usize)).ok_or(Error::Unknown)?;
        (state.weak_map, state.wm_set)
    };
    let id_val = (id as f64).into_js(ctx)?;
    let mut args = [obj.as_raw(), id_val.as_raw()];
    let res = unsafe { qjs::JS_Call(ctx.as_raw().as_ptr(), wm_set, wm, 2, args.as_mut_ptr()) };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(res) == qjs::JS_TAG_EXCEPTION } {
        unsafe { qjs::JS_FreeValue(ctx.as_raw().as_ptr(), res) };
        return Err(Error::Exception);
    }
    unsafe { qjs::JS_FreeValue(ctx.as_raw().as_ptr(), res) };
    Ok(())
}

pub fn lookup_context_meta(ctx: &Ctx<'_>, obj: &Value<'_>) -> Result<Option<ContextMeta>> {
    let Some(id) = lookup_context_id(ctx, obj)? else {
        return Ok(None);
    };
    let rt = ensure_runtime_state(ctx)?;
    let states = runtime_states().lock().unwrap();
    let state = states.get(&(rt as usize)).ok_or(Error::Unknown)?;
    Ok(state.child_contexts.get(&id).cloned())
}

pub fn is_context_js<'js>(ctx: Ctx<'js>, sandbox: Opt<Value<'js>>) -> Result<bool> {
    let val = sandbox
        .0
        .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    is_context_impl(ctx, val)
}

pub fn is_context_impl<'js>(ctx: Ctx<'js>, sandbox: Value<'js>) -> Result<bool> {
    if !is_non_function_object(&sandbox) {
        return Err(throw_invalid_arg_type(
            &ctx,
            "object",
            "of type object",
            &sandbox,
        ));
    }
    Ok(lookup_context_id(&ctx, &sandbox)?.is_some())
}

pub fn move_port_to_context_js<'js>(
    ctx: Ctx<'js>,
    sandbox: Value<'js>,
    factory_script: String,
    bridge: Value<'js>,
) -> Result<Value<'js>> {
    if !is_non_function_object(&sandbox) {
        return Err(throw_invalid_arg_type(
            &ctx,
            "contextifiedSandbox",
            "must be an object",
            &sandbox,
        ));
    }
    let Some(meta) = lookup_context_meta(&ctx, &sandbox)? else {
        return Err(throw_invalid_arg_type(
            &ctx,
            "contextifiedSandbox",
            "must be a vm.Context",
            &sandbox,
        ));
    };
    if !meta.alive.load(Ordering::Acquire) {
        return Err(Exception::throw_type(&ctx, "VM context has been disposed"));
    }
    let target_ctx = unsafe { Ctx::from_raw(NonNull::new_unchecked(meta.raw_ctx)) };
    let factory: Function = target_ctx
        .eval(factory_script)
        .map_err(|_| unsafe { transfer_exception(meta.raw_ctx, &ctx) })?;
    let bridge_raw = into_raw_value(bridge);
    let bridge_in_target: Value = unsafe { Value::from_raw(target_ctx.clone(), bridge_raw) };
    let port_val: Value = factory
        .call((bridge_in_target,))
        .map_err(|_| unsafe { transfer_exception(meta.raw_ctx, &ctx) })?;
    let port_raw = into_raw_value(port_val);
    Ok(unsafe { Value::from_raw(ctx, port_raw) })
}

#[derive(Default)]
pub struct CreateContextOptions {
    pub name: String,
    pub origin: String,
    pub codegen_strings: bool,
    pub codegen_wasm: bool,
    pub microtask_mode_after_eval: bool,
}

impl Default for CreateContextOptionsInternal {
    fn default() -> Self {
        Self {
            name: None,
            origin: String::new(),
            codegen_strings: true,
            codegen_wasm: true,
            microtask_mode_after_eval: false,
        }
    }
}

struct CreateContextOptionsInternal {
    name: Option<String>,
    origin: String,
    codegen_strings: bool,
    codegen_wasm: bool,
    microtask_mode_after_eval: bool,
}

pub fn parse_create_context_options(
    ctx: &Ctx<'_>,
    options: Option<Value<'_>>,
    prefix: &str,
) -> Result<CreateContextOptions> {
    let mut parsed = CreateContextOptionsInternal::default();
    if let Some(opts_val) = options {
        if !opts_val.is_undefined() {
            if !is_non_function_object(&opts_val) {
                return Err(throw_invalid_arg_type(
                    ctx,
                    "options",
                    "of type object",
                    &opts_val,
                ));
            }
            let opts = opts_val.as_object().unwrap();

            let name_key = if prefix.is_empty() {
                "name"
            } else {
                "contextName"
            };
            let name_val: Value = opts.get(name_key)?;
            if !name_val.is_undefined() {
                let s = name_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(
                        ctx,
                        &format!("options.{name_key}"),
                        "of type string",
                        &name_val,
                    )
                })?;
                parsed.name = Some(s.to_string()?);
            }

            let origin_key = if prefix.is_empty() {
                "origin"
            } else {
                "contextOrigin"
            };
            let origin_val: Value = opts.get(origin_key)?;
            if !origin_val.is_undefined() {
                let s = origin_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(
                        ctx,
                        &format!("options.{origin_key}"),
                        "of type string",
                        &origin_val,
                    )
                })?;
                parsed.origin = s.to_string()?;
            }

            let cg_key = if prefix.is_empty() {
                "codeGeneration"
            } else {
                "contextCodeGeneration"
            };
            let cg_val: Value = opts.get(cg_key)?;
            if !cg_val.is_undefined() {
                if !is_non_function_object(&cg_val) {
                    return Err(throw_invalid_arg_type(
                        ctx,
                        &format!("options.{cg_key}"),
                        "of type object",
                        &cg_val,
                    ));
                }
                let cg = cg_val.as_object().unwrap();
                let strings_val: Value = cg.get("strings")?;
                if !strings_val.is_undefined() {
                    parsed.codegen_strings = strings_val.as_bool().ok_or_else(|| {
                        throw_invalid_arg_type(
                            ctx,
                            &format!("options.{cg_key}.strings"),
                            "of type boolean",
                            &strings_val,
                        )
                    })?;
                }
                let wasm_val: Value = cg.get("wasm")?;
                if !wasm_val.is_undefined() {
                    parsed.codegen_wasm = wasm_val.as_bool().ok_or_else(|| {
                        throw_invalid_arg_type(
                            ctx,
                            &format!("options.{cg_key}.wasm"),
                            "of type boolean",
                            &wasm_val,
                        )
                    })?;
                }
            }

            let mt_val: Value = opts.get("microtaskMode")?;
            if !mt_val.is_undefined() {
                let s = mt_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(ctx, "options.microtaskMode", "of type string", &mt_val)
                })?;
                let mode = s.to_string()?;
                match mode.as_str() {
                    "afterEvaluate" => parsed.microtask_mode_after_eval = true,
                    "explicit" => parsed.microtask_mode_after_eval = false,
                    _ => {
                        return Err(throw_invalid_arg_value(
                            ctx,
                            "options.microtaskMode",
                            &format!("{mode:?}"),
                            "must be 'afterEvaluate'",
                        ));
                    },
                }
            }
        }
    }

    Ok(CreateContextOptions {
        name: parsed.name.unwrap_or_default(),
        origin: parsed.origin,
        codegen_strings: parsed.codegen_strings,
        codegen_wasm: parsed.codegen_wasm,
        microtask_mode_after_eval: parsed.microtask_mode_after_eval,
    })
}

pub fn create_context_js<'js>(
    ctx: Ctx<'js>,
    context_object: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    create_context_impl(ctx, context_object.0, options.0)
}

pub fn create_context_impl<'js>(
    ctx: Ctx<'js>,
    context_object: Option<Value<'js>>,
    options: Option<Value<'js>>,
) -> Result<Value<'js>> {
    create_context_with_prefix(&ctx, context_object, options, "")
}

pub fn create_context_with_prefix<'js>(
    ctx: &Ctx<'js>,
    context_object: Option<Value<'js>>,
    options: Option<Value<'js>>,
    option_prefix: &str,
) -> Result<Value<'js>> {
    let rt = ensure_runtime_state(ctx)?;
    let opts = parse_create_context_options(ctx, options, option_prefix)?;

    let (sandbox, is_dont_contextify) = match context_object {
        None => (Object::new(ctx.clone())?.into_value(), false),
        Some(v) if v.is_undefined() => (Object::new(ctx.clone())?.into_value(), false),
        Some(v) if is_dont_contextify_symbol(ctx, &v)? => (v, true),
        Some(v) => {
            if !is_non_function_object(&v) {
                return Err(throw_invalid_arg_type(
                    ctx,
                    "contextObject",
                    "an object",
                    &v,
                ));
            }
            if lookup_context_id(ctx, &v)?.is_some() {
                return Ok(v);
            }
            (v, false)
        },
    };

    let child_raw = unsafe { qjs::JS_NewContext(rt) };
    let Some(child_nonnull) = NonNull::new(child_raw) else {
        return Err(Error::Allocation);
    };
    let child_ctx = unsafe { Ctx::from_raw(child_nonnull) };

    if let Err(err) = install_wasm_in_child(&child_ctx, opts.codegen_wasm) {
        let transferred = if matches!(err, Error::Exception) {
            unsafe { transfer_exception(child_raw, ctx) }
        } else {
            err
        };
        unsafe { qjs::JS_FreeContext(child_raw) };
        return Err(transferred);
    }

    let returned_obj: Value<'js> = if is_dont_contextify {
        let global_raw = unsafe { qjs::JS_GetGlobalObject(child_raw) };
        unsafe { Value::from_raw(ctx.clone(), global_raw) }
    } else {
        let host_proto_raw = {
            let states = runtime_states().lock().unwrap();
            states.get(&(rt as usize)).unwrap().main_object_proto
        };
        let proxy_res = install_proxy_global(ctx, &child_ctx, &sandbox, host_proto_raw);
        let proxy_raw = match proxy_res {
            Ok(raw) => raw,
            Err(err) => {
                unsafe { qjs::JS_FreeContext(child_raw) };
                return Err(err);
            },
        };
        unsafe {
            // JS_SetGlobalObject frees the previous global_obj and takes ownership of proxy_raw.
            qjs::JS_SetGlobalObject(child_raw, proxy_raw);
        }
        sandbox
    };

    if !opts.codegen_strings {
        unsafe {
            qjs::JS_SetCodegenFromStrings(child_raw, false);
        }
    }

    let id = {
        let mut states = runtime_states().lock().unwrap();
        let state = states.get_mut(&(rt as usize)).ok_or(Error::Unknown)?;
        let id = state.next_context_id;
        state.next_context_id += 1;
        let name = if opts.name.is_empty() {
            format!("VM Context {id}")
        } else {
            opts.name
        };
        let meta = ContextMeta {
            raw_ctx: child_raw,
            alive: Arc::new(AtomicBool::new(true)),
            microtask_mode_after_eval: opts.microtask_mode_after_eval,
            _name: name,
            _origin: opts.origin,
            _is_dont_contextify: is_dont_contextify,
        };
        state.child_contexts.insert(id, meta);
        state.child_ctx_ptrs.insert(child_raw as usize, true);
        id
    };

    if let Err(e) = store_context_id(ctx, &returned_obj, id) {
        let mut states = runtime_states().lock().unwrap();
        if let Some(state) = states.get_mut(&(rt as usize)) {
            state.child_contexts.remove(&id);
            state.child_ctx_ptrs.remove(&(child_raw as usize));
        }
        unsafe {
            qjs::JS_SetGlobalObject(child_raw, qjs::JS_UNDEFINED);
            qjs::JS_FreeContext(child_raw);
        }
        return Err(e);
    }

    Ok(returned_obj)
}

fn install_wasm_in_child(child_ctx: &Ctx<'_>, codegen_wasm: bool) -> Result<()> {
    if let Some(wasm_init) = WASM_INITIALIZER.get() {
        wasm_init(child_ctx)?;
        let _: () = child_ctx.eval(
            r#"(() => {
                const desc = Reflect.getOwnPropertyDescriptor(globalThis, "WebAssembly");
                if (desc && desc.configurable) {
                    Reflect.defineProperty(globalThis, "WebAssembly", {
                        value: desc.value,
                        writable: desc.writable,
                        enumerable: false,
                        configurable: desc.configurable,
                    });
                }
            })()"#,
        )?;
    } else if !codegen_wasm {
        let _: () = child_ctx.eval(
            r#"(() => {
                class CompileError extends Error { constructor(m) { super(m); this.name = "CompileError"; } }
                class LinkError extends Error { constructor(m) { super(m); this.name = "LinkError"; } }
                class RuntimeError extends Error { constructor(m) { super(m); this.name = "RuntimeError"; } }
                const wa = {
                    CompileError,
                    LinkError,
                    RuntimeError,
                    Module: function() { throw new CompileError("WebAssembly.Module(): Wasm code generation disallowed by embedder"); },
                    validate: () => true,
                    compile: () => Promise.reject(new CompileError("WebAssembly.compile(): Wasm code generation disallowed by embedder")),
                    instantiate: () => Promise.reject(new CompileError("WebAssembly.instantiate(): Wasm code generation disallowed by embedder")),
                    compileStreaming: () => Promise.reject(new CompileError("WebAssembly.compileStreaming(): Wasm code generation disallowed by embedder")),
                    instantiateStreaming: () => Promise.reject(new CompileError("WebAssembly.instantiateStreaming(): Wasm code generation disallowed by embedder")),
                };
                Object.defineProperty(wa, Symbol.toStringTag, { value: "WebAssembly", configurable: true });
                Object.defineProperty(globalThis, "WebAssembly", { value: wa, writable: true, enumerable: false, configurable: true });
            })()"#,
        )?;
        return Ok(());
    }

    if !codegen_wasm {
        let _: () = child_ctx.eval(
            r#"(() => {
                const wa = globalThis.WebAssembly;
                if (!wa) return;
                const CompileError = wa.CompileError || Error;
                const OrigModule = wa.Module;
                const origInstantiate = wa.instantiate;
                const errMsg = (fnName) => `WebAssembly.${fnName}(): Wasm code generation disallowed by embedder`;
                function DisallowedModule() {
                    throw new CompileError(errMsg("Module"));
                }
                if (OrigModule && OrigModule.prototype) {
                    DisallowedModule.prototype = OrigModule.prototype;
                }
                if (OrigModule && typeof OrigModule.exports === "function") {
                    DisallowedModule.exports = OrigModule.exports;
                }
                if (OrigModule && typeof OrigModule.imports === "function") {
                    DisallowedModule.imports = OrigModule.imports;
                }
                if (OrigModule && typeof OrigModule.customSections === "function") {
                    DisallowedModule.customSections = OrigModule.customSections;
                }
                wa.Module = DisallowedModule;
                wa.compile = () => Promise.reject(new CompileError(errMsg("compile")));
                wa.compileStreaming = () => Promise.reject(new CompileError(errMsg("compileStreaming")));
                wa.instantiate = function(modOrBytes, imports) {
                    if (OrigModule && modOrBytes instanceof OrigModule) {
                        return origInstantiate.call(wa, modOrBytes, imports);
                    }
                    return Promise.reject(new CompileError(errMsg("instantiate")));
                };
                wa.instantiateStreaming = () => Promise.reject(new CompileError(errMsg("instantiateStreaming")));
            })()"#,
        )?;
    }

    Ok(())
}

fn install_proxy_global<'parent, 'child>(
    parent_ctx: &Ctx<'parent>,
    child_ctx: &Ctx<'child>,
    sandbox: &Value<'parent>,
    host_proto_raw: qjs::JSValue,
) -> Result<qjs::JSValue> {
    let setup_res: Result<Function<'child>> = child_ctx.eval(
        r#"(function __lexe_vm_setup(rawGlobal, sandbox, hostObjectProto) {
            const childObjectProto = Object.prototype;
            const ChildSet = Set;
            const setHas = Set.prototype.has;
            const setAdd = Set.prototype.add;
            const hasOwn = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
            const rGet = Reflect.get;
            const rSet = Reflect.set;
            const rHas = Reflect.has;
            const rDesc = Reflect.getOwnPropertyDescriptor;
            const rDef = Reflect.defineProperty;
            const rDel = Reflect.deleteProperty;
            const rOwnKeys = Reflect.ownKeys;
            const rIsExt = Reflect.isExtensible;
            const rPrevExt = Reflect.preventExtensions;
            const rGetProto = Reflect.getPrototypeOf;
            const rSetProto = Reflect.setPrototypeOf;

            function findSandboxProtoOwner(prop) {
                let cur = rGetProto(sandbox);
                while (cur !== null && cur !== hostObjectProto && cur !== childObjectProto) {
                    if (hasOwn(cur, prop)) return cur;
                    cur = rGetProto(cur);
                }
                return null;
            }

            let proxy;
            proxy = new Proxy(rawGlobal, {
                get(target, prop, receiver) {
                    const rawDesc = rDesc(rawGlobal, prop);
                    if (rawDesc && !rawDesc.configurable && !rawDesc.writable && !rawDesc.get && !rawDesc.set) {
                        return rawDesc.value;
                    }
                    if (hasOwn(sandbox, prop)) {
                        return rGet(sandbox, prop, receiver === proxy ? sandbox : receiver);
                    }
                    if (rawDesc) {
                        if (rawDesc.get || rawDesc.set) {
                            return rGet(rawGlobal, prop, receiver === proxy ? rawGlobal : receiver);
                        }
                        return rawDesc.value;
                    }
                    const owner = findSandboxProtoOwner(prop);
                    if (owner !== null) {
                        return rGet(owner, prop, receiver === proxy ? sandbox : receiver);
                    }
                    return rGet(rawGlobal, prop, receiver === proxy ? rawGlobal : receiver);
                },
                set(target, prop, value, receiver) {
                    const rawDesc = rDesc(rawGlobal, prop);
                    if (rawDesc && !rawDesc.configurable) {
                        if (!rawDesc.writable && !rawDesc.set) {
                            return false;
                        }
                        return rSet(rawGlobal, prop, value, receiver === proxy ? rawGlobal : receiver);
                    }
                    return rSet(sandbox, prop, value, receiver === proxy ? sandbox : receiver);
                },
                has(target, prop) {
                    if (hasOwn(sandbox, prop) || hasOwn(rawGlobal, prop)) {
                        return true;
                    }
                    if (findSandboxProtoOwner(prop) !== null) {
                        return true;
                    }
                    return rHas(rawGlobal, prop);
                },
                getOwnPropertyDescriptor(target, prop) {
                    const rawDesc = rDesc(rawGlobal, prop);
                    if (rawDesc && !rawDesc.configurable) {
                        const sbDesc = rDesc(sandbox, prop);
                        if (sbDesc && !sbDesc.configurable && sbDesc.writable && rawDesc.writable) {
                            rDef(rawGlobal, prop, sbDesc);
                            return sbDesc;
                        }
                        return rawDesc;
                    }
                    const sbDesc = rDesc(sandbox, prop);
                    if (sbDesc !== undefined) {
                        if (!sbDesc.configurable && rIsExt(rawGlobal)) {
                            rDef(rawGlobal, prop, sbDesc);
                        }
                        return sbDesc;
                    }
                    return rawDesc;
                },
                defineProperty(target, prop, desc) {
                    const rawDesc = rDesc(rawGlobal, prop);
                    if (rawDesc && !rawDesc.configurable) {
                        return rDef(rawGlobal, prop, desc);
                    }
                    const ok = rDef(sandbox, prop, desc);
                    if (ok) {
                        const finalDesc = rDesc(sandbox, prop);
                        if (finalDesc && !finalDesc.configurable && rIsExt(rawGlobal)) {
                            rDef(rawGlobal, prop, finalDesc);
                        }
                    }
                    return ok;
                },
                deleteProperty(target, prop) {
                    const rawDesc = rDesc(rawGlobal, prop);
                    if (rawDesc && !rawDesc.configurable) {
                        return false;
                    }
                    let ok = true;
                    if (hasOwn(sandbox, prop)) {
                        ok = rDel(sandbox, prop);
                    }
                    if (ok && rawDesc) {
                        ok = rDel(rawGlobal, prop);
                    }
                    return ok;
                },
                ownKeys(target) {
                    const seen = new ChildSet();
                    const out = [];
                    let i = 0;
                    const sbKeys = rOwnKeys(sandbox);
                    for (let j = 0; j < sbKeys.length; j++) {
                        const k = sbKeys[j];
                        if (!setHas.call(seen, k)) {
                            setAdd.call(seen, k);
                            out[i++] = k;
                        }
                    }
                    const rawKeys = rOwnKeys(rawGlobal);
                    for (let j = 0; j < rawKeys.length; j++) {
                        const k = rawKeys[j];
                        if (!setHas.call(seen, k)) {
                            setAdd.call(seen, k);
                            out[i++] = k;
                        }
                    }
                    return out;
                },
                getPrototypeOf() {
                    return rGetProto(rawGlobal);
                },
                setPrototypeOf(target, proto) {
                    return rSetProto(rawGlobal, proto);
                },
                isExtensible() {
                    return rIsExt(rawGlobal);
                },
                preventExtensions() {
                    if (rIsExt(rawGlobal)) {
                        const sbKeys = rOwnKeys(sandbox);
                        for (let j = 0; j < sbKeys.length; j++) {
                            const k = sbKeys[j];
                            const d = rDesc(sandbox, k);
                            if (d && !d.configurable && !rDesc(rawGlobal, k)) {
                                rDef(rawGlobal, k, d);
                            }
                        }
                    }
                    const a = rPrevExt(sandbox);
                    const b = rPrevExt(rawGlobal);
                    return a && b;
                },
            });

            rDef(rawGlobal, "globalThis", {
                value: proxy,
                writable: true,
                enumerable: false,
                configurable: true,
            });
            return proxy;
        })"#,
    );

    let setup_fn = match setup_res {
        Ok(f) => f,
        Err(Error::Exception) => {
            return Err(unsafe { transfer_exception(child_ctx.as_raw().as_ptr(), parent_ctx) })
        },
        Err(e) => return Err(e),
    };

    let child_raw = child_ctx.as_raw().as_ptr();
    let raw_global_val =
        unsafe { Value::from_raw(child_ctx.clone(), qjs::JS_GetGlobalObject(child_raw)) };
    let sb_dup = unsafe {
        Value::from_raw(
            child_ctx.clone(),
            qjs::JS_DupValue(child_raw, sandbox.as_raw()),
        )
    };
    let proto_dup = unsafe {
        Value::from_raw(
            child_ctx.clone(),
            qjs::JS_DupValue(child_raw, host_proto_raw),
        )
    };

    let res = setup_fn.call::<_, Value<'child>>((raw_global_val, sb_dup, proto_dup));
    match res {
        Ok(proxy_val) => Ok(into_raw_value(proxy_val)),
        Err(Error::Exception) => Err(unsafe { transfer_exception(child_raw, parent_ctx) }),
        Err(e) => Err(e),
    }
}

pub fn drain_child_microtasks_if_needed(
    parent_ctx: &Ctx<'_>,
    target_ctx: *mut qjs::JSContext,
    after_eval: bool,
    guard: &interrupt::InterruptGuard,
) -> Result<()> {
    if !after_eval {
        return Ok(());
    }
    let rt = unsafe { qjs::JS_GetRuntime(target_ctx) };
    loop {
        let rc = unsafe { qjs::JS_ExecutePendingJobForContext(rt, target_ctx) };
        if let Some(err) = unsafe { guard.map_triggered_error(target_ctx, parent_ctx) } {
            return Err(err);
        }
        if rc == 0 {
            return Ok(());
        }
        if rc < 0 {
            return Err(unsafe { transfer_exception(target_ctx, parent_ctx) });
        }
    }
}

pub fn measure_memory_js<'js>(ctx: Ctx<'js>, options: Opt<Value<'js>>) -> Result<Value<'js>> {
    measure_memory_impl(ctx, options.0)
}

pub fn measure_memory_impl<'js>(ctx: Ctx<'js>, options: Option<Value<'js>>) -> Result<Value<'js>> {
    let rt = ensure_runtime_state(&ctx)?;
    let mut mode = "summary".to_string();
    let mut eager = false;

    if let Some(opts_val) = options {
        if !opts_val.is_undefined() {
            if !is_non_function_object(&opts_val) {
                return Err(throw_invalid_arg_type(
                    &ctx,
                    "options",
                    "of type object",
                    &opts_val,
                ));
            }
            let opts = opts_val.as_object().unwrap();
            let mode_val: Value = opts.get("mode")?;
            if !mode_val.is_undefined() {
                let s = mode_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(&ctx, "options.mode", "of type string", &mode_val)
                })?;
                mode = s.to_string()?;
                if mode != "summary" && mode != "detailed" {
                    return Err(throw_invalid_arg_value(
                        &ctx,
                        "options.mode",
                        &format!("{mode:?}"),
                        "must be 'summary' or 'detailed'",
                    ));
                }
            }
            let exec_val: Value = opts.get("execution")?;
            if !exec_val.is_undefined() {
                let s = exec_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(&ctx, "options.execution", "of type string", &exec_val)
                })?;
                let exec = s.to_string()?;
                match exec.as_str() {
                    "default" => eager = false,
                    "eager" => eager = true,
                    _ => {
                        return Err(throw_invalid_arg_value(
                            &ctx,
                            "options.execution",
                            &format!("{exec:?}"),
                            "must be 'default' or 'eager'",
                        ));
                    },
                }
            }
        }
    }

    if eager {
        unsafe { qjs::JS_RunGC(rt) };
    }

    let mut usage = MaybeUninit::<qjs::JSMemoryUsage>::uninit();
    unsafe { qjs::JS_ComputeMemoryUsage(rt, usage.as_mut_ptr()) };
    let usage = unsafe { usage.assume_init() };
    let estimate = (usage.memory_used_size.max(0)) as f64;

    let make_mem = |ctx: &Ctx<'js>, est: f64| -> Result<Object<'js>> {
        let obj = Object::new(ctx.clone())?;
        obj.set("jsMemoryEstimate", est)?;
        obj.set("jsMemoryRange", vec![est, est])?;
        Ok(obj)
    };

    let result = Object::new(ctx.clone())?;
    result.set("total", make_mem(&ctx, estimate)?)?;
    if mode == "detailed" {
        result.set("current", make_mem(&ctx, estimate)?)?;
        result.set("other", Vec::<Value<'js>>::new())?;
    }
    let wasm = Object::new(ctx.clone())?;
    wasm.set("code", 0.0_f64)?;
    wasm.set("metadata", 0.0_f64)?;
    result.set("WebAssembly", wasm)?;

    let promise_ctor: Object<'_> = ctx.globals().get(PredefinedAtom::Promise)?;
    let resolve_fn: Function<'_> = promise_ctor.get("resolve")?;
    let promise: Value<'_> = resolve_fn.call((This(promise_ctor), result))?;
    Ok(promise)
}

pub fn parse_dynamic_import_option<'js>(
    ctx: &Ctx<'js>,
    val: &Value<'js>,
    referencing: &Value<'js>,
    prop_name: &str,
) -> Result<DynamicImportSpec<'js>> {
    if val.is_undefined() {
        return Ok(DynamicImportSpec::Missing);
    }
    if is_default_loader_symbol(ctx, val)? {
        return Ok(DynamicImportSpec::MainContextDefaultLoader);
    }
    if let Some(f) = val.clone().into_function() {
        return Ok(DynamicImportSpec::Custom {
            callback: f,
            referencing: referencing.clone(),
        });
    }
    Err(throw_invalid_arg_type(
        ctx,
        prop_name,
        "of type function or vm.constants.USE_MAIN_CONTEXT_DEFAULT_LOADER",
        val,
    ))
}

fn to_stored_import_hook(ctx: &Ctx<'_>, spec: &DynamicImportSpec<'_>) -> StoredDynamicImportHook {
    let ctx_ptr = ctx.as_raw().as_ptr();
    match spec {
        DynamicImportSpec::Missing => StoredDynamicImportHook {
            kind: StoredImportHookKind::None,
        },
        DynamicImportSpec::MainContextDefaultLoader => StoredDynamicImportHook {
            kind: StoredImportHookKind::UseMainContextDefaultLoader,
        },
        DynamicImportSpec::Custom {
            callback,
            referencing,
        } => StoredDynamicImportHook {
            kind: StoredImportHookKind::Custom {
                callback: unsafe { qjs::JS_DupValue(ctx_ptr, callback.as_raw()) },
                referencing: unsafe { qjs::JS_DupValue(ctx_ptr, referencing.as_raw()) },
                cb_ctx: ctx_ptr,
            },
        },
    }
}

pub fn register_named_import_hook(
    ctx: &Ctx<'_>,
    target_ctx: *mut qjs::JSContext,
    name: &str,
    spec: &DynamicImportSpec<'_>,
) -> Result<()> {
    let rt = ensure_runtime_state(ctx)?;
    let key = (target_ctx as usize, name.to_string());
    let mut states = runtime_states().lock().unwrap();
    let state = states.get_mut(&(rt as usize)).ok_or(Error::Unknown)?;
    if matches!(spec, DynamicImportSpec::Missing) {
        if let Some(old) = state.named_import_hooks.remove(&key) {
            unsafe { free_stored_import_hook(rt, old) };
        }
        return Ok(());
    }
    let hook = to_stored_import_hook(ctx, spec);
    if let Some(old) = state.named_import_hooks.insert(key, hook) {
        unsafe { free_stored_import_hook(rt, old) };
    }
    Ok(())
}

pub struct DynamicImportGuard {
    rt: *mut qjs::JSRuntime,
    active: bool,
}

impl DynamicImportGuard {
    pub fn enter(
        ctx: &Ctx<'_>,
        target_ctx: *mut qjs::JSContext,
        spec: &DynamicImportSpec<'_>,
    ) -> Result<Self> {
        let rt = ensure_runtime_state(ctx)?;
        let hook = to_stored_import_hook(ctx, spec);
        let mut states = runtime_states().lock().unwrap();
        let state = states.get_mut(&(rt as usize)).ok_or(Error::Unknown)?;
        state
            .active_import_hooks
            .push(ActiveImportScopeEntry { target_ctx, hook });
        Ok(Self { rt, active: true })
    }
}

impl Drop for DynamicImportGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut states = runtime_states().lock().unwrap();
        if let Some(state) = states.get_mut(&(self.rt as usize)) {
            if let Some(entry) = state.active_import_hooks.pop() {
                unsafe { free_stored_import_hook(self.rt, entry.hook) };
            }
        }
    }
}

pub fn register_synthetic_module(
    ctx: &Ctx<'_>,
    module_ptr: *mut qjs::JSModuleDef,
    callback: Function<'_>,
    module_js: Value<'_>,
) -> Result<()> {
    let rt = ensure_runtime_state(ctx)?;
    let cb_ctx = ctx.as_raw().as_ptr();
    let cb_raw = unsafe { qjs::JS_DupValue(cb_ctx, callback.as_raw()) };
    let mod_raw = unsafe { qjs::JS_DupValue(cb_ctx, module_js.as_raw()) };
    let mut states = runtime_states().lock().unwrap();
    let state = states.get_mut(&(rt as usize)).ok_or(Error::Unknown)?;
    if let Some(prev) = state.synthetic_callbacks.insert(
        module_ptr as usize,
        SyntheticEntry {
            callback: cb_raw,
            module_js: mod_raw,
            cb_ctx,
        },
    ) {
        unsafe {
            qjs::JS_FreeValueRT(rt, prev.callback);
            qjs::JS_FreeValueRT(rt, prev.module_js);
        }
    }
    Ok(())
}

pub unsafe extern "C" fn synthetic_module_init(
    ctx: *mut qjs::JSContext,
    m: *mut qjs::JSModuleDef,
) -> std::ffi::c_int {
    let rt = qjs::JS_GetRuntime(ctx);
    let entry_dup = {
        let states = runtime_states().lock().unwrap();
        let Some(state) = states.get(&(rt as usize)) else {
            return 0;
        };
        let Some(entry) = state.synthetic_callbacks.get(&(m as usize)) else {
            return 0;
        };
        (
            qjs::JS_DupValue(entry.cb_ctx, entry.callback),
            qjs::JS_DupValue(entry.cb_ctx, entry.module_js),
            entry.cb_ctx,
        )
    };

    let (cb_raw, mod_raw, cb_ctx) = entry_dup;
    let ret = qjs::JS_Call(cb_ctx, cb_raw, mod_raw, 0, ptr::null_mut());
    qjs::JS_FreeValue(cb_ctx, cb_raw);
    qjs::JS_FreeValue(cb_ctx, mod_raw);
    if qjs::JS_VALUE_GET_NORM_TAG(ret) == qjs::JS_TAG_EXCEPTION {
        qjs::JS_FreeValue(cb_ctx, ret);
        if cb_ctx != ctx {
            let exc = qjs::JS_GetException(cb_ctx);
            qjs::JS_Throw(ctx, exc);
        }
        return -1;
    }
    qjs::JS_FreeValue(cb_ctx, ret);
    0
}

enum ResolvedHookAction {
    NotHandled,
    Missing,
    DefaultLoader {
        main_ctx: *mut qjs::JSContext,
        helper: qjs::JSValue,
    },
    Custom {
        callback: qjs::JSValue,
        referencing: qjs::JSValue,
        cb_ctx: *mut qjs::JSContext,
        promise_unwrap_helper: qjs::JSValue,
        module_unwrap_fn: Option<qjs::JSValue>,
    },
}

unsafe fn clone_action_from_hook(
    state: &RuntimeVmState,
    hook: &StoredDynamicImportHook,
) -> ResolvedHookAction {
    match &hook.kind {
        StoredImportHookKind::None => ResolvedHookAction::Missing,
        StoredImportHookKind::UseMainContextDefaultLoader => ResolvedHookAction::DefaultLoader {
            main_ctx: state.main_ctx.0,
            helper: qjs::JS_DupValue(state.main_ctx.0, state.default_import_helper),
        },
        StoredImportHookKind::Custom {
            callback,
            referencing,
            cb_ctx,
        } => ResolvedHookAction::Custom {
            callback: qjs::JS_DupValue(*cb_ctx, *callback),
            referencing: qjs::JS_DupValue(*cb_ctx, *referencing),
            cb_ctx: *cb_ctx,
            promise_unwrap_helper: qjs::JS_DupValue(*cb_ctx, state.promise_unwrap_helper),
            module_unwrap_fn: state.module_unwrap_fn.map(|f| qjs::JS_DupValue(*cb_ctx, f)),
        },
    }
}

unsafe extern "C" fn vm_dynamic_import_hook(
    ctx: *mut qjs::JSContext,
    basename: qjs::JSValue,
    specifier: qjs::JSValue,
    attributes: qjs::JSValue,
    opaque: *mut c_void,
) -> qjs::JSValue {
    let rt = opaque as *mut qjs::JSRuntime;
    let caller_ctx = Ctx::from_raw(NonNull::new_unchecked(ctx));
    let base_val = Value::from_raw(caller_ctx.clone(), qjs::JS_DupValue(ctx, basename));
    let base_name = JsCString::from_value(&caller_ctx, &base_val)
        .ok()
        .and_then(|s| s.as_str().ok().map(|str_ref| str_ref.to_string()));
    drop(base_val);

    let action = {
        let states = runtime_states().lock().unwrap();
        let Some(state) = states.get(&(rt as usize)) else {
            return qjs::JS_UNINITIALIZED;
        };
        if state.bypass_default_import {
            ResolvedHookAction::NotHandled
        } else if let Some(top) = state
            .active_import_hooks
            .iter()
            .rev()
            .find(|e| e.target_ctx == ctx)
        {
            clone_action_from_hook(state, &top.hook)
        } else if let Some(name) = base_name.as_ref() {
            if let Some(named) = state.named_import_hooks.get(&(ctx as usize, name.clone())) {
                clone_action_from_hook(state, named)
            } else if state.child_ctx_ptrs.contains_key(&(ctx as usize)) {
                ResolvedHookAction::Missing
            } else {
                ResolvedHookAction::NotHandled
            }
        } else if state.child_ctx_ptrs.contains_key(&(ctx as usize)) {
            ResolvedHookAction::Missing
        } else {
            ResolvedHookAction::NotHandled
        }
    };

    match action {
        ResolvedHookAction::NotHandled => qjs::JS_UNINITIALIZED,
        ResolvedHookAction::Missing => {
            let err = make_dynamic_import_missing_error(&caller_ctx);
            qjs::JS_Throw(ctx, into_raw_value(err));
            qjs::JS_EXCEPTION
        },
        ResolvedHookAction::DefaultLoader { main_ctx, helper } => {
            {
                let mut states = runtime_states().lock().unwrap();
                if let Some(state) = states.get_mut(&(rt as usize)) {
                    state.bypass_default_import = true;
                }
            }
            let spec_dup = qjs::JS_DupValue(main_ctx, specifier);
            let attrs_dup = qjs::JS_DupValue(main_ctx, attributes);
            let mut args = [spec_dup, attrs_dup];
            let res = qjs::JS_Call(main_ctx, helper, qjs::JS_UNDEFINED, 2, args.as_mut_ptr());
            qjs::JS_FreeValue(main_ctx, helper);
            qjs::JS_FreeValue(main_ctx, spec_dup);
            qjs::JS_FreeValue(main_ctx, attrs_dup);
            {
                let mut states = runtime_states().lock().unwrap();
                if let Some(state) = states.get_mut(&(rt as usize)) {
                    state.bypass_default_import = false;
                }
            }
            if qjs::JS_VALUE_GET_NORM_TAG(res) == qjs::JS_TAG_EXCEPTION {
                qjs::JS_FreeValue(main_ctx, res);
                if main_ctx != ctx {
                    let exc = qjs::JS_GetException(main_ctx);
                    qjs::JS_Throw(ctx, exc);
                }
                return qjs::JS_EXCEPTION;
            }
            res
        },
        ResolvedHookAction::Custom {
            callback,
            referencing,
            cb_ctx,
            promise_unwrap_helper,
            module_unwrap_fn,
        } => {
            let attrs_arg = if qjs::JS_VALUE_GET_NORM_TAG(attributes) == qjs::JS_TAG_UNDEFINED {
                qjs::JS_NewObjectProto(
                    cb_ctx,
                    into_raw_value(Value::new_null(Ctx::from_raw(NonNull::new_unchecked(
                        cb_ctx,
                    )))),
                )
            } else {
                qjs::JS_DupValue(cb_ctx, attributes)
            };
            let spec_dup = qjs::JS_DupValue(cb_ctx, specifier);
            let mut args = [spec_dup, referencing, attrs_arg];
            let res = qjs::JS_Call(cb_ctx, callback, qjs::JS_UNDEFINED, 3, args.as_mut_ptr());
            qjs::JS_FreeValue(cb_ctx, callback);
            qjs::JS_FreeValue(cb_ctx, spec_dup);
            qjs::JS_FreeValue(cb_ctx, referencing);
            qjs::JS_FreeValue(cb_ctx, attrs_arg);

            if qjs::JS_VALUE_GET_NORM_TAG(res) == qjs::JS_TAG_EXCEPTION {
                qjs::JS_FreeValue(cb_ctx, res);
                qjs::JS_FreeValue(cb_ctx, promise_unwrap_helper);
                if let Some(ufn) = module_unwrap_fn {
                    qjs::JS_FreeValue(cb_ctx, ufn);
                }
                if cb_ctx != ctx {
                    let exc = qjs::JS_GetException(cb_ctx);
                    qjs::JS_Throw(ctx, exc);
                }
                return qjs::JS_EXCEPTION;
            }

            let ufn_arg = module_unwrap_fn.unwrap_or(qjs::JS_UNDEFINED);
            let mut wrap_args = [res, ufn_arg];
            let wrapped = qjs::JS_Call(
                cb_ctx,
                promise_unwrap_helper,
                qjs::JS_UNDEFINED,
                2,
                wrap_args.as_mut_ptr(),
            );
            qjs::JS_FreeValue(cb_ctx, res);
            qjs::JS_FreeValue(cb_ctx, ufn_arg);
            qjs::JS_FreeValue(cb_ctx, promise_unwrap_helper);
            if qjs::JS_VALUE_GET_NORM_TAG(wrapped) == qjs::JS_TAG_EXCEPTION {
                qjs::JS_FreeValue(cb_ctx, wrapped);
                if cb_ctx != ctx {
                    let exc = qjs::JS_GetException(cb_ctx);
                    qjs::JS_Throw(ctx, exc);
                }
                return qjs::JS_EXCEPTION;
            }
            wrapped
        },
    }
}
