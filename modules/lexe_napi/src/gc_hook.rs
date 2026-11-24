// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use rquickjs::qjs::{self, JSClassDef, JSClassID, JSContext, JSRuntime, JSValue};

use crate::env::Env;
use crate::types::{napi_env, napi_finalize, napi_ref};

static HOLDER_CLASS_IDS: Lazy<Mutex<HashMap<usize, JSClassID>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static NEXT_GC_ENTRY: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GcEntryKind {
    Wrap,
    Finalizer,
    External,
    WeakRef,
}

struct HolderOpaque {
    entry_id: usize,
}

pub struct GcEntry {
    pub kind: GcEntryKind,
    pub data: usize,
    pub finalize: napi_finalize,
    pub hint: usize,
    pub env: usize,
    pub weak_ref: Option<usize>,
}

struct GcState {
    entries: HashMap<usize, GcEntry>,
    pending: Vec<usize>,
}

static GC_STATE: Lazy<Mutex<GcState>> = Lazy::new(|| {
    Mutex::new(GcState {
        entries: HashMap::new(),
        pending: Vec::new(),
    })
});

fn holder_property_name(entry_id: usize) -> CString {
    CString::new(format!("__napi_gc_holder_{entry_id}")).expect("gc holder key")
}

pub fn register_holder_class(rt: *mut JSRuntime) -> JSClassID {
    let rt_key = rt as usize;
    if let Some(&class_id) = HOLDER_CLASS_IDS.lock().get(&rt_key) {
        return class_id;
    }
    let mut class_id: JSClassID = 0;
    unsafe {
        qjs::JS_NewClassID(rt, &mut class_id);
        let def = JSClassDef {
            class_name: c"NapiFinalizerHolder".as_ptr(),
            finalizer: Some(holder_class_finalizer),
            gc_mark: None,
            call: None,
            exotic: ptr::null_mut(),
        };
        qjs::JS_NewClass(rt, class_id, &def);
    }
    HOLDER_CLASS_IDS.lock().insert(rt_key, class_id);
    class_id
}

unsafe extern "C" fn holder_class_finalizer(_rt: *mut JSRuntime, val: JSValue) {
    let mut class_id: JSClassID = 0;
    let opaque = unsafe { qjs::JS_GetAnyOpaque(val, &mut class_id) };
    if opaque.is_null() {
        return;
    }
    let holder = unsafe { Box::from_raw(opaque as *mut HolderOpaque) };
    enqueue_gc(holder.entry_id);
}

unsafe fn free_holder_without_finalizer(ctx: *mut JSContext, holder: JSValue) {
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let class_id = register_holder_class(rt);
    let opaque = unsafe { qjs::JS_GetOpaque(holder, class_id) };
    if !opaque.is_null() {
        unsafe {
            qjs::JS_SetOpaque(holder, ptr::null_mut());
            let _ = Box::from_raw(opaque as *mut HolderOpaque);
        }
    }
    unsafe {
        qjs::JS_FreeValue(ctx, holder);
    }
}

pub fn enqueue_gc_entry(entry_id: usize) {
    let mut state = GC_STATE.lock();
    if let Some(entry) = state.entries.get_mut(&entry_id) {
        if let Some(reference) = entry.weak_ref {
            unsafe {
                let nref = &mut *(reference as napi_ref as *mut crate::refs::NapiRef);
                nref.dead = true;
            }
        }
    }
    if !state.pending.contains(&entry_id) {
        state.pending.push(entry_id);
    }
}

/// Attach a GC holder for a standalone weak reference (not via wrap).
pub fn attach_weak_ref(
    ctx: *mut JSContext,
    target: JSValue,
    weak_ref: napi_ref,
    env: napi_env,
) -> Option<usize> {
    let id = register_gc_entry(
        GcEntryKind::WeakRef,
        ptr::null_mut(),
        None,
        ptr::null_mut(),
        env,
        Some(weak_ref),
    );
    if attach_holder(ctx, target, id) {
        Some(id)
    } else {
        remove_gc_entry(id);
        None
    }
}

pub fn clear_weak_ref(weak_ref: napi_ref) {
    if weak_ref.is_null() {
        return;
    }
    let key = weak_ref as usize;
    let mut state = GC_STATE.lock();
    let mut remove_ids = Vec::new();
    for (id, entry) in state.entries.iter_mut() {
        if entry.weak_ref == Some(key) {
            entry.weak_ref = None;
            if entry.kind == GcEntryKind::WeakRef {
                remove_ids.push(*id);
            }
        }
    }
    for id in remove_ids {
        state.entries.remove(&id);
    }
}

pub fn remove_gc_entry(entry_id: usize) {
    GC_STATE.lock().entries.remove(&entry_id);
}

fn enqueue_gc(entry_id: usize) {
    enqueue_gc_entry(entry_id);
}

pub fn has_pending_finalizers() -> bool {
    !GC_STATE.lock().pending.is_empty()
}

/// Drop queued finalizer ids that no longer have registry entries.
pub fn compact_stale_pending() {
    let mut state = GC_STATE.lock();
    let live: std::collections::HashSet<usize> = state.entries.keys().copied().collect();
    state.pending.retain(|id| live.contains(id));
}

pub fn register_gc_entry(
    kind: GcEntryKind,
    data: *mut c_void,
    finalize: napi_finalize,
    hint: *mut c_void,
    env: napi_env,
    weak_ref: Option<napi_ref>,
) -> usize {
    let id = NEXT_GC_ENTRY.fetch_add(1, Ordering::Relaxed);
    GC_STATE.lock().entries.insert(
        id,
        GcEntry {
            kind,
            data: data as usize,
            finalize,
            hint: hint as usize,
            env: env as usize,
            weak_ref: weak_ref.map(|r| r as usize),
        },
    );
    id
}

pub fn attach_holder(ctx: *mut JSContext, target: JSValue, entry_id: usize) -> bool {
    let rt = unsafe { qjs::JS_GetRuntime(ctx) };
    let class_id = register_holder_class(rt);
    let holder = unsafe { qjs::JS_NewObjectClass(ctx, class_id) };
    let boxed = Box::new(HolderOpaque { entry_id });
    let opaque = Box::into_raw(boxed);
    if unsafe { qjs::JS_SetOpaque(holder, opaque as *mut c_void) } < 0 {
        unsafe {
            let _ = Box::from_raw(opaque);
            qjs::JS_FreeValue(ctx, holder);
        }
        return false;
    }
    let key = holder_property_name(entry_id);
    let atom = unsafe { qjs::JS_NewAtom(ctx, key.as_ptr()) };
    let ret = unsafe {
        qjs::JS_DefinePropertyValue(ctx, target, atom, holder, qjs::JS_PROP_CONFIGURABLE as i32)
    };
    unsafe {
        qjs::JS_FreeAtom(ctx, atom);
    }
    if ret <= 0 {
        unsafe {
            free_holder_without_finalizer(ctx, holder);
        }
        return false;
    }
    true
}

pub fn detach_holder(ctx: *mut JSContext, target: JSValue, entry_id: usize) {
    let key = holder_property_name(entry_id);
    let atom = unsafe { qjs::JS_NewAtom(ctx, key.as_ptr()) };
    unsafe {
        qjs::JS_DeleteProperty(ctx, target, atom, 0);
        qjs::JS_FreeAtom(ctx, atom);
    }
    remove_gc_entry(entry_id);
}

pub fn drain_pending_finalizers(env: &mut Env) {
    let env_key = env.as_napi_env() as usize;
    let pending = {
        let mut state = GC_STATE.lock();
        let all_pending = std::mem::take(&mut state.pending);
        let (pending, other): (Vec<usize>, Vec<usize>) = all_pending
            .into_iter()
            .partition(|id| state.entries.get(id).is_some_and(|e| e.env == env_key));
        if !other.is_empty() {
            state.pending.extend(other);
        }
        pending
    };
    for entry_id in pending {
        let Some(entry) = GC_STATE.lock().entries.remove(&entry_id) else {
            continue;
        };
        match entry.kind {
            GcEntryKind::Wrap => {
                env.wraps.remove_by_id(entry_id);
            },
            GcEntryKind::Finalizer => {
                env.finalizers.remove_by_id(entry_id);
            },
            GcEntryKind::External | GcEntryKind::WeakRef => {},
        }
        if let Some(f) = entry.finalize {
            unsafe {
                f(
                    entry.env as napi_env,
                    entry.data as *mut c_void,
                    entry.hint as *mut c_void,
                )
            };
        }
    }
}

pub fn run_all_remaining(env: &mut Env) {
    let env_key = env.as_napi_env() as usize;
    let ids: Vec<usize> = GC_STATE
        .lock()
        .entries
        .iter()
        .filter(|(_, entry)| entry.env == env_key)
        .map(|(id, _)| *id)
        .collect();
    let processed: std::collections::HashSet<usize> = ids.iter().copied().collect();
    for entry_id in ids {
        let Some(entry) = GC_STATE.lock().entries.remove(&entry_id) else {
            continue;
        };
        match entry.kind {
            GcEntryKind::Wrap => {
                env.wraps.remove_by_id(entry_id);
            },
            GcEntryKind::Finalizer => {
                env.finalizers.remove_by_id(entry_id);
            },
            GcEntryKind::External | GcEntryKind::WeakRef => {},
        }
        if let Some(f) = entry.finalize {
            unsafe {
                f(
                    entry.env as napi_env,
                    entry.data as *mut c_void,
                    entry.hint as *mut c_void,
                )
            };
        }
    }
    GC_STATE.lock().pending.retain(|id| !processed.contains(id));
}

#[cfg(test)]
pub fn entry_count() -> usize {
    GC_STATE.lock().entries.len()
}

#[cfg(test)]
pub fn reset_for_tests() {
    let mut state = GC_STATE.lock();
    state.entries.clear();
    state.pending.clear();
    HOLDER_CLASS_IDS.lock().clear();
}

pub fn unregister_holder_class(rt: *mut JSRuntime) {
    HOLDER_CLASS_IDS.lock().remove(&(rt as usize));
}
