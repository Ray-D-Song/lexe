//! Additional V8 value / buffer helpers exported as C symbols.

use std::os::raw::c_void;

use crate::value_ops::*;

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_layout_kind(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out_kind: *mut u8,
    out_smi: *mut i32,
) -> crate::bridge::LexeV8Status {
    value_layout_kind(ctx, root, out_kind, out_smi)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_is_object(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_is_object(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_is_array(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_is_array(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_is_function(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_is_function(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_is_number(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_is_number(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_is_int32(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_is_int32(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_is_bigint(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_is_bigint(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_is_boolean(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_is_boolean(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_to_boolean(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_to_boolean(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_strict_equals(
    ctx: *mut crate::bridge::LexeV8ContextState,
    a: u64,
    b: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_strict_equals(ctx, a, b, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_to_float64(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut f64,
) -> crate::bridge::LexeV8Status {
    value_to_float64(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_to_int32(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut i32,
) -> crate::bridge::LexeV8Status {
    value_to_int32(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_value_to_int64(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut i64,
    lossless: *mut bool,
) -> crate::bridge::LexeV8Status {
    value_to_int64(ctx, root, out, lossless)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_array_length(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut u32,
) -> crate::bridge::LexeV8Status {
    array_length(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_function_new_instance(
    ctx: *mut crate::bridge::LexeV8ContextState,
    func_root: u64,
    argc: i32,
    args: *const u64,
    out: *mut u64,
) -> crate::bridge::LexeV8Status {
    function_new_instance(ctx, func_root, argc, args, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_buffer_new_copy(
    ctx: *mut crate::bridge::LexeV8ContextState,
    data: *const u8,
    len: usize,
    out: *mut u64,
) -> crate::bridge::LexeV8Status {
    buffer_new_copy(ctx, data, len, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_buffer_new_external(
    ctx: *mut crate::bridge::LexeV8ContextState,
    data: *mut u8,
    len: usize,
    callback: Option<unsafe extern "C" fn(*mut u8, *mut c_void)>,
    hint: *mut c_void,
    out: *mut u64,
) -> crate::bridge::LexeV8Status {
    buffer_new_external(ctx, data, len, callback, hint, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_buffer_data(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> crate::bridge::LexeV8Status {
    buffer_data(ctx, root, out_ptr, out_len)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_internal_field_get(
    ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    index: i32,
    out: *mut *mut std::ffi::c_void,
) -> crate::bridge::LexeV8Status {
    crate::js_ops::internal_field_get(ctx, object_root, index, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_internal_field_set(
    ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    index: i32,
    ptr: *mut std::ffi::c_void,
) -> crate::bridge::LexeV8Status {
    crate::js_ops::internal_field_set(ctx, object_root, index, ptr)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_object_internal_field_count(
    ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    out: *mut i32,
) -> crate::bridge::LexeV8Status {
    crate::js_ops::object_internal_field_count(ctx, object_root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_object_reserve_internal_fields(
    ctx: *mut crate::bridge::LexeV8ContextState,
    object_root: u64,
    count: i32,
) -> crate::bridge::LexeV8Status {
    crate::js_ops::object_reserve_internal_fields(ctx, object_root, count)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_root_id_for_js_object(
    ctx: *mut crate::bridge::LexeV8ContextState,
    object_ptr: *mut std::ffi::c_void,
    out: *mut u64,
) -> crate::bridge::LexeV8Status {
    crate::js_ops::root_id_for_js_object(ctx, object_ptr, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_object_ptr_for_root(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut *mut std::ffi::c_void,
) -> crate::bridge::LexeV8Status {
    crate::js_ops::object_ptr_for_root(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_root_restrong_from_object_ptr(
    ctx: *mut crate::bridge::LexeV8ContextState,
    object_ptr: *mut std::ffi::c_void,
    out: *mut u64,
) -> crate::bridge::LexeV8Status {
    crate::js_ops::root_restrong_from_object_ptr(ctx, object_ptr, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_unregister_weak_for_object_ptr(
    ctx: *mut crate::bridge::LexeV8ContextState,
    object_ptr: *mut std::ffi::c_void,
) {
    crate::js_ops::unregister_weak_for_object_ptr(ctx, object_ptr);
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_buffer_has_instance(
    ctx: *mut crate::bridge::LexeV8ContextState,
    root: u64,
    out: *mut bool,
) -> crate::bridge::LexeV8Status {
    buffer_has_instance(ctx, root, out)
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_add_env_cleanup_hook(
    isolate: *mut crate::bridge::LexeV8IsolateState,
    cb: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    arg: *mut std::ffi::c_void,
) {
    add_env_cleanup_hook(isolate, cb, arg);
}

#[no_mangle]
pub unsafe extern "C" fn lexe_v8_remove_env_cleanup_hook(
    isolate: *mut crate::bridge::LexeV8IsolateState,
    cb: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    arg: *mut std::ffi::c_void,
) {
    remove_env_cleanup_hook(isolate, cb, arg);
}
