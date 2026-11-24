// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rquickjs::qjs::{self, JSClassDef, JSClassID, JSContext, JSRuntime, JSValue};

use crate::env::Env;
use crate::types::{napi_env, napi_finalize};

static EXTERNAL_CLASS_IDS: Lazy<Mutex<HashMap<usize, JSClassID>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static EXTERNAL_CLASS_ENV_REFS: Lazy<Mutex<HashMap<usize, usize>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static RUNTIME_FINALIZER_REGISTERED: Lazy<Mutex<HashMap<usize, AtomicBool>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Opaque pointers for external objects not yet collected by GC.
static LIVING_EXTERNALS_BY_ENV: Lazy<Mutex<HashMap<usize, Vec<usize>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static LIVING_EXTERNALS_BY_RUNTIME: Lazy<Mutex<HashMap<usize, Vec<usize>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub struct ExternalData {
    pub ptr: *mut c_void,
    pub finalize: napi_finalize,
    pub hint: *mut c_void,
    pub env: crate::types::napi_env,
    pub runtime_key: usize,
    pub addon_finalized: bool,
}

fn register_living_external(runtime_key: usize, env: napi_env, opaque: *mut ExternalData) {
    let opaque_key = opaque as usize;
    let env_key = env as usize;
    LIVING_EXTERNALS_BY_ENV
        .lock()
        .entry(env_key)
        .or_default()
        .push(opaque_key);
    LIVING_EXTERNALS_BY_RUNTIME
        .lock()
        .entry(runtime_key)
        .or_default()
        .push(opaque_key);
}

fn unregister_living_external(opaque: *mut ExternalData) {
    let opaque_key = opaque as usize;
    let env_key = unsafe { (*opaque).env as usize };
    let runtime_key = unsafe { (*opaque).runtime_key };
    if let Some(list) = LIVING_EXTERNALS_BY_ENV.lock().get_mut(&env_key) {
        list.retain(|p| *p != opaque_key);
    }
    if let Some(list) = LIVING_EXTERNALS_BY_RUNTIME.lock().get_mut(&runtime_key) {
        list.retain(|p| *p != opaque_key);
    }
}

fn reclaim_external_opaque(opaque: *mut ExternalData) {
    if opaque.is_null() {
        return;
    }
    unregister_living_external(opaque);
    let data = unsafe { Box::from_raw(opaque) };
    if data.addon_finalized {
        return;
    }
    if let Some(f) = data.finalize {
        unsafe {
            f(data.env, data.ptr, data.hint);
        }
    }
}

/// Run finalize callbacks for external objects still tracked at env teardown.
///
/// Marks surviving externals as `addon_finalized` tombstones. A late QuickJS
/// class finalizer may still reclaim the opaque `Box` after class mapping is
/// gone; it must not re-run the addon finalize callback.
pub fn finalize_surviving_externals(env: napi_env) {
    let opaques: Vec<usize> = LIVING_EXTERNALS_BY_ENV
        .lock()
        .remove(&(env as usize))
        .unwrap_or_default();
    for opaque_key in opaques {
        let opaque = opaque_key as *mut ExternalData;
        let data = unsafe { &mut *opaque };
        if !data.addon_finalized {
            if let Some(f) = data.finalize {
                unsafe {
                    f(data.env, data.ptr, data.hint);
                }
            }
            data.addon_finalized = true;
            data.finalize = None;
            data.ptr = std::ptr::null_mut();
        }
    }
}

/// Drop runtime-keyed registration tables only. Opaque boxes may still be
/// referenced by live JS objects until QuickJS collects them or frees the
/// runtime.
fn clear_runtime_external_registry(rt: *mut JSRuntime) {
    let rt_key = rt as usize;
    EXTERNAL_CLASS_IDS.lock().remove(&rt_key);
    EXTERNAL_CLASS_ENV_REFS.lock().remove(&rt_key);
    RUNTIME_FINALIZER_REGISTERED.lock().remove(&rt_key);
}

/// Reclaim opaque boxes still tracked after all JS class finalizers have run.
#[cfg(feature = "v8-compat")]
pub(crate) fn drain_runtime_externals(rt: *mut JSRuntime) {
    drain_residual_external_opaques(rt);
    clear_runtime_external_registry(rt);
}

fn drain_residual_external_opaques(rt: *mut JSRuntime) {
    let rt_key = rt as usize;
    let opaques = LIVING_EXTERNALS_BY_RUNTIME
        .lock()
        .remove(&rt_key)
        .unwrap_or_default();
    for opaque_key in opaques {
        reclaim_external_opaque(opaque_key as *mut ExternalData);
    }
}

#[cfg(test)]
pub fn clear_runtime_external_registry_for_tests(rt: *mut JSRuntime) {
    clear_runtime_external_registry(rt);
}

#[cfg(test)]
pub fn living_external_count_for_runtime(rt_key: usize) -> usize {
    LIVING_EXTERNALS_BY_RUNTIME
        .lock()
        .get(&rt_key)
        .map(|v| v.len())
        .unwrap_or(0)
}

#[cfg(test)]
pub fn reset_for_tests() {
    LIVING_EXTERNALS_BY_ENV.lock().clear();
    LIVING_EXTERNALS_BY_RUNTIME.lock().clear();
    EXTERNAL_CLASS_IDS.lock().clear();
    EXTERNAL_CLASS_ENV_REFS.lock().clear();
    RUNTIME_FINALIZER_REGISTERED.lock().clear();
}

unsafe extern "C" fn runtime_external_finalizer(_rt: *mut JSRuntime, rt_key: *mut c_void) {
    let rt = rt_key as *mut JSRuntime;
    clear_runtime_external_registry(rt);
    drain_residual_external_opaques(rt);
}

fn ensure_runtime_finalizer(rt: *mut JSRuntime) {
    let rt_key = rt as usize;
    let mut registered = RUNTIME_FINALIZER_REGISTERED.lock();
    let entry = registered
        .entry(rt_key)
        .or_insert_with(|| AtomicBool::new(false));
    if entry.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        qjs::JS_AddRuntimeFinalizer(rt, Some(runtime_external_finalizer), rt_key as *mut c_void);
    }
}

fn external_class_id_for_runtime(rt: *mut JSRuntime) -> Option<JSClassID> {
    EXTERNAL_CLASS_IDS.lock().get(&(rt as usize)).copied()
}

fn is_external_class_for_runtime(rt: *mut JSRuntime, class_id: JSClassID) -> bool {
    external_class_id_for_runtime(rt) == Some(class_id)
}

unsafe extern "C" fn external_class_finalizer(_rt: *mut JSRuntime, val: JSValue) {
    let mut class_id: JSClassID = 0;
    let opaque = unsafe { qjs::JS_GetAnyOpaque(val, &mut class_id) };
    if opaque.is_null() {
        return;
    }
    let data_ptr = opaque as *mut ExternalData;
    unregister_living_external(data_ptr);
    let data = unsafe { Box::from_raw(data_ptr) };
    if data.addon_finalized {
        return;
    }
    let entry_id = crate::gc_hook::register_gc_entry(
        crate::gc_hook::GcEntryKind::External,
        data.ptr,
        data.finalize,
        data.hint,
        data.env,
        None,
    );
    crate::gc_hook::enqueue_gc_entry(entry_id);
}

pub fn acquire_external_class_for_env(rt: *mut JSRuntime) -> JSClassID {
    let rt_key = rt as usize;
    {
        let mut refs = EXTERNAL_CLASS_ENV_REFS.lock();
        *refs.entry(rt_key).or_insert(0) += 1;
    }
    ensure_external_class_registered(rt)
}

pub fn release_external_class_for_env(rt: *mut JSRuntime) {
    let rt_key = rt as usize;
    let should_unregister = {
        let mut refs = EXTERNAL_CLASS_ENV_REFS.lock();
        let count = refs.get_mut(&rt_key);
        match count {
            None => false,
            Some(0) => false,
            Some(n) => {
                *n -= 1;
                if *n == 0 {
                    refs.remove(&rt_key);
                    true
                } else {
                    false
                }
            },
        }
    };
    if should_unregister {
        unregister_external_class(rt);
    }
}

fn ensure_external_class_registered(rt: *mut JSRuntime) -> JSClassID {
    let rt_key = rt as usize;
    if let Some(&class_id) = EXTERNAL_CLASS_IDS.lock().get(&rt_key) {
        return class_id;
    }
    ensure_runtime_finalizer(rt);
    let mut class_id: JSClassID = 0;
    unsafe {
        qjs::JS_NewClassID(rt, &mut class_id);
        let def = JSClassDef {
            class_name: c"NapiExternal".as_ptr(),
            finalizer: Some(external_class_finalizer),
            gc_mark: None,
            call: None,
            exotic: ptr::null_mut(),
        };
        qjs::JS_NewClass(rt, class_id, &def);
    }
    EXTERNAL_CLASS_IDS.lock().insert(rt_key, class_id);
    class_id
}

pub fn unregister_external_class(rt: *mut JSRuntime) {
    EXTERNAL_CLASS_IDS.lock().remove(&(rt as usize));
}

pub fn class_id_for_runtime(rt: *mut JSRuntime) -> Option<JSClassID> {
    external_class_id_for_runtime(rt)
}

pub fn create_external_object(
    env: &mut Env,
    data: *mut c_void,
    finalize: napi_finalize,
    hint: *mut c_void,
) -> JSValue {
    let ctx = env.ctx_ptr();
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let runtime_key = rt as usize;
    let class_id = env.ensure_external_class();
    let obj = unsafe { qjs::JS_NewObjectClass(ctx, class_id) };
    let boxed = Box::new(ExternalData {
        ptr: data,
        finalize,
        hint,
        env: env.as_napi_env(),
        runtime_key,
        addon_finalized: false,
    });
    let opaque = Box::into_raw(boxed);
    register_living_external(runtime_key, env.as_napi_env(), opaque);
    unsafe {
        qjs::JS_SetOpaque(obj, opaque as *mut c_void);
    }
    obj
}

fn external_opaque(ctx: *mut JSContext, val: JSValue) -> Option<*mut ExternalData> {
    if !unsafe { qjs::JS_IsObject(val) } {
        return None;
    }
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let mut class_id: JSClassID = 0;
    let opaque = unsafe { qjs::JS_GetAnyOpaque(val, &mut class_id) };
    if opaque.is_null() || !is_external_class_for_runtime(rt, class_id) {
        return None;
    }
    Some(opaque as *mut ExternalData)
}

pub fn get_external_pointer(ctx: *mut JSContext, val: JSValue) -> Option<*mut c_void> {
    let data = external_opaque(ctx, val)?;
    Some(unsafe { (*data).ptr })
}

pub fn is_external_object(ctx: *mut JSContext, val: JSValue) -> bool {
    external_opaque(ctx, val).is_some()
}
