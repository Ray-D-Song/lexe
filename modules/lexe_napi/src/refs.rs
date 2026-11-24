// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use rquickjs::qjs::{self, JSContext, JSValue};

use crate::types::{napi_env, napi_ref, napi_status, napi_value};

static REF_ID: AtomicU64 = AtomicU64::new(1);

pub struct NapiRef {
    pub id: u64,
    pub value: JSValue,
    pub refcount: u32,
    pub weak: bool,
    pub dead: bool,
    pub gc_entry_id: Option<usize>,
}

pub struct RefTable {
    /// Raw pointers into heap `NapiRef` boxes (non-owning; freed only in `delete`).
    live: HashSet<usize>,
}

impl RefTable {
    pub fn new() -> Self {
        Self {
            live: HashSet::new(),
        }
    }

    pub fn create(
        &mut self,
        ctx: *mut JSContext,
        value: JSValue,
        initial_refcount: u32,
        env: napi_env,
        link_gc: bool,
    ) -> napi_ref {
        let weak = initial_refcount == 0;
        let stored = if weak {
            value
        } else {
            unsafe { qjs::JS_DupValue(ctx, value) }
        };
        let id = REF_ID.fetch_add(1, Ordering::Relaxed);
        let nref = Box::new(NapiRef {
            id,
            value: stored,
            refcount: initial_refcount,
            weak,
            dead: false,
            gc_entry_id: None,
        });
        let ptr = Box::into_raw(nref);
        let reference = ptr as napi_ref;
        if weak && link_gc {
            unsafe {
                (*ptr).gc_entry_id = crate::gc_hook::attach_weak_ref(ctx, value, reference, env);
            }
        }
        self.live.insert(ptr as usize);
        reference
    }

    pub unsafe fn get(&self, reference: napi_ref) -> Option<&NapiRef> {
        if reference.is_null() || !self.live.contains(&(reference as usize)) {
            return None;
        }
        Some(&*(reference as *const NapiRef))
    }

    pub unsafe fn get_mut(&mut self, reference: napi_ref) -> Option<&mut NapiRef> {
        if reference.is_null() || !self.live.contains(&(reference as usize)) {
            return None;
        }
        Some(&mut *(reference as *mut NapiRef))
    }

    pub fn release_all(&mut self, ctx: *mut JSContext) {
        for ptr in self.live.drain() {
            let nref = unsafe { Box::from_raw(ptr as *mut NapiRef) };
            crate::gc_hook::clear_weak_ref(ptr as napi_ref);
            if !nref.weak || nref.refcount > 0 {
                unsafe {
                    qjs::JS_FreeValue(ctx, nref.value);
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    pub unsafe fn delete(&mut self, ctx: *mut JSContext, reference: napi_ref) -> napi_status {
        if reference.is_null() {
            return napi_status::napi_invalid_arg;
        }
        let ptr = reference as usize;
        if self.live.remove(&ptr) {
            crate::gc_hook::clear_weak_ref(reference);
            let nref = Box::from_raw(reference as *mut NapiRef);
            if !nref.weak || nref.refcount > 0 {
                unsafe {
                    qjs::JS_FreeValue(ctx, nref.value);
                }
            }
            napi_status::napi_ok
        } else {
            napi_status::napi_invalid_arg
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_reference(
    env: napi_env,
    value: napi_value,
    initial_refcount: u32,
    result: *mut napi_ref,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env = unsafe { crate::env::Env::from_napi_env(env) };
    let js_val = match unsafe { crate::value::napi_to_value(env, value) } {
        Some(v) => v,
        None => return napi_status::napi_invalid_arg,
    };
    let reference = env.refs.create(
        env.ctx_ptr(),
        js_val,
        initial_refcount,
        env.as_napi_env(),
        true,
    );
    unsafe {
        *result = reference;
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_reference(env: napi_env, reference: napi_ref) -> napi_status {
    if env.is_null() || reference.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env = unsafe { crate::env::Env::from_napi_env(env) };
    env.refs.delete(env.ctx_ptr(), reference)
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_ref(
    env: napi_env,
    reference: napi_ref,
    result: *mut u32,
) -> napi_status {
    if env.is_null() || reference.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env = unsafe { crate::env::Env::from_napi_env(env) };
    let ctx = env.ctx_ptr();
    let nref = match unsafe { env.refs.get_mut(reference) } {
        Some(r) => r,
        None => return napi_status::napi_invalid_arg,
    };
    if nref.dead {
        return napi_status::napi_invalid_arg;
    }
    if nref.weak && nref.refcount == 0 {
        let duped = unsafe { qjs::JS_DupValue(ctx, nref.value) };
        nref.value = duped;
        nref.weak = false;
    }
    nref.refcount = nref.refcount.saturating_add(1);
    if !result.is_null() {
        unsafe {
            *result = nref.refcount;
        }
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_unref(
    env: napi_env,
    reference: napi_ref,
    result: *mut u32,
) -> napi_status {
    if env.is_null() || reference.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env = unsafe { crate::env::Env::from_napi_env(env) };
    let ctx = env.ctx_ptr();
    let napi_env = env.as_napi_env();
    let nref = match unsafe { env.refs.get_mut(reference) } {
        Some(r) => r,
        None => return napi_status::napi_invalid_arg,
    };
    if nref.refcount == 0 {
        return napi_status::napi_invalid_arg;
    }
    nref.refcount = nref.refcount.saturating_sub(1);
    if nref.refcount == 0 && !nref.weak {
        let value = nref.value;
        nref.weak = true;
        if nref.gc_entry_id.is_none() {
            nref.gc_entry_id = crate::gc_hook::attach_weak_ref(ctx, value, reference, napi_env);
        }
        unsafe {
            qjs::JS_FreeValue(ctx, value);
        }
    }
    if !result.is_null() {
        unsafe {
            *result = nref.refcount;
        }
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_reference_value(
    env: napi_env,
    reference: napi_ref,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || reference.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env = unsafe { crate::env::Env::from_napi_env(env) };
    let nref = match unsafe { env.refs.get(reference) } {
        Some(r) => r,
        None => return napi_status::napi_invalid_arg,
    };
    unsafe {
        if nref.dead {
            *result = crate::value::value_to_napi_owned(env, qjs::JS_UNDEFINED);
        } else {
            *result = crate::value::value_to_napi_borrowed(env, nref.value);
        }
    }
    napi_status::napi_ok
}
