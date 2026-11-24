// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::ptr::NonNull;

use rquickjs::function::{MutFn, Rest, This};
use rquickjs::qjs::{self, JSValue};
use rquickjs::{Ctx, Function, Result as JsResult, Value};

use crate::env::Env;
use crate::external::{create_external_object, get_external_pointer, is_external_object};
use crate::js_helpers::{
    napi_to_js_typedarray_type, new_float64, new_int32, new_int64, new_uint32, to_uint32,
    try_buffer_from,
};
use crate::types::*;
use crate::value::{
    bytes_from_js, cstr_from_js, napi_value_for_slot, string_from_bytes, string_from_cstr,
    value_to_napi_borrowed, value_to_napi_owned,
};

thread_local! {
    static PENDING_MODULE: RefCell<Option<napi_module>> = const { RefCell::new(None) };
}

/// Stable C string for `napi_node_version.release` (must outlive the process).
static NODE_RELEASE: &[u8] = b"node\0";

#[repr(transparent)]
struct StaticNodeVersion(napi_node_version);
// SAFETY: only contains POD + a pointer into static `NODE_RELEASE`.
unsafe impl Sync for StaticNodeVersion {}

static NODE_VERSION: StaticNodeVersion = StaticNodeVersion(napi_node_version {
    major: 24,
    minor: 3,
    patch: 0,
    release: NODE_RELEASE.as_ptr().cast(),
});

fn with_env<F, R>(env: napi_env, f: F) -> R
where
    F: FnOnce(&mut Env) -> R,
{
    let env_ref = unsafe { Env::from_napi_env(env) };
    drain_driver_jobs(env_ref);
    if crate::gc_hook::has_pending_finalizers() {
        crate::gc_hook::drain_pending_finalizers(env_ref);
    }
    let result = f(env_ref);
    drain_driver_jobs(env_ref);
    if crate::gc_hook::has_pending_finalizers() {
        crate::gc_hook::drain_pending_finalizers(env_ref);
    }
    result
}

fn drain_driver_jobs(env: &mut Env) {
    if let Some(driver) = env.driver.clone() {
        if driver.should_ensure_loop() {
            driver.ensure_loop(env);
        }
        driver.drain_ready_jobs(env);
    }
    if crate::async_work::has_pending_tsfn() {
        crate::async_work::drain_threadsafe_functions(env.as_napi_env());
    }
}

/// Called from the event loop (e.g. timer poll) while the runtime lock is held.
pub fn poll_pending_drivers(rt: *mut rquickjs::qjs::JSRuntime) {
    let env_ptrs: Vec<*mut Env> = crate::dlopen::env_ptrs_for_runtime(rt);
    for ptr in env_ptrs {
        let env = unsafe { &mut *ptr };
        drain_driver_jobs(env);
    }
}

/// Whether this runtime still has driver work (async work, referenced TSFNs,
/// queued jobs). Used by the process drain loop, which cannot see these tasks
/// through the event-loop reference count.
pub fn drivers_busy(rt: *mut rquickjs::qjs::JSRuntime) -> bool {
    crate::async_work::has_refed_tsfn()
        || crate::async_work::has_pending_async_work()
        || crate::dlopen::env_ptrs_for_runtime(rt)
            .into_iter()
            .any(|ptr| {
                let env = unsafe { &*ptr };
                env.driver
                    .as_ref()
                    .is_some_and(|driver| driver.should_keep_loop_alive())
            })
}

// --- Module registration ---

#[no_mangle]
pub unsafe extern "C" fn napi_module_register(mod_: *const napi_module) {
    if mod_.is_null() {
        return;
    }
    PENDING_MODULE.with(|p| {
        *p.borrow_mut() = Some(unsafe { *mod_ });
    });
}

pub fn take_pending_module() -> Option<napi_module> {
    PENDING_MODULE.with(|p| p.borrow_mut().take())
}

// --- Version / env info ---

#[no_mangle]
pub unsafe extern "C" fn napi_get_version(env: napi_env, result: *mut u32) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    unsafe {
        *result = crate::NAPI_VERSION;
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_node_version(
    env: napi_env,
    version: *mut *const napi_node_version,
) -> napi_status {
    if env.is_null() || version.is_null() {
        return napi_status::napi_invalid_arg;
    }
    unsafe {
        *version = &NODE_VERSION.0;
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_last_error_info(
    env: napi_env,
    result: *mut *const napi_extended_error_info,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| unsafe {
        *result = e.last_error_info_ptr();
    });
    napi_status::napi_ok
}

// --- Handle scopes ---

#[no_mangle]
pub unsafe extern "C" fn napi_open_handle_scope(
    env: napi_env,
    result: *mut napi_handle_scope,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        e.scopes.open();
        unsafe {
            *result = e.scopes.depth() as napi_handle_scope;
        }
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_close_handle_scope(
    env: napi_env,
    scope: napi_handle_scope,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        if e.scopes.depth() == 0 {
            return napi_status::napi_handle_scope_mismatch;
        }
        let ctx = e.ctx_ptr();
        if !e.scopes.close_handle(ctx) {
            return napi_status::napi_handle_scope_mismatch;
        }
        let _ = scope;
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_open_escapable_handle_scope(
    env: napi_env,
    result: *mut napi_escapable_handle_scope,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        e.scopes.open_escapable();
        unsafe {
            *result = e.scopes.escapable_depth() as napi_escapable_handle_scope;
        }
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_close_escapable_handle_scope(
    env: napi_env,
    scope: napi_escapable_handle_scope,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        if e.scopes.escapable_depth() == 0 {
            return napi_status::napi_handle_scope_mismatch;
        }
        let ctx = e.ctx_ptr();
        if !e.scopes.close_escapable(ctx) {
            return napi_status::napi_handle_scope_mismatch;
        }
        let _ = scope;
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_escape_handle(
    env: napi_env,
    scope: napi_escapable_handle_scope,
    escapee: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || escapee.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        if e.scopes.escapable_depth() == 0 {
            return napi_status::napi_handle_scope_mismatch;
        }
        if e.scopes.escapable_already_escaped() {
            return napi_status::napi_escape_called_twice;
        }
        let nv = unsafe { &*(escapee as *const crate::value::NapiValue) };
        let js_val = match e.scopes.resolve_value(nv.slot) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let duped = unsafe { qjs::JS_DupValue(ctx, js_val) };
        let escape_slot = match e.scopes.escape_into_slot(duped) {
            Some(slot) => slot,
            None => {
                unsafe {
                    qjs::JS_FreeValue(ctx, duped);
                }
                return napi_status::napi_invalid_arg;
            },
        };
        unsafe {
            *result = napi_value_for_slot(e, escape_slot);
        }
        let _ = scope;
        napi_status::napi_ok
    })
}

// --- Exceptions ---

#[no_mangle]
pub unsafe extern "C" fn napi_throw(env: napi_env, error: napi_value) -> napi_status {
    if env.is_null() || error.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value_dup(e, error) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        e.set_pending_exception(val);
        napi_status::napi_ok
    })
}

/// Throw via the matching global constructor (Error / TypeError / RangeError).
fn throw_error_kind(
    e: &mut Env,
    code: *const c_char,
    msg: *const c_char,
    ctor_name: &str,
) -> napi_status {
    let ctx = e.ctx_ptr();
    let message = if msg.is_null() {
        "Error"
    } else {
        unsafe { CStr::from_ptr(msg) }.to_str().unwrap_or("Error")
    };

    let global = unsafe { qjs::JS_GetGlobalObject(ctx) };
    let ctor_c = CString::new(ctor_name).unwrap();
    let ctor = unsafe { qjs::JS_GetPropertyStr(ctx, global, ctor_c.as_ptr()) };
    unsafe { qjs::JS_FreeValue(ctx, global) };
    if unsafe { qjs::JS_IsException(ctor) } {
        return e.status_from_throw();
    }

    let msg_c = CString::new(message).unwrap();
    let msg_val = unsafe { qjs::JS_NewStringLen(ctx, msg_c.as_ptr(), message.len() as u64) };
    if unsafe { qjs::JS_IsException(msg_val) } {
        unsafe { qjs::JS_FreeValue(ctx, ctor) };
        return e.status_from_throw();
    }

    let mut argv = [msg_val];
    let err_obj = unsafe { qjs::JS_CallConstructor(ctx, ctor, 1, argv.as_mut_ptr()) };
    // CallConstructor consumes argv ownership on success paths; free temps.
    unsafe {
        qjs::JS_FreeValue(ctx, ctor);
        // msg_val may have been consumed by constructor; free only if still valid
        // — QuickJS CallConstructor dups args, so free our msg_val.
        qjs::JS_FreeValue(ctx, msg_val);
    }
    if unsafe { qjs::JS_IsException(err_obj) } {
        return e.status_from_throw();
    }

    if !code.is_null() {
        let code_str = unsafe { CStr::from_ptr(code) }.to_string_lossy();
        let code_c = CString::new(code_str.as_ref()).unwrap();
        let code_val = unsafe { qjs::JS_NewStringLen(ctx, code_c.as_ptr(), code_str.len() as u64) };
        if unsafe { qjs::JS_IsException(code_val) } {
            unsafe { qjs::JS_FreeValue(ctx, err_obj) };
            return e.status_from_throw();
        }
        if unsafe { qjs::JS_SetPropertyStr(ctx, err_obj, c"code".as_ptr(), code_val) } < 0 {
            unsafe { qjs::JS_FreeValue(ctx, err_obj) };
            return e.status_from_throw();
        }
    }

    e.set_pending_exception(err_obj);
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw_error(
    env: napi_env,
    code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| throw_error_kind(e, code, msg, "Error"))
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw_type_error(
    env: napi_env,
    code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| throw_error_kind(e, code, msg, "TypeError"))
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw_range_error(
    env: napi_env,
    code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| throw_error_kind(e, code, msg, "RangeError"))
}

/// Create an Error object without throwing (napi_create_error family).
///
/// Constructs via the global `Error` / `TypeError` / `RangeError` constructor so
/// the instance has the correct prototype and `instanceof` behavior.
fn create_error_object(
    e: &mut Env,
    code: napi_value,
    msg: napi_value,
    result: *mut napi_value,
    name: &str,
) -> napi_status {
    let ctx = e.ctx_ptr();

    // message is required and must be a string.
    if msg.is_null() {
        e.set_last_error(napi_status::napi_invalid_arg, Some("msg must be a string"));
        return napi_status::napi_invalid_arg;
    }
    let msg_js = match unsafe { crate::value::napi_to_value(e, msg) } {
        Some(v) => v,
        None => {
            e.set_last_error(napi_status::napi_invalid_arg, Some("invalid msg handle"));
            return napi_status::napi_invalid_arg;
        },
    };
    if !unsafe { qjs::JS_IsString(msg_js) } {
        e.set_last_error(
            napi_status::napi_string_expected,
            Some("msg must be a string"),
        );
        return napi_status::napi_string_expected;
    }

    // code: null = omit; non-null non-string = invalid; non-null string = set.
    let code_js = if code.is_null() {
        None
    } else {
        match unsafe { crate::value::napi_to_value(e, code) } {
            Some(v) if unsafe { qjs::JS_IsString(v) } => Some(v),
            Some(_) => {
                e.set_last_error(
                    napi_status::napi_string_expected,
                    Some("code must be a string"),
                );
                return napi_status::napi_string_expected;
            },
            None => {
                e.set_last_error(napi_status::napi_invalid_arg, Some("invalid code handle"));
                return napi_status::napi_invalid_arg;
            },
        }
    };

    // Resolve global constructor: Error | TypeError | RangeError.
    let global = unsafe { qjs::JS_GetGlobalObject(ctx) };
    if unsafe { qjs::JS_IsException(global) } {
        return e.status_from_throw();
    }
    let name_c = match CString::new(name) {
        Ok(s) => s,
        Err(_) => {
            unsafe { qjs::JS_FreeValue(ctx, global) };
            e.set_last_error(napi_status::napi_invalid_arg, Some("invalid error name"));
            return napi_status::napi_invalid_arg;
        },
    };
    let ctor = unsafe { qjs::JS_GetPropertyStr(ctx, global, name_c.as_ptr()) };
    unsafe { qjs::JS_FreeValue(ctx, global) };
    if unsafe { qjs::JS_IsException(ctor) } {
        return e.status_from_throw();
    }

    // Construct with the message string (argv is borrowed; CallConstructor does not take ownership).
    let mut argv = [msg_js];
    let err_obj = unsafe { qjs::JS_CallConstructor(ctx, ctor, 1, argv.as_mut_ptr()) };
    unsafe { qjs::JS_FreeValue(ctx, ctor) };
    if unsafe { qjs::JS_IsException(err_obj) } {
        return e.status_from_throw();
    }

    if let Some(code_val) = code_js {
        let code_dup = unsafe { qjs::JS_DupValue(ctx, code_val) };
        let set_rc = unsafe { qjs::JS_SetPropertyStr(ctx, err_obj, c"code".as_ptr(), code_dup) };
        if set_rc < 0 {
            unsafe { qjs::JS_FreeValue(ctx, err_obj) };
            return e.status_from_throw();
        }
    }

    unsafe {
        *result = value_to_napi_owned(e, err_obj);
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_error(
    env: napi_env,
    code: napi_value,
    msg: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| create_error_object(e, code, msg, result, "Error"))
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_type_error(
    env: napi_env,
    code: napi_value,
    msg: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        create_error_object(e, code, msg, result, "TypeError")
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_range_error(
    env: napi_env,
    code: napi_value,
    msg: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        create_error_object(e, code, msg, result, "RangeError")
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_exception_pending(
    env: napi_env,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| unsafe {
        *result = qjs::JS_HasException(e.ctx_ptr());
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_and_clear_last_exception(
    env: napi_env,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let exc = unsafe { qjs::JS_GetException(ctx) };
        unsafe {
            *result = value_to_napi_owned(e, exc);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_fatal_error(
    location: *const c_char,
    location_len: usize,
    message: *const c_char,
    message_len: usize,
) {
    let _loc = if location.is_null() {
        "unknown"
    } else {
        unsafe {
            let _ = std::slice::from_raw_parts(location as *const u8, location_len.min(256));
        }
        "napi"
    };
    let _msg = if message.is_null() {
        "fatal napi error"
    } else {
        unsafe {
            let _ = std::slice::from_raw_parts(message as *const u8, message_len.min(256));
        }
        "fatal napi error"
    };
    let _ = _loc;
    std::process::abort();
}

// --- Primitives ---

#[no_mangle]
pub unsafe extern "C" fn napi_get_undefined(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        *result = value_to_napi_owned(e, qjs::JS_UNDEFINED);
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_null(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        *result = value_to_napi_owned(e, qjs::JS_NULL);
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_global(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let global = unsafe { qjs::JS_GetGlobalObject(ctx) };
        unsafe {
            *result = value_to_napi_owned(e, global);
        }
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_boolean(
    env: napi_env,
    value: bool,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = if value { qjs::JS_TRUE } else { qjs::JS_FALSE };
        *result = value_to_napi_owned(e, val);
    });
    napi_status::napi_ok
}

// --- Numbers ---

#[no_mangle]
pub unsafe extern "C" fn napi_create_double(
    env: napi_env,
    value: f64,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = new_float64(value);
        *result = value_to_napi_owned(e, val);
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_int32(
    env: napi_env,
    value: i32,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = new_int32(value);
        *result = value_to_napi_owned(e, val);
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_uint32(
    env: napi_env,
    value: u32,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = new_uint32(value);
        *result = value_to_napi_owned(e, val);
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_int64(
    env: napi_env,
    value: i64,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = new_int64(value);
        *result = value_to_napi_owned(e, val);
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_double(
    env: napi_env,
    value: napi_value,
    result: *mut f64,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let mut out = 0.0f64;
        if unsafe { qjs::JS_ToFloat64(ctx, &mut out as *mut _, val) } < 0 {
            return e.status_from_throw();
        }
        unsafe {
            *result = out;
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int32(
    env: napi_env,
    value: napi_value,
    result: *mut i32,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let mut out = 0i32;
        if unsafe { qjs::JS_ToInt32(ctx, &mut out as *mut _, val) } < 0 {
            return e.status_from_throw();
        }
        unsafe {
            *result = out;
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_uint32(
    env: napi_env,
    value: napi_value,
    result: *mut u32,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let out = match unsafe { to_uint32(ctx, val) } {
            Ok(v) => v,
            Err(()) => return e.status_from_throw(),
        };
        unsafe {
            *result = out;
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int64(
    env: napi_env,
    value: napi_value,
    result: *mut i64,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let mut out = 0i64;
        if unsafe { qjs::JS_ToInt64(ctx, &mut out as *mut _, val) } < 0 {
            return e.status_from_throw();
        }
        unsafe {
            *result = out;
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bool(
    env: napi_env,
    value: napi_value,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        unsafe {
            *result = qjs::JS_ToBool(e.ctx_ptr(), val) != 0;
        }
        napi_status::napi_ok
    })
}

// --- Coerce (ToBoolean / ToNumber / ToObject / ToString) ---

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_bool(
    env: napi_env,
    value: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let b = unsafe { qjs::JS_ToBool(ctx, val) };
        if b < 0 {
            return e.status_from_throw();
        }
        let out = if b != 0 { qjs::JS_TRUE } else { qjs::JS_FALSE };
        unsafe {
            *result = value_to_napi_owned(e, out);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_number(
    env: napi_env,
    value: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let mut num: f64 = 0.0;
        if unsafe { qjs::JS_ToFloat64(ctx, &mut num, val) } < 0 {
            return e.status_from_throw();
        }
        let out = qjs::JS_NewFloat64(num);
        unsafe {
            *result = value_to_napi_owned(e, out);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_object(
    env: napi_env,
    value: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        // Node: ToObject — null/undefined throw TypeError (napi_pending_exception).
        let out = unsafe { qjs::JS_ToObject(ctx, val) };
        if unsafe { qjs::JS_IsException(out) } {
            return e.status_from_throw();
        }
        unsafe {
            *result = value_to_napi_owned(e, out);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_string(
    env: napi_env,
    value: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let out = unsafe { qjs::JS_ToString(ctx, val) };
        if unsafe { qjs::JS_IsException(out) } {
            return e.status_from_throw();
        }
        unsafe {
            *result = value_to_napi_owned(e, out);
        }
        napi_status::napi_ok
    })
}

// --- Strings ---

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf8(
    env: napi_env,
    str_: *const c_char,
    length: usize,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| match string_from_cstr(e, str_, length) {
        Ok(v) => {
            unsafe {
                *result = v;
            }
            napi_status::napi_ok
        },
        Err(s) => s,
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_latin1(
    env: napi_env,
    str_: *const c_char,
    length: usize,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let len = if length == NAPI_AUTO_LENGTH {
            if str_.is_null() {
                e.set_last_error(napi_status::napi_invalid_arg, None);
                return napi_status::napi_invalid_arg;
            }
            unsafe { libc::strlen(str_) }
        } else {
            length
        };
        match string_from_bytes(e, str_ as *const u8, len, true) {
            Ok(v) => {
                unsafe { *result = v };
                napi_status::napi_ok
            },
            Err(s) => s,
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf8(
    env: napi_env,
    value: napi_value,
    buf: *mut c_char,
    bufsize: usize,
    result: *mut usize,
) -> napi_status {
    if env.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        cstr_from_js(e.ctx_ptr(), val, buf, bufsize, result)
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_latin1(
    env: napi_env,
    value: napi_value,
    buf: *mut c_char,
    bufsize: usize,
    result: *mut usize,
) -> napi_status {
    if env.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        bytes_from_js(e.ctx_ptr(), val, buf, bufsize, result, true)
    })
}

// --- typeof ---

#[no_mangle]
pub unsafe extern "C" fn napi_typeof(
    env: napi_env,
    value: napi_value,
    result: *mut napi_valuetype,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let ty = unsafe {
            if is_external_object(ctx, val) {
                napi_valuetype::napi_external
            } else if qjs::JS_IsSymbol(val) {
                napi_valuetype::napi_symbol
            } else if qjs::JS_IsUndefined(val) {
                napi_valuetype::napi_undefined
            } else if qjs::JS_IsNull(val) {
                napi_valuetype::napi_null
            } else if qjs::JS_IsBool(val) {
                napi_valuetype::napi_boolean
            } else if qjs::JS_IsNumber(val) {
                napi_valuetype::napi_number
            } else if qjs::JS_IsBigInt(val) {
                napi_valuetype::napi_bigint
            } else if qjs::JS_IsString(val) {
                napi_valuetype::napi_string
            } else if qjs::JS_IsFunction(ctx, val) {
                napi_valuetype::napi_function
            } else if qjs::JS_IsObject(val) {
                napi_valuetype::napi_object
            } else {
                napi_valuetype::napi_undefined
            }
        };
        unsafe {
            *result = ty;
        }
        napi_status::napi_ok
    })
}

// --- Objects / arrays ---

#[no_mangle]
pub unsafe extern "C" fn napi_create_object(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = unsafe { qjs::JS_NewObject(ctx) };
        unsafe {
            *result = value_to_napi_owned(e, obj);
        }
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_array(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let arr = unsafe { qjs::JS_NewArray(ctx) };
        unsafe {
            *result = value_to_napi_owned(e, arr);
        }
    });
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_symbol(
    env: napi_env,
    description: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let sym = if description.is_null() {
            unsafe { qjs::JS_NewSymbol(ctx, ptr::null(), false) }
        } else {
            let mut buf = [0u8; 256];
            let mut len = 0usize;
            let status = unsafe {
                napi_get_value_string_utf8(
                    env,
                    description,
                    buf.as_mut_ptr() as *mut libc::c_char,
                    buf.len() - 1,
                    &mut len,
                )
            };
            if status == napi_status::napi_string_expected {
                unsafe { qjs::JS_NewSymbol(ctx, ptr::null(), false) }
            } else if status != napi_status::napi_ok {
                return status;
            } else {
                buf[len] = 0;
                unsafe { qjs::JS_NewSymbol(ctx, buf.as_ptr() as *const libc::c_char, false) }
            }
        };
        if unsafe { qjs::JS_IsException(sym) } {
            return e.status_from_throw();
        }
        unsafe {
            *result = value_to_napi_owned(e, sym);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_array_length(
    env: napi_env,
    value: napi_value,
    result: *mut u32,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let len_atom = CString::new("length").unwrap();
        let len_val = unsafe { qjs::JS_GetPropertyStr(ctx, val, len_atom.as_ptr()) };
        let len = match unsafe { to_uint32(ctx, len_val) } {
            Ok(v) => v,
            Err(()) => {
                unsafe { qjs::JS_FreeValue(ctx, len_val) };
                return e.status_from_throw();
            },
        };
        unsafe {
            qjs::JS_FreeValue(ctx, len_val);
            *result = len;
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    value: napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || key.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let key_val = match crate::value::napi_to_value_dup(e, key) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let val = match crate::value::napi_to_value_dup(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        if unsafe { crate::js_helpers::set_property(ctx, obj, key_val, val) } < 0 {
            return e.status_from_throw();
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || key.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let key_val = match crate::value::napi_to_value(e, key) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let prop = unsafe { crate::js_helpers::get_property(ctx, obj, key_val) };
        if unsafe { qjs::JS_IsException(prop) } {
            return napi_status::napi_pending_exception;
        }
        unsafe {
            *result = value_to_napi_owned(e, prop);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_named_property(
    env: napi_env,
    object: napi_value,
    utf8name: *const c_char,
    value: napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || utf8name.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let val = match crate::value::napi_to_value_dup(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        if unsafe { qjs::JS_SetPropertyStr(ctx, obj, utf8name, val) } < 0 {
            return e.status_from_throw();
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_named_property(
    env: napi_env,
    object: napi_value,
    utf8name: *const c_char,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || utf8name.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let prop = unsafe { qjs::JS_GetPropertyStr(ctx, obj, utf8name) };
        if unsafe { qjs::JS_IsException(prop) } {
            return napi_status::napi_pending_exception;
        }
        unsafe {
            *result = value_to_napi_owned(e, prop);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || object.is_null() || key.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let key_val = match crate::value::napi_to_value(e, key) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let atom = unsafe { crate::js_helpers::value_to_atom(ctx, key_val) };
        let has = unsafe { qjs::JS_HasProperty(ctx, obj, atom) };
        unsafe { qjs::JS_FreeAtom(ctx, atom) };
        unsafe {
            *result = has > 0;
        }
        if has < 0 {
            return e.status_from_throw();
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_named_property(
    env: napi_env,
    object: napi_value,
    utf8name: *const c_char,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || object.is_null() || utf8name.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let atom = unsafe { qjs::JS_NewAtom(ctx, utf8name) };
        let has = unsafe { qjs::JS_HasProperty(ctx, obj, atom) };
        unsafe { qjs::JS_FreeAtom(ctx, atom) };
        unsafe {
            *result = has > 0;
        }
        if has < 0 {
            return e.status_from_throw();
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || object.is_null() || key.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let key_val = match crate::value::napi_to_value(e, key) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let atom = unsafe { crate::js_helpers::value_to_atom(ctx, key_val) };
        let flags = qjs::JS_PROP_THROW as i32;
        let ret = unsafe { qjs::JS_DeleteProperty(ctx, obj, atom, flags) };
        unsafe { qjs::JS_FreeAtom(ctx, atom) };
        if !result.is_null() {
            unsafe {
                *result = ret > 0;
            }
        }
        if ret < 0 {
            return e.status_from_throw();
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_element(
    env: napi_env,
    object: napi_value,
    index: u32,
    value: napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let val = match crate::value::napi_to_value_dup(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        if unsafe { qjs::JS_SetPropertyUint32(ctx, obj, index, val) } < 0 {
            return e.status_from_throw();
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_element(
    env: napi_env,
    object: napi_value,
    index: u32,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let prop = unsafe { qjs::JS_GetPropertyUint32(ctx, obj, index) };
        if unsafe { qjs::JS_IsException(prop) } {
            return napi_status::napi_pending_exception;
        }
        unsafe {
            *result = value_to_napi_owned(e, prop);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_element(
    env: napi_env,
    object: napi_value,
    index: u32,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || object.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let prop = unsafe { qjs::JS_GetPropertyUint32(ctx, obj, index) };
        unsafe {
            *result = !qjs::JS_IsUndefined(prop);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_element(
    env: napi_env,
    object: napi_value,
    index: u32,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || object.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let atom = unsafe { qjs::JS_NewAtomUInt32(ctx, index) };
        let flags = qjs::JS_PROP_THROW as i32;
        let ret = unsafe { qjs::JS_DeleteProperty(ctx, obj, atom, flags) };
        unsafe { qjs::JS_FreeAtom(ctx, atom) };
        if !result.is_null() {
            unsafe {
                *result = ret > 0;
            }
        }
        if ret < 0 {
            return e.status_from_throw();
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property_names(
    env: napi_env,
    object: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let mut props_len = 0u32;
        let mut props: *mut qjs::JSPropertyEnum = ptr::null_mut();
        let ret = unsafe {
            qjs::JS_GetOwnPropertyNames(
                ctx,
                &mut props,
                &mut props_len,
                obj,
                (qjs::JS_GPN_STRING_MASK | qjs::JS_GPN_ENUM_ONLY) as i32,
            )
        };
        if ret < 0 {
            return napi_status::napi_generic_failure;
        }
        let arr = unsafe { qjs::JS_NewArray(ctx) };
        for i in 0..props_len {
            let atom = unsafe { (*props.add(i as usize)).atom };
            let name = unsafe { qjs::JS_AtomToString(ctx, atom) };
            unsafe {
                qjs::JS_SetPropertyUint32(ctx, arr, i, name);
            }
        }
        unsafe {
            qjs::JS_FreePropertyEnum(ctx, props, props_len);
            *result = value_to_napi_owned(e, arr);
        }
        napi_status::napi_ok
    })
}

// --- Functions ---

pub(crate) fn clear_function_callbacks() {}

#[repr(C)]
pub struct NapiCallbackInfo {
    pub argc: usize,
    pub argv: *mut napi_value,
    pub this_arg: napi_value,
    pub data: *mut c_void,
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_function(
    env: napi_env,
    utf8name: *const c_char,
    length: usize,
    cb: napi_callback,
    data: *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let name = if utf8name.is_null() {
            CString::new("").unwrap()
        } else {
            let len = if length == NAPI_AUTO_LENGTH {
                unsafe { libc::strlen(utf8name) }
            } else {
                length
            };
            let slice = unsafe { std::slice::from_raw_parts(utf8name as *const u8, len) };
            CString::new(slice).unwrap_or_default()
        };
        // Temporary Ctx for Function construction only. Capture raw ctx pointer
        // (not Ctx) in the MutFn — permanent Ctx would DupContext until GC and
        // leave residual JS_CONTEXT at FreeRuntime.
        let ctx_js = unsafe { Ctx::from_raw(NonNull::new_unchecked(ctx)) };
        let env_ptr = env as *mut Env;
        let ctx_ptr = ctx;
        let user_callback = cb;
        let user_data = data;
        let func_result = Function::new(
            ctx_js,
            MutFn::new(
                move |this: This<Value<'_>>, args: Rest<Value<'_>>| -> JsResult<Value<'_>> {
                    // Ephemeral DupContext for this call only; dropped before return.
                    let ctx = unsafe { Ctx::from_raw(NonNull::new_unchecked(ctx_ptr)) };
                    let env = unsafe { &mut *env_ptr };
                    let env_box = env.as_napi_env();
                    env.scopes.open();

                    let argc = args.len();
                    let mut napi_args: Vec<napi_value> = Vec::with_capacity(argc);
                    for arg in args.iter() {
                        napi_args.push(value_to_napi_borrowed(env, arg.as_raw()));
                    }
                    let this_arg = value_to_napi_borrowed(env, this.0.as_raw());

                    let info = Box::into_raw(Box::new(NapiCallbackInfo {
                        argc,
                        argv: napi_args.as_mut_ptr(),
                        this_arg,
                        data: user_data,
                    }));

                    let result = if let Some(f) = user_callback {
                        f(env_box, info as napi_callback_info)
                    } else {
                        ptr::null_mut()
                    };
                    let _ = unsafe { Box::from_raw(info) };

                    if let Some(driver) = env.driver.clone() {
                        driver.drain_ready_jobs(env);
                    }
                    if crate::async_work::has_pending_tsfn() {
                        crate::async_work::drain_threadsafe_functions(env_box);
                    }
                    if crate::gc_hook::has_pending_finalizers() {
                        crate::gc_hook::drain_pending_finalizers(env);
                    }

                    let return_value = if result.is_null() {
                        None
                    } else {
                        unsafe { crate::value::napi_to_value_dup(env, result) }
                    };
                    env.scopes.close(env.ctx_ptr());

                    if let Some(v) = return_value {
                        unsafe { Ok(Value::from_raw(ctx, v)) }
                    } else {
                        Ok(Value::new_undefined(ctx))
                    }
                },
            ),
        );
        match func_result {
            Ok(func) => {
                if !name.as_bytes().is_empty() {
                    let _ = func.set_name(name.to_str().unwrap_or(""));
                }
                unsafe {
                    *result = value_to_napi_borrowed(e, func.as_raw());
                }
                napi_status::napi_ok
            },
            Err(_) => e.status_from_throw(),
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_cb_info(
    env: napi_env,
    cbinfo: napi_callback_info,
    argc: *mut usize,
    argv: *mut napi_value,
    this_arg: *mut napi_value,
    data: *mut *mut c_void,
) -> napi_status {
    if env.is_null() || cbinfo.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let info = unsafe { &*(cbinfo as *const NapiCallbackInfo) };
    if !argc.is_null() {
        unsafe {
            *argc = info.argc;
        }
    }
    if !argv.is_null() && !info.argv.is_null() {
        unsafe {
            ptr::copy_nonoverlapping(info.argv, argv, info.argc);
        }
    }
    if !this_arg.is_null() {
        unsafe {
            *this_arg = info.this_arg;
        }
    }
    if !data.is_null() {
        unsafe {
            *data = info.data;
        }
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_function(
    env: napi_env,
    recv: napi_value,
    func: napi_value,
    argc: usize,
    argv: *const napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || func.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let fn_val = match crate::value::napi_to_value(e, func) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let this_val = if recv.is_null() {
            qjs::JS_UNDEFINED
        } else {
            match crate::value::napi_to_value(e, recv) {
                Some(v) => v,
                None => return napi_status::napi_invalid_arg,
            }
        };
        let mut js_argv: Vec<JSValue> = Vec::with_capacity(argc);
        for i in 0..argc {
            let arg = unsafe { *argv.add(i) };
            let v = match crate::value::napi_to_value_dup(e, arg) {
                Some(v) => v,
                None => return napi_status::napi_invalid_arg,
            };
            js_argv.push(v);
        }
        let ret = unsafe { qjs::JS_Call(ctx, fn_val, this_val, argc as i32, js_argv.as_mut_ptr()) };
        for v in js_argv {
            unsafe { qjs::JS_FreeValue(ctx, v) };
        }
        if unsafe { qjs::JS_IsException(ret) } {
            return napi_status::napi_pending_exception;
        }
        if !result.is_null() {
            unsafe {
                *result = value_to_napi_owned(e, ret);
            }
        } else {
            unsafe { qjs::JS_FreeValue(ctx, ret) };
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_new_instance(
    env: napi_env,
    constructor: napi_value,
    argc: usize,
    argv: *const napi_value,
    result: *mut napi_value,
) -> napi_status {
    napi_call_function(env, ptr::null_mut(), constructor, argc, argv, result)
}

// --- define_properties / define_class ---

#[no_mangle]
pub unsafe extern "C" fn napi_define_properties(
    env: napi_env,
    object: napi_value,
    property_count: usize,
    properties: *const napi_property_descriptor,
) -> napi_status {
    if env.is_null() || object.is_null() || properties.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let obj = match crate::value::napi_to_value(e, object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        for i in 0..property_count {
            let desc = unsafe { &*properties.add(i) };
            let name = if !desc.utf8name.is_null() {
                unsafe { CStr::from_ptr(desc.utf8name) }
                    .to_string_lossy()
                    .into_owned()
            } else if !desc.name.is_null() {
                return napi_status::napi_invalid_arg;
            } else {
                continue;
            };
            let name_c = CString::new(name.as_str()).unwrap();
            if let Some(method) = desc.method {
                let mut fn_result: napi_value = ptr::null_mut();
                let status = napi_create_function(
                    env,
                    name_c.as_ptr(),
                    name.len(),
                    Some(method),
                    desc.data,
                    &mut fn_result,
                );
                if status != napi_status::napi_ok {
                    return status;
                }
                let fn_val = match crate::value::napi_to_value_dup(e, fn_result) {
                    Some(v) => v,
                    None => return napi_status::napi_invalid_arg,
                };
                if unsafe { qjs::JS_SetPropertyStr(ctx, obj, name_c.as_ptr(), fn_val) } < 0 {
                    return e.status_from_throw();
                }
            } else if let Some(getter) = desc.getter {
                let mut getter_result: napi_value = ptr::null_mut();
                let status = napi_create_function(
                    env,
                    name_c.as_ptr(),
                    name.len(),
                    Some(getter),
                    desc.data,
                    &mut getter_result,
                );
                if status != napi_status::napi_ok {
                    return status;
                }
                let getter_val = match crate::value::napi_to_value_dup(e, getter_result) {
                    Some(v) => v,
                    None => return napi_status::napi_invalid_arg,
                };
                let setter_val = if let Some(setter) = desc.setter {
                    let mut setter_result: napi_value = ptr::null_mut();
                    let status = napi_create_function(
                        env,
                        name_c.as_ptr(),
                        name.len(),
                        Some(setter),
                        desc.data,
                        &mut setter_result,
                    );
                    if status != napi_status::napi_ok {
                        return status;
                    }
                    match crate::value::napi_to_value_dup(e, setter_result) {
                        Some(v) => v,
                        None => return napi_status::napi_invalid_arg,
                    }
                } else {
                    qjs::JS_UNDEFINED
                };
                let atom = unsafe { qjs::JS_NewAtom(ctx, name_c.as_ptr()) };
                let flags = (qjs::JS_PROP_ENUMERABLE | qjs::JS_PROP_CONFIGURABLE) as i32;
                let ret = unsafe {
                    qjs::JS_DefinePropertyGetSet(ctx, obj, atom, getter_val, setter_val, flags)
                };
                unsafe { qjs::JS_FreeAtom(ctx, atom) };
                if ret < 0 {
                    return e.status_from_throw();
                }
            } else if !desc.value.is_null() {
                let val = match crate::value::napi_to_value_dup(e, desc.value) {
                    Some(v) => v,
                    None => return napi_status::napi_invalid_arg,
                };
                if unsafe { qjs::JS_SetPropertyStr(ctx, obj, name_c.as_ptr(), val) } < 0 {
                    return e.status_from_throw();
                }
            }
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_define_class(
    env: napi_env,
    utf8name: *const c_char,
    length: usize,
    constructor: napi_callback,
    data: *mut c_void,
    property_count: usize,
    properties: *const napi_property_descriptor,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || utf8name.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut ctor: napi_value = ptr::null_mut();
    let status = napi_create_function(env, utf8name, length, constructor, data, &mut ctor);
    if status != napi_status::napi_ok {
        return status;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let prototype = unsafe { qjs::JS_NewObject(ctx) };
        if property_count > 0 && !properties.is_null() {
            for i in 0..property_count {
                let desc = unsafe { &*properties.add(i) };
                let is_static =
                    (desc.attributes as u32) & (napi_property_attributes::napi_static as u32) != 0;
                let name = if !desc.utf8name.is_null() {
                    unsafe { CStr::from_ptr(desc.utf8name) }
                        .to_string_lossy()
                        .into_owned()
                } else {
                    continue;
                };
                let name_c = CString::new(name.as_str()).unwrap();
                let target = if is_static {
                    match crate::value::napi_to_value_dup(e, ctor) {
                        Some(v) => v,
                        None => {
                            unsafe { qjs::JS_FreeValue(ctx, prototype) };
                            return napi_status::napi_invalid_arg;
                        },
                    }
                } else {
                    prototype
                };
                if let Some(method) = desc.method {
                    let mut fn_result: napi_value = ptr::null_mut();
                    let st = napi_create_function(
                        env,
                        name_c.as_ptr(),
                        name.len(),
                        Some(method),
                        desc.data,
                        &mut fn_result,
                    );
                    if st != napi_status::napi_ok {
                        if is_static {
                            unsafe { qjs::JS_FreeValue(ctx, target) };
                        }
                        unsafe { qjs::JS_FreeValue(ctx, prototype) };
                        return st;
                    }
                    let fn_val = match crate::value::napi_to_value_dup(e, fn_result) {
                        Some(v) => v,
                        None => {
                            if is_static {
                                unsafe { qjs::JS_FreeValue(ctx, target) };
                            }
                            unsafe { qjs::JS_FreeValue(ctx, prototype) };
                            return napi_status::napi_invalid_arg;
                        },
                    };
                    if unsafe { qjs::JS_SetPropertyStr(ctx, target, name_c.as_ptr(), fn_val) } < 0 {
                        if is_static {
                            unsafe { qjs::JS_FreeValue(ctx, target) };
                        }
                        unsafe { qjs::JS_FreeValue(ctx, prototype) };
                        return e.status_from_throw();
                    }
                } else if !desc.value.is_null() {
                    let val = match crate::value::napi_to_value_dup(e, desc.value) {
                        Some(v) => v,
                        None => {
                            if is_static {
                                unsafe { qjs::JS_FreeValue(ctx, target) };
                            }
                            unsafe { qjs::JS_FreeValue(ctx, prototype) };
                            return napi_status::napi_invalid_arg;
                        },
                    };
                    if unsafe { qjs::JS_SetPropertyStr(ctx, target, name_c.as_ptr(), val) } < 0 {
                        if is_static {
                            unsafe { qjs::JS_FreeValue(ctx, target) };
                        }
                        unsafe { qjs::JS_FreeValue(ctx, prototype) };
                        return e.status_from_throw();
                    }
                }
                if is_static {
                    unsafe { qjs::JS_FreeValue(ctx, target) };
                }
            }
        }
        let ctor_val = match crate::value::napi_to_value_dup(e, ctor) {
            Some(v) => v,
            None => {
                unsafe { qjs::JS_FreeValue(ctx, prototype) };
                return napi_status::napi_invalid_arg;
            },
        };
        if unsafe { qjs::JS_SetPropertyStr(ctx, ctor_val, c"prototype".as_ptr(), prototype) } < 0 {
            unsafe { qjs::JS_FreeValue(ctx, ctor_val) };
            return e.status_from_throw();
        }
        let ctor_dup = unsafe { qjs::JS_DupValue(ctx, ctor_val) };
        if unsafe { qjs::JS_SetPropertyStr(ctx, prototype, c"constructor".as_ptr(), ctor_dup) } < 0
        {
            unsafe { qjs::JS_FreeValue(ctx, ctor_val) };
            return e.status_from_throw();
        }
        // Mark as constructor so `class X extends NapiClass` works in QuickJS
        // (JS_IsConstructor must be true for the parent).
        unsafe {
            qjs::JS_SetConstructorBit(ctx, ctor_val, true);
        }
        unsafe { qjs::JS_FreeValue(ctx, ctor_val) };
        unsafe {
            *result = ctor;
        }
        napi_status::napi_ok
    })
}

// --- Instance data / cleanup ---

#[no_mangle]
pub unsafe extern "C" fn napi_set_instance_data(
    env: napi_env,
    data: *mut c_void,
    finalize_cb: napi_finalize,
    finalize_hint: *mut c_void,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        e.instance_data.insert(0, data);
        if finalize_cb.is_some() {
            e.instance_finalize = Some((finalize_cb, finalize_hint));
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_instance_data(
    env: napi_env,
    data: *mut *mut c_void,
) -> napi_status {
    if env.is_null() || data.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        unsafe {
            *data = e.instance_data.get(&0).copied().unwrap_or(ptr::null_mut());
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_env_cleanup_hook(
    env: napi_env,
    fun: unsafe extern "C" fn(*mut c_void),
    arg: *mut c_void,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        e.cleanup_hooks.push((fun, arg));
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_env_cleanup_hook(
    env: napi_env,
    fun: unsafe extern "C" fn(*mut c_void),
    arg: *mut c_void,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        e.cleanup_hooks
            .retain(|&(f, a)| f as usize != fun as usize || a != arg);
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_adjust_external_memory(
    env: napi_env,
    change_in_bytes: i64,
    adjusted_value: *mut i64,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        e.external_memory += change_in_bytes;
        if !adjusted_value.is_null() {
            unsafe {
                *adjusted_value = e.external_memory;
            }
        }
        napi_status::napi_ok
    })
}

// --- External / buffers / typed arrays (Tier B) ---

#[no_mangle]
pub unsafe extern "C" fn napi_create_external(
    env: napi_env,
    data: *mut c_void,
    finalize_cb: napi_finalize,
    finalize_hint: *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let obj = create_external_object(e, data, finalize_cb, finalize_hint);
        unsafe {
            *result = value_to_napi_owned(e, obj);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_external(
    env: napi_env,
    value: napi_value,
    result: *mut *mut c_void,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        unsafe {
            *result = get_external_pointer(e.ctx_ptr(), val).unwrap_or(ptr::null_mut());
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_arraybuffer(
    env: napi_env,
    byte_length: usize,
    data: *mut *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let buf = unsafe {
            let layout = std::alloc::Layout::from_size_align(byte_length.max(1), 1).unwrap();
            let ptr = std::alloc::alloc(layout);
            if !data.is_null() {
                *data = ptr as *mut c_void;
            }
            qjs::JS_NewArrayBuffer(
                ctx,
                ptr,
                byte_length as u64,
                Some(arraybuffer_free),
                layout.size() as *mut c_void,
                false,
            )
        };
        unsafe {
            *result = value_to_napi_owned(e, buf);
        }
        napi_status::napi_ok
    })
}

unsafe extern "C" fn arraybuffer_free(
    _rt: *mut qjs::JSRuntime,
    opaque: *mut c_void,
    ptr: *mut c_void,
) {
    if !ptr.is_null() {
        let size = opaque as usize;
        if size > 0 {
            let layout = std::alloc::Layout::from_size_align(size, 1)
                .unwrap_or_else(|_| std::alloc::Layout::from_size_align(1, 1).unwrap());
            unsafe {
                std::alloc::dealloc(ptr as *mut u8, layout);
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_arraybuffer_info(
    env: napi_env,
    arraybuffer: napi_value,
    data: *mut *mut c_void,
    byte_length: *mut usize,
) -> napi_status {
    if env.is_null() || arraybuffer.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, arraybuffer) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let mut len: u64 = 0;
        let ptr = unsafe { qjs::JS_GetArrayBuffer(ctx, &mut len, val) };
        if !data.is_null() {
            unsafe {
                *data = ptr as *mut c_void;
            }
        }
        if !byte_length.is_null() {
            unsafe {
                *byte_length = len as usize;
            }
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer_copy(
    env: napi_env,
    length: usize,
    data: *const c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || data.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let slice = unsafe { std::slice::from_raw_parts(data as *const u8, length) };
        let u8arr = unsafe { qjs::JS_NewUint8ArrayCopy(ctx, slice.as_ptr(), length as u64) };
        let buf = unsafe { try_buffer_from(ctx, u8arr) };
        unsafe {
            *result = value_to_napi_owned(e, buf);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_typedarray(
    env: napi_env,
    type_: napi_typedarray_type,
    length: usize,
    arraybuffer: napi_value,
    byte_offset: usize,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || arraybuffer.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let ab = match crate::value::napi_to_value_dup(e, arraybuffer) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let offset_val = new_int64(byte_offset as i64);
        let length_val = new_int64(length as i64);
        let mut argv = [ab, offset_val, length_val];
        let js_type = napi_to_js_typedarray_type(type_);
        let ta = unsafe { qjs::JS_NewTypedArray(ctx, 3, argv.as_mut_ptr(), js_type) };
        unsafe {
            qjs::JS_FreeValue(ctx, ab);
            qjs::JS_FreeValue(ctx, offset_val);
            qjs::JS_FreeValue(ctx, length_val);
        }
        if unsafe { qjs::JS_IsException(ta) } {
            return napi_status::napi_pending_exception;
        }
        unsafe {
            *result = value_to_napi_owned(e, ta);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_finalizer(
    env: napi_env,
    js_object: napi_value,
    finalize_data: *mut c_void,
    finalize_cb: napi_finalize,
    finalize_hint: *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || js_object.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let obj = match crate::value::napi_to_value(e, js_object) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        if !e.finalizers.add(
            e.ctx_ptr(),
            obj,
            finalize_data,
            finalize_cb,
            finalize_hint,
            e.as_napi_env(),
        ) {
            return napi_status::napi_generic_failure;
        }
        if !result.is_null() {
            unsafe {
                *result = js_object;
            }
        }
        napi_status::napi_ok
    })
}

// --- BigInt ---

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_int64(
    env: napi_env,
    value: i64,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let ctx = e.ctx_ptr();
        let val = unsafe { qjs::JS_NewBigInt64(ctx, value) };
        unsafe {
            *result = value_to_napi_owned(e, val);
        }
        napi_status::napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_int64(
    env: napi_env,
    value: napi_value,
    result: *mut i64,
    lossless: *mut bool,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env(env, |e| {
        let val = match crate::value::napi_to_value(e, value) {
            Some(v) => v,
            None => return napi_status::napi_invalid_arg,
        };
        let ctx = e.ctx_ptr();
        let mut out = 0i64;
        if unsafe { qjs::JS_ToBigInt64(ctx, &mut out as *mut _, val) } < 0 {
            return e.status_from_throw();
        }
        unsafe {
            *result = out;
            if !lossless.is_null() {
                *lossless = true;
            }
        }
        napi_status::napi_ok
    })
}

// --- Stub: uv event loop ---

#[no_mangle]
pub unsafe extern "C" fn napi_get_uv_event_loop(
    _env: napi_env,
    loop_: *mut *mut c_void,
) -> napi_status {
    if !loop_.is_null() {
        unsafe {
            *loop_ = ptr::null_mut();
        }
    }
    napi_status::napi_generic_failure
}

// Re-export refs and wrap functions are in their modules
