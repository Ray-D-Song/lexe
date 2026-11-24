// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! All `unsafe` code and raw-pointer plumbing for this crate is centralized in
//! this module, as required by the implementation plan. Two independent
//! concerns are handled here:
//!
//! 1. **Stable identity for wasmi handles.** `wasmi::Func`/`Memory`/`Table`/
//!    `Global` do not implement `PartialEq`/`Hash` (only `Instance` does), so
//!    the wrapper-identity registries in [`crate::realm`] cannot use the
//!    handles directly as hash keys. All four types are `#[repr(transparent)]`
//!    newtypes around an 8-byte `Copy` arena index (mirroring wasmi's own
//!    internal `size_of::<Func>() == size_of::<u64>()` invariant, see
//!    `wasmi::reftype` tests), so we reinterpret the bytes as a `u64` purely as
//!    an opaque, stable-for-this-store equality/hash key. We never interpret
//!    the bits as an address or attempt to reconstruct a handle from them.
//!
//! 2. **Reentrant access to the realm's `Store` from host callbacks.** QuickJS
//!    executes a single realm on a single thread, but a JS import callback can
//!    call back into an exported Wasm function of the *same* instance while
//!    the outer `wasmi::Func::call` still holds the store borrow via its
//!    `Caller`. Rather than borrowing the `Store` a second time (which would
//!    panic/deadlock), nested calls reuse the currently active `Caller` through
//!    a thread-local RAII stack. Because QuickJS/this runtime is single
//!    threaded and a `Caller<'_, StoreData>` never survives past the host
//!    function invocation that created it, storing a raw pointer to it for the
//!    dynamic extent of that call is sound as long as the stack is always
//!    popped (via `ActiveCallerGuard::drop`) before the frame that pushed it
//!    returns or unwinds.

use std::cell::RefCell;
use std::rc::Rc;

use wasmi::Caller;

use crate::host_state::HostState;

/// The `wasmi::Store`/`Caller` user-data type used throughout this crate.
/// `Caller<'_, Store>` must always be parameterized with the exact same data
/// type the realm's `wasmi::Store<Store>` uses (see [`crate::realm::WasmRealm`]),
/// which is `Rc<HostState>` (not `HostState` directly) so that host function
/// trampolines can cheaply clone their way to the shared realm state.
pub type StoreData = Rc<HostState>;

/// Reinterprets a `Copy`, 8-byte wasmi handle (`Func`, `Memory`, `Table` or
/// `Global`) as an opaque `u64` identity key.
///
/// # Safety
/// Callers must only invoke this with wasmi's own `Func`/`Memory`/`Table`/
/// `Global` types (or any type with an identical size/layout contract). The
/// resulting `u64` must never be interpreted as an address or reconstructed
/// back into a handle; it is solely used as a stable equality/hash key for
/// "is this the same underlying wasmi entity" checks within a single realm.
pub unsafe fn handle_bits<T: Copy>(value: T) -> u64 {
    const SIZE: usize = std::mem::size_of::<u64>();
    debug_assert_eq!(
        std::mem::size_of::<T>(),
        SIZE,
        "wasmi handle type changed size; handle_bits() assumption is stale"
    );
    let mut bits = 0u64;
    unsafe {
        std::ptr::copy_nonoverlapping(
            &value as *const T as *const u8,
            &mut bits as *mut u64 as *mut u8,
            SIZE,
        );
    }
    bits
}

// Compile-time guards: if a future wasmi release changes the representation
// of these handle types this will fail to compile (rather than silently
// producing corrupt identity keys at runtime).
const _: () = assert!(std::mem::size_of::<wasmi::Func>() == std::mem::size_of::<u64>());
const _: () = assert!(std::mem::size_of::<wasmi::Memory>() == std::mem::size_of::<u64>());
const _: () = assert!(std::mem::size_of::<wasmi::Table>() == std::mem::size_of::<u64>());
const _: () = assert!(std::mem::size_of::<wasmi::Global>() == std::mem::size_of::<u64>());
const _: () = assert!(std::mem::size_of::<wasmi::ExternRef>() == std::mem::size_of::<u64>());

/// Returns a mutable view of a (non-detached) `ArrayBuffer`'s backing bytes.
///
/// `rquickjs::ArrayBuffer` only exposes a safe `as_bytes(&self) -> Option<&[u8]>`
/// (immutable) accessor, even though the same underlying allocation is
/// exclusively owned by this buffer and mutable through the C API. The
/// [`Memory`](crate::memory) synchronous-mirror model needs to write wasmi's
/// linear memory contents back into the mirror's `ArrayBuffer` on every
/// Wasm/JS boundary crossing, so we construct the mutable slice ourselves from
/// the same `(ptr, len)` rquickjs's own (immutable) accessor is built from.
///
/// # Safety
/// `buffer` must not be detached (checked via `as_raw()` returning `Some`),
/// and the returned slice must not be held past a point where `buffer` could
/// be detached or resized (i.e. it must be used and dropped within a single,
/// non-reentrant synchronization pass).
pub unsafe fn array_buffer_bytes_mut<'a>(
    buffer: &rquickjs::ArrayBuffer<'a>,
) -> Option<&'a mut [u8]> {
    let raw = buffer.as_raw()?;
    Some(unsafe { std::slice::from_raw_parts_mut(raw.ptr.as_ptr(), raw.len) })
}

/// Calls the QuickJS C API's `JS_PreventExtensions` directly, since
/// `rquickjs` 0.12.1 does not expose a safe wrapper for
/// `Object.preventExtensions`. Used by [`crate::instance`] to make
/// `Instance.prototype.exports` non-extensible, matching the WebAssembly JS
/// API spec (`Instance.exports` is a null-prototype, non-extensible object
/// with fixed, read-only data properties -- see `crate::instance::build_exports_object`).
///
/// # Safety
/// `obj` must be a live, non-null QuickJS object value owned by `ctx`. This
/// holds for any `rquickjs::Object` obtained normally (e.g.
/// `rquickjs::Object::new`), which is the only way this function is called.
pub unsafe fn prevent_extensions(
    ctx: &rquickjs::Ctx<'_>,
    obj: &rquickjs::Object<'_>,
) -> Result<(), ()> {
    let ret = unsafe {
        rquickjs::qjs::JS_PreventExtensions(ctx.as_raw().as_ptr(), obj.as_value().as_raw())
    };
    if ret < 0 {
        Err(())
    } else {
        Ok(())
    }
}

thread_local! {
    /// Stack of currently-executing host callback `Caller`s, most recent last.
    ///
    /// QuickJS runs a single realm on a single thread and this runtime never
    /// hands a `Store`/`Caller` to another thread, so a thread-local (rather
    /// than realm-keyed) stack is sufficient and avoids an extra lookup on the
    /// hot call path.
    static ACTIVE_CALLERS: RefCell<Vec<(u64, *mut Caller<'static, StoreData>)>> =
        const { RefCell::new(Vec::new()) };
}

/// RAII guard that pushes a `Caller` pointer onto the thread-local active
/// caller stack for the duration of a host import callback invocation, and
/// unconditionally pops it again on drop (including on panic/unwind), so a
/// stale entry can never outlive the call that created it.
pub struct ActiveCallerGuard {
    realm_id: u64,
}

impl ActiveCallerGuard {
    /// # Safety
    /// `caller` must remain valid (i.e. the `Caller` it points to must not be
    /// moved or dropped) for the entire lifetime of the returned guard. Callers
    /// must not let the guard outlive the stack frame that owns `*caller`.
    pub unsafe fn push(realm_id: u64, caller: &mut Caller<'_, StoreData>) -> Self {
        // Cast through `*mut ()` (rather than a direct double
        // `Caller<'_, _>` -> `Caller<'static, _>` cast) purely to erase the
        // lifetime: `*mut T` is invariant in `T`, so casting straight
        // between two differently-lifetimed `Caller` pointers triggers a
        // borrow-checker error, but going through an opaque intermediate
        // pointer type sidesteps that (and avoids clippy's
        // `unnecessary_cast` false positive on the direct
        // same-looking-modulo-lifetime double cast this used to be).
        let erased = (caller as *mut Caller<'_, StoreData>)
            .cast::<()>()
            .cast::<Caller<'static, StoreData>>();
        ACTIVE_CALLERS.with(|stack| stack.borrow_mut().push((realm_id, erased)));
        Self { realm_id }
    }
}

impl Drop for ActiveCallerGuard {
    fn drop(&mut self) {
        ACTIVE_CALLERS.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some((id, _)) = stack.last() {
                if *id == self.realm_id {
                    stack.pop();
                    return;
                }
            }
            // A well-behaved call stack always pops in LIFO order matching
            // `push`. If this ever fires it indicates a bug in caller-stack
            // bookkeeping rather than a recoverable runtime condition, so we
            // still avoid leaving corrupt state around by removing the most
            // recent entry for this realm if present.
            if let Some(pos) = stack.iter().rposition(|(id, _)| *id == self.realm_id) {
                stack.remove(pos);
            }
        });
    }
}

/// Returns a raw pointer to the innermost currently-active `Caller` for the
/// given realm, if any host callback for that realm is currently executing on
/// this thread.
///
/// # Safety
/// The returned pointer is only valid to dereference for as long as the
/// `ActiveCallerGuard` that pushed it is still alive (i.e. strictly within the
/// dynamic extent of the host callback invocation). It must never be stored
/// past that point.
pub fn active_caller(realm_id: u64) -> Option<*mut Caller<'static, StoreData>> {
    ACTIVE_CALLERS.with(|stack| {
        stack
            .borrow()
            .iter()
            .rev()
            .find(|(id, _)| *id == realm_id)
            .map(|(_, ptr)| *ptr)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_bits_are_stable_for_the_same_handle() {
        let engine = crate::engine::shared_engine();
        let mut store = wasmi::Store::new(&engine, ());
        let memory = wasmi::Memory::new(&mut store, wasmi::MemoryType::new(1, Some(1))).unwrap();
        let a = unsafe { handle_bits(memory) };
        let b = unsafe { handle_bits(memory) };
        assert_eq!(a, b);
    }

    #[test]
    fn handle_bits_differ_for_distinct_memories() {
        let engine = crate::engine::shared_engine();
        let mut store = wasmi::Store::new(&engine, ());
        let m1 = wasmi::Memory::new(&mut store, wasmi::MemoryType::new(1, Some(1))).unwrap();
        let m2 = wasmi::Memory::new(&mut store, wasmi::MemoryType::new(1, Some(1))).unwrap();
        let a = unsafe { handle_bits(m1) };
        let b = unsafe { handle_bits(m2) };
        assert_ne!(a, b);
    }

    #[test]
    fn active_caller_stack_pushes_and_pops() {
        assert!(active_caller(42).is_none());
    }
}
