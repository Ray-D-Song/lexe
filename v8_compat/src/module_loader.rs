use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_char;
use std::ptr;
use std::sync::OnceLock;

use parking_lot::Mutex;
use rquickjs::qjs::{self, JSContext, JSValue};

use crate::abi::NODE_MODULE_VERSION;
use crate::bridge::with_bridge_roots;
use crate::isolate::{ContextState, IsolateState};

thread_local! {
    static NATIVE_LOAD_STACK: RefCell<Vec<NativeLoadFrame>> = const { RefCell::new(Vec::new()) };
}

static ISOLATES: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
static CONTEXTS: OnceLock<Mutex<HashMap<usize, ContextRecord>>> = OnceLock::new();

#[derive(Clone, Copy)]
struct ContextRecord {
    runtime_key: usize,
    state_ptr: usize,
}

fn isolates() -> &'static Mutex<HashMap<usize, usize>> {
    ISOLATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn contexts() -> &'static Mutex<HashMap<usize, ContextRecord>> {
    CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone, Copy)]
pub struct NativeLoadFrame {
    pub ctx: *mut JSContext,
    pub isolate: *mut IsolateState,
    pub context_state: *mut ContextState,
    pub pending_modules_start: usize,
}

pub struct NativeLoadGuard {
    frame: NativeLoadFrame,
}

impl NativeLoadGuard {
    pub fn enter(
        ctx: *mut JSContext,
        isolate: *mut IsolateState,
        context_state: *mut ContextState,
    ) -> Self {
        let pending_modules_start = pending_modules_count();
        let frame = NativeLoadFrame {
            ctx,
            isolate,
            context_state,
            pending_modules_start,
        };
        NATIVE_LOAD_STACK.with(|stack| stack.borrow_mut().push(frame));
        crate::bridge::set_active_bridge_context(ctx);
        unsafe {
            lexe_v8_set_current_context(context_state as *mut _);
            lexe_v8_set_current_isolate(isolate as *mut _);
        }
        Self { frame }
    }
}

impl NativeLoadGuard {
    pub fn pending_modules_start(&self) -> usize {
        self.frame.pending_modules_start
    }
}

impl Drop for NativeLoadGuard {
    fn drop(&mut self) {
        NATIVE_LOAD_STACK.with(|stack| {
            let mut s = stack.borrow_mut();
            s.pop();
            if let Some(prev) = s.last() {
                unsafe {
                    lexe_v8_set_current_context(prev.context_state as *mut _);
                    lexe_v8_set_current_isolate(prev.isolate as *mut _);
                }
                crate::bridge::set_active_bridge_context(prev.ctx);
            } else {
                unsafe {
                    lexe_v8_set_current_context(ptr::null_mut());
                    lexe_v8_set_current_isolate(ptr::null_mut());
                }
                crate::bridge::clear_active_bridge_context();
            }
        });
    }
}

pub fn push_native_load(
    ctx: *mut JSContext,
    isolate: *mut IsolateState,
    context_state: *mut ContextState,
) -> NativeLoadGuard {
    NativeLoadGuard::enter(ctx, isolate, context_state)
}

#[repr(C)]
pub struct NodeModule {
    pub nm_version: i32,
    pub nm_flags: u32,
    pub nm_dso_handle: *mut std::ffi::c_void,
    pub nm_filename: *const c_char,
    pub nm_register_func: Option<
        unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void),
    >,
    pub nm_context_register_func: Option<
        unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ),
    >,
    pub nm_modname: *const c_char,
    pub nm_priv: *mut std::ffi::c_void,
    pub nm_link: *mut NodeModule,
}

pub struct V8ModuleRecord {
    pub module: *mut NodeModule,
}

extern "C" {
    fn lexe_v8_set_current_context(ctx: *mut ContextState);
    fn lexe_v8_set_current_isolate(isolate: *mut IsolateState);
    fn lexe_v8_current_context() -> *mut ContextState;
    fn lexe_v8_current_isolate() -> *mut IsolateState;
    fn lexe_v8_create_isolate() -> *mut IsolateState;
    fn lexe_v8_destroy_isolate(isolate: *mut IsolateState);
    fn lexe_v8_create_context() -> *mut ContextState;
    fn lexe_v8_destroy_context(ctx: *mut ContextState);
    fn lexe_v8_open_handle_scope(ctx: *mut ContextState);
    fn lexe_v8_close_handle_scope(ctx: *mut ContextState);
    fn lexe_v8_set_context_root_id(ctx: *mut ContextState, root_id: u64);
    fn lexe_v8_set_context_quickjs_key(ctx: *mut ContextState, key: usize);
    fn lexe_v8_run_module_init(
        ctx: *mut ContextState,
        module: *mut NodeModule,
        exports_root_id: u64,
        module_root_id: u64,
        out_exports_root_id: *mut u64,
    ) -> i32;
}

/// Install C++/Rust active context+isolate for ObjectWrap weak-callback teardown.
///
/// Teardown must not rely on whatever TLS a previous V8 callback happened to leave.
/// Callers that never registered a C++ ContextState (bridge-only / pure JS) must
/// not invoke this; use [`try_activate_v8_context_for_teardown`] instead.
///
/// # Safety
///
/// `ctx` must be a live QuickJS context on the current thread.
pub(crate) unsafe fn activate_v8_context_for_teardown(ctx: *mut JSContext) -> Result<(), String> {
    if ctx.is_null() {
        return Err("cannot activate a null V8 teardown context".into());
    }

    let runtime = unsafe { qjs::JS_GetRuntime(ctx) };
    if runtime.is_null() {
        return Err("V8 teardown context has no QuickJS runtime".into());
    }

    let context_state = context_state_ptr_for_ctx(ctx)
        .ok_or_else(|| "V8 teardown context state is not registered".to_string())?;

    let isolate = isolate_ptr_for_runtime(runtime as usize)
        .ok_or_else(|| "V8 teardown isolate is not registered".to_string())?;

    unsafe {
        lexe_v8_set_current_context(context_state as *mut ContextState);
        lexe_v8_set_current_isolate(isolate as *mut IsolateState);
    }
    crate::bridge::set_active_bridge_context(ctx);

    let active_ctx = unsafe { lexe_v8_current_context() };
    let active_iso = unsafe { lexe_v8_current_isolate() };
    if active_ctx != context_state as *mut ContextState
        || active_iso != isolate as *mut IsolateState
    {
        return Err("failed to activate the V8 teardown context".into());
    }

    Ok(())
}

/// Activate teardown TLS when C++ context state exists; otherwise allow a
/// bridge-only teardown only when no C++-owned resources are registered.
///
/// A weak callback can be registered on a context that never went through
/// `ensure_context_for_ctx` (see `register_weak_callback`'s
/// `with_state_for_object_root` fallback). Bridge-only teardown is therefore
/// permitted only when `teardown_counts_for_ctx` reports no weak callbacks,
/// pending weaks, or persistents for this context.
///
/// # Safety
///
/// `ctx` must be a live QuickJS context on the current thread.
pub(crate) unsafe fn try_activate_v8_context_for_teardown(
    ctx: *mut JSContext,
) -> Result<(), String> {
    if ctx.is_null() {
        return Err("cannot activate a null V8 teardown context".into());
    }
    if context_state_ptr_for_ctx(ctx).is_none() {
        let counts = crate::bridge::teardown_counts_for_ctx(ctx);
        if counts.weak_callbacks != 0
            || counts.pending_weak != 0
            || counts.strong_persistents != 0
            || counts.weak_persistents != 0
        {
            return Err(format!(
                "V8 teardown has C++-owned resources without a registered context: {counts:?}"
            ));
        }
        crate::bridge::set_active_bridge_context(ctx);
        return Ok(());
    }
    unsafe { activate_v8_context_for_teardown(ctx) }
}

pub fn ensure_isolate_for_runtime(rt: *mut qjs::JSRuntime) -> *mut IsolateState {
    let key = rt as usize;
    let mut guard = isolates().lock();
    if let Some(isolate) = guard.get(&key) {
        return *isolate as *mut IsolateState;
    }
    let isolate = unsafe { lexe_v8_create_isolate() };
    guard.insert(key, isolate as usize);
    crate::runtime_state::isolate_key(isolate as usize);
    isolate
}

pub fn js_context_for_state(state: *mut ContextState) -> Option<*mut JSContext> {
    let target = state as usize;
    contexts().lock().iter().find_map(|(ctx_key, record)| {
        if record.state_ptr == target {
            Some(*ctx_key as *mut JSContext)
        } else {
            None
        }
    })
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn ensure_context_for_ctx(ctx: *mut JSContext) -> *mut ContextState {
    let key = ctx as usize;
    let mut guard = contexts().lock();
    if let Some(record) = guard.get(&key) {
        unsafe { lexe_v8_set_context_quickjs_key(record.state_ptr as *mut ContextState, key) };
        return record.state_ptr as *mut ContextState;
    }
    let state = unsafe { lexe_v8_create_context() };
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let context_root = with_bridge_roots(|ctx, roots| {
        let obj = unsafe { qjs::JS_NewObject(ctx) };
        let root =
            roots.insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, obj) });
        unsafe { lexe_v8_set_context_root_id(state, root) };
        root
    });
    let _ = context_root;
    unsafe { lexe_v8_set_context_quickjs_key(state, key) };
    guard.insert(
        key,
        ContextRecord {
            runtime_key: rt as usize,
            state_ptr: state as usize,
        },
    );
    crate::runtime_state::context_key(state as usize);
    state
}

pub fn isolate_ptr_for_runtime(rt_key: usize) -> Option<usize> {
    isolates().lock().get(&rt_key).copied()
}

pub fn other_contexts_on_runtime(rt_key: usize, excluding_ctx_key: usize) -> bool {
    contexts()
        .lock()
        .iter()
        .any(|(ctx_key, record)| record.runtime_key == rt_key && *ctx_key != excluding_ctx_key)
}

pub fn context_state_ptr_for_ctx(ctx: *mut JSContext) -> Option<usize> {
    contexts()
        .lock()
        .get(&(ctx as usize))
        .map(|record| record.state_ptr)
}

pub fn contexts_for_runtime(rt: *mut qjs::JSRuntime) -> usize {
    let rt_key = rt as usize;
    contexts()
        .lock()
        .values()
        .filter(|record| record.runtime_key == rt_key)
        .count()
}

/// Tear down V8 bridge state, side tables, and ContextState for one QuickJS context.
///
/// # Safety
///
/// `ctx` must be a valid `JSContext` pointer for the current thread that was
/// wired through the V8 compat bridge, and must only be destroyed once.
pub unsafe fn shutdown_context(ctx: *mut JSContext) -> Result<(), String> {
    // Bridge teardown first (needs side tables for weak/internal-field work).
    // No-bridge contexts return Ok(()) immediately, so tables still get
    // cleaned below. Only remove tables after success — failing mid-bridge
    // teardown must not leave bridge/ContextState alive with empty tables.
    unsafe { crate::bridge::shutdown_bridge_for_context(ctx)? };

    // Idempotent if bridge already cleared tables; required when no bridge.
    crate::context_tables::remove_context_tables(ctx);

    let ctx_key = ctx as usize;
    if let Some(record) = contexts().lock().remove(&ctx_key) {
        unsafe {
            lexe_v8_destroy_context(record.state_ptr as *mut ContextState);
        }
        crate::runtime_state::forget_context(record.state_ptr);
    }
    if crate::bridge::has_bridge_for_ctx(ctx) {
        return Err(format!(
            "v8 shutdown_context: bridge state remains for ctx {ctx_key:#x}"
        ));
    }
    Ok(())
}

/// Tear down V8/N-API environment for one QuickJS context before `AsyncContext` drop.
///
/// # Safety
///
/// `ctx` must be a valid wired context on the current thread.
pub unsafe fn shutdown_environment(ctx: *mut JSContext) -> Result<(), String> {
    shutdown_context(ctx)
}

/// GC passes with weak-callback dispatch while the V8 bridge is still wired.
///
/// Run after module teardown (e.g. `JS_FreeAllModules`) so ObjectWrap weak
/// callbacks can delete native wrappers once JS references are gone.
///
/// # Safety
///
/// `ctx` must be a valid wired context on the current thread.
pub unsafe fn run_pre_bridge_teardown_gc(ctx: *mut JSContext) -> Result<(), String> {
    // Hard-activate when C++ state exists; bridge-only paths only set Rust TLS.
    unsafe { try_activate_v8_context_for_teardown(ctx)? };

    crate::js_ops::force_invoke_registered_weak_callbacks(ctx);

    let rt = unsafe { qjs::JS_GetRuntime(ctx) };

    for _ in 0..16 {
        unsafe {
            qjs::JS_RunGC(rt);
        }
        crate::bridge::with_state_for_ctx_if(ctx, |state| {
            state.drain_weak_holds(ctx);
        });
        crate::js_ops::dispatch_pending_weak_callbacks_for_ctx(ctx);
    }

    Ok(())
}

/// Tear down all V8 state for a QuickJS runtime once every context env is gone.
///
/// # Safety
///
/// `rt` must be a valid `JSRuntime` pointer for the current thread and must not
/// have begun `JS_FreeRuntime`.
pub unsafe fn shutdown_runtime(rt: *mut qjs::JSRuntime) -> Result<(), String> {
    let rt_key = rt as usize;

    let stale_contexts: Vec<usize> = contexts()
        .lock()
        .iter()
        .filter_map(|(ctx_key, record)| {
            if record.runtime_key == rt_key {
                Some(*ctx_key)
            } else {
                None
            }
        })
        .collect();

    for ctx_key in stale_contexts {
        unsafe { shutdown_context(ctx_key as *mut JSContext)? };
    }

    for _ in 0..5 {
        unsafe { qjs::JS_RunGC(rt) };
    }

    if let Some(isolate) = isolates().lock().remove(&rt_key) {
        crate::runtime_state::run_cleanup_hooks(isolate);
        lexe_v8_destroy_isolate(isolate as *mut IsolateState);
        crate::runtime_state::forget_isolate(isolate);
    }

    crate::js_ops::clear_runtime_v8_object_class(rt);

    let remaining_contexts = contexts_for_runtime(rt);
    if remaining_contexts != 0 {
        return Err(format!(
            "v8 shutdown_runtime: {remaining_contexts} context(s) remain for runtime {rt_key:#x}"
        ));
    }
    let remaining_roots = crate::bridge::residual_root_count_for_runtime(rt);
    if remaining_roots != 0 {
        return Err(format!(
            "v8 shutdown_runtime: {remaining_roots} bridge root(s) remain for runtime {rt_key:#x}"
        ));
    }
    Ok(())
}

/// # Safety
///
/// `rt_addr` must be the address of a live QuickJS runtime returned from
/// [`capture_v8_runtime_and_shutdown_context`] after its context has been shut down.
pub unsafe fn shutdown_runtime_addr(rt_addr: usize) -> Result<(), String> {
    shutdown_runtime(rt_addr as *mut qjs::JSRuntime)
}

pub fn drain_pending_v8_modules() -> Vec<*mut NodeModule> {
    drain_pending_v8_modules_since(0)
}

pub fn drain_pending_v8_modules_since(start: usize) -> Vec<*mut NodeModule> {
    extern "C" {
        fn lexe_v8_pending_modules_count() -> usize;
        fn lexe_v8_take_pending_module(index: usize) -> *mut NodeModule;
    }
    let mut out = Vec::new();
    while unsafe { lexe_v8_pending_modules_count() } > start {
        let module = unsafe { lexe_v8_take_pending_module(start) };
        if !module.is_null() {
            out.push(module);
        }
    }
    out
}

fn pending_modules_count() -> usize {
    extern "C" {
        fn lexe_v8_pending_modules_count() -> usize;
    }
    unsafe { lexe_v8_pending_modules_count() }
}

pub fn clear_pending_v8_modules() {
    let _ = drain_pending_v8_modules();
}

/// Initialize a V8-style native module and return the final `exports` value.
///
/// # Safety
/// `ctx`, `module`, and `exports` must be valid QuickJS/V8 handles for the current thread.
pub unsafe fn run_v8_module_init(
    ctx: *mut JSContext,
    module: *mut NodeModule,
    exports: JSValue,
) -> Result<JSValue, String> {
    if (*module).nm_version != NODE_MODULE_VERSION {
        return Err(format!(
            "unsupported NODE_MODULE_VERSION {} (expected {})",
            (*module).nm_version,
            NODE_MODULE_VERSION
        ));
    }
    let rt = qjs::JS_GetRuntime(ctx);
    let isolate = ensure_isolate_for_runtime(rt);
    let context_state = ensure_context_for_ctx(ctx);
    let _guard = push_native_load(ctx, isolate, context_state);

    let exports_root = with_bridge_roots(|ctx, roots| roots.insert_borrowed(ctx, exports));
    let module_obj = qjs::JS_NewObject(ctx);
    let exports_dup = qjs::JS_DupValue(ctx, exports);
    qjs::JS_SetPropertyStr(ctx, module_obj, c"exports".as_ptr(), exports_dup);
    let module_root = with_bridge_roots(|ctx, roots| {
        roots.insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, module_obj) })
    });

    let mut out_exports = exports_root;
    let status = lexe_v8_run_module_init(
        context_state,
        module,
        exports_root,
        module_root,
        &mut out_exports,
    );
    if status != 0 {
        with_bridge_roots(|ctx, roots| {
            roots.drop_root(ctx, module_root);
            roots.drop_root(ctx, exports_root);
            if out_exports != exports_root {
                roots.drop_root(ctx, out_exports);
            }
        });
        return Err(format!("V8 module init failed with status {}", status));
    }

    let result = with_bridge_roots(|ctx, roots| {
        let Some(module_val) = roots.get(module_root) else {
            roots.drop_root(ctx, module_root);
            roots.drop_root(ctx, exports_root);
            if out_exports != exports_root {
                roots.drop_root(ctx, out_exports);
            }
            return None;
        };
        let exports_val = qjs::JS_GetPropertyStr(ctx, module_val, c"exports".as_ptr());
        if qjs::JS_IsException(exports_val) {
            roots.drop_root(ctx, module_root);
            roots.drop_root(ctx, exports_root);
            if out_exports != exports_root {
                roots.drop_root(ctx, out_exports);
            }
            return None;
        }
        roots.drop_root(ctx, module_root);
        roots.drop_root(ctx, exports_root);
        if out_exports != exports_root {
            roots.drop_root(ctx, out_exports);
        }
        Some(exports_val)
    })
    .ok_or_else(|| "V8 module init lost exports root".to_string())?;

    Ok(result)
}

/// Copy `src` exports onto the `dst` object when an addon replaces `module.exports`.
///
/// # Safety
/// `ctx`, `dst`, and `src` must be valid JS values on `ctx`.
pub unsafe fn materialize_exports(ctx: *mut JSContext, dst: JSValue, src: JSValue) {
    if qjs::JS_IsStrictEqual(ctx, src, dst) {
        qjs::JS_FreeValue(ctx, src);
        return;
    }
    let flags = (qjs::JS_GPN_STRING_MASK | qjs::JS_GPN_ENUM_ONLY) as i32;
    let mut keys: *mut qjs::JSPropertyEnum = ptr::null_mut();
    let mut len: u32 = 0;
    if qjs::JS_GetOwnPropertyNames(ctx, &mut keys, &mut len, dst, flags) >= 0 {
        for i in 0..len {
            let atom = (*keys.add(i as usize)).atom;
            qjs::JS_DeleteProperty(ctx, dst, atom, 0);
        }
        qjs::JS_FreePropertyEnum(ctx, keys, len);
    }
    keys = ptr::null_mut();
    len = 0;
    if qjs::JS_GetOwnPropertyNames(ctx, &mut keys, &mut len, src, flags) < 0 {
        qjs::JS_FreeValue(ctx, src);
        return;
    }
    for i in 0..len {
        let atom = (*keys.add(i as usize)).atom;
        let val = qjs::JS_GetProperty(ctx, src, atom);
        qjs::JS_SetProperty(ctx, dst, atom, val);
    }
    qjs::JS_FreePropertyEnum(ctx, keys, len);
    qjs::JS_FreeValue(ctx, src);
}
