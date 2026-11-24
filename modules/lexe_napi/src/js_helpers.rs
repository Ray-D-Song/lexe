// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::qjs::{self, JSContext, JSValue};

#[inline]
pub fn new_int32(val: i32) -> JSValue {
    if (-(1 << 30)..=(1 << 30)).contains(&val) {
        qjs::JS_MKVAL(qjs::JS_TAG_INT, val)
    } else {
        qjs::JS_NewFloat64(val as f64)
    }
}

#[inline]
pub fn new_uint32(val: u32) -> JSValue {
    if val <= (1 << 30) {
        qjs::JS_MKVAL(qjs::JS_TAG_INT, val as i32)
    } else {
        qjs::JS_NewFloat64(val as f64)
    }
}

#[inline]
pub fn new_int64(val: i64) -> JSValue {
    if (-(1 << 30)..=(1 << 30)).contains(&val) {
        qjs::JS_MKVAL(qjs::JS_TAG_INT, val as i32)
    } else {
        qjs::JS_NewFloat64(val as f64)
    }
}

#[inline]
pub fn new_float64(val: f64) -> JSValue {
    qjs::JS_NewFloat64(val)
}

pub unsafe fn to_uint32(ctx: *mut JSContext, val: JSValue) -> Result<u32, ()> {
    let mut out = 0i32;
    if qjs::JS_ToInt32(ctx, &mut out, val) < 0 {
        return Err(());
    }
    Ok(out as u32)
}

pub unsafe fn value_to_atom(ctx: *mut JSContext, val: JSValue) -> qjs::JSAtom {
    qjs::JS_ValueToAtom(ctx, val)
}

pub unsafe fn set_property(ctx: *mut JSContext, obj: JSValue, key: JSValue, val: JSValue) -> i32 {
    let atom = value_to_atom(ctx, key);
    let ret = qjs::JS_SetProperty(ctx, obj, atom, val);
    qjs::JS_FreeAtom(ctx, atom);
    ret
}

pub unsafe fn get_property(ctx: *mut JSContext, obj: JSValue, key: JSValue) -> JSValue {
    let atom = value_to_atom(ctx, key);
    let val = qjs::JS_GetProperty(ctx, obj, atom);
    qjs::JS_FreeAtom(ctx, atom);
    val
}

pub unsafe fn try_buffer_from(ctx: *mut JSContext, uint8array: JSValue) -> JSValue {
    let global = qjs::JS_GetGlobalObject(ctx);
    let buffer_ctor = qjs::JS_GetPropertyStr(ctx, global, c"Buffer".as_ptr());
    qjs::JS_FreeValue(ctx, global);
    if !qjs::JS_IsFunction(ctx, buffer_ctor) {
        qjs::JS_FreeValue(ctx, buffer_ctor);
        return uint8array;
    }
    let mut argv = [uint8array];
    let result = qjs::JS_Call(ctx, buffer_ctor, qjs::JS_UNDEFINED, 1, argv.as_mut_ptr());
    qjs::JS_FreeValue(ctx, buffer_ctor);
    if qjs::JS_IsException(result) {
        qjs::JS_FreeValue(ctx, result);
        return uint8array;
    }
    qjs::JS_FreeValue(ctx, uint8array);
    result
}

/// Hidden id property that can be deleted on `napi_remove_wrap` (still non-enumerable).
pub unsafe fn define_hidden_usize_configurable(
    ctx: *mut JSContext,
    obj: JSValue,
    key: *const std::os::raw::c_char,
    id: usize,
) -> bool {
    let id_val = new_int64(id as i64);
    let atom = qjs::JS_NewAtom(ctx, key);
    let ret = qjs::JS_DefinePropertyValue(ctx, obj, atom, id_val, qjs::JS_PROP_CONFIGURABLE as i32);
    qjs::JS_FreeAtom(ctx, atom);
    ret > 0
}

pub unsafe fn delete_hidden_property(
    ctx: *mut JSContext,
    obj: JSValue,
    key: *const std::os::raw::c_char,
) {
    let atom = qjs::JS_NewAtom(ctx, key);
    qjs::JS_DeleteProperty(ctx, obj, atom, 0);
    qjs::JS_FreeAtom(ctx, atom);
}

pub unsafe fn read_hidden_usize(
    ctx: *mut JSContext,
    obj: JSValue,
    key: *const std::os::raw::c_char,
) -> Option<usize> {
    let id_val = qjs::JS_GetPropertyStr(ctx, obj, key);
    if qjs::JS_IsException(id_val) || qjs::JS_IsUndefined(id_val) {
        qjs::JS_FreeValue(ctx, id_val);
        return None;
    }
    let mut id = 0i64;
    if qjs::JS_ToInt64(ctx, &mut id, id_val) < 0 {
        qjs::JS_FreeValue(ctx, id_val);
        return None;
    }
    qjs::JS_FreeValue(ctx, id_val);
    Some(id as usize)
}

pub fn napi_to_js_typedarray_type(
    type_: crate::types::napi_typedarray_type,
) -> qjs::JSTypedArrayEnum {
    use crate::types::napi_typedarray_type::*;
    match type_ {
        napi_int8_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_INT8,
        napi_uint8_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_UINT8,
        napi_uint8_clamped_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_UINT8C,
        napi_int16_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_INT16,
        napi_uint16_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_UINT16,
        napi_int32_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_INT32,
        napi_uint32_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_UINT32,
        napi_float32_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_FLOAT32,
        napi_float64_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_FLOAT64,
        napi_bigint64_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_BIG_INT64,
        napi_biguint64_array => qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_BIG_UINT64,
    }
}
