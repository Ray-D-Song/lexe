// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Per-`JSRuntime` reference counting for Node-compatible `ref()` / `unref()` handles.
//!
//! When the count reaches zero, registered quiescence notifiers wake so background
//! tasks (sockets, timer loop) can stop blocking `AsyncRuntime::idle()`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use tokio::sync::Notify;

type RtKey = usize;

pub type RuntimeLoopKey = RtKey;

struct RuntimeLoopState {
    refs: AtomicUsize,
    /// In-flight HTTP request handlers (keeps timer loop polling unref timers).
    http_inflight: AtomicUsize,
    /// Fired whenever `refs` transitions to zero.
    quiescent: Arc<Notify>,
}

impl RuntimeLoopState {
    fn new() -> Self {
        Self {
            refs: AtomicUsize::new(0),
            http_inflight: AtomicUsize::new(0),
            quiescent: Arc::new(Notify::new()),
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

fn increment_ref(rt: RtKey) {
    state_for(rt).refs.fetch_add(1, Ordering::AcqRel);
}

fn decrement_ref(rt: RtKey) {
    let state = state_for(rt);
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

/// Shared notify fired when this runtime's ref count hits zero.
pub fn quiescence_watch(rt: RtKey) -> Arc<Notify> {
    state_for(rt).quiescent.clone()
}

/// A single ref/unref handle (timer, socket, message port, …).
#[derive(Debug)]
pub struct EventLoopRef {
    rt: RtKey,
    referenced: AtomicBool,
}

impl EventLoopRef {
    /// Create a handle that starts referenced (increments the runtime ref count).
    pub fn new_referenced(rt: RtKey) -> Arc<Self> {
        increment_ref(rt);
        Arc::new(Self {
            rt,
            referenced: AtomicBool::new(true),
        })
    }

    /// Create an unreferenced handle (does not increment the runtime ref count).
    pub fn new_unreferenced(rt: RtKey) -> Arc<Self> {
        Arc::new(Self {
            rt,
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
            increment_ref(self.rt);
        }
    }

    pub fn unreference(&self) {
        if self.referenced.swap(false, Ordering::AcqRel) {
            decrement_ref(self.rt);
        }
    }
}

impl Drop for EventLoopRef {
    fn drop(&mut self) {
        if self.referenced.load(Ordering::Acquire) {
            decrement_ref(self.rt);
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
