use std::collections::HashMap;
use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use rquickjs::qjs::{JS_MKVAL, JS_TAG_INT};

use crate::bridge::{with_state, with_state_ref, LexeV8Status};

fn value_from_root(
    state: &crate::bridge::BridgeState,
    root: u64,
) -> Option<rquickjs::qjs::JSValue> {
    state.roots.get(root)
}

#[repr(u8)]
#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub enum ValueLayoutKind {
    Object = 0,
    String = 1,
    Oddball = 2,
    HeapNumber = 3,
    Int32Smi = 4,
}

pub unsafe extern "C" fn value_layout_kind(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out_kind: *mut u8,
    out_smi: *mut i32,
) -> LexeV8Status {
    if out_kind.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        let kind = if rquickjs::qjs::JS_IsString(val) {
            ValueLayoutKind::String
        } else if rquickjs::qjs::JS_IsBool(val)
            || rquickjs::qjs::JS_IsNull(val)
            || rquickjs::qjs::JS_IsUndefined(val)
        {
            ValueLayoutKind::Oddball
        } else if unsafe { rquickjs::qjs::JS_VALUE_GET_NORM_TAG(val) == rquickjs::qjs::JS_TAG_INT }
        {
            ValueLayoutKind::Int32Smi
        } else if rquickjs::qjs::JS_IsNumber(val) {
            ValueLayoutKind::HeapNumber
        } else {
            ValueLayoutKind::Object
        };
        *out_kind = kind as u8;
        if !out_smi.is_null() && kind == ValueLayoutKind::Int32Smi {
            let mut n = 0i32;
            if rquickjs::qjs::JS_ToInt32(state.ctx_ptr(), &mut n, val) < 0 {
                return LexeV8Status::Exception;
            }
            *out_smi = n;
        }
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_is_object(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out = rquickjs::qjs::JS_IsObject(val);
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_is_array(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out = rquickjs::qjs::JS_IsArray(val);
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_is_function(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out = rquickjs::qjs::JS_IsFunction(state.ctx_ptr(), val);
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_is_number(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out = rquickjs::qjs::JS_IsNumber(val);
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_is_int32(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out = unsafe { rquickjs::qjs::JS_VALUE_GET_NORM_TAG(val) == rquickjs::qjs::JS_TAG_INT };
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_is_bigint(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out =
            unsafe { rquickjs::qjs::JS_VALUE_GET_NORM_TAG(val) == rquickjs::qjs::JS_TAG_BIG_INT };
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_is_boolean(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out = rquickjs::qjs::JS_IsBool(val);
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_to_boolean(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out = rquickjs::qjs::JS_ToBool(state.ctx_ptr(), val) != 0;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_strict_equals(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    a: u64,
    b: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(va) = value_from_root(state, a) else {
            return LexeV8Status::Error;
        };
        let Some(vb) = value_from_root(state, b) else {
            return LexeV8Status::Error;
        };
        *out = rquickjs::qjs::JS_IsSameValue(state.ctx_ptr(), va, vb);
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_to_float64(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut f64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        let mut d = 0.0;
        if rquickjs::qjs::JS_ToFloat64(state.ctx_ptr(), &mut d, val) < 0 {
            return LexeV8Status::Exception;
        }
        *out = d;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_to_int32(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut i32,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        let mut n = 0i32;
        if rquickjs::qjs::JS_ToInt32(state.ctx_ptr(), &mut n, val) < 0 {
            return LexeV8Status::Exception;
        }
        *out = n;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn value_to_int64(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut i64,
    lossless: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        let mut n = 0i64;
        let rc = rquickjs::qjs::JS_ToBigInt64(state.ctx_ptr(), &mut n, val);
        if rc < 0 {
            return LexeV8Status::Exception;
        }
        *out = n;
        if !lossless.is_null() {
            *lossless = rc == 0;
        }
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn array_length(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut u32,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        let len = rquickjs::qjs::JS_GetPropertyStr(state.ctx_ptr(), val, c"length".as_ptr());
        if rquickjs::qjs::JS_IsException(len) {
            return LexeV8Status::Exception;
        }
        let mut n = 0i32;
        if rquickjs::qjs::JS_ToInt32(state.ctx_ptr(), &mut n, len) < 0 {
            rquickjs::qjs::JS_FreeValue(state.ctx_ptr(), len);
            return LexeV8Status::Exception;
        }
        rquickjs::qjs::JS_FreeValue(state.ctx_ptr(), len);
        *out = n.max(0) as u32;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn function_new_instance(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    func_root: u64,
    argc: c_int,
    args: *const u64,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    let (ctx, func, mut argv) = match with_state_ref(|state| {
        let (_func_root, func) = match crate::bridge::resolve_constructor_root(state, func_root) {
            Some(pair) => pair,
            None => return Err(LexeV8Status::Error),
        };
        let ctx = state.ctx_ptr();
        let mut argv: Vec<rquickjs::qjs::JSValue> = Vec::with_capacity(argc.max(0) as usize);
        if !args.is_null() && argc > 0 {
            for &root in std::slice::from_raw_parts(args, argc as usize) {
                let Some(arg) = value_from_root(state, root) else {
                    return Err(LexeV8Status::Error);
                };
                argv.push(unsafe { rquickjs::qjs::JS_DupValue(ctx, arg) });
            }
        }
        Ok((ctx, func, argv))
    }) {
        Ok(v) => v,
        Err(status) => return status,
    };
    let result = unsafe { rquickjs::qjs::JS_CallConstructor(ctx, func, argc, argv.as_mut_ptr()) };
    for arg in argv {
        unsafe { rquickjs::qjs::JS_FreeValue(ctx, arg) };
    }
    if unsafe { rquickjs::qjs::JS_IsException(result) } {
        return LexeV8Status::Exception;
    }
    with_state(|state| {
        *out = state
            .roots
            .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, result) });
        LexeV8Status::Ok
    })
}

const MAX_BUFFER_VIEW_I32: usize = i32::MAX as usize;

fn buffer_view_i32_params(byte_offset: usize, byte_length: usize) -> Option<(i32, i32)> {
    if byte_offset > MAX_BUFFER_VIEW_I32 || byte_length > MAX_BUFFER_VIEW_I32 {
        return None;
    }
    Some((byte_offset as i32, byte_length as i32))
}

unsafe fn buffer_from_global(
    ctx: *mut rquickjs::qjs::JSContext,
    data: *const u8,
    len: usize,
) -> rquickjs::qjs::JSValue {
    if len != 0 && data.is_null() {
        return rquickjs::qjs::JS_EXCEPTION;
    }
    let array_buffer = rquickjs::qjs::JS_NewArrayBufferCopy(ctx, data, len as u64);
    if rquickjs::qjs::JS_IsException(array_buffer) {
        return rquickjs::qjs::JS_EXCEPTION;
    }
    let Some((offset, length)) = buffer_view_i32_params(0, len) else {
        rquickjs::qjs::JS_FreeValue(ctx, array_buffer);
        return rquickjs::qjs::JS_EXCEPTION;
    };
    let buf = buffer_from_array_buffer(ctx, array_buffer, offset, length);
    rquickjs::qjs::JS_FreeValue(ctx, array_buffer);
    buf
}

unsafe fn buffer_from_array_buffer(
    ctx: *mut rquickjs::qjs::JSContext,
    array_buffer: rquickjs::qjs::JSValue,
    byte_offset: i32,
    byte_length: i32,
) -> rquickjs::qjs::JSValue {
    let global = rquickjs::qjs::JS_GetGlobalObject(ctx);
    let buffer_ctor = rquickjs::qjs::JS_GetPropertyStr(ctx, global, c"Buffer".as_ptr());
    rquickjs::qjs::JS_FreeValue(ctx, global);
    if rquickjs::qjs::JS_IsException(buffer_ctor) {
        rquickjs::qjs::JS_FreeValue(ctx, buffer_ctor);
        return rquickjs::qjs::JS_EXCEPTION;
    }
    let from_fn = rquickjs::qjs::JS_GetPropertyStr(ctx, buffer_ctor, c"from".as_ptr());
    rquickjs::qjs::JS_FreeValue(ctx, buffer_ctor);
    if rquickjs::qjs::JS_IsException(from_fn) {
        rquickjs::qjs::JS_FreeValue(ctx, from_fn);
        return rquickjs::qjs::JS_EXCEPTION;
    }
    let offset_val = JS_MKVAL(JS_TAG_INT, byte_offset);
    let length_val = JS_MKVAL(JS_TAG_INT, byte_length);
    if rquickjs::qjs::JS_IsException(offset_val) || rquickjs::qjs::JS_IsException(length_val) {
        rquickjs::qjs::JS_FreeValue(ctx, from_fn);
        rquickjs::qjs::JS_FreeValue(ctx, offset_val);
        rquickjs::qjs::JS_FreeValue(ctx, length_val);
        return rquickjs::qjs::JS_EXCEPTION;
    }
    let mut argv = [array_buffer, offset_val, length_val];
    let buf = rquickjs::qjs::JS_Call(
        ctx,
        from_fn,
        rquickjs::qjs::JS_UNDEFINED,
        3,
        argv.as_mut_ptr(),
    );
    rquickjs::qjs::JS_FreeValue(ctx, from_fn);
    rquickjs::qjs::JS_FreeValue(ctx, offset_val);
    rquickjs::qjs::JS_FreeValue(ctx, length_val);
    buf
}

unsafe fn buffer_is_buffer(
    ctx: *mut rquickjs::qjs::JSContext,
    val: rquickjs::qjs::JSValue,
) -> bool {
    let global = rquickjs::qjs::JS_GetGlobalObject(ctx);
    let buffer_ctor = rquickjs::qjs::JS_GetPropertyStr(ctx, global, c"Buffer".as_ptr());
    rquickjs::qjs::JS_FreeValue(ctx, global);
    if rquickjs::qjs::JS_IsException(buffer_ctor) {
        rquickjs::qjs::JS_FreeValue(ctx, buffer_ctor);
        return false;
    }
    let is_buf_fn = rquickjs::qjs::JS_GetPropertyStr(ctx, buffer_ctor, c"isBuffer".as_ptr());
    rquickjs::qjs::JS_FreeValue(ctx, buffer_ctor);
    if rquickjs::qjs::JS_IsException(is_buf_fn) {
        rquickjs::qjs::JS_FreeValue(ctx, is_buf_fn);
        return false;
    }
    let mut arg = rquickjs::qjs::JS_DupValue(ctx, val);
    let result = rquickjs::qjs::JS_Call(ctx, is_buf_fn, rquickjs::qjs::JS_UNDEFINED, 1, &mut arg);
    rquickjs::qjs::JS_FreeValue(ctx, is_buf_fn);
    rquickjs::qjs::JS_FreeValue(ctx, arg);
    if rquickjs::qjs::JS_IsException(result) {
        rquickjs::qjs::JS_FreeValue(ctx, result);
        return false;
    }
    let ok = rquickjs::qjs::JS_ToBool(ctx, result) != 0;
    rquickjs::qjs::JS_FreeValue(ctx, result);
    ok
}

struct ExternalBufferRelease {
    callback: unsafe extern "C" fn(*mut u8, *mut c_void),
    hint: usize,
}

static NEXT_EXTERNAL_BUFFER_ID: AtomicUsize = AtomicUsize::new(1);
static EXTERNAL_BUFFER_RELEASES: Lazy<Mutex<HashMap<usize, ExternalBufferRelease>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

unsafe extern "C" fn external_buffer_finalizer(
    _rt: *mut rquickjs::qjs::JSRuntime,
    opaque: *mut c_void,
    ptr: *mut c_void,
) {
    let key = opaque as usize;
    if let Some(rel) = EXTERNAL_BUFFER_RELEASES.lock().remove(&key) {
        (rel.callback)(ptr as *mut u8, rel.hint as *mut c_void);
    }
}

unsafe fn buffer_view_data(
    ctx: *mut rquickjs::qjs::JSContext,
    val: rquickjs::qjs::JSValue,
) -> Option<(*mut u8, usize)> {
    let backing = rquickjs::qjs::JS_GetPropertyStr(ctx, val, c"buffer".as_ptr());
    if !rquickjs::qjs::JS_IsException(backing) {
        let offset_val = rquickjs::qjs::JS_GetPropertyStr(ctx, val, c"byteOffset".as_ptr());
        let length_val = rquickjs::qjs::JS_GetPropertyStr(ctx, val, c"byteLength".as_ptr());
        if !rquickjs::qjs::JS_IsException(offset_val) && !rquickjs::qjs::JS_IsException(length_val)
        {
            let mut ab_len = 0u64;
            let ab_ptr = rquickjs::qjs::JS_GetArrayBuffer(ctx, &mut ab_len, backing);
            let mut offset: i64 = 0;
            let mut view_len: i64 = 0;
            let ok = rquickjs::qjs::JS_ToInt64(ctx, &mut offset, offset_val) == 0
                && rquickjs::qjs::JS_ToInt64(ctx, &mut view_len, length_val) == 0
                && !ab_ptr.is_null()
                && offset >= 0
                && view_len >= 0
                && {
                    let offset_u = offset as u64;
                    let len_u = view_len as u64;
                    offset_u <= ab_len && len_u <= ab_len.saturating_sub(offset_u)
                };
            rquickjs::qjs::JS_FreeValue(ctx, backing);
            rquickjs::qjs::JS_FreeValue(ctx, offset_val);
            rquickjs::qjs::JS_FreeValue(ctx, length_val);
            if ok {
                return Some((unsafe { ab_ptr.add(offset as usize) }, view_len as usize));
            }
        } else {
            rquickjs::qjs::JS_FreeValue(ctx, backing);
            rquickjs::qjs::JS_FreeValue(ctx, offset_val);
            rquickjs::qjs::JS_FreeValue(ctx, length_val);
        }
    } else {
        rquickjs::qjs::JS_FreeValue(ctx, backing);
    }

    if rquickjs::qjs::JS_IsArrayBuffer(val) {
        let mut len: u64 = 0;
        let ptr = rquickjs::qjs::JS_GetArrayBuffer(ctx, &mut len, val);
        if !ptr.is_null() {
            return Some((ptr, len as usize));
        }
    }
    None
}

pub unsafe extern "C" fn buffer_new_copy(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    data: *const u8,
    len: usize,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() || (len != 0 && data.is_null()) {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let buf = buffer_from_global(ctx, data, len);
        if rquickjs::qjs::JS_IsException(buf) {
            return LexeV8Status::Exception;
        }
        *out = state
            .roots
            .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, buf) });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn buffer_new_external(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    data: *mut u8,
    len: usize,
    callback: Option<unsafe extern "C" fn(*mut u8, *mut c_void)>,
    hint: *mut c_void,
    out: *mut u64,
) -> LexeV8Status {
    if out.is_null() || (len != 0 && data.is_null()) {
        return LexeV8Status::Error;
    }
    let Some((offset, length)) = buffer_view_i32_params(0, len) else {
        return LexeV8Status::Error;
    };
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let release_key = NEXT_EXTERNAL_BUFFER_ID.fetch_add(1, Ordering::Relaxed);
        if let Some(cb) = callback {
            EXTERNAL_BUFFER_RELEASES.lock().insert(
                release_key,
                ExternalBufferRelease {
                    callback: cb,
                    hint: hint as usize,
                },
            );
        }
        let free_func: rquickjs::qjs::JSFreeArrayBufferDataFunc = if callback.is_some() {
            Some(external_buffer_finalizer)
        } else {
            None
        };
        let array_buffer = rquickjs::qjs::JS_NewArrayBuffer(
            ctx,
            data,
            len as u64,
            free_func,
            if callback.is_some() {
                release_key as *mut c_void
            } else {
                std::ptr::null_mut()
            },
            false,
        );
        if rquickjs::qjs::JS_IsException(array_buffer) {
            EXTERNAL_BUFFER_RELEASES.lock().remove(&release_key);
            return LexeV8Status::Exception;
        }
        let buf = unsafe { buffer_from_array_buffer(ctx, array_buffer, offset, length) };
        rquickjs::qjs::JS_FreeValue(ctx, array_buffer);
        if rquickjs::qjs::JS_IsException(buf) {
            EXTERNAL_BUFFER_RELEASES.lock().remove(&release_key);
            return LexeV8Status::Exception;
        }
        *out = state
            .roots
            .insert_owned(unsafe { crate::owned_js_value::OwnedJsValue::new(ctx, buf) });
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn buffer_data(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> LexeV8Status {
    if out_ptr.is_null() || out_len.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        let ctx = state.ctx_ptr();
        let view = unsafe { buffer_view_data(ctx, val) };
        let Some((ptr, len)) = view else {
            return LexeV8Status::Error;
        };
        *out_ptr = ptr;
        *out_len = len;
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn buffer_has_instance(
    _ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> LexeV8Status {
    if out.is_null() {
        return LexeV8Status::Error;
    }
    with_state(|state| {
        let ctx = state.ctx_ptr();
        let Some(val) = value_from_root(state, root) else {
            return LexeV8Status::Error;
        };
        *out = buffer_is_buffer(ctx, val);
        LexeV8Status::Ok
    })
}

pub unsafe extern "C" fn add_env_cleanup_hook(
    isolate: *mut crate::bridge::LexeV8IsolateState,
    cb: Option<unsafe extern "C" fn(*mut c_void)>,
    arg: *mut c_void,
) {
    if isolate.is_null() {
        return;
    }
    if let Some(cb) = cb {
        extern "C" {
            fn lexe_v8_current_context() -> *mut crate::bridge::LexeV8ContextState;
        }
        let ctx = unsafe { lexe_v8_current_context() };
        let scope_ptr = if ctx.is_null() {
            isolate as usize
        } else {
            ctx as usize
        };
        crate::runtime_state::add_cleanup_hook(scope_ptr, cb, arg);
        if ctx.is_null() {
            crate::runtime_state::isolate_key(isolate as usize);
        } else {
            crate::runtime_state::context_key(ctx as usize);
        }
    }
}

pub unsafe extern "C" fn remove_env_cleanup_hook(
    isolate: *mut crate::bridge::LexeV8IsolateState,
    cb: Option<unsafe extern "C" fn(*mut c_void)>,
    arg: *mut c_void,
) {
    if isolate.is_null() {
        return;
    }
    if let Some(cb) = cb {
        extern "C" {
            fn lexe_v8_current_context() -> *mut crate::bridge::LexeV8ContextState;
        }
        let ctx = unsafe { lexe_v8_current_context() };
        let scope_ptr = if ctx.is_null() {
            isolate as usize
        } else {
            ctx as usize
        };
        crate::runtime_state::remove_cleanup_hook(scope_ptr, cb, arg);
    }
}
