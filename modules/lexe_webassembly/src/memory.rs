// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Memory` and the synchronous-mirror model that keeps a
//! QuickJS-owned `ArrayBuffer` and wasmi's own linear memory storage in sync
//! across every JS/Wasm boundary crossing, per the implementation plan.
//!
//! wasm32, non-shared memory only (first version scope): `shared: true`,
//! `address: "i64"`, or an `initial`/`maximum` outside the 32-bit wasm page
//! limit (65536 pages / 4 GiB) are all rejected with a `TypeError`/`RangeError`
//! at construction time.
//!
//! Every function below that needs to touch the `wasmi::Store` takes a
//! `&mut dyn wasmi::AsContextMut<Data = Rc<HostState>>` rather than a
//! concrete `Store`/`Caller` type, so the same code path works whether it is
//! reached from a top-level JS call (via [`crate::realm::with_context_mut`]
//! borrowing the realm's `Store` directly) or reentrantly from inside a host
//! import callback (where it is instead given the callback's live
//! `wasmi::Caller`; see `crate::store_access` and `crate::realm`).

use std::rc::Rc;

use rquickjs::{
    atom::PredefinedAtom, class::Trace, ArrayBuffer, Class, Ctx, IntoJs, Object, Persistent,
    Result, Value,
};
use wasmi::{AsContextMut, Memory, MemoryType};

use crate::host_state::{HostState, WrapKind};

/// Upper bound on Wasm32 linear memory pages (4 GiB / 64 KiB).
const MAX_WASM32_PAGES: u64 = 1 << 16;

/// Bookkeeping for one materialized `Memory.prototype.buffer` mirror. Kept in
/// [`HostState::memory_mirrors`], keyed by [`crate::store_access::handle_bits`]
/// of the underlying `wasmi::Memory`, so boundary-crossing sync passes only
/// ever touch memories that JS code has actually observed via `.buffer`.
///
/// Stores the `wasmi::Memory` handle itself (a small `Copy` handle into the
/// `Store`, not the memory's actual bytes) alongside the mirror so that the
/// boundary-crossing sync passes (`sync_all_js_to_wasm`/`sync_all_wasm_to_js`)
/// can recover it directly from `HostState::memory_mirrors` -- which is
/// itself dropped, along with every entry in it, whenever the owning realm
/// is torn down (see `crate::realm`). An earlier version of this module
/// instead kept a *separate*, thread-local `bits -> Memory` map for this
/// purpose; being thread-local (not realm-scoped) it had no cleanup path at
/// all and grew for the lifetime of the thread across every realm ever
/// created on it, independent of any individual realm's own teardown.
pub struct MemoryMirrorEntry {
    buffer: Persistent<Value<'static>>,
    byte_len: usize,
    memory: Memory,
}

#[derive(rquickjs::JsLifetime)]
#[rquickjs::class(rename = "Memory")]
pub struct WasmMemory {
    pub(crate) realm_id: u64,
    pub(crate) handle: Memory,
}

impl<'js> Trace<'js> for WasmMemory {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

fn parse_memory_descriptor<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    descriptor: &Object<'js>,
) -> Result<MemoryType> {
    let initial_value =
        crate::descriptor::get_required(ctx, host, descriptor, "initial", "WebAssembly.Memory")?;
    let initial = crate::descriptor::to_u32_enforce_range(ctx, host, initial_value, "initial")?;
    let maximum = crate::descriptor::optional_u32_enforce_range(ctx, host, descriptor, "maximum")?;
    let shared = crate::descriptor::optional_bool(ctx, descriptor, "shared", false)?;
    let address = crate::descriptor::optional_string(ctx, descriptor, "address")?;

    if shared {
        return Err(host.throw_type_error(ctx, "shared memory is not supported"));
    }
    if matches!(address.as_deref(), Some("i64")) {
        return Err(host.throw_type_error(ctx, "64-bit ('i64') memory addressing is not supported"));
    }
    if u64::from(initial) > MAX_WASM32_PAGES {
        return Err(host.throw_range_error(ctx, "initial memory size exceeds the wasm32 limit"));
    }
    if let Some(max) = maximum {
        if u64::from(max) > MAX_WASM32_PAGES {
            return Err(host.throw_range_error(ctx, "maximum memory size exceeds the wasm32 limit"));
        }
        if max < initial {
            return Err(
                host.throw_range_error(ctx, "maximum memory size is smaller than initial size")
            );
        }
    }
    Ok(MemoryType::new(initial, maximum))
}

#[rquickjs::methods]
impl WasmMemory {
    #[qjs(constructor)]
    pub fn new<'js>(ctx: Ctx<'js>, descriptor: Object<'js>) -> Result<Self> {
        let realm = crate::realm::realm(&ctx)?;
        let host = realm.state.clone();
        let ty = parse_memory_descriptor(&ctx, &host, &descriptor)?;
        let handle =
            crate::realm::with_context_mut(&realm, |store| Memory::new(store.as_context_mut(), ty))
                .map_err(|err| host.throw_range_error(&ctx, err.to_string()))?;
        Ok(Self {
            realm_id: host.realm_id,
            handle,
        })
    }

    #[qjs(get)]
    pub fn buffer<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        crate::realm::with_context_mut(&realm, |store| materialize(&ctx, &host, store, self.handle))
    }

    pub fn grow(&self, ctx: Ctx<'_>, delta: u32) -> Result<u32> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        crate::realm::with_context_mut(&realm, |store| grow(&ctx, &host, store, self.handle, delta))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "WebAssembly.Memory"
    }
}

fn require_same_realm(ctx: &Ctx<'_>, realm_id: u64) -> Result<Rc<crate::realm::WasmRealm>> {
    let realm = crate::realm::realm(ctx)?;
    if realm.state.realm_id != realm_id {
        return Err(realm
            .state
            .throw_link_error(ctx, "Memory belongs to a different realm"));
    }
    Ok(realm)
}

/// Wraps `memory` (an export, or a Memory reachable via `Instance.exports`) as
/// a `WebAssembly.Memory` JS wrapper, reusing the cached wrapper if one
/// already exists for this exact `wasmi::Memory` in this realm.
pub fn wrap_memory<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    memory: Memory,
) -> Result<Class<'js, WasmMemory>> {
    let bits = unsafe { crate::store_access::handle_bits(memory) };
    if let Some(existing) = host.cached_wrapper(ctx, WrapKind::Memory, bits) {
        if let Ok(class) = Class::<WasmMemory>::from_value(&existing) {
            return Ok(class);
        }
    }
    let instance = Class::instance(
        ctx.clone(),
        WasmMemory {
            realm_id: host.realm_id,
            handle: memory,
        },
    )?;
    host.cache_wrapper(WrapKind::Memory, bits, instance.clone().into_js(ctx)?);
    Ok(instance)
}

/// Returns `true` if `memory` has a materialized `ArrayBuffer` mirror in this
/// realm already (i.e. JS code has previously observed `.buffer`).
///
/// Only ever used from this module's own tests, to assert the plan's "只同步
/// 已 materialize 的 Memory" laziness invariant directly; `sync_all_js_to_wasm`/
/// `sync_all_wasm_to_js` get the same laziness "for free" by iterating
/// `host.memory_mirrors`'s keys rather than by calling this on every live
/// `Memory`.
#[cfg(test)]
pub(crate) fn is_materialized(host: &HostState, memory: Memory) -> bool {
    let bits = unsafe { crate::store_access::handle_bits(memory) };
    host.memory_mirrors.borrow().contains_key(&bits)
}

/// Test-only fault injection: replaces `memory`'s already-materialized
/// mirror entry with one whose `buffer` is not actually an `ArrayBuffer`,
/// which deterministically makes the next `sync_all_js_to_wasm`/
/// `sync_all_wasm_to_js` pass that touches it fail with a `RuntimeError`
/// (see `copy_js_to_wasm_one`/`copy_wasm_to_existing_mirror`'s
/// `ArrayBuffer::from_value(..).ok_or_else(..)` check).
///
/// Used by `crate::instance`'s
/// `callback_exception_identity_wins_over_injected_sync_failure` test to
/// verify the P2 fix's exact ordering requirement: a JS import callback's
/// original thrown value must be captured *before* the best-effort
/// Wasm-boundary resync runs, so a resync failure like this one can never
/// clobber it.
#[cfg(test)]
pub(crate) fn corrupt_mirror_for_test(ctx: &Ctx<'_>, host: &HostState, memory: Memory) {
    let bits = unsafe { crate::store_access::handle_bits(memory) };
    let not_an_array_buffer: Value = Object::new(ctx.clone()).unwrap().into_value();
    let mut mirrors = host.memory_mirrors.borrow_mut();
    let entry = mirrors.get_mut(&bits).expect(
        "memory must already be materialized (.buffer accessed) before injecting this fault",
    );
    entry.buffer = Persistent::save(ctx, not_an_array_buffer);
}

/// Returns (and lazily creates) the `ArrayBuffer` mirror for `memory`,
/// synchronizing it with wasmi's current bytes first.
pub fn materialize<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    memory: Memory,
) -> Result<Value<'js>> {
    let bits = unsafe { crate::store_access::handle_bits(memory) };
    let current_len = memory.data_size(store.as_context());

    let existing = host
        .memory_mirrors
        .borrow()
        .get(&bits)
        .map(|entry| entry.byte_len);
    if let Some(byte_len) = existing {
        if byte_len == current_len {
            // Already materialized at the right size: just copy fresh bytes in.
            copy_wasm_to_existing_mirror(ctx, host, store, bits, memory)?;
        } else {
            recreate_mirror(ctx, host, store, bits, memory, current_len)?;
        }
    } else {
        recreate_mirror(ctx, host, store, bits, memory, current_len)?;
    }

    let persistent = host
        .memory_mirrors
        .borrow()
        .get(&bits)
        .expect("mirror was just created")
        .buffer
        .clone();
    persistent.restore(ctx)
}

fn copy_wasm_to_existing_mirror(
    ctx: &Ctx<'_>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    bits: u64,
    memory: Memory,
) -> Result<()> {
    let persistent = host
        .memory_mirrors
        .borrow()
        .get(&bits)
        .expect("checked by caller")
        .buffer
        .clone();
    let value = persistent.restore(ctx)?;
    let buffer = ArrayBuffer::from_value(value)
        .ok_or_else(|| host.throw_runtime_error(ctx, "Memory mirror was not an ArrayBuffer"))?;
    let wasm_bytes = memory.data(store.as_context());
    if let Some(mirror_bytes) = unsafe { crate::store_access::array_buffer_bytes_mut(&buffer) } {
        mirror_bytes.copy_from_slice(wasm_bytes);
    }
    Ok(())
}

fn recreate_mirror(
    ctx: &Ctx<'_>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    bits: u64,
    memory: Memory,
    new_len: usize,
) -> Result<()> {
    // Detach the previous JS-visible buffer (if any) per the Memory.grow /
    // memory.grow semantics: any live TypedArray views over the old buffer
    // must observe `byteLength === 0` after this point.
    if let Some(entry) = host.memory_mirrors.borrow_mut().remove(&bits) {
        if let Ok(value) = entry.buffer.restore(ctx) {
            if let Some(mut old) = ArrayBuffer::from_value(value) {
                old.detach();
            }
        }
    }
    let bytes = memory.data(store.as_context());
    debug_assert_eq!(bytes.len(), new_len);
    // Deliberately `ArrayBuffer::new_copy` (which lets QuickJS's own
    // allocator own the backing bytes) rather than `ArrayBuffer::new` (which
    // hands QuickJS a Rust-`Vec`-backed buffer freed via a custom
    // `drop_raw::<u8>` finalizer callback). `ArrayBuffer::detach()` (used
    // below and in `Memory.prototype.grow`) does not clear that custom
    // finalizer, so a *later* real GC finalization of an already-detached
    // `ArrayBuffer::new`-created buffer calls `drop_raw` a second time with a
    // null/stale pointer -- an actual double-free, reproduced directly
    // against upstream rquickjs 0.12.1 in this module's
    // `bisect_detach_new_array_buffer_then_drop` test during development.
    // `new_copy`'s buffers are owned entirely by QuickJS's own allocator, so
    // detach + eventual finalization is exactly the same well-tested path any
    // ordinary `new ArrayBuffer(n)` from JS already goes through.
    let buffer = ArrayBuffer::new_copy(ctx.clone(), bytes)?;
    let value: Value = buffer.into_value();
    host.memory_mirrors.borrow_mut().insert(
        bits,
        MemoryMirrorEntry {
            buffer: Persistent::save(ctx, value),
            byte_len: new_len,
            memory,
        },
    );
    Ok(())
}

/// `Memory.prototype.grow`: writes back any pending mirror edits, grows the
/// underlying wasmi memory, detaches the old mirror and materializes a new
/// one (even for `grow(0)`, per spec/Node semantics), and returns the previous
/// size in pages.
pub fn grow(
    ctx: &Ctx<'_>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    memory: Memory,
    delta: u32,
) -> Result<u32> {
    let bits = unsafe { crate::store_access::handle_bits(memory) };
    if host.memory_mirrors.borrow().contains_key(&bits) {
        copy_js_to_wasm_one(ctx, host, store, bits, memory)?;
    }
    let old_pages = memory.size(store.as_context());
    memory
        .grow(store.as_context_mut(), u64::from(delta))
        .map_err(|err| host.throw_range_error(ctx, err.to_string()))?;
    let new_len = memory.data_size(store.as_context());
    recreate_mirror(ctx, host, store, bits, memory, new_len)?;
    Ok(old_pages as u32)
}

fn copy_js_to_wasm_one(
    ctx: &Ctx<'_>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    bits: u64,
    memory: Memory,
) -> Result<()> {
    let persistent = host
        .memory_mirrors
        .borrow()
        .get(&bits)
        .expect("checked by caller")
        .buffer
        .clone();
    let value = persistent.restore(ctx)?;
    let buffer = ArrayBuffer::from_value(value)
        .ok_or_else(|| host.throw_runtime_error(ctx, "Memory mirror was not an ArrayBuffer"))?;
    if let Some(bytes) = buffer.as_bytes() {
        let bytes = bytes.to_vec();
        memory
            .write(store.as_context_mut(), 0, &bytes)
            .map_err(|err| host.throw_runtime_error(ctx, err.to_string()))?;
    }
    Ok(())
}

/// Copies every materialized mirror's current JS-visible bytes back into
/// wasmi memory. Called immediately before crossing from JS into Wasm
/// (calling an exported function, or resuming after a host import callback).
pub fn sync_all_js_to_wasm(
    ctx: &Ctx<'_>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
) -> Result<()> {
    let entries: Vec<(u64, Memory)> = host
        .memory_mirrors
        .borrow()
        .iter()
        .map(|(bits, entry)| (*bits, entry.memory))
        .collect();
    for (bits, memory) in entries {
        copy_js_to_wasm_one(ctx, host, store, bits, memory)?;
    }
    Ok(())
}

/// Copies wasmi memory's current bytes back into every materialized mirror,
/// recreating (and detaching the old) buffer if the memory was grown from
/// inside Wasm execution. Called immediately after crossing from Wasm back
/// into JS (returning from an exported function call, or entering a host
/// import callback).
pub fn sync_all_wasm_to_js(
    ctx: &Ctx<'_>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
) -> Result<()> {
    let entries: Vec<(u64, Memory)> = host
        .memory_mirrors
        .borrow()
        .iter()
        .map(|(bits, entry)| (*bits, entry.memory))
        .collect();
    for (bits, memory) in entries {
        let current_len = memory.data_size(store.as_context());
        let existing_len = host.memory_mirrors.borrow().get(&bits).map(|e| e.byte_len);
        if existing_len == Some(current_len) {
            copy_wasm_to_existing_mirror(ctx, host, store, bits, memory)?;
        } else {
            recreate_mirror(ctx, host, store, bits, memory, current_len)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_sync_with;

    fn setup(ctx: &Ctx<'_>) -> Rc<crate::realm::WasmRealm> {
        let namespace = rquickjs::Object::new(ctx.clone()).unwrap();
        let errors = crate::errors::install(ctx, &namespace).unwrap();
        crate::realm::install(ctx, errors).unwrap()
    }

    #[tokio::test]
    async fn buffer_identity_is_stable_across_repeated_access() {
        test_sync_with(|ctx| {
            let realm = setup(&ctx);
            let memory = crate::realm::with_context_mut(&realm, |store| {
                Memory::new(store.as_context_mut(), MemoryType::new(1, Some(1)))
            })
            .unwrap();

            let b1 = realm.state.clone();
            let b1 = crate::realm::with_context_mut(&realm, |store| {
                materialize(&ctx, &b1, store, memory)
            })?;
            let b2 = realm.state.clone();
            let b2 = crate::realm::with_context_mut(&realm, |store| {
                materialize(&ctx, &b2, store, memory)
            })?;
            let ptr1 = unsafe { rquickjs::qjs::JS_VALUE_GET_PTR(b1.as_raw()) };
            let ptr2 = unsafe { rquickjs::qjs::JS_VALUE_GET_PTR(b2.as_raw()) };
            assert_eq!(
                ptr1, ptr2,
                "repeated .buffer access must return the same object"
            );
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn grow_detaches_old_buffer_and_preserves_bytes() {
        test_sync_with(|ctx| {
            let realm = setup(&ctx);
            let memory = crate::realm::with_context_mut(&realm, |store| {
                Memory::new(store.as_context_mut(), MemoryType::new(1, Some(2)))
            })
            .unwrap();
            crate::realm::with_context_mut(&realm, |store| {
                memory.write(store.as_context_mut(), 0, b"hello").unwrap();
            });

            let host = realm.state.clone();
            let old_buffer = crate::realm::with_context_mut(&realm, |store| {
                materialize(&ctx, &host, store, memory)
            })?;
            let old_array_buffer = ArrayBuffer::from_value(old_buffer).unwrap();
            assert_eq!(old_array_buffer.len(), 65536);

            let host = realm.state.clone();
            let old_pages = crate::realm::with_context_mut(&realm, |store| {
                grow(&ctx, &host, store, memory, 1)
            })?;
            assert_eq!(old_pages, 1);
            assert!(
                old_array_buffer.as_bytes().is_none(),
                "old buffer must be detached"
            );

            let host = realm.state.clone();
            let new_buffer = crate::realm::with_context_mut(&realm, |store| {
                materialize(&ctx, &host, store, memory)
            })?;
            let new_array_buffer = ArrayBuffer::from_value(new_buffer).unwrap();
            assert_eq!(new_array_buffer.len(), 131072);
            assert_eq!(&new_array_buffer.as_bytes().unwrap()[0..5], b"hello");
            Ok(())
        })
        .await;
    }

    /// Per the implementation plan: "只同步已 materialize 的 Memory，避免为
    /// 从未访问 .buffer 的内存产生复制" -- a freshly created `Memory` must
    /// not be considered materialized (and therefore not participate in
    /// `sync_all_js_to_wasm`/`sync_all_wasm_to_js`'s copy loop) until JS
    /// code actually observes its `.buffer`.
    /// A throwing `initial`/`maximum`/`shared` descriptor getter must
    /// propagate with its exact original identity, not get swallowed by
    /// `.ok()`/`.unwrap_or(...)`-style optional-property reading (which
    /// would let construction spuriously succeed) nor get replaced by a
    /// synthetic `TypeError`.
    #[tokio::test]
    async fn descriptor_getter_thrown_value_identity_is_preserved() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            for key in ["initial", "maximum", "shared", "address"] {
                let ok: bool = ctx.eval(format!(
                    r#"
                    (() => {{
                        const thrown = {{}};
                        const descriptor = {{
                            initial: 1,
                            get {key}() {{ throw thrown; }},
                        }};
                        try {{
                            new WebAssembly.Memory(descriptor);
                            return false;
                        }} catch (e) {{
                            return e === thrown;
                        }}
                    }})()
                    "#
                ))?;
                assert!(
                    ok,
                    "descriptor.{key} getter's thrown value must propagate as-is"
                );
            }
            Ok(())
        })
        .await;
    }

    /// Regression tests for the reviewed descriptor-coercion bug:
    /// `initial`/`maximum` must go through the WebIDL `[EnforceRange]
    /// unsigned long` algorithm (`ToNumber` then range/finiteness checks),
    /// not `rquickjs`'s strict, JS-`FromJs`-derived numeric conversion --
    /// e.g. a numeric-looking string must coerce, while `NaN` must be
    /// rejected outright (not silently accepted as `0`).
    #[tokio::test]
    async fn initial_and_maximum_use_enforce_range_number_coercion() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    // A numeric string coerces via `ToNumber`, matching Node.
                    const m = new WebAssembly.Memory({ initial: "1", maximum: "2" });
                    return m.buffer.byteLength === 65536;
                })()
                "#,
            )?;
            assert!(
                ok,
                "a numeric string 'initial'/'maximum' must coerce like Node"
            );

            let threw: bool = ctx.eval(
                r#"
                (() => {
                    try {
                        new WebAssembly.Memory({ initial: NaN });
                        return false;
                    } catch (e) {
                        return e instanceof TypeError;
                    }
                })()
                "#,
            )?;
            assert!(
                threw,
                "'initial: NaN' must throw TypeError, not be silently accepted as 0"
            );
            Ok(())
        })
        .await;
    }

    /// Regression test for the P2 fix removing the old thread-local
    /// `MEMORY_BY_BITS` reverse-lookup map (which had no cleanup path and
    /// grew for the lifetime of the thread, across every realm ever created
    /// on it): repeatedly creating a realm, materializing a `Memory`'s
    /// `.buffer`, and dropping the realm (without ever registering it in
    /// `crate::realm`'s `Runtime`-scoped registry) must not panic and must
    /// not accumulate any global/thread-local state -- `MemoryMirrorEntry`
    /// now owns the `wasmi::Memory` handle directly, so this is scoped
    /// entirely to each realm's own `HostState` and needs no separate
    /// bookkeeping to clean up.
    #[tokio::test]
    async fn repeated_realm_creation_and_teardown_with_materialized_memory_does_not_panic() {
        test_sync_with(|ctx| {
            for _ in 0..50 {
                let namespace = rquickjs::Object::new(ctx.clone())?;
                let errors = crate::errors::install(&ctx, &namespace)?;
                let realm = std::rc::Rc::new(crate::realm::WasmRealm::new(&ctx, errors));
                let host = realm.state.clone();
                let memory = crate::realm::with_context_mut(&realm, |store| {
                    Memory::new(store.as_context_mut(), MemoryType::new(1, Some(1)))
                })
                .unwrap();
                crate::realm::with_context_mut(&realm, |store| {
                    materialize(&ctx, &host, store, memory)
                })?;
                assert!(is_materialized(&host, memory));
                // `realm` (and, with it, `host`/`HostState::memory_mirrors`)
                // is dropped here at the end of this iteration.
            }
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn memory_is_not_materialized_until_buffer_is_accessed() {
        test_sync_with(|ctx| {
            let realm = setup(&ctx);
            let host = realm.state.clone();
            let memory = crate::realm::with_context_mut(&realm, |store| {
                Memory::new(store.as_context_mut(), MemoryType::new(1, Some(1)))
            })
            .unwrap();

            assert!(
                !is_materialized(&host, memory),
                "must not be materialized before first .buffer access"
            );
            crate::realm::with_context_mut(&realm, |store| {
                materialize(&ctx, &host, store, memory)
            })?;
            assert!(
                is_materialized(&host, memory),
                "must be materialized after .buffer access"
            );
            Ok(())
        })
        .await;
    }
}
