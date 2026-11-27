// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Per-realm `WasmRealm` (the `wasmi::Store` + [`HostState`] pair) and the
//! private, non-enumerable/non-configurable holder object that anchors its
//! lifetime to a single QuickJS context's `globalThis`.
//!
//! We deliberately do *not* use `Ctx::store_userdata` to store the `Store`
//! *directly* the way the implementation plan originally sketched (see the
//! plan's own caveat): in rquickjs 0.12.1 that storage is keyed off
//! `JS_GetRuntimeOpaque`, i.e. it is shared by every `JSContext` that lives on
//! the same `JSRuntime` -- including the bare child contexts created by
//! `vm.runInNewContext`/`vm.createContext`. A single realm's `Store` would
//! therefore leak into unrelated child contexts if it were the *only* thing
//! keyed there.
//!
//! Instead we use a hybrid of two mechanisms:
//!
//! 1. A private native holder ([`WasmRealmHolder`]) is installed directly on
//!    `globalThis`, holding *only* a plain `u64` realm id (plus a handle to a
//!    plain, non-GC-reachable removal queue -- see point 3 below) -- no
//!    `Ctx`, no `Persistent`, nothing GC-cycle-shaped.
//! 2. The actual [`WasmRealm`] (which, transitively through [`HostState`],
//!    owns every `Persistent<Value/Object/Function/Constructor>` this crate
//!    ever creates -- the callback/wrapper/externref registries, the pending
//!    exception slot, the captured error constructors) lives in a
//!    [`RealmRegistry`] stored via `Ctx::store_userdata`, keyed internally by
//!    realm id so that multiple realms safely share the one per-`Runtime`
//!    userdata slot.
//! 3. When a [`WasmRealmHolder`] is finalized, it queues its realm id into a
//!    `condemned` list rather than removing it from the registry itself (see
//!    that type's own doc comment for why); [`reap_condemned`] drains that
//!    list from ordinary (non-finalizer) call sites. In practice, per
//!    rquickjs 0.12.1's own behavior, a `WasmRealmHolder` is *not* finalized
//!    until its whole `AsyncRuntime` is torn down (see the
//!    `context_teardown_defers_realm_removal_until_runtime_teardown` test in
//!    this module's `tests` submodule for the empirical confirmation and
//!    full explanation); this split is still worth keeping; both because it
//!    is the only mechanism that can behave correctly if that upstream
//!    behavior ever changes, and because it is what makes eventual removal
//!    (whenever it happens) safe -- see point 3's cross-reference below.
//!
//! Why this split, and not simply putting `Rc<WasmRealm>` directly inside
//! [`WasmRealmHolder`] (as an earlier iteration of this module did)? Because
//! `rquickjs::Persistent`'s own documentation is direct about this:
//!
//! > Be careful and ensure that no persistent links outlives the runtime,
//! > otherwise Runtime will abort the process when dropped.
//!
//! This is *not* a QuickJS GC cycle-collection problem -- no `Trace`/
//! `Tracer::mark_ctx` scheme fixes it, because it is not a cycle at all in the
//! sense the collector understands. It is simply that a live `Persistent`
//! embeds a real, `JS_DupContext`-backed `Ctx`, and *nothing* ever frees that
//! dup unless the `Persistent` itself is dropped -- and a native class
//! instance reachable only via ordinary (non-cyclic) JS refcounting is not
//! guaranteed to be finalized before `AsyncRuntime`/`AsyncContext` run their
//! own `Drop` chain. This was confirmed empirically (see the bisection tests
//! in this module's `tests` submodule during development): even a
//! `Persistent` held in a plain, JS-unreachable `thread_local` map, or inside
//! a class whose `Trace` is a genuine no-op, aborts the process at
//! `JS_FreeRuntime` if it is still alive when the `Runtime` is dropped.
//!
//! The one place a live `Persistent` is *guaranteed* to be dropped before
//! `JS_FreeRuntime` runs is `Ctx`/`Runtime`-userdata: rquickjs's
//! `RawRuntime::drop` explicitly calls `Opaque::clear()` (which drops all
//! userdata) *before* calling `qjs::JS_FreeRuntime`. Routing every
//! `Persistent`-owning structure through `store_userdata` -- rather than
//! through a `Trace`d class field -- is therefore both necessary and
//! sufficient, and needs no `unsafe`/cycle-collector involvement at all.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{class::Trace, object::Property, Class, Ctx, JsLifetime, Result};

use crate::host_state::{ErrorConstructors, HostState};

/// Hidden property name used to anchor the realm holder to `globalThis`.
///
/// The leading NUL byte keeps this out of the way of any realistic
/// user-defined property name; the property itself is installed with no
/// `enumerable`/`configurable`/`writable` flags (i.e. all `false`), so it is
/// invisible to `for..in`, `Object.keys`, `JSON.stringify`, cannot be
/// reassigned, and cannot be deleted or redefined by user code.
const HOLDER_KEY: &str = "\0lexe:webassembly_realm";

/// The realm-scoped `wasmz::Engine` + `wasmz::Store` + [`HostState`].
///
/// Lives *only* inside [`RealmRegistry`] (itself `Runtime`-userdata).
pub struct WasmRealm {
    pub state: Rc<HostState>,
    pub store: wasmz::Store,
    pub engine: wasmz::Engine,
}

impl WasmRealm {
    pub(crate) fn new(ctx: &Ctx<'_>, errors: ErrorConstructors) -> Self {
        let engine = crate::engine::new_engine().expect("wasmz Engine::new must succeed");
        let store = wasmz::Store::new(&engine).expect("wasmz Store::new must succeed");
        let state = Rc::new(HostState::new(ctx, errors));
        Self {
            state,
            store,
            engine,
        }
    }
}

impl Drop for WasmRealm {
    fn drop(&mut self) {}
}

/// A list of realm ids whose owning context has been torn down but whose
/// `WasmRealm` entry has not actually been removed (and dropped) from
/// [`RealmRegistry`] yet. Deliberately just a plain `Rc<RefCell<Vec<u64>>>`
/// containing no `Ctx`, no `Persistent`, nothing GC-cycle-shaped -- see
/// [`WasmRealmHolder`] for why only *this* (and not the realm map itself) is
/// safe to share into a JS-reachable class field.
type CondemnedList = Rc<RefCell<Vec<u64>>>;

/// `Runtime`-userdata-scoped table of every live [`WasmRealm`] on this
/// `Runtime`, keyed by realm id, plus the [`CondemnedList`] of ids pending
/// removal. See the module docs for why this lives in `Ctx`-userdata (rather
/// than a `Ctx`-scoped slot, or inside a JS-reachable class instance)
/// at all.
///
/// One `Runtime` may back multiple top-level contexts over its lifetime (and,
/// via `vm.runInNewContext`, child contexts that never get an entry here at
/// all -- see `crate::realm`'s "install exactly once per top-level context"
/// contract), so `realms` is a map rather than a single slot.
///
/// Unlike an earlier version of this module, entries here *are* eventually
/// removed once the context that created them is torn down, rather than
/// only when the whole `Runtime` goes away -- a long-lived `Runtime` that
/// creates and destroys many contexts over its lifetime (e.g. repeated
/// `vm.createContext`/`vm.runInNewContext` usage) would otherwise accumulate
/// one `WasmRealm` (and everything it transitively owns: `Store`,
/// callback/wrapper/externref registries) per context ever created, for the
/// entire remaining lifetime of the process. That removal is deliberately
/// *not* performed directly from [`WasmRealmHolder`]'s `Drop` impl -- see its
/// doc comment for why doing so crashes -- but instead deferred: `Drop` only
/// pushes the id onto `condemned` (cheap, pure-Rust bookkeeping that touches
/// no JS value), and [`reap_condemned`] actually removes (and drops) those
/// entries the next time any ordinary (non-finalizer) `WebAssembly` call
/// touches this registry, on any context sharing this `Runtime`. Any entry
/// that is never reaped this way (e.g. the single-top-level-context-per-
/// `Runtime` case, where no *later* call ever comes along to trigger a reap)
/// is still safely, unconditionally dropped before `JS_FreeRuntime` when the
/// whole `Runtime` goes away, exactly as before.
#[derive(Default)]
struct RealmRegistry {
    realms: RefCell<HashMap<u64, Rc<WasmRealm>>>,
    condemned: CondemnedList,
}

unsafe impl<'js> JsLifetime<'js> for RealmRegistry {
    type Changed<'to> = RealmRegistry;
}

/// Removes (and drops) every realm queued in `registry.condemned` from
/// `registry.realms`.
///
/// Only ever called from ordinary `WebAssembly` API entry points below (via
/// [`with_registry`]) -- i.e. from a normal JS-to-native call stack, never
/// from inside a QuickJS class-instance finalizer callback. That distinction
/// is exactly why this is safe where doing the same work directly inside
/// [`WasmRealmHolder`]'s `Drop` is not: dropping the removed `Rc<WasmRealm>`s
/// here drops every `Persistent<Value/Function/...>` their `HostState`s own,
/// which calls back into the engine to release JS values -- safe from
/// ordinary code, but not reentrantly from inside the engine's own
/// object-freeing machinery.
fn reap_condemned(registry: &RealmRegistry) {
    let ids = std::mem::take(&mut *registry.condemned.borrow_mut());
    if ids.is_empty() {
        return;
    }
    let mut removed = Vec::with_capacity(ids.len());
    {
        let mut realms = registry.realms.borrow_mut();
        for id in ids {
            if let Some(realm) = realms.remove(&id) {
                removed.push(realm);
            }
        }
    }
    // Dropped here, deliberately outside of the `realms` borrow above.
    drop(removed);
}

/// Runs `f` with access to this `Runtime`'s [`RealmRegistry`], creating an
/// empty one on first use and reaping any condemned realms first (see
/// [`reap_condemned`]).
fn with_registry<R>(ctx: &Ctx<'_>, f: impl FnOnce(&RealmRegistry) -> R) -> R {
    if ctx.userdata::<RealmRegistry>().is_none() {
        // Best-effort: if another realm on this same `Runtime` raced us to
        // install the registry first, `store_userdata` returning an error
        // here is fine -- `ctx.userdata()` below will find theirs.
        let _ = ctx.store_userdata(RealmRegistry::default());
    }
    let registry = ctx
        .userdata::<RealmRegistry>()
        .expect("just ensured the registry exists");
    reap_condemned(&registry);
    f(&registry)
}

pub(crate) fn insert_into_registry(ctx: &Ctx<'_>, realm: Rc<WasmRealm>) {
    with_registry(ctx, |registry| {
        registry
            .realms
            .borrow_mut()
            .insert(realm.state.realm_id, realm);
    });
}

fn lookup_in_registry(ctx: &Ctx<'_>, realm_id: u64) -> Option<Rc<WasmRealm>> {
    with_registry(ctx, |registry| {
        registry.realms.borrow().get(&realm_id).cloned()
    })
}

fn condemned_list(ctx: &Ctx<'_>) -> CondemnedList {
    with_registry(ctx, |registry| registry.condemned.clone())
}

/// The private `globalThis` holder. Deliberately holds only a plain `u64`
/// realm id plus a clone of the shared [`CondemnedList`] -- see the module
/// docs for why it must never hold a `Ctx`, `Persistent`, or anything else
/// that embeds one; `CondemnedList` itself is a plain `Rc<RefCell<..>>` and
/// satisfies that.
///
/// When this holder is eventually finalized by QuickJS -- in practice, per
/// rquickjs 0.12.1's own behavior, only once the whole `AsyncRuntime` this
/// context lives on is torn down, *not* eagerly when just this one context's
/// `globalThis` becomes unreachable (see the module docs' point 3, and the
/// `context_teardown_defers_realm_removal_until_runtime_teardown` test, for
/// why) -- its [`Drop`] impl below queues this realm's id for removal.
/// It deliberately does *not* remove (and thus drop) the realm itself right
/// there: this `Drop` impl runs as a QuickJS class-instance *finalizer
/// callback*, i.e. reentrantly from inside the engine's own object-freeing
/// machinery (this holder's underlying JS object is itself only being freed
/// as a side effect of the owning context's `globalThis` being freed).
/// Dropping a `WasmRealm` there would drop every `Persistent<Value>` its
/// `HostState` owns, each of which calls back into the engine to release a
/// JS value -- and doing that reentrantly, from inside another value's own
/// finalizer, was confirmed empirically (during development of this fix) to
/// corrupt the engine's object bookkeeping enough to abort the process at a
/// later, unrelated `JS_FreeRuntime` with QuickJS's own
/// `assert(list_empty(&rt->gc_obj_list))`. Queueing the id instead, and
/// letting [`reap_condemned`] remove/drop it from a later *non*-finalizer
/// call stack, sidesteps that hazard entirely.
#[derive(rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct WasmRealmHolder {
    pub realm_id: u64,
    condemned: CondemnedList,
}

impl Drop for WasmRealmHolder {
    fn drop(&mut self) {
        self.condemned.borrow_mut().push(self.realm_id);
    }
}

/// Genuinely a no-op: `WasmRealmHolder` holds nothing GC-reachable (see the
/// module docs for the full reasoning). This impl exists only to satisfy
/// `#[rquickjs::class]`'s requirement that every class implement `Trace`.
impl<'js> Trace<'js> for WasmRealmHolder {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

/// Creates the realm for `ctx` and anchors it to `globalThis` via the private
/// holder property. Must be called exactly once per context, during
/// `WebAssembly` global installation.
pub fn install<'js>(ctx: &Ctx<'js>, errors: ErrorConstructors) -> Result<Rc<WasmRealm>> {
    let realm = Rc::new(WasmRealm::new(ctx, errors));
    let realm_id = realm.state.realm_id;
    insert_into_registry(ctx, realm.clone());
    let condemned = condemned_list(ctx);
    let holder = Class::instance(
        ctx.clone(),
        WasmRealmHolder {
            realm_id,
            condemned,
        },
    )?;
    ctx.globals().prop(HOLDER_KEY, Property::from(holder))?;
    Ok(realm)
}

/// Fetches the realm installed for `ctx`.
///
/// Returns a `TypeError` if `WebAssembly` was never installed for this
/// context (should not happen for any context created through
/// `ModuleBuilder`, but child contexts created via `vm.runInNewContext` do not
/// currently run global attachment at all -- see API.md).
pub fn realm(ctx: &Ctx<'_>) -> Result<Rc<WasmRealm>> {
    let holder: Class<WasmRealmHolder> = ctx.globals().get(HOLDER_KEY).map_err(|_| {
        rquickjs::Exception::throw_internal(
            ctx,
            "WebAssembly realm state is not initialized for this context",
        )
    })?;
    let realm_id = holder.try_borrow().map(|b| b.realm_id).map_err(|_| {
        rquickjs::Exception::throw_internal(ctx, "WebAssembly realm is being torn down")
    })?;
    lookup_in_registry(ctx, realm_id).ok_or_else(|| {
        rquickjs::Exception::throw_internal(
            ctx,
            "WebAssembly realm state is not initialized for this context",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_sync_with;

    #[tokio::test]
    async fn realm_teardown_drops_state_without_panicking() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = install(&ctx, errors)?;
            assert!(realm.state.realm_id > 0);
            assert!(super::realm(&ctx).is_ok());
            Ok(())
        })
        .await;
        // Dropping the runtime/context tears down `globalThis` (freeing the
        // `WasmRealmHolder`, which holds nothing GC-reachable) and then, via
        // rquickjs's `RawRuntime::drop`, clears the `Runtime`-userdata
        // `RealmRegistry` -- dropping the `Rc<WasmRealm>` and every
        // `Persistent` it transitively owns -- *before* `JS_FreeRuntime` runs.
        // This must complete without an assertion failure or panic inside
        // QuickJS's GC object list bookkeeping.
    }

    #[tokio::test]
    async fn context_teardown_with_function_imports_and_externref_does_not_panic() {
        // Exercises the implementation plan's "增加 context teardown/GC 测
        // 试" requirement end-to-end through the full public `WebAssembly`
        // surface (`crate::init`, not the lower-level `realm::install`
        // used by the other tests in this module): a JS function import
        // callback registration (`HostState::callback`'s `Persistent`
        // registry, populated by instantiating a module that imports a JS
        // function) and a live externref (`HostState`'s externref
        // registry, populated by storing a JS object in an `externref`
        // `Table`) are both left alive -- reachable only from a
        // JS-side local variable inside the IIFE below, i.e. via ordinary
        // (non-cyclic) refcounting, exactly like the scenario this
        // module's doc comment describes -- when the `AsyncRuntime`/
        // `AsyncContext` created by `test_sync_with` are dropped at the
        // end of this test. If either registry's `Persistent`s were not
        // routed through `Runtime`-userdata (see this module's doc
        // comment), this would abort the process with a QuickJS
        // `gc_obj_list` assertion instead of returning normally.
        let wasm = wat::parse_str(
            r#"
            (module
                (import "env" "log" (func $log (param i32)))
                (func (export "run") (param i32) local.get 0 call $log)
            )
            "#,
        )
        .unwrap();

        test_sync_with(move |ctx| {
            crate::init(&ctx)?;
            let bytes = rquickjs::ArrayBuffer::new_copy(ctx.clone(), &wasm)?;
            ctx.globals().set("__wasmBytes", bytes)?;

            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const instance = new WebAssembly.Instance(module, {
                        env: { log(x) { globalThis.__lastLogged = x; } },
                    });
                    instance.exports.run(7);
                    if (globalThis.__lastLogged !== 7) return false;

                    const table = new WebAssembly.Table({ element: "externref", initial: 1 });
                    table.set(0, { tag: "externref-teardown-probe" });
                    globalThis.__keepAlive = { instance, table };
                    return true;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// Per the P2 fix: a `WasmRealm` should ideally be removed from the
    /// shared, `Runtime`-scoped registry as soon as the context that created
    /// it is torn down, *not* only when the whole `Runtime` is dropped.
    ///
    /// That ideal is *not* achievable today, and this test documents why
    /// empirically rather than asserting it: rquickjs 0.12.1 caches each
    /// registered class's prototype object in the *`Runtime`-scoped* opaque
    /// data (`Ctx::get_opaque().get_or_insert_prototype::<C>()`, backing
    /// `Class::instance`), keyed only by `TypeId`, shared by every context on
    /// that `Runtime` -- not per-context. A minimal reproduction (a bare
    /// class holding nothing but an `AtomicBool`, no `Persistent`s, no
    /// `WebAssembly` state at all) confirms that simply dropping the
    /// `AsyncContext` that created such an instance -- even combined with an
    /// explicit `Ctx::run_gc()` on a second, sibling context of the same
    /// `Runtime` -- does *not* finalize it. It is only finalized when the
    /// whole `Runtime` (and with it, that class-prototype cache) is dropped.
    /// So there is currently no JS-object-finalizer-based signal available to
    /// this module (or any other rquickjs consumer) for "this specific
    /// top-level context, as opposed to the whole `Runtime`, is done" --
    /// this is a library-level constraint, not a bug in this module's
    /// `WasmRealmHolder`/`RealmRegistry` design.
    ///
    /// This is why `WasmRealmHolder::drop` only *queues* (`condemned`) rather
    /// than removes: it is written to be correct *whenever* it eventually
    /// fires (verified below to be safe even when several realms are queued
    /// at once), in case a future rquickjs version scopes class prototypes
    /// per-context instead of per-`Runtime`, without requiring another
    /// rewrite of this module. It is intentionally *not* load-bearing for
    /// this fix's actual, achievable goal: eliminating `MEMORY_BY_BITS` (a
    /// thread-local map with no cleanup path at all, unconditionally
    /// growing for the entire life of the *thread*, potentially outliving
    /// even a single `Runtime`) in favor of storing `wasmi::Memory` directly
    /// in `MemoryMirrorEntry`, scoped to (and reclaimed together with) each
    /// realm's own `HostState` -- see `crate::memory`.
    ///
    /// Practically, this limitation does not matter for `lexe`
    /// today: production creates exactly one top-level `AsyncContext` per
    /// `AsyncRuntime`, living for the whole process (`Vm::from_options` in
    /// `lexe_core`), and `vm.runInNewContext`'s bare child contexts
    /// (the only other context-creation path, and the only place a `Runtime`
    /// hosts more than one context at a time) never get `WebAssembly`
    /// installed at all. If a future persistent multi-top-level-context
    /// feature (e.g. a stateful `vm.createContext`) is added, per-context
    /// `WebAssembly` cleanup will need an explicit, non-finalizer-based
    /// teardown hook wired through whatever native code creates and
    /// destroys those contexts (mirroring `lexe_vm`'s
    /// `ChildContext::drop`, which frees its `JSContext` from ordinary,
    /// non-reentrant Rust code, not a JS finalizer) rather than relying on
    /// `WasmRealmHolder`'s finalizer at all.
    #[tokio::test]
    async fn context_teardown_defers_realm_removal_until_runtime_teardown() {
        let rt = rquickjs::AsyncRuntime::new().unwrap();

        let realm_id = {
            let ctx1 = rquickjs::AsyncContext::full(&rt).await.unwrap();
            let id = ctx1
                .with(|ctx| -> Result<u64> {
                    let namespace = rquickjs::Object::new(ctx.clone())?;
                    let errors = crate::errors::install(&ctx, &namespace)?;
                    let realm = install(&ctx, errors)?;
                    Ok(realm.state.realm_id)
                })
                .await
                .unwrap();
            let present = ctx1
                .with(|ctx| lookup_in_registry(&ctx, id).is_some())
                .await;
            assert!(
                present,
                "realm must be registered while its context is alive"
            );
            id
            // `ctx1` (the `AsyncContext`, and with it this context's
            // `globalThis` and `WasmRealmHolder`) is dropped here.
        };

        // As documented above: a second, sibling context on the *same*
        // `Runtime`, even after an explicit GC pass, still observes the
        // first context's realm, because rquickjs has not yet finalized its
        // `WasmRealmHolder`. This is the documented, currently-unavoidable
        // behavior -- not the goal of this fix.
        let ctx2 = rquickjs::AsyncContext::full(&rt).await.unwrap();
        ctx2.with(|ctx| ctx.run_gc()).await;
        let still_present = ctx2
            .with(|ctx| lookup_in_registry(&ctx, realm_id).is_some())
            .await;
        assert!(
            still_present,
            "documenting current rquickjs behavior: the realm is not reaped \
             before Runtime teardown -- see this test's doc comment"
        );

        // What *is* guaranteed, and what this fix's `reap_condemned`/
        // `condemned` plumbing must handle safely: once the whole `Runtime`
        // (both contexts, and every class-prototype cache backing them) is
        // torn down, no `gc_obj_list` assertion or other abort occurs, even
        // with a realm still sitting in the registry. `ctx2`, `rt` dropping
        // here exercises exactly that.
    }

    /// Regression test for the `WasmRealmHolder::drop`/`reap_condemned`
    /// split itself (independent of *when* rquickjs happens to finalize a
    /// holder -- see the test above): several realms' holders finalizing in
    /// a batch, all queuing into the same `condemned` list before any reap
    /// runs, must not panic, and `reap_condemned` must remove every one of
    /// them.
    #[tokio::test]
    async fn reap_condemned_removes_every_batched_realm() {
        test_sync_with(|ctx| {
            let mut ids = Vec::new();
            for _ in 0..5 {
                let namespace = rquickjs::Object::new(ctx.clone())?;
                let errors = crate::errors::install(&ctx, &namespace)?;
                let realm = WasmRealm::new(&ctx, errors);
                ids.push(realm.state.realm_id);
                insert_into_registry(&ctx, Rc::new(realm));
            }
            for id in &ids {
                assert!(lookup_in_registry(&ctx, *id).is_some());
            }
            // Simulate all five holders finalizing at once, in a batch,
            // before anything triggers a reap.
            let condemned = condemned_list(&ctx);
            condemned.borrow_mut().extend(ids.iter().copied());
            for id in &ids {
                assert!(
                    lookup_in_registry(&ctx, *id).is_none(),
                    "reap_condemned (triggered by lookup_in_registry's `with_registry` call) \
                     must remove every batched id, not just the first"
                );
            }
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn multiple_realms_on_same_runtime_do_not_collide() {
        // Simulates two independent top-level contexts on the same
        // `Runtime` (the scenario `RealmRegistry` is a `HashMap` for) by
        // installing WebAssembly twice into the *same* context -- this is
        // not something real code does, but it exercises the "registry
        // already exists, insert a second entry" path in
        // `insert_into_registry` without needing a second `AsyncContext`.
        test_sync_with(|ctx| {
            let namespace1 = rquickjs::Object::new(ctx.clone())?;
            let errors1 = crate::errors::install(&ctx, &namespace1)?;
            let realm1 = WasmRealm::new(&ctx, errors1);
            let id1 = realm1.state.realm_id;
            insert_into_registry(&ctx, Rc::new(realm1));

            let namespace2 = rquickjs::Object::new(ctx.clone())?;
            let errors2 = crate::errors::install(&ctx, &namespace2)?;
            let realm2 = WasmRealm::new(&ctx, errors2);
            let id2 = realm2.state.realm_id;
            insert_into_registry(&ctx, Rc::new(realm2));

            assert_ne!(id1, id2);
            assert!(lookup_in_registry(&ctx, id1).is_some());
            assert!(lookup_in_registry(&ctx, id2).is_some());
            Ok(())
        })
        .await;
    }
}
