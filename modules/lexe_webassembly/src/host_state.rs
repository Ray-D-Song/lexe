// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Per-realm state shared between `wasmz` entities and every JS-facing
//! wrapper object created for that realm.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use rquickjs::{function::Constructor, qjs, Ctx, Function, Persistent, Value};

use crate::module::FuncSigMeta;

pub type RealmId = u64;

fn next_realm_id() -> RealmId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WrapKind {
    Memory,
    Table,
    Global,
}

/// Identifies an exported/re-exported Wasm function uniquely within a realm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncKey {
    pub instance_id: u64,
    pub func_idx: u32,
}

/// Metadata kept per JS-wrapped exported function so `Table.set` / re-exports /
/// `funcref` conversions can resolve both the owner `Instance` and its full signature
/// (including `v128` rejection at the JS boundary).
#[derive(Clone)]
pub struct ExportedFuncEntry {
    pub instance: Weak<wasmz::Instance>,
    pub func_idx: u32,
    pub sig: FuncSigMeta,
}

pub struct ErrorConstructors {
    pub compile_error: Persistent<Constructor<'static>>,
    pub link_error: Persistent<Constructor<'static>>,
    pub runtime_error: Persistent<Constructor<'static>>,
}

/// Sentinel trap message when a JS host callback already saved an uncaught JS exception on `HostState`.
pub const JS_EXCEPTION_SENTINEL: &str = "__lexe_js_exception__";

pub struct HostState {
    pub realm_id: RealmId,
    ctx_ptr: NonNull<qjs::JSContext>,
    pub errors: ErrorConstructors,

    callbacks: RefCell<HashMap<u32, Persistent<Function<'static>>>>,
    next_callback_id: Cell<u32>,

    wrappers: RefCell<HashMap<(WrapKind, u64), Persistent<Value<'static>>>>,
    func_wrappers: RefCell<HashMap<FuncKey, Persistent<Value<'static>>>>,
    func_by_object: RefCell<HashMap<usize, FuncKey>>,
    func_entries: RefCell<HashMap<FuncKey, ExportedFuncEntry>>,
    instances_by_id: RefCell<HashMap<u64, Weak<wasmz::Instance>>>,

    externref_by_object_ptr: RefCell<HashMap<usize, usize>>,
    externref_objects: RefCell<HashMap<usize, Persistent<Value<'static>>>>,
    next_externref_id: Cell<usize>,

    pending_exception: RefCell<Option<Persistent<Value<'static>>>>,

    pub(crate) memory_mirrors: RefCell<HashMap<u64, crate::memory::MemoryMirrorEntry>>,

    wasi_hosts: RefCell<HashMap<u64, Rc<RefCell<crate::wasi::WasiState>>>>,
    next_wasi_id: Cell<u64>,
    wasi_stub_ptrs: RefCell<HashSet<usize>>,
    wasi_stubs: RefCell<Vec<Persistent<Value<'static>>>>,
}

impl HostState {
    pub fn new(ctx: &Ctx<'_>, errors: ErrorConstructors) -> Self {
        Self {
            realm_id: next_realm_id(),
            ctx_ptr: ctx.as_raw(),
            errors,
            callbacks: RefCell::new(HashMap::new()),
            next_callback_id: Cell::new(0),
            wrappers: RefCell::new(HashMap::new()),
            func_wrappers: RefCell::new(HashMap::new()),
            func_by_object: RefCell::new(HashMap::new()),
            func_entries: RefCell::new(HashMap::new()),
            instances_by_id: RefCell::new(HashMap::new()),
            externref_by_object_ptr: RefCell::new(HashMap::new()),
            externref_objects: RefCell::new(HashMap::new()),
            // 0 is reserved for WebAssembly null externref.
            next_externref_id: Cell::new(1),
            pending_exception: RefCell::new(None),
            memory_mirrors: RefCell::new(HashMap::new()),
            wasi_hosts: RefCell::new(HashMap::new()),
            next_wasi_id: Cell::new(1),
            wasi_stub_ptrs: RefCell::new(HashSet::new()),
            wasi_stubs: RefCell::new(Vec::new()),
        }
    }

    /// Reconstructs a `Ctx<'js>` for this realm.
    ///
    /// # Safety
    /// Caller must be executing synchronously on the QuickJS context's owning thread while the
    /// context is alive.
    pub unsafe fn ctx<'js>(&self) -> Ctx<'js> {
        unsafe { Ctx::from_raw(self.ctx_ptr) }
    }

    pub fn register_instance(&self, instance: &Rc<wasmz::Instance>) {
        self.instances_by_id
            .borrow_mut()
            .insert(instance.id(), Rc::downgrade(instance));
    }

    pub fn instance_by_id(&self, id: u64) -> Option<Rc<wasmz::Instance>> {
        self.instances_by_id
            .borrow()
            .get(&id)
            .and_then(Weak::upgrade)
    }

    pub fn register_callback(&self, func: Function<'_>) -> u32 {
        let id = self.next_callback_id.get();
        self.next_callback_id.set(id.wrapping_add(1));
        let ctx = func.ctx().clone();
        let persistent = Persistent::save(&ctx, func);
        self.callbacks.borrow_mut().insert(id, persistent);
        id
    }

    pub fn callback<'js>(&self, ctx: &Ctx<'js>, id: u32) -> Option<Function<'js>> {
        let persistent = self.callbacks.borrow().get(&id).cloned()?;
        persistent.restore(ctx).ok()
    }

    pub fn cached_wrapper<'js>(
        &self,
        ctx: &Ctx<'js>,
        kind: WrapKind,
        bits: u64,
    ) -> Option<Value<'js>> {
        let persistent = self.wrappers.borrow().get(&(kind, bits)).cloned()?;
        persistent.restore(ctx).ok()
    }

    pub fn cache_wrapper(&self, kind: WrapKind, bits: u64, value: Value<'_>) {
        let ctx = value.ctx().clone();
        let persistent = Persistent::save(&ctx, value);
        self.wrappers.borrow_mut().insert((kind, bits), persistent);
    }

    pub fn cached_func_wrapper<'js>(&self, ctx: &Ctx<'js>, key: FuncKey) -> Option<Value<'js>> {
        let persistent = self.func_wrappers.borrow().get(&key).cloned()?;
        persistent.restore(ctx).ok()
    }

    pub fn cache_func_wrapper(&self, key: FuncKey, entry: ExportedFuncEntry, value: Value<'_>) {
        let ctx = value.ctx().clone();
        let ptr = unsafe { qjs::JS_VALUE_GET_PTR(value.as_raw()) } as usize;
        let persistent = Persistent::save(&ctx, value);
        self.func_by_object.borrow_mut().insert(ptr, key);
        self.func_entries.borrow_mut().insert(key, entry);
        self.func_wrappers.borrow_mut().insert(key, persistent);
    }

    pub fn unwrap_func_key(&self, value: &Value<'_>) -> Option<FuncKey> {
        if !value.is_function() {
            return None;
        }
        let ptr = unsafe { qjs::JS_VALUE_GET_PTR(value.as_raw()) } as usize;
        self.func_by_object.borrow().get(&ptr).copied()
    }

    pub fn func_entry(&self, key: FuncKey) -> Option<ExportedFuncEntry> {
        self.func_entries.borrow().get(&key).cloned()
    }

    pub fn intern_externref(&self, value: &Value<'_>) -> usize {
        let has_stable_identity_ptr =
            value.is_object() || value.is_string() || value.is_symbol() || value.is_big_int();

        if has_stable_identity_ptr {
            let ptr = unsafe { qjs::JS_VALUE_GET_PTR(value.as_raw()) } as usize;
            if let Some(id) = self.externref_by_object_ptr.borrow().get(&ptr).copied() {
                return id;
            }
            let id = self.allocate_externref_id(value);
            self.externref_by_object_ptr.borrow_mut().insert(ptr, id);
            return id;
        }

        self.allocate_externref_id(value)
    }

    fn allocate_externref_id(&self, value: &Value<'_>) -> usize {
        let mut id = self.next_externref_id.get();
        if id == 0 {
            id = 1;
        }
        self.next_externref_id.set(id.wrapping_add(1).max(1));
        self.externref_objects
            .borrow_mut()
            .insert(id, Persistent::save(value.ctx(), value.clone()));
        id
    }

    pub fn externref_object<'js>(&self, ctx: &Ctx<'js>, id: usize) -> Option<Value<'js>> {
        if id == 0 {
            return Some(Value::new_null(ctx.clone()));
        }
        let persistent = self.externref_objects.borrow().get(&id).cloned()?;
        persistent.restore(ctx).ok()
    }

    pub fn set_pending_exception(&self, value: Value<'_>) {
        let ctx = value.ctx().clone();
        *self.pending_exception.borrow_mut() = Some(Persistent::save(&ctx, value));
    }

    pub fn take_pending_exception<'js>(&self, ctx: &Ctx<'js>) -> Option<Value<'js>> {
        let persistent = self.pending_exception.borrow_mut().take()?;
        persistent.restore(ctx).ok()
    }

    pub fn register_wasi(&self, state: crate::wasi::WasiState) -> u64 {
        let id = self.next_wasi_id.get();
        self.next_wasi_id.set(id.wrapping_add(1).max(1));
        self.wasi_hosts
            .borrow_mut()
            .insert(id, Rc::new(RefCell::new(state)));
        id
    }

    pub fn wasi_state(&self, id: u64) -> Option<Rc<RefCell<crate::wasi::WasiState>>> {
        self.wasi_hosts.borrow().get(&id).cloned()
    }

    pub fn register_wasi_stub(&self, value: &Value<'_>) {
        let ptr = unsafe { qjs::JS_VALUE_GET_PTR(value.as_raw()) } as usize;
        if self.wasi_stub_ptrs.borrow_mut().insert(ptr) {
            self.wasi_stubs
                .borrow_mut()
                .push(Persistent::save(value.ctx(), value.clone()));
        }
    }

    pub fn is_wasi_stub(&self, value: &Value<'_>) -> bool {
        if !value.is_function() {
            return false;
        }
        let ptr = unsafe { qjs::JS_VALUE_GET_PTR(value.as_raw()) } as usize;
        self.wasi_stub_ptrs.borrow().contains(&ptr)
    }
}
