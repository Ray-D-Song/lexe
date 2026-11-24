// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::os::raw::c_void;

use rquickjs::qjs::{self, JSContext, JSValue};

use crate::gc_hook::{self, GcEntryKind};
use crate::js_helpers::read_hidden_usize;
use crate::types::{napi_finalize, napi_status};

pub struct WrapEntry {
    pub native: *mut c_void,
    pub finalize: napi_finalize,
    pub finalize_hint: *mut c_void,
}

pub struct WrapTable {
    by_id: HashMap<usize, WrapEntry>,
}

impl WrapTable {
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set(
        &mut self,
        ctx: *mut JSContext,
        obj: JSValue,
        native: *mut c_void,
        finalize: napi_finalize,
        hint: *mut c_void,
        env: crate::types::napi_env,
        weak_ref: Option<crate::types::napi_ref>,
    ) -> Option<usize> {
        let id =
            gc_hook::register_gc_entry(GcEntryKind::Wrap, native, finalize, hint, env, weak_ref);
        if !unsafe {
            crate::js_helpers::define_hidden_usize_configurable(
                ctx,
                obj,
                c"__napi_wrap_id".as_ptr(),
                id,
            )
        } {
            gc_hook::remove_gc_entry(id);
            return None;
        }
        if !gc_hook::attach_holder(ctx, obj, id) {
            unsafe {
                crate::js_helpers::delete_hidden_property(ctx, obj, c"__napi_wrap_id".as_ptr());
            }
            gc_hook::remove_gc_entry(id);
            return None;
        }
        self.by_id.insert(
            id,
            WrapEntry {
                native,
                finalize,
                finalize_hint: hint,
            },
        );
        Some(id)
    }

    pub fn get(&self, ctx: *mut JSContext, obj: JSValue) -> Option<*mut c_void> {
        let id = unsafe { read_hidden_usize(ctx, obj, c"__napi_wrap_id".as_ptr())? };
        self.by_id.get(&id).map(|e| e.native)
    }

    pub fn remove(&mut self, ctx: *mut JSContext, obj: JSValue) -> Option<WrapEntry> {
        let id = unsafe { read_hidden_usize(ctx, obj, c"__napi_wrap_id".as_ptr())? };
        gc_hook::detach_holder(ctx, obj, id);
        unsafe {
            crate::js_helpers::delete_hidden_property(ctx, obj, c"__napi_wrap_id".as_ptr());
        }
        self.by_id.remove(&id)
    }

    pub fn remove_by_id(&mut self, id: usize) -> Option<WrapEntry> {
        self.by_id.remove(&id)
    }

    pub fn clear(&mut self) {
        self.by_id.clear();
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_wrap(
    env: crate::types::napi_env,
    js_object: crate::types::napi_value,
    native_object: *mut c_void,
    finalize_cb: napi_finalize,
    finalize_hint: *mut c_void,
    result: *mut crate::types::napi_ref,
) -> napi_status {
    if env.is_null() || js_object.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env = unsafe { crate::env::Env::from_napi_env(env) };
    let ctx = env.ctx_ptr();
    let obj = match unsafe { crate::value::napi_to_value(env, js_object) } {
        Some(v) => v,
        None => return napi_status::napi_invalid_arg,
    };
    if !unsafe { qjs::JS_IsObject(obj) } {
        return napi_status::napi_object_expected;
    }
    if env.wraps.get(ctx, obj).is_some() {
        return napi_status::napi_invalid_arg;
    }
    let mut weak_ref = std::ptr::null_mut();
    if !result.is_null() {
        let weak_ref_val = env.refs.create(ctx, obj, 0, env.as_napi_env(), false);
        weak_ref = weak_ref_val;
    }
    let weak_opt = if weak_ref.is_null() {
        None
    } else {
        Some(weak_ref)
    };
    if env
        .wraps
        .set(
            ctx,
            obj,
            native_object,
            finalize_cb,
            finalize_hint,
            env.as_napi_env(),
            weak_opt,
        )
        .is_none()
    {
        if !weak_ref.is_null() {
            let _ = unsafe { crate::refs::napi_delete_reference(env.as_napi_env(), weak_ref) };
        }
        return napi_status::napi_generic_failure;
    }
    if !result.is_null() {
        unsafe {
            *result = weak_ref;
        }
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_unwrap(
    env: crate::types::napi_env,
    js_object: crate::types::napi_value,
    result: *mut *mut c_void,
) -> napi_status {
    if env.is_null() || js_object.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env = unsafe { crate::env::Env::from_napi_env(env) };
    let ctx = env.ctx_ptr();
    let obj = match unsafe { crate::value::napi_to_value(env, js_object) } {
        Some(v) => v,
        None => return napi_status::napi_invalid_arg,
    };
    match env.wraps.get(ctx, obj) {
        Some(native) => {
            unsafe {
                *result = native;
            }
            napi_status::napi_ok
        },
        None => napi_status::napi_generic_failure,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_wrap(
    env: crate::types::napi_env,
    js_object: crate::types::napi_value,
    result: *mut *mut c_void,
) -> napi_status {
    if env.is_null() || js_object.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env = unsafe { crate::env::Env::from_napi_env(env) };
    let ctx = env.ctx_ptr();
    let obj = match unsafe { crate::value::napi_to_value(env, js_object) } {
        Some(v) => v,
        None => return napi_status::napi_invalid_arg,
    };
    match env.wraps.remove(ctx, obj) {
        Some(entry) => {
            unsafe {
                *result = entry.native;
            }
            napi_status::napi_ok
        },
        None => napi_status::napi_generic_failure,
    }
}
