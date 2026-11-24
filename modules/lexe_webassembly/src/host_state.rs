// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Per-realm state shared between the `wasmi::Store` and every JS-facing
//! wrapper object (`Module`/`Instance`/`Memory`/`Table`/`Global`/error
//! classes) created for that realm.
//!
//! A [`HostState`] is always reached through an `Rc<HostState>` so that it can
//! simultaneously be:
//! - the `T` in `wasmi::Store<Rc<HostState>>` (so host function trampolines can
//!   cheaply clone their way to it via `Caller::data()`), and
//! - held directly by [`crate::realm::WasmRealm`] and by every wrapper class
//!   instance that needs to validate "same realm" before an operation.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::NonNull;

use rquickjs::{function::Constructor, qjs, Ctx, Function, Persistent, Value};

/// A monotonically increasing identifier for a realm, used to key the
/// thread-local active-caller stack in [`crate::store_access`].
pub type RealmId = u64;

fn next_realm_id() -> RealmId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The kind of wasmi extern a cached JS wrapper was created for. Used together
/// with an opaque handle identity (see [`crate::store_access::handle_bits`]) as
/// the wrapper-registry cache key so that re-exporting or re-importing the same
/// underlying wasmi entity always yields the same JS wrapper object
/// (`instance.exports.foo === instance.exports.alias`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WrapKind {
    Func,
    Memory,
    Table,
    Global,
}

/// Persisted constructors captured once at realm-init time so that internal
/// error classification never has to re-read (and thus never observes a
/// user-replaced) `globalThis.WebAssembly` or `globalThis.Error`.
pub struct ErrorConstructors {
    pub compile_error: Persistent<Constructor<'static>>,
    pub link_error: Persistent<Constructor<'static>>,
    pub runtime_error: Persistent<Constructor<'static>>,
}

/// Per-realm host state. See module docs.
pub struct HostState {
    pub realm_id: RealmId,
    /// Raw pointer to the QuickJS context that owns this realm. Deliberately
    /// *not* wrapped in an owned `Ctx` (which would perform a real, held
    /// `JS_DupContext`): `HostState` (and everything it owns, including every
    /// `Persistent<Value/Object/Function/Constructor>` below) lives only in
    /// this realm's entry in the `Runtime`-userdata-backed realm registry
    /// (see `crate::realm`), never directly inside a JS-reachable class
    /// instance, precisely so it never has to participate in QuickJS's GC
    /// tracing at all. `rquickjs::Persistent`'s own documentation is explicit
    /// that *it* (not the cycle collector) is what must not outlive the
    /// `Runtime`: "ensure that no persistent links outlives the runtime,
    /// otherwise Runtime will abort the process when dropped". The realm
    /// registry satisfies that by living in `Ctx::store_userdata`, which
    /// rquickjs's own `RawRuntime::drop` explicitly clears (dropping every
    /// `HostState`, and every `Persistent` it owns) *before* calling
    /// `JS_FreeRuntime` -- see `crate::realm` module docs for the full
    /// reasoning and the bisection tests that led here.
    ///
    /// Reconstructing a transient `Ctx` from this pointer (via
    /// [`Self::ctx`]) is only ever done synchronously, on the single thread
    /// that owns this context, while the context is known to still be alive.
    ctx_ptr: NonNull<qjs::JSContext>,
    pub errors: ErrorConstructors,

    // -- JS function import callback registry -----------------------------
    callbacks: RefCell<HashMap<u32, Persistent<Function<'static>>>>,
    next_callback_id: Cell<u32>,

    // -- Wrapper identity cache (Func/Memory/Table/Global -> JS wrapper) ---
    wrappers: RefCell<HashMap<(WrapKind, u64), Persistent<Value<'static>>>>,

    // -- externref <-> JS object identity registry -------------------------
    externref_by_object_ptr: RefCell<HashMap<usize, u32>>,
    externref_objects: RefCell<HashMap<u32, Persistent<Value<'static>>>>,
    externref_handles: RefCell<HashMap<u32, wasmi::ExternRef>>,
    next_externref_id: Cell<u32>,

    // -- pending JS exception sentinel passthrough --------------------------
    pending_exception: RefCell<Option<Persistent<Value<'static>>>>,

    // -- Memory synchronous-mirror bookkeeping (see crate::memory) ---------
    pub(crate) memory_mirrors: RefCell<HashMap<u64, crate::memory::MemoryMirrorEntry>>,
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
            externref_by_object_ptr: RefCell::new(HashMap::new()),
            externref_objects: RefCell::new(HashMap::new()),
            externref_handles: RefCell::new(HashMap::new()),
            next_externref_id: Cell::new(0),
            pending_exception: RefCell::new(None),
            memory_mirrors: RefCell::new(HashMap::new()),
        }
    }

    /// Reconstructs a `Ctx<'js>` for this realm.
    ///
    /// # Safety
    /// Must only be called on the thread that owns this realm's QuickJS
    /// context, and only while that context is still alive (i.e. from within
    /// code reachable while this `HostState` itself has not yet been dropped).
    /// This crate upholds that invariant by only ever calling it synchronously
    /// from within a wasmi host function trampoline or from JS-facing glue
    /// that already holds a live `Ctx` for the same realm.
    pub unsafe fn ctx<'js>(&self) -> Ctx<'js> {
        unsafe { Ctx::from_raw(self.ctx_ptr) }
    }

    // -- callbacks ----------------------------------------------------------

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

    // -- wrapper identity cache ----------------------------------------------

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

    // -- externref registry ---------------------------------------------------

    /// Returns the stable numeric id and cached `wasmi::ExternRef` for `value`,
    /// allocating a fresh id/`ExternRef` pair the first time this particular JS
    /// value is seen by this realm's externref table.
    ///
    /// Identity/dedup handling is split by whether `value` actually *has* a
    /// stable identity in JS terms:
    ///
    /// - Objects, strings, symbols, and `BigInt`s are QuickJS ref-counted
    ///   heap values: `JS_VALUE_GET_PTR` reads the real heap pointer stored
    ///   in their `JSValue`, which is stable for as long as the value is
    ///   alive (which it is here, since we hold our own `Persistent` to it)
    ///   and safe to use as a dedup key -- two `JSValue`s of one of these
    ///   kinds compare `===` in JS if and only if they carry the same
    ///   pointer.
    /// - Everything else (`undefined`, booleans, numbers) is a non-ref-
    ///   -counted, inline-encoded `JSValue`: its `.u.ptr` union field is
    ///   *not* a pointer at all, it is whatever bits the boolean/number/
    ///   `undefined` payload happens to occupy that union slot with (and, on
    ///   builds using NaN-boxing, may not even be independently addressable
    ///   from the tag at all). Reusing `JS_VALUE_GET_PTR`'s output as a
    ///   cross-type dedup key for these previously caused genuine identity
    ///   collisions (e.g. `undefined` colliding with `null`, or `true`
    ///   colliding with the number `1`). None of these kinds need identity
    ///   preservation to be spec-correct in the first place -- JS `===` for
    ///   them is by value, not by reference, and `externref_object` always
    ///   hands back a `Persistent` that already holds the exact original
    ///   value -- so we simply skip the ptr-keyed dedup step for them
    ///   entirely and allocate a fresh id every time. This is always
    ///   correct (never returns a wrong value) and merely forgoes reusing
    ///   the same `wasmi::ExternRef` handle across repeated interning of an
    ///   equal primitive, which is not observable from JS.
    pub fn intern_externref(
        &self,
        store: &mut dyn wasmi::AsContextMut<Data = std::rc::Rc<HostState>>,
        value: &Value<'_>,
    ) -> wasmi::ExternRef {
        let has_stable_identity_ptr =
            value.is_object() || value.is_string() || value.is_symbol() || value.is_big_int();

        if has_stable_identity_ptr {
            let ptr = unsafe { qjs::JS_VALUE_GET_PTR(value.as_raw()) } as usize;
            if let Some(id) = self.externref_by_object_ptr.borrow().get(&ptr).copied() {
                if let Some(handle) = self.externref_handles.borrow().get(&id).copied() {
                    return handle;
                }
            }
            let id = self.allocate_externref_id(store, value);
            self.externref_by_object_ptr.borrow_mut().insert(ptr, id);
            return *self
                .externref_handles
                .borrow()
                .get(&id)
                .expect("just inserted above");
        }

        let id = self.allocate_externref_id(store, value);
        *self
            .externref_handles
            .borrow()
            .get(&id)
            .expect("just inserted above")
    }

    fn allocate_externref_id(
        &self,
        store: &mut dyn wasmi::AsContextMut<Data = std::rc::Rc<HostState>>,
        value: &Value<'_>,
    ) -> u32 {
        let id = self.next_externref_id.get();
        self.next_externref_id.set(id.wrapping_add(1));
        let handle = wasmi::ExternRef::new(store.as_context_mut(), id);
        self.externref_objects
            .borrow_mut()
            .insert(id, Persistent::save(value.ctx(), value.clone()));
        self.externref_handles.borrow_mut().insert(id, handle);
        id
    }

    /// Recovers the JS object identity previously registered for a
    /// `wasmi::ExternRef` produced by [`Self::intern_externref`] on this realm.
    pub fn externref_object<'js>(
        &self,
        ctx: &Ctx<'js>,
        store: &mut dyn wasmi::AsContextMut<Data = std::rc::Rc<HostState>>,
        externref: wasmi::ExternRef,
    ) -> Option<Value<'js>> {
        let id = *externref.data(store.as_context()).downcast_ref::<u32>()?;
        let persistent = self.externref_objects.borrow().get(&id).cloned()?;
        persistent.restore(ctx).ok()
    }

    // -- pending exception sentinel -----------------------------------------

    pub fn set_pending_exception(&self, value: Value<'_>) {
        let ctx = value.ctx().clone();
        *self.pending_exception.borrow_mut() = Some(Persistent::save(&ctx, value));
    }

    pub fn take_pending_exception<'js>(&self, ctx: &Ctx<'js>) -> Option<Value<'js>> {
        let persistent = self.pending_exception.borrow_mut().take()?;
        persistent.restore(ctx).ok()
    }
}
