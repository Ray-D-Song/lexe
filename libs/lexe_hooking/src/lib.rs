// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use lexe_utils::{object::ObjectExt, provider::ProviderType};
use rquickjs::{Ctx, Exception, Function, JsLifetime, Object, Result, Value};

/// Process-wide allocator for async-resource hook IDs.
///
/// All counter-backed async resource providers (TickObject, Microtask,
/// Timeout, ...) share this single counter so their IDs cannot collide in
/// the per-context async-hooks `id_map`. Previously each module kept an
/// independent counter (`TICK_ID` / `MICROTASK_ID` / `TIMER_ID` all started
/// at 1), so a microtask and a nextTick could both receive ID 1 and clobber
/// each other's mapping — causing AsyncLocalStorage context cross-talk
/// (`[["micro","tick"],["tick",null]]` instead of
/// `[["micro","micro"],["tick","tick"]]`).
///
/// `0` is reserved as the "no resource" sentinel returned by `get_id_map`,
/// and `1` is reserved for the bootstrap execution context, so the first
/// usable id is `2`. Pointer-backed providers (Promise / DNS / module
/// loading) keep using raw object pointers as their id; those are large
/// heap addresses and never collide with these small sequential ids.
static HOOK_RESOURCE_ID: AtomicUsize = AtomicUsize::new(1);

/// Allocate a process-wide unique async-resource hook ID (see
/// `HOOK_RESOURCE_ID`).
pub fn allocate_hook_resource_id() -> usize {
    loop {
        let id = HOOK_RESOURCE_ID.fetch_add(1, Ordering::Relaxed);
        if id > 1 {
            return id;
        }
    }
}

/// Per-context flag: true when internal consumers (ALS) exist or user hooks may run.
/// Stored as userdata so promise/timer fast paths avoid string allocations.
pub struct AsyncTrackingState {
    pub active: Cell<bool>,
}

impl Default for AsyncTrackingState {
    fn default() -> Self {
        Self {
            active: Cell::new(false),
        }
    }
}

unsafe impl<'js> JsLifetime<'js> for AsyncTrackingState {
    type Changed<'to> = AsyncTrackingState;
}

pub fn is_async_tracking_active(ctx: &Ctx<'_>) -> bool {
    ctx.userdata::<AsyncTrackingState>()
        .map(|s| s.active.get())
        .unwrap_or(false)
}

pub fn set_async_tracking_active(ctx: &Ctx<'_>, active: bool) {
    if let Some(state) = ctx.userdata::<AsyncTrackingState>() {
        state.active.set(active);
    }
}

#[derive(PartialEq)]
pub enum HookType {
    Init,
    Before,
    After,
}

/// Dispatch async lifecycle events into the runtime dispatcher (when present).
///
/// Init is skipped when no consumers need tracking (cheap early exit). Before/After
/// always reach the dispatcher so the execution stack can be balanced for
/// already-mapped resources.
pub fn invoke_async_hook(
    ctx: &Ctx<'_>,
    hook_type: HookType,
    provider_type: ProviderType,
    uid: usize,
) -> Result<()> {
    if hook_type == HookType::Init && !is_async_tracking_active(ctx) {
        return Ok(());
    }

    let hook_ = match hook_type {
        HookType::Init => "init",
        HookType::Before => "before",
        HookType::After => "after",
    };

    let provider_ = match provider_type {
        ProviderType::None if hook_type != HookType::Init => "",
        ProviderType::None => {
            return Err(Exception::throw_type(
                ctx,
                "Asynchronous types cannot be omitted in init hooks.",
            ))
        },
        ProviderType::Resource(s) => &["Resource(", &s, ")"].concat(),
        // Userland provider types
        ProviderType::Immediate => "Immediate",
        ProviderType::Interval => "Interval",
        ProviderType::MessagePort => "MessagePort",
        ProviderType::Microtask => "Microtask",
        ProviderType::TickObject => "TickObject",
        ProviderType::Timeout => "Timeout",
        // Internal provider types
        ProviderType::FsReqCallback => "FSREQCALLBACK",
        ProviderType::GetAddrInfoReqWrap => "GETADDRINFOREQWRAP",
        ProviderType::GetNameInfoReqWrap => "GETNAMEINFOREQWRAP",
        ProviderType::PipeWrap => "PIPEWRAP",
        ProviderType::StatWatcher => "STATWACHER",
        ProviderType::TcpWrap => "TCPWRAP",
        ProviderType::TimerWrap => "TIMERWRAP",
        ProviderType::TlsWrap => "TLSWRAP",
        ProviderType::UdpWrap => "UDPWRAP",
    };

    let invoke_async_hook = ctx
        .globals()
        .get_optional::<_, Function>("invokeAsyncHook")?;
    if let Some(func) = &invoke_async_hook {
        func.call::<_, ()>((hook_, provider_, uid))?;
    }
    Ok(())
}

/// Register a GC finalizer that removes `uid` from the async id map once
/// `target` is collected.
///
/// `expected_async_id`, when set, is carried through to the finalization
/// callback so it can verify the id map entry for `uid` still belongs to the
/// same async resource before deleting it. Without this check, a resource
/// that is freed and whose finalizer is delayed (FinalizationRegistry
/// callbacks are not synchronous with collection) can have its identity
/// (a raw pointer) reused by a brand-new, unrelated resource; the stale
/// finalizer would then delete the new resource's live mapping. High-churn
/// callers (promises) must pass the assigned async id; low-churn callers
/// (DNS/module loading) may pass `None` to keep the previous unconditional
/// removal behavior.
pub fn register_finalization_registry<'js>(
    ctx: &Ctx<'js>,
    target: Value<'js>,
    uid: usize,
    expected_async_id: Option<u64>,
) -> Result<()> {
    if !is_async_tracking_active(ctx) {
        return Ok(());
    }

    if let Ok(register) =
        ctx.eval::<Function<'js>, &str>("globalThis.asyncFinalizationRegistry.register")
    {
        let held = Object::new(ctx.clone())?;
        held.set("uid", uid)?;
        if let Some(async_id) = expected_async_id {
            held.set("asyncId", async_id)?;
        }
        let _ = register.call::<_, ()>((target, held));
    }
    Ok(())
}
