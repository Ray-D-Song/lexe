use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe, UnwindSafe};
use std::sync::OnceLock;

use rquickjs::qjs::{self, JSContext, JSValue};

use crate::abi::{NAPI_API_VERSION, NODE_MODULE_VERSION};
use crate::js_ops::new_v8_object;
use crate::module_loader::{ensure_context_for_ctx, ensure_isolate_for_runtime};
use crate::root::RootTable;

#[repr(C)]
#[derive(PartialEq)]
pub enum LexeV8Status {
    Ok = 0,
    Exception = 1,
    Unsupported = 2,
    WrongThread = 3,
    Error = 4,
}

#[repr(C)]
pub struct LexeV8ContextState {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LexeV8IsolateState {
    _private: [u8; 0],
}

type RootDupFn = unsafe extern "C" fn(u64, *mut u64) -> LexeV8Status;
type RootDropFn = unsafe extern "C" fn(u64) -> LexeV8Status;
type RootMakeWeakFn = unsafe extern "C" fn(u64) -> LexeV8Status;
type RootFromJsFn = unsafe extern "C" fn(*mut LexeV8ContextState, u64, *mut u64) -> LexeV8Status;
type RootToJsFn = unsafe extern "C" fn(*mut LexeV8ContextState, u64, *mut u64) -> LexeV8Status;
type ThrowTypeErrorFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, *const c_char) -> LexeV8Status;
type FatalFn = unsafe extern "C" fn(*const c_char, *const c_char);
type StringNewUtf8Fn =
    unsafe extern "C" fn(*mut LexeV8ContextState, *const c_char, i32, *mut u64) -> LexeV8Status;
type ObjectNewFn = unsafe extern "C" fn(*mut LexeV8ContextState, *mut u64) -> LexeV8Status;
type ObjectSetFn = unsafe extern "C" fn(*mut LexeV8ContextState, u64, u64, u64) -> LexeV8Status;
type FunctionTemplateNewFn = unsafe extern "C" fn(
    *mut LexeV8ContextState,
    u32,
    *mut std::ffi::c_void,
    u64,
    *mut u32,
) -> LexeV8Status;
type FunctionTemplateGetFunctionFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u32, *mut u64) -> LexeV8Status;
type DispatchFunctionFn = unsafe extern "C" fn(
    *mut LexeV8ContextState,
    u32,
    u64,
    u64,
    *const u64,
    i32,
    *mut u64,
) -> LexeV8Status;
type RunModuleInitFn = unsafe extern "C" fn(
    *mut LexeV8ContextState,
    unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void),
    u64,
    u64,
    *mut u64,
) -> LexeV8Status;
type ObjectGetFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, u64, *mut u64) -> LexeV8Status;
type ObjectGetIndexFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, u32, *mut u64) -> LexeV8Status;
type ObjectSetIndexFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, u32, u64) -> LexeV8Status;
type ObjectDefineOwnPropertyFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, u64, u64, i32, *mut bool) -> LexeV8Status;
type ObjectHasOwnPropertyFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, u64, *mut bool) -> LexeV8Status;
type ObjectGetPrototypeFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, *mut u64) -> LexeV8Status;
type ArrayNewFn = unsafe extern "C" fn(*mut LexeV8ContextState, i32, *mut u64) -> LexeV8Status;
type NumberNewFn = unsafe extern "C" fn(*mut LexeV8ContextState, f64, *mut u64) -> LexeV8Status;
type BigIntNewFn = unsafe extern "C" fn(*mut LexeV8ContextState, i64, *mut u64) -> LexeV8Status;
type IntegerNewFn = unsafe extern "C" fn(*mut LexeV8ContextState, i32, *mut u64) -> LexeV8Status;
type StringNewLatin1Fn =
    unsafe extern "C" fn(*mut LexeV8ContextState, *const u8, i32, *mut u64) -> LexeV8Status;
type StringToUtf8Fn = unsafe extern "C" fn(
    *mut LexeV8ContextState,
    u64,
    *mut *mut c_char,
    *mut usize,
) -> LexeV8Status;
type StringFreeUtf8Fn = unsafe extern "C" fn(*mut LexeV8ContextState, *mut c_char) -> LexeV8Status;
type FunctionCallFn = unsafe extern "C" fn(
    *mut LexeV8ContextState,
    u64,
    u64,
    i32,
    *const u64,
    *mut u64,
) -> LexeV8Status;
type ThrowValueFn = unsafe extern "C" fn(*mut LexeV8ContextState, u64) -> LexeV8Status;
type NewExceptionFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, i32, *mut u64) -> LexeV8Status;
type ExternalNewFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, *mut std::ffi::c_void, *mut u64) -> LexeV8Status;
type InternalFieldSetFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, i32, *mut std::ffi::c_void) -> LexeV8Status;
type InternalFieldGetFn = unsafe extern "C" fn(
    *mut LexeV8ContextState,
    u64,
    i32,
    *mut *mut std::ffi::c_void,
) -> LexeV8Status;
type SymbolIteratorFn = unsafe extern "C" fn(*mut LexeV8ContextState, *mut u64) -> LexeV8Status;
type GetCreationContextFn =
    unsafe extern "C" fn(*mut LexeV8ContextState, u64, *mut u64) -> LexeV8Status;
type RegisterWeakCallbackFn = unsafe extern "C" fn(
    *mut LexeV8ContextState,
    u64,
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
) -> LexeV8Status;
type GetContextRootFn = unsafe extern "C" fn(*mut LexeV8ContextState, *mut u64) -> LexeV8Status;

#[repr(C)]
pub struct LexeV8BridgeV1 {
    pub version: u32,
    pub node_module_version: u32,
    pub root_dup: Option<RootDupFn>,
    pub root_drop: Option<RootDropFn>,
    pub root_make_weak: Option<RootMakeWeakFn>,
    pub root_from_js: Option<RootFromJsFn>,
    pub root_to_js: Option<RootToJsFn>,
    pub throw_type_error: Option<ThrowTypeErrorFn>,
    pub fatal: Option<FatalFn>,
    pub string_new_utf8: Option<StringNewUtf8Fn>,
    pub object_new: Option<ObjectNewFn>,
    pub object_set: Option<ObjectSetFn>,
    pub function_template_new: Option<FunctionTemplateNewFn>,
    pub function_template_get_function: Option<FunctionTemplateGetFunctionFn>,
    pub dispatch_function: Option<DispatchFunctionFn>,
    pub run_module_init: Option<RunModuleInitFn>,
    pub object_get: Option<ObjectGetFn>,
    pub object_get_index: Option<ObjectGetIndexFn>,
    pub object_set_index: Option<ObjectSetIndexFn>,
    pub object_define_own_property: Option<ObjectDefineOwnPropertyFn>,
    pub object_has_own_property: Option<ObjectHasOwnPropertyFn>,
    pub object_get_prototype: Option<ObjectGetPrototypeFn>,
    pub array_new: Option<ArrayNewFn>,
    pub number_new: Option<NumberNewFn>,
    pub bigint_new: Option<BigIntNewFn>,
    pub integer_new: Option<IntegerNewFn>,
    pub string_new_latin1: Option<StringNewLatin1Fn>,
    pub string_to_utf8: Option<StringToUtf8Fn>,
    pub string_free_utf8: Option<StringFreeUtf8Fn>,
    pub function_call: Option<FunctionCallFn>,
    pub throw_value: Option<ThrowValueFn>,
    pub new_exception: Option<NewExceptionFn>,
    pub external_new: Option<ExternalNewFn>,
    pub internal_field_set: Option<InternalFieldSetFn>,
    pub internal_field_get: Option<InternalFieldGetFn>,
    pub symbol_iterator: Option<SymbolIteratorFn>,
    pub get_creation_context: Option<GetCreationContextFn>,
    pub register_weak_callback: Option<RegisterWeakCallbackFn>,
    pub get_context_root: Option<GetContextRootFn>,
}

struct SendPtr<T>(*mut T);
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

pub struct BridgeState {
    pub roots: RootTable,
    ctx: SendPtr<JSContext>,
    runtime_key: usize,
    weak_holds: HashMap<usize, JSValue>,
    /// Maps a root id moved into `weak_holds` by `root_make_weak` back to its object key.
    weak_root_ids: HashMap<u64, usize>,
    function_roots: HashMap<u32, u64>,
    root_function_ids: HashMap<u64, u32>,
}

impl BridgeState {
    pub(crate) fn ctx_ptr(&self) -> *mut JSContext {
        self.ctx.0
    }

    pub(crate) fn new(roots: RootTable, ctx: *mut JSContext) -> Self {
        let runtime_key = unsafe { qjs::JS_GetRuntime(ctx) as usize };
        Self {
            roots,
            ctx: SendPtr(ctx),
            runtime_key,
            weak_holds: HashMap::new(),
            weak_root_ids: HashMap::new(),
            function_roots: HashMap::new(),
            root_function_ids: HashMap::new(),
        }
    }

    /// Resolve a JSObject for a bridge root, including roots moved into weak holds.
    pub(crate) fn get_object_for_root(&self, root_id: u64) -> Option<JSValue> {
        if let Some(value) = self.roots.get(root_id) {
            return Some(value);
        }
        let key = self.weak_root_ids.get(&root_id).copied()?;
        self.weak_holds.get(&key).copied()
    }

    pub(crate) fn record_weak_root(&mut self, root_id: u64, object_key: usize) {
        self.weak_root_ids.insert(root_id, object_key);
    }

    pub(crate) fn forget_weak_roots_for_key(&mut self, object_key: usize) {
        self.weak_root_ids.retain(|_, key| *key != object_key);
    }

    pub(crate) fn insert_weak_hold(&mut self, key: usize, value: JSValue) {
        self.forget_weak_roots_for_key(key);
        self.weak_holds.insert(key, value);
    }

    pub(crate) fn take_weak_hold(&mut self, key: usize) -> Option<JSValue> {
        self.forget_weak_roots_for_key(key);
        self.weak_holds.remove(&key)
    }

    pub(crate) fn weak_hold_keys(&self) -> Vec<usize> {
        self.weak_holds.keys().copied().collect()
    }

    pub(crate) fn drain_weak_holds(&mut self, ctx: *mut JSContext) {
        crate::js_ops::process_weak_holds_for_ctx(self);
        crate::js_ops::dispatch_pending_weak_callbacks_for_ctx(ctx);
        self.weak_root_ids.clear();
        for (_, value) in self.weak_holds.drain() {
            unsafe { qjs::JS_FreeValue(ctx, value) };
        }
    }
}

thread_local! {
    static BRIDGE_STATES: RefCell<HashMap<usize, BridgeState>> = RefCell::new(HashMap::new());
    static ACTIVE_BRIDGE_CTX: RefCell<Option<usize>> = const { RefCell::new(None) };
}

fn ctx_key(ctx: *mut JSContext) -> usize {
    ctx as usize
}

pub fn set_active_bridge_context(ctx: *mut JSContext) {
    ACTIVE_BRIDGE_CTX.with(|cell| *cell.borrow_mut() = Some(ctx_key(ctx)));
}

pub fn clear_active_bridge_context() {
    ACTIVE_BRIDGE_CTX.with(|cell| *cell.borrow_mut() = None);
}

fn active_ctx_key() -> usize {
    ACTIVE_BRIDGE_CTX.with(|cell| {
        if let Some(key) = *cell.borrow() {
            return key;
        }
        BRIDGE_STATES.with(|states| {
            states
                .borrow()
                .keys()
                .next()
                .copied()
                .expect("active bridge context not set")
        })
    })
}

pub(crate) fn with_state_ref_for_ctx<F, R>(ctx: *mut JSContext, f: F) -> R
where
    F: FnOnce(&BridgeState) -> R,
{
    let key = ctx_key(ctx);
    BRIDGE_STATES.with(|cell| {
        let guard = cell.borrow();
        let state = guard
            .get(&key)
            .expect("v8 bridge not initialized for context");
        f(state)
    })
}

pub fn has_bridge_for_ctx(ctx: *mut JSContext) -> bool {
    let key = ctx_key(ctx);
    BRIDGE_STATES.with(|cell| cell.borrow().contains_key(&key))
}

pub fn residual_root_count_for_runtime(rt: *mut qjs::JSRuntime) -> usize {
    let rt_key = rt as usize;
    BRIDGE_STATES.with(|cell| {
        cell.borrow()
            .values()
            .filter(|state| state.runtime_key == rt_key)
            .map(|state| state.function_roots.len() + state.weak_holds.len() + state.roots.len())
            .sum()
    })
}

/// Per-context V8 teardown counters for shutdown progress checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeardownCounts {
    pub strong_roots: usize,
    pub weak_holds: usize,
    pub weak_callbacks: usize,
    pub pending_weak: usize,
    pub strong_persistents: usize,
    pub weak_persistents: usize,
}

impl TeardownCounts {
    pub fn is_zero(self) -> bool {
        self.strong_roots == 0
            && self.weak_holds == 0
            && self.weak_callbacks == 0
            && self.pending_weak == 0
            && self.strong_persistents == 0
            && self.weak_persistents == 0
    }
}

fn persistent_counts_for_ctx(ctx: *mut JSContext, key: usize) -> (usize, usize) {
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let rt_key = rt as usize;
    let isolate_ptr = crate::module_loader::isolate_ptr_for_runtime(rt_key);
    let Some(isolate_ptr) = isolate_ptr else {
        return (0, 0);
    };
    extern "C" {
        fn lexe_v8_persistent_counts_for_context(
            isolate: *mut std::ffi::c_void,
            context_key: usize,
            strong_out: *mut usize,
            weak_out: *mut usize,
        );
    }
    let mut strong = 0usize;
    let mut weak = 0usize;
    unsafe {
        lexe_v8_persistent_counts_for_context(
            isolate_ptr as *mut std::ffi::c_void,
            key,
            &mut strong,
            &mut weak,
        );
    }
    (strong, weak)
}

pub fn teardown_counts_for_ctx(ctx: *mut JSContext) -> TeardownCounts {
    let key = ctx_key(ctx);
    let bridge = BRIDGE_STATES.with(|cell| {
        cell.borrow().get(&key).map(|state| {
            (
                state.roots.len(),
                state.weak_holds.len(),
                state.function_roots.len(),
            )
        })
    });
    let (roots_len, weak_holds_len, function_roots_len) = bridge.unwrap_or((0, 0, 0));
    let (weak_callbacks, pending_weak) = crate::context_tables::weak_table_counts(ctx);
    let (strong_persistents, weak_persistents) = persistent_counts_for_ctx(ctx, key);
    TeardownCounts {
        strong_roots: roots_len + function_roots_len,
        weak_holds: weak_holds_len,
        weak_callbacks,
        pending_weak,
        strong_persistents,
        weak_persistents,
    }
}

fn dispose_strong_persistents_for_ctx(ctx: *mut JSContext, key: usize) -> usize {
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let rt_key = rt as usize;
    let Some(isolate_ptr) = crate::module_loader::isolate_ptr_for_runtime(rt_key) else {
        return 0;
    };
    extern "C" {
        fn lexe_v8_dispose_strong_context_persistents(
            isolate: *mut std::ffi::c_void,
            context_key: usize,
        ) -> usize;
    }
    unsafe { lexe_v8_dispose_strong_context_persistents(isolate_ptr as *mut std::ffi::c_void, key) }
}

fn dispose_weak_persistents_for_ctx(ctx: *mut JSContext, key: usize) -> usize {
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let rt_key = rt as usize;
    let Some(isolate_ptr) = crate::module_loader::isolate_ptr_for_runtime(rt_key) else {
        return 0;
    };
    extern "C" {
        fn lexe_v8_dispose_weak_context_persistents(
            isolate: *mut std::ffi::c_void,
            context_key: usize,
        ) -> usize;
    }
    unsafe { lexe_v8_dispose_weak_context_persistents(isolate_ptr as *mut std::ffi::c_void, key) }
}

const MAX_TEARDOWN_PASSES: usize = 32;

pub(crate) fn with_state_for_ctx_if<F, R>(ctx: *mut JSContext, f: F) -> Option<R>
where
    F: FnOnce(&mut BridgeState) -> R,
{
    let key = ctx_key(ctx);
    BRIDGE_STATES.with(|cell| {
        let mut guard = cell.borrow_mut();
        guard.get_mut(&key).map(f)
    })
}

/// Look up bridge state by rooted object id (used when thread-local active context is unset).
pub(crate) fn with_state_for_object_root<F, R>(object_root: u64, f: F) -> Option<R>
where
    F: FnOnce(*mut JSContext, JSValue) -> R,
{
    BRIDGE_STATES.with(|cell| {
        let guard = cell.borrow();
        for state in guard.values() {
            if let Some(obj) = state.roots.get(object_root) {
                return Some(f(state.ctx_ptr(), obj));
            }
        }
        None
    })
}

/// Look up bridge state by root table id (for root_drop/root_make_weak without active TLS).
pub(crate) fn with_state_for_root_id<F, R>(root_id: u64, f: F) -> Option<R>
where
    F: FnOnce(&mut BridgeState) -> R,
{
    BRIDGE_STATES.with(|cell| {
        let mut guard = cell.borrow_mut();
        for state in guard.values_mut() {
            if state.roots.get(root_id).is_some() {
                return Some(f(state));
            }
        }
        None
    })
}

pub(crate) fn with_state_for_ctx<F, R>(ctx: *mut JSContext, f: F) -> R
where
    F: FnOnce(&mut BridgeState) -> R,
{
    let key = ctx_key(ctx);
    BRIDGE_STATES.with(|cell| {
        let mut guard = cell.borrow_mut();
        let state = guard
            .get_mut(&key)
            .expect("v8 bridge not initialized for context");
        f(state)
    })
}

pub(crate) fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut BridgeState) -> R,
{
    let key = active_ctx_key();
    BRIDGE_STATES.with(|cell| {
        let mut guard = cell.borrow_mut();
        let state = guard
            .get_mut(&key)
            .expect("v8 bridge not initialized for context");
        f(state)
    })
}

pub(crate) fn with_state_ref<F, R>(f: F) -> R
where
    F: FnOnce(&BridgeState) -> R,
{
    let key = active_ctx_key();
    BRIDGE_STATES.with(|cell| {
        let guard = cell.borrow();
        let state = guard
            .get(&key)
            .expect("v8 bridge not initialized for context");
        f(state)
    })
}

fn catch_panic<F>(f: F) -> LexeV8Status
where
    F: FnOnce() -> LexeV8Status + UnwindSafe,
{
    match catch_unwind(f) {
        Ok(status) => status,
        Err(_) => LexeV8Status::Error,
    }
}

pub(crate) fn resolve_constructor_root(
    state: &BridgeState,
    func_root: u64,
) -> Option<(u64, rquickjs::qjs::JSValue)> {
    let ctx = state.ctx_ptr();
    if let Some(func) = state.roots.get(func_root) {
        if unsafe { rquickjs::qjs::JS_IsConstructor(ctx, func) } {
            return Some((func_root, func));
        }
    }
    let function_id = state.root_function_ids.get(&func_root).copied();
    if let Some(function_id) = function_id {
        let cached_root = state.function_roots.get(&function_id).copied();
        if let Some(root) = cached_root {
            if let Some(func) = state.roots.get(root) {
                if unsafe { rquickjs::qjs::JS_IsConstructor(ctx, func) } {
                    return Some((root, func));
                }
            }
        }
    }
    None
}

pub(crate) fn function_root_for_id(state: &BridgeState, function_id: u32) -> Option<u64> {
    state
        .function_roots
        .get(&function_id)
        .copied()
        .filter(|&root| state.roots.get(root).is_some())
}

pub(crate) fn function_id_for_root(state: &BridgeState, root: u64) -> Option<u32> {
    state.root_function_ids.get(&root).copied()
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_function_id_for_root(
    _ctx: *mut LexeV8ContextState,
    root_id: u64,
    out_function_id: *mut u32,
) -> LexeV8Status {
    if out_function_id.is_null() {
        return LexeV8Status::Error;
    }
    if let Some(function_id) = with_state_ref(|state| function_id_for_root(state, root_id)) {
        *out_function_id = function_id;
        LexeV8Status::Ok
    } else {
        LexeV8Status::Error
    }
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_function_root_for_id(
    _ctx: *mut LexeV8ContextState,
    function_id: u32,
    out_root_id: *mut u64,
) -> LexeV8Status {
    if out_root_id.is_null() {
        return LexeV8Status::Error;
    }
    if let Some(root) = with_state_ref(|state| function_root_for_id(state, function_id)) {
        *out_root_id = root;
        LexeV8Status::Ok
    } else {
        LexeV8Status::Error
    }
}

unsafe extern "C" fn root_dup(id: u64, out: *mut u64) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        if out.is_null() {
            return LexeV8Status::Error;
        }
        with_state(|state| match state.roots.dup(state.ctx_ptr(), id) {
            Some(new_id) => {
                if let Some(&function_id) = state.root_function_ids.get(&id) {
                    state.root_function_ids.insert(new_id, function_id);
                    state.function_roots.insert(function_id, new_id);
                }
                *out = new_id;
                LexeV8Status::Ok
            },
            None => LexeV8Status::Error,
        })
    }))
}

unsafe extern "C" fn root_drop(id: u64) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        if let Some(status) = with_state_for_root_id(id, |state| {
            if let Some(function_id) = state.root_function_ids.remove(&id) {
                state.function_roots.remove(&function_id);
            }
            state.roots.drop_root(state.ctx_ptr(), id);
            LexeV8Status::Ok
        }) {
            return status;
        }
        with_state(|state| {
            if let Some(function_id) = state.root_function_ids.remove(&id) {
                state.function_roots.remove(&function_id);
            }
            state.roots.drop_root(state.ctx_ptr(), id);
            LexeV8Status::Ok
        })
    }))
}

unsafe extern "C" fn root_make_weak(id: u64) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        if let Some(status) =
            with_state_for_root_id(id, |state| crate::js_ops::root_make_weak(state, id))
        {
            return status;
        }
        with_state(|state| crate::js_ops::root_make_weak(state, id))
    }))
}

unsafe extern "C" fn string_new_utf8(
    _ctx: *mut LexeV8ContextState,
    data: *const c_char,
    length: i32,
    out: *mut u64,
) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        if data.is_null() || out.is_null() {
            return LexeV8Status::Error;
        }
        with_state(|state| {
            let bytes = if length < 0 {
                std::ffi::CStr::from_ptr(data).to_bytes()
            } else {
                std::slice::from_raw_parts(data as *const u8, length as usize)
            };
            let js = qjs::JS_NewStringLen(
                state.ctx_ptr(),
                bytes.as_ptr() as *const c_char,
                bytes.len() as u64,
            );
            *out = state.roots.insert_owned(unsafe {
                crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), js)
            });
            LexeV8Status::Ok
        })
    }))
}

unsafe extern "C" fn object_new(_ctx: *mut LexeV8ContextState, out: *mut u64) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        if out.is_null() {
            return LexeV8Status::Error;
        }
        with_state(|state| {
            let obj = crate::js_ops::new_v8_object(state.ctx_ptr());
            *out = state.roots.insert_owned(unsafe {
                crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), obj)
            });
            LexeV8Status::Ok
        })
    }))
}

unsafe extern "C" fn object_set(
    _ctx: *mut LexeV8ContextState,
    object_root: u64,
    key_root: u64,
    value_root: u64,
) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        with_state(|state| {
            let Some(obj) = state.roots.get(object_root) else {
                return LexeV8Status::Error;
            };
            let Some(key) = state.roots.get(key_root) else {
                return LexeV8Status::Error;
            };
            let Some(val) = state.roots.get(value_root) else {
                return LexeV8Status::Error;
            };
            let atom = qjs::JS_ValueToAtom(state.ctx_ptr(), key);
            if atom == 0 {
                return LexeV8Status::Error;
            }
            let rc = qjs::JS_SetProperty(
                state.ctx_ptr(),
                obj,
                atom,
                qjs::JS_DupValue(state.ctx_ptr(), val),
            );
            qjs::JS_FreeAtom(state.ctx_ptr(), atom);
            if rc < 0 {
                return LexeV8Status::Exception;
            }
            LexeV8Status::Ok
        })
    }))
}

unsafe extern "C" fn throw_type_error(
    _ctx: *mut LexeV8ContextState,
    message: *const c_char,
) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        with_state(|state| {
            let msg = if message.is_null() {
                "V8 ABI error"
            } else {
                std::ffi::CStr::from_ptr(message)
                    .to_str()
                    .unwrap_or("V8 ABI error")
            };
            let err = qjs::JS_NewError(state.ctx_ptr());
            qjs::JS_SetPropertyStr(
                state.ctx_ptr(),
                err,
                c"message".as_ptr(),
                qjs::JS_NewStringLen(
                    state.ctx_ptr(),
                    msg.as_ptr() as *const c_char,
                    msg.len() as u64,
                ),
            );
            qjs::JS_Throw(state.ctx_ptr(), err);
            LexeV8Status::Exception
        })
    }))
}

unsafe extern "C" fn fatal(api: *const c_char, message: *const c_char) {
    let api = if api.is_null() {
        "v8"
    } else {
        std::ffi::CStr::from_ptr(api).to_str().unwrap_or("v8")
    };
    let message = if message.is_null() {
        "fatal ABI violation"
    } else {
        std::ffi::CStr::from_ptr(message)
            .to_str()
            .unwrap_or("fatal ABI violation")
    };
    panic!("V8 ABI fatal in {api}: {message}");
}

unsafe extern "C" fn unsupported_fn() -> LexeV8Status {
    LexeV8Status::Unsupported
}

unsafe extern "C" fn unsupported_template_new(
    _ctx: *mut LexeV8ContextState,
    _template_id: u32,
    _callback: *mut std::ffi::c_void,
    _data_root: u64,
    _out: *mut u32,
) -> LexeV8Status {
    LexeV8Status::Unsupported
}

/// Returns:
/// - `JSValue`: owned temporary reference; caller must free exactly once.
/// - `u64`: RootTable-owned duplicate.
unsafe fn make_v8_constructor_from_id(
    ctx: *mut JSContext,
    state: &mut BridgeState,
    function_id: u32,
) -> (JSValue, u64) {
    let func = qjs::JS_NewCFunction2(
        ctx,
        mem::transmute::<qjs::JSCFunctionMagic, qjs::JSCFunction>(Some(v8_js_cfunc_magic)),
        c"v8".as_ptr(),
        0,
        qjs::JSCFunctionEnum_JS_CFUNC_constructor_or_func_magic,
        function_id as i32,
    );
    qjs::JS_SetConstructorBit(ctx, func, true);
    let root = state.roots.insert_borrowed(ctx, func);
    state.function_roots.insert(function_id, root);
    state.root_function_ids.insert(root, function_id);
    (func, root)
}

unsafe extern "C" fn v8_js_method_magic(
    ctx: *mut JSContext,
    this_val: JSValue,
    argc: c_int,
    argv: *mut JSValue,
    magic: c_int,
) -> JSValue {
    v8_js_trampoline_inner(ctx, this_val, argc, argv, magic, false)
}

unsafe fn make_v8_method_from_id(
    ctx: *mut JSContext,
    state: &mut BridgeState,
    function_id: u32,
) -> (JSValue, u64) {
    let func = qjs::JS_NewCFunction2(
        ctx,
        mem::transmute::<qjs::JSCFunctionMagic, qjs::JSCFunction>(Some(v8_js_method_magic)),
        c"v8".as_ptr(),
        0,
        qjs::JSCFunctionEnum_JS_CFUNC_generic_magic,
        function_id as i32,
    );
    let root = state.roots.insert_borrowed(ctx, func);
    state.function_roots.insert(function_id, root);
    state.root_function_ids.insert(root, function_id);
    (func, root)
}

unsafe fn make_accessor_getter(ctx: *mut JSContext, accessor_id: u32) -> JSValue {
    qjs::JS_NewCFunction2(
        ctx,
        mem::transmute::<qjs::JSCFunctionMagic, qjs::JSCFunction>(Some(v8_accessor_getter)),
        c"get".as_ptr(),
        0,
        qjs::JSCFunctionEnum_JS_CFUNC_generic_magic,
        accessor_id as i32,
    )
}

unsafe extern "C" fn v8_accessor_getter(
    ctx: *mut JSContext,
    this_val: JSValue,
    _argc: c_int,
    _argv: *mut JSValue,
    magic: c_int,
) -> JSValue {
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let isolate = ensure_isolate_for_runtime(rt);
    let context_state = ensure_context_for_ctx(ctx);
    unsafe {
        lexe_v8_set_current_context(context_state as *mut LexeV8ContextState);
        lexe_v8_set_current_isolate(isolate as *mut LexeV8IsolateState);
    }
    set_active_bridge_context(ctx);
    let accessor_id = magic as u32;
    let receiver_root =
        with_state_ref_for_ctx(ctx, |state| state.roots.insert_borrowed(ctx, this_val));
    let embedder = crate::js_ops::embedder_ptr_for_object(ctx, this_val, receiver_root, 0);
    let _embedder_scope = crate::js_ops::EmbedderScopeGuard::enter();
    if embedder != 0 {
        crate::js_ops::set_embedder_field0_in_frame(embedder);
    }
    let mut result_root = 0u64;
    let status = unsafe {
        lexe_v8_dispatch_accessor(
            accessor_id,
            receiver_root,
            embedder as *mut c_void,
            &mut result_root,
        )
    };
    with_state_ref_for_ctx(ctx, |state| {
        state.roots.drop_root(ctx, receiver_root);
    });
    if status != LexeV8Status::Ok || result_root == 0 {
        return qjs::JS_UNDEFINED;
    }
    let result = with_state_ref_for_ctx(ctx, |state| {
        state
            .roots
            .get(result_root)
            .map(|v| unsafe { qjs::JS_DupValue(ctx, v) })
            .unwrap_or(qjs::JS_UNDEFINED)
    });
    with_state_ref_for_ctx(ctx, |state| {
        state.roots.drop_root(ctx, result_root);
    });
    result
}

unsafe fn install_function_prototype(
    ctx: *mut JSContext,
    state: &mut BridgeState,
    template_id: u32,
    func: JSValue,
) {
    let proto_template_id = lexe_v8_function_prototype_template_id(template_id);
    if proto_template_id == 0 {
        let proto = unsafe { qjs::JS_NewObject(ctx) };
        unsafe {
            let _ = qjs::JS_SetConstructor(ctx, func, proto);
            qjs::JS_FreeValue(ctx, proto);
        }
        return;
    }
    let proto = unsafe { qjs::JS_NewObject(ctx) };
    let count = lexe_v8_object_template_property_count(proto_template_id);
    for i in 0..count {
        let mut key_root = 0u64;
        let mut child_template_id = 0u32;
        if lexe_v8_object_template_property_at(
            proto_template_id,
            i,
            &mut key_root,
            &mut child_template_id,
        ) == 0
        {
            continue;
        }
        let Some(key) = state.roots.get(key_root) else {
            continue;
        };
        let atom = unsafe { qjs::JS_ValueToAtom(ctx, key) };
        if atom == 0 {
            continue;
        }
        let child_function_id =
            unsafe { lexe_v8_register_function_for_template(child_template_id) };
        let (child_fn, _) = unsafe { make_v8_method_from_id(ctx, state, child_function_id) };
        unsafe {
            let _ = qjs::JS_SetProperty(ctx, proto, atom, child_fn);
            qjs::JS_FreeAtom(ctx, atom);
        }
    }
    let native_count = lexe_v8_object_template_native_property_count(proto_template_id);
    for i in 0..native_count {
        let mut name_root = 0u64;
        let mut accessor_id = 0u32;
        if lexe_v8_object_template_native_property_at(
            proto_template_id,
            i,
            &mut name_root,
            &mut accessor_id,
        ) == 0
        {
            continue;
        }
        let Some(key) = state.roots.get(name_root) else {
            continue;
        };
        let atom = unsafe { qjs::JS_ValueToAtom(ctx, key) };
        if atom == 0 {
            continue;
        }
        let getter = unsafe { make_accessor_getter(ctx, accessor_id) };
        unsafe {
            let _ = qjs::JS_DefinePropertyGetSet(
                ctx,
                proto,
                atom,
                getter,
                qjs::JS_UNDEFINED,
                (qjs::JS_PROP_ENUMERABLE | qjs::JS_PROP_CONFIGURABLE) as i32,
            );
            qjs::JS_FreeAtom(ctx, atom);
        }
    }
    let proto_root = state.roots.insert_borrowed(ctx, proto);
    unsafe {
        lexe_v8_set_function_template_prototype_root(template_id, proto_root);
        let _ = qjs::JS_SetConstructor(ctx, func, proto);
        qjs::JS_FreeValue(ctx, proto);
    }
}

unsafe extern "C" fn function_template_get_function(
    _ctx: *mut LexeV8ContextState,
    function_id: u32,
    out: *mut u64,
) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        if out.is_null() {
            return LexeV8Status::Error;
        }
        with_state(|state| {
            let ctx = state.ctx_ptr();
            let cached_root = state
                .function_roots
                .get(&function_id)
                .copied()
                .filter(|&root| state.roots.get(root).is_some());
            let root = if let Some(root) = cached_root {
                state.root_function_ids.insert(root, function_id);
                root
            } else {
                let (func, root) = make_v8_constructor_from_id(ctx, state, function_id);
                let template_id = unsafe { lexe_v8_function_template_id(function_id) };
                install_function_prototype(ctx, state, template_id, func);
                unsafe {
                    qjs::JS_FreeValue(ctx, func);
                }
                root
            };
            *out = root;
            LexeV8Status::Ok
        })
    }))
}

unsafe extern "C" fn dispatch_function(
    _ctx: *mut LexeV8ContextState,
    function_id: u32,
    receiver_root: u64,
    _new_target_root: u64,
    arg_root_ids: *const u64,
    argc: i32,
    out_result_root: *mut u64,
) -> LexeV8Status {
    catch_panic(AssertUnwindSafe(|| {
        if out_result_root.is_null() {
            return LexeV8Status::Error;
        }
        let args = if arg_root_ids.is_null() || argc <= 0 {
            &[]
        } else {
            std::slice::from_raw_parts(arg_root_ids, argc as usize)
        };
        let mut result_root = 0u64;
        let status = lexe_v8_dispatch_callback(
            function_id,
            receiver_root,
            _new_target_root,
            args.as_ptr(),
            argc,
            &mut result_root,
        );
        if status == LexeV8Status::Ok {
            *out_result_root = result_root;
        }
        status
    }))
}

unsafe extern "C" fn v8_js_cfunc_magic(
    ctx: *mut JSContext,
    this_val: JSValue,
    argc: c_int,
    argv: *mut JSValue,
    magic: c_int,
) -> JSValue {
    v8_js_trampoline_inner(ctx, this_val, argc, argv, magic, true)
}

unsafe extern "C" fn v8_js_trampoline(
    ctx: *mut JSContext,
    this_val: JSValue,
    argc: c_int,
    argv: *mut JSValue,
    magic: c_int,
    _func_data: *mut JSValue,
) -> JSValue {
    v8_js_trampoline_inner(ctx, this_val, argc, argv, magic, false)
}

unsafe fn v8_js_trampoline_inner(
    ctx: *mut JSContext,
    this_val: JSValue,
    argc: c_int,
    argv: *mut JSValue,
    magic: c_int,
    use_constructor_semantics: bool,
) -> JSValue {
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let isolate = ensure_isolate_for_runtime(rt);
    let context_state = ensure_context_for_ctx(ctx);
    unsafe {
        lexe_v8_set_current_context(context_state as *mut LexeV8ContextState);
        lexe_v8_set_current_isolate(isolate as *mut LexeV8IsolateState);
    }
    set_active_bridge_context(ctx);

    let function_id = magic as u32;
    // QuickJS passes `undefined` for ordinary calls on JS_CFUNC_constructor_or_func_magic.
    // For C function constructors it passes new_target (= the ctor function) as `this`.
    let is_construct = use_constructor_semantics && unsafe { !qjs::JS_IsUndefined(this_val) };

    let (receiver_root, new_target_root, instance_val) = if is_construct {
        let template_id = unsafe { lexe_v8_function_template_id(function_id) };
        let field_count = unsafe { lexe_v8_instance_internal_field_count(template_id) };
        let ctor_is_new_target = unsafe { qjs::JS_IsFunction(ctx, this_val) };
        let proto = if ctor_is_new_target {
            let installed = with_state_ref_for_ctx(ctx, |state| {
                let proto_root = unsafe { lexe_v8_function_template_prototype_root(template_id) };
                proto_root
                    .ne(&0)
                    .then_some(proto_root)
                    .and_then(|proto_root| state.roots.get(proto_root))
                    .map(|v| unsafe { qjs::JS_DupValue(ctx, v) })
            });
            installed.unwrap_or_else(|| unsafe {
                qjs::JS_GetPropertyStr(ctx, this_val, c"prototype".as_ptr())
            })
        } else {
            with_state_ref_for_ctx(ctx, |state| {
                let proto_root = unsafe { lexe_v8_function_template_prototype_root(template_id) };
                if proto_root != 0 {
                    state
                        .roots
                        .get(proto_root)
                        .map(|v| unsafe { qjs::JS_DupValue(ctx, v) })
                } else {
                    None
                }
            })
            .unwrap_or(qjs::JS_UNDEFINED)
        };
        let instance = unsafe {
            if ctor_is_new_target {
                if qjs::JS_IsUndefined(proto) {
                    new_v8_object(ctx)
                } else {
                    let obj = qjs::JS_NewObject(ctx);
                    qjs::JS_SetPrototype(ctx, obj, proto);
                    qjs::JS_FreeValue(ctx, proto);
                    obj
                }
            } else {
                qjs::JS_DupValue(ctx, this_val)
            }
        };
        let receiver_root =
            with_state_ref_for_ctx(ctx, |state| state.roots.insert_borrowed(ctx, instance));
        if field_count > 0 {
            unsafe {
                lexe_v8_object_reserve_internal_fields(
                    context_state as *mut LexeV8ContextState,
                    receiver_root,
                    field_count,
                );
            }
        }
        let new_target_root = with_state_ref_for_ctx(ctx, |state| {
            if ctor_is_new_target {
                state.roots.insert_borrowed(ctx, this_val)
            } else {
                function_root_for_id(state, function_id)
                    .and_then(|root| state.roots.get(root))
                    .map(|func| state.roots.insert_borrowed(ctx, func))
                    .unwrap_or(0)
            }
        });
        (receiver_root, new_target_root, Some(instance))
    } else {
        let receiver_root =
            with_state_ref_for_ctx(ctx, |state| state.roots.insert_borrowed(ctx, this_val));
        (receiver_root, 0, None)
    };

    let mut arg_roots = Vec::with_capacity(argc.max(0) as usize);
    if !argv.is_null() && argc > 0 {
        let slice = std::slice::from_raw_parts(argv, argc as usize);
        with_state_ref_for_ctx(ctx, |state| {
            for &arg in slice {
                arg_roots.push(state.roots.insert_borrowed(ctx, arg));
            }
        });
    }
    let embedder_patch = None::<usize>;
    let _embedder_scope = crate::js_ops::EmbedderScopeGuard::enter();
    let mut result_root = 0u64;
    let status = unsafe {
        lexe_v8_dispatch_callback(
            function_id,
            receiver_root,
            new_target_root,
            arg_roots.as_ptr(),
            argc,
            &mut result_root,
        )
    };
    let _ = embedder_patch;
    if status == LexeV8Status::Exception || unsafe { qjs::JS_HasException(ctx) } {
        with_state_ref_for_ctx(ctx, |state| {
            state.roots.drop_root(ctx, receiver_root);
            if new_target_root != 0 {
                state.roots.drop_root(ctx, new_target_root);
            }
            for id in arg_roots {
                state.roots.drop_root(ctx, id);
            }
        });
        if let Some(instance) = instance_val {
            unsafe { qjs::JS_FreeValue(ctx, instance) };
        }
        return unsafe { qjs::JS_GetException(ctx) };
    }
    if status != LexeV8Status::Ok {
        return qjs::JS_UNDEFINED;
    }
    if let Some(instance) = instance_val {
        with_state_ref_for_ctx(ctx, |state| {
            state.roots.drop_root(ctx, receiver_root);
            if new_target_root != 0 {
                state.roots.drop_root(ctx, new_target_root);
            }
            for id in arg_roots {
                state.roots.drop_root(ctx, id);
            }
        });
        if result_root != 0 && result_root != receiver_root {
            with_state_ref_for_ctx(ctx, |state| {
                state.roots.drop_root(ctx, result_root);
            });
        }
        return instance;
    }
    if result_root != 0 && result_root != receiver_root {
        with_state_ref_for_ctx(ctx, |state| {
            state.roots.drop_root(ctx, receiver_root);
            if new_target_root != 0 {
                state.roots.drop_root(ctx, new_target_root);
            }
            for id in arg_roots {
                state.roots.drop_root(ctx, id);
            }
        });
        let result = with_state_ref_for_ctx(ctx, |state| {
            state
                .roots
                .get(result_root)
                .map(|v| unsafe { qjs::JS_DupValue(ctx, v) })
                .unwrap_or(qjs::JS_UNDEFINED)
        });
        with_state_ref_for_ctx(ctx, |state| {
            state.roots.drop_root(ctx, result_root);
        });
        return result;
    }
    if result_root != 0 && result_root == receiver_root {
        let result = with_state_ref_for_ctx(ctx, |state| {
            state
                .roots
                .get(receiver_root)
                .map(|v| unsafe { qjs::JS_DupValue(ctx, v) })
                .unwrap_or(qjs::JS_UNDEFINED)
        });
        with_state_ref_for_ctx(ctx, |state| {
            state.roots.drop_root(ctx, receiver_root);
            if new_target_root != 0 {
                state.roots.drop_root(ctx, new_target_root);
            }
            for id in arg_roots {
                state.roots.drop_root(ctx, id);
            }
        });
        return result;
    }
    with_state_ref_for_ctx(ctx, |state| {
        state.roots.drop_root(ctx, receiver_root);
        if new_target_root != 0 {
            state.roots.drop_root(ctx, new_target_root);
        }
        for id in arg_roots {
            state.roots.drop_root(ctx, id);
        }
    });
    if result_root != 0 {
        let result = with_state_ref_for_ctx(ctx, |state| {
            state
                .roots
                .get(result_root)
                .map(|v| unsafe { qjs::JS_DupValue(ctx, v) })
                .unwrap_or(qjs::JS_UNDEFINED)
        });
        with_state_ref_for_ctx(ctx, |state| {
            state.roots.drop_root(ctx, result_root);
        });
        return result;
    }
    qjs::JS_UNDEFINED
}

extern "C" {
    fn lexe_v8_bind_bridge(bridge: *const LexeV8BridgeV1);
    fn lexe_v8_set_oddball_root_fn(
        callback: unsafe extern "C" fn(*mut LexeV8ContextState, c_int, *mut u64) -> LexeV8Status,
    );
    fn lexe_v8_force_link();
    fn lexe_v8_set_current_context(ctx: *mut LexeV8ContextState);
    fn lexe_v8_set_current_isolate(isolate: *mut LexeV8IsolateState);
    fn lexe_v8_dispatch_callback(
        function_id: u32,
        receiver_root: u64,
        new_target_root: u64,
        arg_roots: *const u64,
        argc: i32,
        out_result_root: *mut u64,
    ) -> LexeV8Status;
    fn lexe_v8_function_template_id(function_id: u32) -> u32;
    fn lexe_v8_instance_internal_field_count(template_id: u32) -> i32;
    fn lexe_v8_function_prototype_template_id(template_id: u32) -> u32;
    fn lexe_v8_set_function_template_prototype_root(template_id: u32, root_id: u64);
    fn lexe_v8_function_template_prototype_root(template_id: u32) -> u64;
    fn lexe_v8_function_template_ids_for_context(
        context_key: usize,
        out: *mut u32,
        capacity: usize,
    ) -> usize;
    fn lexe_v8_object_template_property_count(object_template_id: u32) -> usize;
    fn lexe_v8_object_template_property_at(
        object_template_id: u32,
        index: usize,
        key_root: *mut u64,
        value_template_id: *mut u32,
    ) -> i32;
    fn lexe_v8_register_function_for_template(template_id: u32) -> u32;
    fn lexe_v8_object_template_native_property_count(object_template_id: u32) -> usize;
    fn lexe_v8_object_template_native_property_at(
        object_template_id: u32,
        index: usize,
        name_root: *mut u64,
        accessor_id: *mut u32,
    ) -> i32;
    fn lexe_v8_dispatch_accessor(
        accessor_id: u32,
        receiver_root: u64,
        embedder_override: *mut c_void,
        out_result_root: *mut u64,
    ) -> LexeV8Status;
    fn lexe_v8_object_reserve_internal_fields(
        ctx: *mut LexeV8ContextState,
        object_root: u64,
        count: i32,
    ) -> LexeV8Status;
}

static BRIDGE_VTABLE: OnceLock<LexeV8BridgeV1> = OnceLock::new();

#[used]
static FORCE_V8_SHIM_LINK: unsafe extern "C" fn() = lexe_v8_force_link;

/// Prevent the linker from dead-stripping the C++ V8 shim.
pub fn ensure_shim_linked() {
    unsafe {
        lexe_v8_force_link();
    }
}

pub fn bind_bridge(ctx: *mut JSContext) {
    ensure_shim_linked();
    let key = ctx_key(ctx);
    BRIDGE_STATES.with(|cell| {
        cell.borrow_mut()
            .entry(key)
            .or_insert_with(|| BridgeState::new(RootTable::new(), ctx));
    });
    set_active_bridge_context(ctx);

    let bridge = BRIDGE_VTABLE.get_or_init(|| LexeV8BridgeV1 {
        version: 1,
        node_module_version: NODE_MODULE_VERSION as u32,
        root_dup: Some(root_dup),
        root_drop: Some(root_drop),
        root_make_weak: Some(root_make_weak),
        root_from_js: Some(crate::js_ops::root_from_js_value),
        root_to_js: None,
        throw_type_error: Some(throw_type_error),
        fatal: Some(fatal),
        string_new_utf8: Some(string_new_utf8),
        object_new: Some(object_new),
        object_set: Some(object_set),
        function_template_new: Some(unsupported_template_new),
        function_template_get_function: Some(function_template_get_function),
        dispatch_function: Some(dispatch_function),
        run_module_init: None,
        object_get: Some(crate::js_ops::object_get),
        object_get_index: Some(crate::js_ops::object_get_index),
        object_set_index: Some(crate::js_ops::object_set_index),
        object_define_own_property: Some(crate::js_ops::object_define_own_property),
        object_has_own_property: Some(crate::js_ops::object_has_own_property),
        object_get_prototype: Some(crate::js_ops::object_get_prototype),
        array_new: Some(crate::js_ops::array_new),
        number_new: Some(crate::js_ops::number_new),
        bigint_new: Some(crate::js_ops::bigint_new),
        integer_new: Some(crate::js_ops::integer_new),
        string_new_latin1: Some(crate::js_ops::string_new_latin1),
        string_to_utf8: Some(crate::js_ops::string_to_utf8),
        string_free_utf8: Some(crate::js_ops::string_free_utf8),
        function_call: Some(crate::js_ops::function_call),
        throw_value: Some(crate::js_ops::throw_value),
        new_exception: Some(crate::js_ops::new_exception),
        external_new: Some(crate::js_ops::external_new),
        internal_field_set: Some(crate::js_ops::internal_field_set),
        internal_field_get: Some(crate::js_ops::internal_field_get),
        symbol_iterator: Some(crate::js_ops::symbol_iterator),
        get_creation_context: Some(crate::js_ops::get_creation_context),
        register_weak_callback: Some(crate::js_ops::register_weak_callback),
        get_context_root: Some(crate::js_ops::get_context_root),
    });

    unsafe {
        lexe_v8_bind_bridge(bridge);
        lexe_v8_set_oddball_root_fn(crate::js_ops::oddball_root);
    }
}

/// Break constructor/prototype links for installed V8 templates so QuickJS GC can
/// Delete one own property from `obj` by name. Succeeds only when QuickJS reports
/// the property was removed (`1`). Returns an error on `0` (not deleted) or
/// exceptions (`< 0`).
pub(crate) unsafe fn delete_own_property_str(
    ctx: *mut JSContext,
    obj: qjs::JSValue,
    name: &std::ffi::CStr,
) -> Result<(), String> {
    if unsafe { qjs::JS_VALUE_GET_TAG(obj) } != qjs::JS_TAG_OBJECT {
        return Ok(());
    }
    let atom = unsafe { qjs::JS_NewAtom(ctx, name.as_ptr()) };
    let ret = unsafe { qjs::JS_DeleteProperty(ctx, obj, atom, 0) };
    unsafe {
        qjs::JS_FreeAtom(ctx, atom);
    }
    match ret {
        1 => Ok(()),
        0 => Err(format!(
            "own property {:?} was not deleted (missing or non-configurable)",
            name.to_string_lossy()
        )),
        _ => {
            clear_pending_exception(ctx);
            Err(format!(
                "exception while deleting own property {:?}",
                name.to_string_lossy()
            ))
        },
    }
}

unsafe fn clear_pending_exception(ctx: *mut JSContext) {
    if unsafe { qjs::JS_HasException(ctx) } {
        let exc = unsafe { qjs::JS_GetException(ctx) };
        unsafe {
            qjs::JS_FreeValue(ctx, exc);
        }
    }
}

/// Returns true when `Object.prototype.constructor` is still the global `Object` function.
pub(crate) unsafe fn object_intrinsic_constructor_intact(ctx: *mut JSContext) -> bool {
    let global = unsafe { qjs::JS_GetGlobalObject(ctx) };
    let object_fn = unsafe { qjs::JS_GetPropertyStr(ctx, global, c"Object".as_ptr()) };
    let object_proto = unsafe { qjs::JS_GetPropertyStr(ctx, object_fn, c"prototype".as_ptr()) };
    let proto_ctor = unsafe { qjs::JS_GetPropertyStr(ctx, object_proto, c"constructor".as_ptr()) };
    let intact = unsafe { qjs::JS_IsStrictEqual(ctx, proto_ctor, object_fn) };
    unsafe {
        qjs::JS_FreeValue(ctx, proto_ctor);
        qjs::JS_FreeValue(ctx, object_proto);
        qjs::JS_FreeValue(ctx, object_fn);
        qjs::JS_FreeValue(ctx, global);
    }
    intact
}

/// Returns whether `proto` has an own `constructor` property.
pub(crate) unsafe fn prototype_has_own_constructor(
    ctx: *mut JSContext,
    proto: qjs::JSValue,
) -> Result<bool, String> {
    if unsafe { qjs::JS_VALUE_GET_TAG(proto) } != qjs::JS_TAG_OBJECT {
        return Ok(false);
    }
    let atom = unsafe { qjs::JS_NewAtom(ctx, c"constructor".as_ptr()) };
    if atom == 0 {
        return Err("failed to allocate atom for prototype constructor".into());
    }
    let mut desc = qjs::JSPropertyDescriptor {
        flags: 0,
        value: qjs::JS_UNDEFINED,
        getter: qjs::JS_UNDEFINED,
        setter: qjs::JS_UNDEFINED,
    };
    let ret = unsafe { qjs::JS_GetOwnProperty(ctx, &mut desc, proto, atom) };
    unsafe {
        qjs::JS_FreeAtom(ctx, atom);
    }
    if ret < 0 {
        return Err("JS_GetOwnProperty failed for prototype constructor".into());
    }
    if ret > 0 {
        unsafe {
            qjs::JS_FreeValue(ctx, desc.value);
            if !qjs::JS_IsUndefined(desc.getter) {
                qjs::JS_FreeValue(ctx, desc.getter);
            }
            if !qjs::JS_IsUndefined(desc.setter) {
                qjs::JS_FreeValue(ctx, desc.setter);
            }
        }
    }
    Ok(ret > 0)
}

unsafe fn sever_installed_v8_templates(ctx: *mut JSContext, key: usize) -> Result<(), String> {
    let count = unsafe { lexe_v8_function_template_ids_for_context(key, std::ptr::null_mut(), 0) };
    if count == 0 {
        return Ok(());
    }
    let mut ids = vec![0u32; count];
    let written =
        unsafe { lexe_v8_function_template_ids_for_context(key, ids.as_mut_ptr(), count) };
    debug_assert_eq!(written, count);

    with_state_for_ctx(ctx, |state| -> Result<(), String> {
        let ctx = state.ctx_ptr();
        for &template_id in &ids {
            let proto_root = unsafe { lexe_v8_function_template_prototype_root(template_id) };
            if proto_root == 0 {
                continue;
            }
            let Some(proto) = state.roots.get(proto_root) else {
                continue;
            };
            unsafe {
                if prototype_has_own_constructor(ctx, proto)? {
                    delete_own_property_str(ctx, proto, c"constructor")?;
                }
            }
        }
        Ok(())
    })?;

    if unsafe { qjs::JS_HasException(ctx) } {
        return Err("pending exception after V8 template sever".into());
    }
    Ok(())
}

/// Release rooted JS values and bridge state for a single QuickJS context.
///
/// # Safety
///
/// `ctx` must be a valid `JSContext` pointer associated with the V8 compat
/// bridge on the current thread, and must only be torn down once.
pub(crate) unsafe fn shutdown_bridge_for_context(ctx: *mut JSContext) -> Result<(), String> {
    let key = ctx_key(ctx);
    let has_bridge = BRIDGE_STATES.with(|cell| cell.borrow().contains_key(&key));
    if !has_bridge {
        return Ok(());
    }
    // Same TLS provenance as run_pre_bridge_teardown_gc — ObjectWrap weak
    // callbacks and Persistent::Reset need current context/isolate. Hard-fails
    // if ContextState exists without a registered isolate.
    unsafe { crate::module_loader::try_activate_v8_context_for_teardown(ctx)? };
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let rt_key = rt as usize;
    if let Some(context_state) = crate::module_loader::context_state_ptr_for_ctx(ctx) {
        crate::runtime_state::run_cleanup_hooks(context_state);
    }

    unsafe {
        sever_installed_v8_templates(ctx, key)?;
    }

    unsafe {
        crate::js_ops::force_invoke_registered_weak_callbacks(ctx);
    }

    let mut last_counts = teardown_counts_for_ctx(ctx);
    for pass in 0..MAX_TEARDOWN_PASSES {
        with_state_for_ctx(ctx, |state| {
            for id in state.function_roots.values().copied() {
                state.roots.drop_root(state.ctx_ptr(), id);
            }
            state.function_roots.clear();
            state.root_function_ids.clear();
            state.roots.clear(ctx);
            state.drain_weak_holds(ctx);
        });

        let weak_remaining = dispose_strong_persistents_for_ctx(ctx, key);

        unsafe { qjs::JS_RunGC(rt) };
        crate::js_ops::dispatch_pending_weak_callbacks_for_ctx(ctx);
        with_state_for_ctx(ctx, |state| {
            crate::js_ops::process_weak_holds_for_ctx(state);
        });

        let counts = teardown_counts_for_ctx(ctx);
        if counts.is_zero() {
            break;
        }
        if counts.strong_roots == 0
            && counts.weak_holds == 0
            && counts.weak_callbacks == 0
            && counts.pending_weak == 0
            && counts.strong_persistents == 0
            && counts.weak_persistents != 0
        {
            let _ = dispose_weak_persistents_for_ctx(ctx, key);
            unsafe { qjs::JS_RunGC(rt) };
            let after_weak = teardown_counts_for_ctx(ctx);
            if after_weak.is_zero() {
                break;
            }
            if pass > 0 && after_weak == last_counts {
                return Err(format!(
                    "V8 teardown stalled: {after_weak:?} (weak persistents remain after dispose)"
                ));
            }
            last_counts = after_weak;
            continue;
        }
        if pass > 0 && counts == last_counts {
            return Err(format!(
                "V8 teardown stalled: {counts:?} (weak persistents after strong dispose: {weak_remaining})"
            ));
        }
        if pass + 1 == MAX_TEARDOWN_PASSES {
            return Err(format!(
                "V8 teardown incomplete after {MAX_TEARDOWN_PASSES} passes: {counts:?}"
            ));
        }
        last_counts = counts;
    }

    let final_counts = teardown_counts_for_ctx(ctx);
    if final_counts.weak_persistents != 0 {
        let _ = dispose_weak_persistents_for_ctx(ctx, key);
        unsafe { qjs::JS_RunGC(rt) };
        let after_weak = teardown_counts_for_ctx(ctx);
        if after_weak.weak_persistents != 0 {
            return Err(format!(
                "V8 teardown: {} weak persistent(s) remain after dispose",
                after_weak.weak_persistents
            ));
        }
    }
    if final_counts.strong_persistents != 0 {
        let _ = dispose_strong_persistents_for_ctx(ctx, key);
        let after_strong = teardown_counts_for_ctx(ctx);
        if after_strong.strong_persistents != 0 {
            return Err(format!(
                "V8 teardown: {} strong persistent(s) remain after dispose",
                after_strong.strong_persistents
            ));
        }
    }

    unsafe {
        if let Some(class_id) = crate::js_ops::v8_object_class_for_runtime(rt) {
            let proto = qjs::JS_GetClassProto(ctx, class_id);
            if !qjs::JS_IsUndefined(proto) {
                qjs::JS_SetClassProto(ctx, class_id, qjs::JS_UNDEFINED);
                qjs::JS_FreeValue(ctx, proto);
            }
        }
        extern "C" {
            fn JS_ReleaseContextClassProtos(ctx: *mut JSContext);
        }
        JS_ReleaseContextClassProtos(ctx);
    }
    for _ in 0..16 {
        unsafe { qjs::JS_RunGC(rt) };
        crate::js_ops::dispatch_pending_weak_callbacks_for_ctx(ctx);
        with_state_for_ctx_if(ctx, |state| {
            crate::js_ops::process_weak_holds_for_ctx(state);
        });
    }

    crate::js_ops::remove_context_tables_and_gc(ctx);

    if let Some(isolate_ptr) = crate::module_loader::isolate_ptr_for_runtime(rt_key) {
        extern "C" {
            fn lexe_v8_clear_registries_for_context(context_key: usize);
        }
        unsafe {
            lexe_v8_clear_registries_for_context(key);
        }
        let _isolate_ptr = isolate_ptr;
    }

    let remaining = teardown_counts_for_ctx(ctx);
    if !remaining.is_zero() {
        return Err(format!(
            "V8 teardown residual state before post-bridge GC: {remaining:?}"
        ));
    }

    BRIDGE_STATES.with(|cell| {
        cell.borrow_mut().remove(&key);
    });
    ACTIVE_BRIDGE_CTX.with(|cell| {
        if *cell.borrow() == Some(key) {
            *cell.borrow_mut() = None;
        }
    });

    let remaining = teardown_counts_for_ctx(ctx);
    if !remaining.is_zero() {
        return Err(format!("V8 teardown residual state: {remaining:?}"));
    }
    Ok(())
}

/// Backward-compatible alias for per-context bridge teardown.
///
/// # Safety
///
/// Same as [`shutdown_bridge_for_context`].
pub unsafe fn prepare_shutdown(ctx: *mut JSContext) -> Result<(), String> {
    unsafe { shutdown_bridge_for_context(ctx) }
}

pub fn with_bridge_roots<F, R>(f: F) -> R
where
    F: FnOnce(*mut JSContext, &RootTable) -> R,
{
    let key = active_ctx_key();
    BRIDGE_STATES.with(|cell| {
        let guard = cell.borrow();
        let state = guard
            .get(&key)
            .expect("v8 bridge not initialized for context");
        f(state.ctx_ptr(), &state.roots)
    })
}

pub fn node_abi_version() -> u32 {
    NODE_MODULE_VERSION as u32
}

pub fn napi_node_version_u32() -> u32 {
    crate::abi::NAPI_NODE_VERSION_U32
}

pub fn napi_api_version() -> u32 {
    NAPI_API_VERSION
}
