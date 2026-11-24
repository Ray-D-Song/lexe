use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rquickjs::qjs::{self, JSContext, JSValue, JS_MKVAL, JS_TAG_INT};

use crate::bridge::{with_state, with_state_for_ctx, LexeV8Status};

fn new_int32(_ctx: *mut JSContext, val: i32) -> JSValue {
    JS_MKVAL(JS_TAG_INT, val)
}

thread_local! {
    static EMBEDDER_SCOPE_DEPTH: Cell<u32> = const { Cell::new(0) };
    static EMBEDDER_FIELD0_STACK: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn clear_embedder_field0_stack() {
    EMBEDDER_FIELD0_STACK.with(|stack| stack.borrow_mut().clear());
}

pub(crate) fn push_embedder_field0_frame() {
    EMBEDDER_FIELD0_STACK.with(|stack| stack.borrow_mut().push(0));
}

pub(crate) fn pop_embedder_field0_frame() {
    EMBEDDER_FIELD0_STACK.with(|stack| {
        stack.borrow_mut().pop();
    });
}

pub(crate) fn set_embedder_field0_in_frame(ptr: usize) {
    EMBEDDER_FIELD0_STACK.with(|stack| {
        if let Some(top) = stack.borrow_mut().last_mut() {
            *top = ptr;
        }
    });
}

pub(crate) fn current_embedder_field0_in_frame() -> usize {
    EMBEDDER_FIELD0_STACK.with(|stack| stack.borrow().last().copied().unwrap_or(0))
}

pub(crate) struct EmbedderScopeGuard;

impl EmbedderScopeGuard {
    pub fn enter() -> Self {
        EMBEDDER_SCOPE_DEPTH.with(|depth| {
            if depth.get() == 0 {
                clear_embedder_field0_stack();
            }
            depth.set(depth.get() + 1);
        });
        push_embedder_field0_frame();
        Self
    }
}

impl Drop for EmbedderScopeGuard {
    fn drop(&mut self) {
        pop_embedder_field0_frame();
        EMBEDDER_SCOPE_DEPTH.with(|depth| {
            let next = depth.get().saturating_sub(1);
            depth.set(next);
            if next == 0 {
                clear_embedder_field0_stack();
            }
        });
    }
}

static V8_OBJECT_CLASS: Lazy<Mutex<HashMap<usize, qjs::JSClassID>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

use crate::context_tables::{self, WeakPass, WeakPhase, WeakSlot};

extern "C" {
    fn lexe_v8_invoke_weak_callback_first_pass(
        callback: *mut c_void,
        parameter: *mut c_void,
        out_second_pass: *mut *mut c_void,
    ) -> i32;
    fn lexe_v8_invoke_weak_callback_second_pass(callback: *mut c_void, parameter: *mut c_void);
}

pub(crate) fn dispatch_pending_weak_callbacks_for_ctx(ctx: *mut JSContext) {
    loop {
        let pending = context_tables::take_pending_weak_callbacks(ctx);
        if pending.is_empty() {
            break;
        }
        let mut deferred_second = Vec::new();
        for item in pending {
            match item.pass {
                WeakPass::Second => unsafe {
                    lexe_v8_invoke_weak_callback_second_pass(
                        item.callback as *mut c_void,
                        item.parameter as *mut c_void,
                    );
                },
                WeakPass::First => {
                    let mut second_pass: *mut c_void = std::ptr::null_mut();
                    let needs_second = unsafe {
                        lexe_v8_invoke_weak_callback_first_pass(
                            item.callback as *mut c_void,
                            item.parameter as *mut c_void,
                            &mut second_pass,
                        )
                    } != 0;
                    if needs_second && !second_pass.is_null() {
                        deferred_second.push(context_tables::PendingWeak {
                            callback: second_pass as usize,
                            parameter: item.parameter,
                            object_key: item.object_key,
                            pass: WeakPass::Second,
                        });
                    }
                },
            }
        }
        for item in deferred_second {
            unsafe {
                lexe_v8_invoke_weak_callback_second_pass(
                    item.callback as *mut c_void,
                    item.parameter as *mut c_void,
                );
            }
        }
    }
}

pub fn dispatch_pending_weak_callbacks() {
    with_state(|state| {
        dispatch_pending_weak_callbacks_for_ctx(state.ctx_ptr());
    });
}

#[no_mangle]
pub extern "C" fn lexe_v8_dispatch_pending_weak_callbacks() {
    dispatch_pending_weak_callbacks();
}

fn object_ptr_key(obj: JSValue) -> Option<usize> {
    if unsafe { !qjs::JS_IsObject(obj) } {
        return None;
    }
    Some(unsafe { qjs::JS_VALUE_GET_PTR(obj) as usize })
}

fn internal_field_ptr_for_object(
    tables: &context_tables::ContextJsTables,
    obj: JSValue,
    index: usize,
) -> Option<usize> {
    let key = object_ptr_key(obj)?;
    tables
        .internal_fields
        .get(&key)
        .and_then(|fields| fields.get(index).copied())
}

fn internal_field_ptr_for_root(
    tables: &context_tables::ContextJsTables,
    root_id: u64,
    index: usize,
) -> Option<usize> {
    tables
        .internal_fields_by_root
        .get(&root_id)
        .and_then(|fields| fields.get(index).copied())
}

pub(crate) fn embedder_ptr_for_object(
    ctx: *mut JSContext,
    obj: JSValue,
    root_id: u64,
    index: usize,
) -> usize {
    context_tables::with_context_tables(ctx, |tables| {
        if let Some(ptr) = internal_field_ptr_for_object(tables, obj, index) {
            return ptr;
        }
        if let Some(ptr) = internal_field_ptr_for_root(tables, root_id, index) {
            return ptr;
        }
        if index == 0 && unsafe { qjs::JS_IsObject(obj) } {
            let class_id = ensure_v8_object_class(ctx);
            let opaque = unsafe { qjs::JS_GetOpaque(obj, class_id) as usize };
            if opaque != 0 {
                return opaque;
            }
        }
        0
    })
}

fn root_id_for_ptr(state: &crate::bridge::BridgeState, ptr: usize) -> Option<u64> {
    state.roots.find_id_by_ptr(ptr)
}

pub unsafe extern "C" fn root_from_js_value(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    js_value_tag: u64,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() || js_value_tag == 0 {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ptr = js_value_tag as usize;
        if let Some(id) = root_id_for_ptr(state, ptr) {
            *out = id;
            return LexeV8Status::Ok;
        }
        root_restrong_from_object_ptr(_ctx, ptr as *mut c_void, out)
    })
}

pub unsafe extern "C" fn root_id_for_js_object(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_ptr: *mut c_void,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() || object_ptr.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ptr = object_ptr as usize;
        let Some(id) = root_id_for_ptr(state, ptr) else {
            return LexeV8Status::Error;
        };
        *out = id;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn object_ptr_for_root(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut *mut c_void,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(obj) = state.get_object_for_root(root) else {
            return LexeV8Status::Error;
        };
        let Some(key) = object_ptr_key(obj) else {
            return LexeV8Status::Error;
        };
        *out = key as *mut c_void;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn root_restrong_from_object_ptr(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_ptr: *mut c_void,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() || object_ptr.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let ptr = object_ptr as usize;
        if let Some(id) = root_id_for_ptr(state, ptr) {
            *out = id;
            return LexeV8Status::Ok;
        }
        if let Some(value) = state.take_weak_hold(ptr) {
            *out = state
                .roots
                .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, value) });
            return LexeV8Status::Ok;
        }
        let tagged = qjs::JS_MKPTR(qjs::JS_TAG_OBJECT, object_ptr);
        if !qjs::JS_IsObject(tagged) {
            return LexeV8Status::Error;
        }
        let dup = qjs::JS_DupValue(ctx, tagged);
        if qjs::JS_IsException(dup) {
            return LexeV8Status::Exception;
        }
        *out = state
            .roots
            .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, dup) });
        LexeV8Status::Ok
    })
}

pub fn root_make_weak(state: &mut crate::bridge::BridgeState, id: u64) -> LexeV8Status {
    let ctx = state.ctx_ptr();
    let Some(value) = state.roots.detach_root(id) else {
        return LexeV8Status::Error;
    };
    let Some(key) = object_ptr_key(value) else {
        unsafe { qjs::JS_FreeValue(ctx, value) };
        return LexeV8Status::Ok;
    };
    state.insert_weak_hold(key, value);
    state.record_weak_root(id, key);
    LexeV8Status::Ok
}

pub(crate) fn process_weak_holds_for_ctx(state: &mut crate::bridge::BridgeState) {
    let ctx = state.ctx_ptr();
    let keys = state.weak_hold_keys();
    for key in keys {
        if root_id_for_ptr(state, key).is_some() {
            continue;
        }
        let has_weak = context_tables::with_context_tables(ctx, |tables| {
            tables.weak_callbacks.contains_key(&key)
        });
        if !has_weak {
            continue;
        }
        if let Some(value) = state.take_weak_hold(key) {
            unsafe { qjs::JS_FreeValue(ctx, value) };
        }
    }
}

pub unsafe extern "C" fn unregister_weak_for_object_ptr(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_ptr: *mut c_void,
) {
    if object_ptr.is_null() {
        return;
    }
    with_state(|state| {
        context_tables::unregister_weak_callback(state.ctx_ptr(), object_ptr as usize);
    });
}

pub(crate) fn ensure_v8_object_class(ctx: *mut JSContext) -> qjs::JSClassID {
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let rt_key = rt as usize;
    let mut map = V8_OBJECT_CLASS.lock();
    if let Some(&id) = map.get(&rt_key) {
        return id;
    }
    let mut class_id: qjs::JSClassID = 0;
    unsafe {
        qjs::JS_NewClassID(rt, &mut class_id);
        let def = qjs::JSClassDef {
            class_name: c"LexeV8Object".as_ptr(),
            finalizer: Some(v8_object_finalizer),
            gc_mark: None,
            call: None,
            exotic: ptr::null_mut(),
        };
        qjs::JS_NewClass(rt, class_id, &def);
    }
    map.insert(rt_key, class_id);
    class_id
}

unsafe extern "C" fn v8_object_finalizer(rt: *mut qjs::JSRuntime, val: JSValue) {
    if let Some(key) = object_ptr_key(val) {
        context_tables::remove_object_records(rt, key);
    }
}

pub fn new_v8_object(ctx: *mut JSContext) -> JSValue {
    let class_id = ensure_v8_object_class(ctx);
    unsafe { qjs::JS_NewObjectClass(ctx, class_id) }
}

fn value_from_root(ctx: *mut JSContext, root: u64) -> Option<JSValue> {
    with_state(|state| {
        state
            .roots
            .get(root)
            .map(|v| unsafe { qjs::JS_DupValue(ctx, v) })
    })
}

fn atom_from_key(ctx: *mut JSContext, key: JSValue) -> Option<qjs::JSAtom> {
    let atom = unsafe { qjs::JS_ValueToAtom(ctx, key) };
    if atom == 0 {
        None
    } else {
        Some(atom)
    }
}

pub unsafe extern "C" fn object_get(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    key_root: u64,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(obj) = state.roots.get(object_root) else {
            return LexeV8Status::Error;
        };
        let Some(key) = state.roots.get(key_root) else {
            return LexeV8Status::Error;
        };
        let Some(atom) = atom_from_key(state.ctx_ptr(), key) else {
            return LexeV8Status::Error;
        };
        let val = qjs::JS_GetProperty(state.ctx_ptr(), obj, atom);
        qjs::JS_FreeAtom(state.ctx_ptr(), atom);
        if qjs::JS_IsException(val) {
            return LexeV8Status::Exception;
        }
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), val)
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn object_get_index(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    index: u32,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(obj) = state.roots.get(object_root) else {
            return LexeV8Status::Error;
        };
        let val = qjs::JS_GetPropertyUint32(state.ctx_ptr(), obj, index);
        if qjs::JS_IsException(val) {
            return LexeV8Status::Exception;
        }
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), val)
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn object_set_index(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    index: u32,
    value_root: u64,
) -> LexeV8Status {
    with_state(|state| {
        let Some(obj) = state.roots.get(object_root) else {
            return LexeV8Status::Error;
        };
        let Some(val) = state.roots.get(value_root) else {
            return LexeV8Status::Error;
        };
        let rc = qjs::JS_SetPropertyUint32(
            state.ctx_ptr(),
            obj,
            index,
            qjs::JS_DupValue(state.ctx_ptr(), val),
        );
        if rc < 0 {
            return LexeV8Status::Exception;
        }
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn object_define_own_property(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    key_root: u64,
    value_root: u64,
    attr: c_int,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
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
        let Some(atom) = atom_from_key(state.ctx_ptr(), key) else {
            return LexeV8Status::Error;
        };
        let mut flags = qjs::JS_PROP_C_W_E as c_int;
        if attr & 1 == 0 {
            flags |= qjs::JS_PROP_WRITABLE as c_int;
        }
        if attr & 2 == 0 {
            flags |= qjs::JS_PROP_ENUMERABLE as c_int;
        }
        if attr & 4 == 0 {
            flags |= qjs::JS_PROP_CONFIGURABLE as c_int;
        }
        let rc = qjs::JS_DefinePropertyValue(
            state.ctx_ptr(),
            obj,
            atom,
            qjs::JS_DupValue(state.ctx_ptr(), val),
            flags,
        );
        qjs::JS_FreeAtom(state.ctx_ptr(), atom);
        if rc < 0 {
            return LexeV8Status::Exception;
        }
        *out = true;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn object_has_own_property(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    key_root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(obj) = state.roots.get(object_root) else {
            return LexeV8Status::Error;
        };
        let Some(key) = state.roots.get(key_root) else {
            return LexeV8Status::Error;
        };
        let Some(atom) = atom_from_key(state.ctx_ptr(), key) else {
            return LexeV8Status::Error;
        };
        let mut prop = qjs::JSPropertyDescriptor {
            flags: 0,
            value: qjs::JS_UNDEFINED,
            getter: qjs::JS_UNDEFINED,
            setter: qjs::JS_UNDEFINED,
        };
        let rc = qjs::JS_GetOwnProperty(state.ctx_ptr(), &mut prop, obj, atom);
        qjs::JS_FreeAtom(state.ctx_ptr(), atom);
        if rc < 0 {
            return LexeV8Status::Exception;
        }
        if rc > 0 {
            qjs::JS_FreeValue(state.ctx_ptr(), prop.value);
            if !qjs::JS_IsUndefined(prop.getter) {
                qjs::JS_FreeValue(state.ctx_ptr(), prop.getter);
            }
            if !qjs::JS_IsUndefined(prop.setter) {
                qjs::JS_FreeValue(state.ctx_ptr(), prop.setter);
            }
        }
        *out = rc > 0;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn object_get_prototype(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(obj) = state.roots.get(object_root) else {
            return LexeV8Status::Error;
        };
        let proto = qjs::JS_GetPrototype(state.ctx_ptr(), obj);
        if qjs::JS_IsException(proto) {
            return LexeV8Status::Exception;
        }
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), proto)
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn array_new(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    length: c_int,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let arr = qjs::JS_NewArray(state.ctx_ptr());
        if length > 0 {
            let len = new_int32(state.ctx_ptr(), length);
            qjs::JS_SetPropertyStr(state.ctx_ptr(), arr, c"length".as_ptr(), len);
        }
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), arr)
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn number_new(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    value: f64,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let num = qjs::JS_NewFloat64(value);
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), num)
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn bigint_new(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    value: i64,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let bi = qjs::JS_NewBigInt64(state.ctx_ptr(), value);
        *out = state
            .roots
            .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), bi) });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn integer_new(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    value: c_int,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let num = new_int32(state.ctx_ptr(), value);
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), num)
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn string_new_latin1(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    data: *const u8,
    length: c_int,
    out: *mut u64,
) -> LexeV8Status {
    if data.is_null() || out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let bytes = if length < 0 {
            std::ffi::CStr::from_ptr(data as *const c_char).to_bytes()
        } else {
            std::slice::from_raw_parts(data, length as usize)
        };
        let js = qjs::JS_NewStringLen(
            state.ctx_ptr(),
            bytes.as_ptr() as *const c_char,
            bytes.len() as u64,
        );
        *out = state
            .roots
            .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), js) });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn string_to_utf8(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    value_root: u64,
    out_ptr: *mut *mut c_char,
    out_len: *mut usize,
) -> LexeV8Status {
    if out_ptr.is_null() || out_len.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = state.roots.get(value_root) else {
            return LexeV8Status::Error;
        };
        let mut len: usize = 0;
        let ptr = qjs::JS_ToCStringLen(state.ctx_ptr(), &mut len, val);
        if ptr.is_null() {
            return LexeV8Status::Error;
        }
        *out_ptr = ptr as *mut c_char;
        *out_len = len;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn string_free_utf8(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    ptr: *mut c_char,
) -> LexeV8Status {
    with_state(|state| {
        if !ptr.is_null() {
            qjs::JS_FreeCString(state.ctx_ptr(), ptr);
        }
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn function_call(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    func_root: u64,
    recv_root: u64,
    argc: c_int,
    args: *const u64,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(func) = state.roots.get(func_root) else {
            return LexeV8Status::Error;
        };
        let recv = if recv_root == 0 {
            qjs::JS_UNDEFINED
        } else {
            let Some(r) = state.roots.get(recv_root) else {
                return LexeV8Status::Error;
            };
            qjs::JS_DupValue(state.ctx_ptr(), r)
        };
        let mut argv: Vec<JSValue> = Vec::with_capacity(argc.max(0) as usize);
        if !args.is_null() && argc > 0 {
            let slice = std::slice::from_raw_parts(args, argc as usize);
            for &root in slice {
                let Some(arg) = state.roots.get(root) else {
                    return LexeV8Status::Error;
                };
                argv.push(qjs::JS_DupValue(state.ctx_ptr(), arg));
            }
        }
        let result = qjs::JS_Call(state.ctx_ptr(), func, recv, argc, argv.as_mut_ptr());
        for arg in argv {
            qjs::JS_FreeValue(state.ctx_ptr(), arg);
        }
        if recv_root != 0 {
            qjs::JS_FreeValue(state.ctx_ptr(), recv);
        }
        if qjs::JS_IsException(result) {
            return LexeV8Status::Exception;
        }
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), result)
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn throw_value(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    value_root: u64,
) -> LexeV8Status {
    with_state(|state| {
        let Some(val) = state.roots.get(value_root) else {
            return LexeV8Status::Error;
        };
        qjs::JS_Throw(state.ctx_ptr(), qjs::JS_DupValue(state.ctx_ptr(), val));
        LexeV8Status::Exception
    })
}

pub unsafe extern "C" fn new_exception(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    msg_root: u64,
    kind: c_int,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(msg) = state.roots.get(msg_root) else {
            return LexeV8Status::Error;
        };
        let err = match kind {
            1 => {
                let err = qjs::JS_NewError(state.ctx_ptr());
                qjs::JS_SetPropertyStr(
                    state.ctx_ptr(),
                    err,
                    c"name".as_ptr(),
                    qjs::JS_NewStringLen(state.ctx_ptr(), c"TypeError".as_ptr(), 9),
                );
                err
            },
            2 => {
                let err = qjs::JS_NewError(state.ctx_ptr());
                qjs::JS_SetPropertyStr(
                    state.ctx_ptr(),
                    err,
                    c"name".as_ptr(),
                    qjs::JS_NewStringLen(state.ctx_ptr(), c"RangeError".as_ptr(), 10),
                );
                err
            },
            _ => qjs::JS_NewError(state.ctx_ptr()),
        };
        qjs::JS_SetPropertyStr(
            state.ctx_ptr(),
            err,
            c"message".as_ptr(),
            qjs::JS_DupValue(state.ctx_ptr(), msg),
        );
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), err)
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn external_new(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    ptr: *mut c_void,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let obj = new_v8_object(ctx);
        if let Some(key) = object_ptr_key(obj) {
            context_tables::with_context_tables(ctx, |tables| {
                tables
                    .internal_fields
                    .entry(key)
                    .or_default()
                    .push(ptr as usize);
            });
        }
        *out = state
            .roots
            .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, obj) });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn object_reserve_internal_fields(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    count: c_int,
) -> LexeV8Status {
    if count < 0 {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let Some(obj) = state.roots.get(object_root) else {
            return LexeV8Status::Error;
        };
        let Some(key) = object_ptr_key(obj) else {
            return LexeV8Status::Error;
        };
        let count = count as usize;
        context_tables::with_context_tables(ctx, |tables| {
            tables.object_field_counts.insert(key, count);
            let entry = tables.internal_fields.entry(key).or_default();
            if entry.len() < count {
                entry.resize(count, 0);
            }
        });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn object_internal_field_count(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    out: *mut c_int,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let obj = state.get_object_for_root(object_root);
        let key = obj.and_then(object_ptr_key);
        let count = context_tables::with_context_tables(ctx, |tables| {
            key.and_then(|k| {
                tables
                    .object_field_counts
                    .get(&k)
                    .copied()
                    .or_else(|| tables.internal_fields.get(&k).map(|v| v.len()))
            })
            .unwrap_or(0)
        });
        *out = count as c_int;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn internal_field_set(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    index: c_int,
    ptr: *mut c_void,
) -> LexeV8Status {
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let Some(obj) = state.get_object_for_root(object_root) else {
            return LexeV8Status::Error;
        };
        let Some(key) = object_ptr_key(obj) else {
            return LexeV8Status::Error;
        };
        context_tables::with_context_tables(ctx, |tables| {
            let fields = tables.internal_fields.entry(key).or_default();
            let idx = index as usize;
            if fields.len() <= idx {
                fields.resize(idx + 1, 0);
            }
            fields[idx] = ptr as usize;
            let root_fields = tables
                .internal_fields_by_root
                .entry(object_root)
                .or_default();
            if root_fields.len() <= idx {
                root_fields.resize(idx + 1, 0);
            }
            root_fields[idx] = ptr as usize;
        });
        if index == 0 {
            set_embedder_field0_in_frame(ptr as usize);
        }
        let class_id = ensure_v8_object_class(ctx);
        unsafe {
            qjs::JS_SetOpaque(obj, ptr);
            let _ = class_id;
        }
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn internal_field_get(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    index: c_int,
    out: *mut *mut c_void,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let obj = state.get_object_for_root(object_root);
        let key = obj.and_then(object_ptr_key);
        let mut ptr = context_tables::with_context_tables(ctx, |tables| {
            key.and_then(|k| {
                tables
                    .internal_fields
                    .get(&k)
                    .and_then(|fields| fields.get(index as usize).copied())
            })
            .or_else(|| internal_field_ptr_for_root(tables, object_root, index as usize))
            .unwrap_or(0)
        });
        if ptr == 0 && index == 0 {
            if let Some(obj) = obj {
                let class_id = ensure_v8_object_class(ctx);
                ptr = unsafe { qjs::JS_GetOpaque(obj, class_id) as usize };
            }
        }
        unsafe {
            *out = ptr as *mut c_void;
        }
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn symbol_iterator(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let global = qjs::JS_GetGlobalObject(state.ctx_ptr());
        let symbol = qjs::JS_GetPropertyStr(state.ctx_ptr(), global, c"Symbol".as_ptr());
        qjs::JS_FreeValue(state.ctx_ptr(), global);
        if qjs::JS_IsException(symbol) {
            return LexeV8Status::Exception;
        }
        let iter = qjs::JS_GetPropertyStr(state.ctx_ptr(), symbol, c"iterator".as_ptr());
        qjs::JS_FreeValue(state.ctx_ptr(), symbol);
        if qjs::JS_IsException(iter) {
            return LexeV8Status::Exception;
        }
        *out = state.roots.insert_owned(unsafe {
            crate::owned_js_value::OwnedJsValue::new(state.ctx_ptr(), iter)
        });
        LexeV8Status::Ok
    })
}

extern "C" {
    fn lexe_v8_context_root_id(ctx: *mut crate::bridge::LexeV8ContextState) -> u64;
}

pub unsafe extern "C" fn get_context_root(
    ctx: *mut crate::bridge::LexeV8ContextState,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    *out = lexe_v8_context_root_id(ctx);
    LexeV8Status::Ok
}

pub unsafe extern "C" fn oddball_root(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root_index: c_int,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let root = match root_index {
            4 => state.roots.insert_immortal_tag(qjs::JS_UNDEFINED),
            5 => state.roots.insert_immortal_tag(qjs::JS_UNDEFINED),
            6 => state.roots.insert_immortal_tag(qjs::JS_NULL),
            7 => state.roots.insert_immortal_tag(qjs::JS_TRUE),
            8 => state.roots.insert_immortal_tag(qjs::JS_FALSE),
            9 => {
                let empty = qjs::JS_NewStringLen(ctx, b"".as_ptr() as *const c_char, 0);
                state
                    .roots
                    .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, empty) })
            },
            _ => return LexeV8Status::Error,
        };
        *out = root;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn get_creation_context(
    ctx: *mut crate::bridge::LexeV8ContextState,
    _object_root: u64,
    out: *mut u64,
) -> LexeV8Status {
    get_context_root(ctx, out)
}

pub unsafe extern "C" fn register_weak_callback(
    ctx_state: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    parameter: *mut c_void,
    callback: *mut c_void,
) -> LexeV8Status {
    if callback.is_null() {
        return LexeV8Status::Error;
    }
    let register_on = |ctx: *mut qjs::JSContext, obj: JSValue| -> LexeV8Status {
        let Some(key) = object_ptr_key(obj) else {
            return LexeV8Status::Error;
        };
        context_tables::with_context_tables(ctx, |tables| {
            tables.weak_callbacks.insert(
                key,
                WeakSlot {
                    callback: callback as usize,
                    parameter: parameter as usize,
                    phase: WeakPhase::Registered,
                },
            );
        });
        LexeV8Status::Ok
    };

    if let Some(qjs_ctx) =
        crate::module_loader::js_context_for_state(ctx_state as *mut crate::isolate::ContextState)
    {
        return crate::bridge::with_state_for_ctx(qjs_ctx, |state| {
            let Some(obj) = state.get_object_for_root(object_root) else {
                return LexeV8Status::Error;
            };
            register_on(state.ctx_ptr(), obj)
        });
    }

    crate::bridge::with_state_for_object_root(object_root, register_on)
        .unwrap_or(LexeV8Status::Error)
}

/// Invoke every registered ObjectWrap weak callback without waiting for JSObject
/// collection. Used during process shutdown when module refs are already gone.
pub(crate) unsafe fn force_invoke_registered_weak_callbacks(ctx: *mut JSContext) {
    let slots: Vec<(usize, usize, usize)> = context_tables::with_context_tables(ctx, |tables| {
        tables
            .weak_callbacks
            .iter()
            .filter(|(_, slot)| slot.phase == WeakPhase::Registered && slot.callback != 0)
            .map(|(&key, slot)| (key, slot.callback, slot.parameter))
            .collect()
    });
    for (key, callback, parameter) in slots {
        let mut second_pass: *mut c_void = std::ptr::null_mut();
        let needs_second = unsafe {
            lexe_v8_invoke_weak_callback_first_pass(
                callback as *mut c_void,
                parameter as *mut c_void,
                &mut second_pass,
            )
        } != 0;
        if needs_second && !second_pass.is_null() {
            unsafe {
                lexe_v8_invoke_weak_callback_second_pass(second_pass, parameter as *mut c_void);
            }
        }
        context_tables::unregister_weak_callback(ctx, key);
    }
    dispatch_pending_weak_callbacks_for_ctx(ctx);
}

pub fn clear_runtime_v8_object_class(rt: *mut qjs::JSRuntime) {
    V8_OBJECT_CLASS.lock().remove(&(rt as usize));
}

pub fn v8_object_class_for_runtime(rt: *mut qjs::JSRuntime) -> Option<qjs::JSClassID> {
    V8_OBJECT_CLASS.lock().get(&(rt as usize)).copied()
}

pub fn remove_context_tables_and_gc(ctx: *mut JSContext) {
    context_tables::remove_context_tables(ctx);
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    for _ in 0..3 {
        unsafe { qjs::JS_RunGC(rt) };
    }
}

pub fn shutdown_context_tables(ctx: *mut JSContext) {
    with_state_for_ctx(ctx, |state| {
        state.drain_weak_holds(ctx);
    });
    dispatch_pending_weak_callbacks_for_ctx(ctx);
    remove_context_tables_and_gc(ctx);
}

/// Backward-compatible alias for per-context side-table teardown.
pub fn prepare_shutdown(ctx: *mut JSContext) -> Result<(), String> {
    unsafe { crate::bridge::shutdown_bridge_for_context(ctx) }
}

#[no_mangle]
pub extern "C" fn lexe_v8_run_gc() {
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let rt = unsafe { qjs::JS_GetRuntime(ctx) };
        process_weak_holds_for_ctx(state);
        unsafe { qjs::JS_RunGC(rt) };
        dispatch_pending_weak_callbacks_for_ctx(ctx);
    });
}
