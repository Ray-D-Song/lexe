// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Per-`JSRuntime` reference counting for Node-compatible `ref()` / `unref()` handles.
//!
//! When the count reaches zero, registered quiescence notifiers wake so background
//! tasks (sockets, timer loop) can stop blocking `AsyncRuntime::idle()`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use tokio::sync::Notify;

type RtKey = usize;

pub type RuntimeLoopKey = RtKey;

static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

struct RuntimeLoopState {
    refs: AtomicUsize,
    /// In-flight HTTP request handlers (keeps timer loop polling unref timers).
    http_inflight: AtomicUsize,
    /// Fired whenever `refs` transitions to zero.
    quiescent: Arc<Notify>,
    /// Active referenced handle kinds for `process.getActiveResourcesInfo()`.
    active: Mutex<HashMap<u64, &'static str>>,
}

impl RuntimeLoopState {
    fn new() -> Self {
        Self {
            refs: AtomicUsize::new(0),
            http_inflight: AtomicUsize::new(0),
            quiescent: Arc::new(Notify::new()),
            active: Mutex::new(HashMap::new()),
        }
    }
}

static RUNTIMES: LazyLock<Mutex<HashMap<RtKey, Arc<RuntimeLoopState>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn state_for(rt: RtKey) -> Arc<RuntimeLoopState> {
    let mut guard = RUNTIMES.lock().unwrap();
    guard
        .entry(rt)
        .or_insert_with(|| Arc::new(RuntimeLoopState::new()))
        .clone()
}

pub fn unregister_runtime(rt: RtKey) {
    RUNTIMES.lock().unwrap().remove(&rt);
    crate::perf::unregister_runtime(rt);
}

pub fn rt_key_from_ptr(rt: *mut std::ffi::c_void) -> RtKey {
    rt as RtKey
}

pub fn has_refed_handles(rt: RtKey) -> bool {
    state_for(rt).refs.load(Ordering::Acquire) > 0
}

pub fn should_keep_process_alive(rt: RtKey) -> bool {
    has_refed_handles(rt) || has_http_inflight(rt)
}

pub fn has_http_inflight(rt: RtKey) -> bool {
    state_for(rt).http_inflight.load(Ordering::Acquire) > 0
}

/// Track active HTTP `request` handlers so unref timers still fire during SSR.
pub fn adjust_http_inflight(rt: RtKey, delta: i32) {
    let state = state_for(rt);
    if delta >= 0 {
        state
            .http_inflight
            .fetch_add(delta as usize, Ordering::AcqRel);
    } else {
        state
            .http_inflight
            .fetch_sub((-delta) as usize, Ordering::AcqRel);
    }
    crate::driver_poll::wake_native_drivers(rt as *mut rquickjs::qjs::JSRuntime);
}

/// Whether a handle and the runtime as a whole still require background I/O tasks.
pub fn keeps_event_loop_alive(loop_ref: &EventLoopRef) -> bool {
    loop_ref.is_referenced() || has_refed_handles(loop_ref.rt())
}

/// Completes when [`keeps_event_loop_alive`] becomes false for this handle.
pub async fn wait_until_io_unref(loop_ref: Arc<EventLoopRef>) {
    let quiescence = quiescence_watch(loop_ref.rt());
    loop {
        if !keeps_event_loop_alive(&loop_ref) {
            return;
        }
        quiescence.notified().await;
    }
}

fn increment_ref(rt: RtKey, id: u64, kind: &'static str) {
    let state = state_for(rt);
    state.refs.fetch_add(1, Ordering::AcqRel);
    state.active.lock().unwrap().insert(id, kind);
}

fn decrement_ref(rt: RtKey, id: u64) {
    let state = state_for(rt);
    state.active.lock().unwrap().remove(&id);
    let prev = state
        .refs
        .try_update(Ordering::AcqRel, Ordering::Acquire, |v| {
            Some(v.saturating_sub(1))
        })
        .expect("try_update");
    if prev == 1 {
        state.quiescent.notify_waiters();
        // SAFETY: RtKey is the JSRuntime pointer bitcast from callers.
        crate::driver_poll::wake_native_drivers(rt as *mut rquickjs::qjs::JSRuntime);
    }
}

/// Active referenced resource names for `process.getActiveResourcesInfo()`.
pub fn active_resource_names(rt: RtKey) -> Vec<&'static str> {
    let state = state_for(rt);
    let guard = state.active.lock().unwrap();
    let mut entries: Vec<(u64, &'static str)> = guard.iter().map(|(&id, &k)| (id, k)).collect();
    entries.sort_unstable_by_key(|(id, _)| *id);
    entries.into_iter().map(|(_, k)| k).collect()
}

/// Shared notify fired when this runtime's ref count hits zero.
pub fn quiescence_watch(rt: RtKey) -> Arc<Notify> {
    state_for(rt).quiescent.clone()
}

/// A single ref/unref handle (timer, socket, message port, …).
#[derive(Debug)]
pub struct EventLoopRef {
    id: u64,
    rt: RtKey,
    kind: &'static str,
    referenced: AtomicBool,
}

impl EventLoopRef {
    /// Create a handle that starts referenced (increments the runtime ref count).
    pub fn new_referenced(rt: RtKey) -> Arc<Self> {
        Self::new_referenced_named(rt, "Handle")
    }

    /// Create a named handle that starts referenced.
    pub fn new_referenced_named(rt: RtKey, kind: &'static str) -> Arc<Self> {
        let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed);
        increment_ref(rt, id, kind);
        Arc::new(Self {
            id,
            rt,
            kind,
            referenced: AtomicBool::new(true),
        })
    }

    /// Create an unreferenced handle (does not increment the runtime ref count).
    pub fn new_unreferenced(rt: RtKey) -> Arc<Self> {
        Self::new_unreferenced_named(rt, "Handle")
    }

    /// Create a named unreferenced handle.
    pub fn new_unreferenced_named(rt: RtKey, kind: &'static str) -> Arc<Self> {
        let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed);
        Arc::new(Self {
            id,
            rt,
            kind,
            referenced: AtomicBool::new(false),
        })
    }

    pub fn is_referenced(&self) -> bool {
        self.referenced.load(Ordering::Acquire)
    }

    pub fn rt(&self) -> RtKey {
        self.rt
    }

    pub fn reference(&self) {
        if !self.referenced.swap(true, Ordering::AcqRel) {
            increment_ref(self.rt, self.id, self.kind);
        }
    }

    pub fn unreference(&self) {
        if self.referenced.swap(false, Ordering::AcqRel) {
            decrement_ref(self.rt, self.id);
        }
    }
}

impl Drop for EventLoopRef {
    fn drop(&mut self) {
        if self.referenced.load(Ordering::Acquire) {
            decrement_ref(self.rt, self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_counting_and_quiescence() {
        let rt = 1usize;
        assert!(!has_refed_handles(rt));

        let a = EventLoopRef::new_referenced(rt);
        assert!(has_refed_handles(rt));

        a.unreference();
        assert!(!has_refed_handles(rt));

        a.reference();
        assert!(has_refed_handles(rt));
        drop(a);
        assert!(!has_refed_handles(rt));
    }
}
