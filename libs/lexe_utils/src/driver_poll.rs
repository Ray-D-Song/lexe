// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Native N-API driver polling and cross-thread wakeups.
//!
//! The poll hook is stored in a [`OnceLock`] so [`poll_native_drivers`] never
//! holds a mutex across the callback. Per-runtime [`Notify`] handles let worker
//! threads wake the timer loop without touching QuickJS.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use rquickjs::qjs::JSRuntime;
use tokio::sync::Notify;

type DriverPollFn = fn(*mut JSRuntime);
type DriverBusyFn = fn(*mut JSRuntime) -> bool;

static HOOK: std::sync::OnceLock<DriverPollFn> = std::sync::OnceLock::new();
static BUSY_HOOK: std::sync::OnceLock<DriverBusyFn> = std::sync::OnceLock::new();
static DRIVER_NOTIFY: LazyLock<Mutex<HashMap<usize, Arc<Notify>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn set_driver_poll_hook(hook_fn: Option<DriverPollFn>) {
    if let Some(f) = hook_fn {
        let _ = HOOK.set(f);
    }
}

/// Register the hook that reports whether a runtime still has driver work
/// (async work items, referenced threadsafe functions, queued jobs).
pub fn set_driver_busy_hook(hook_fn: Option<DriverBusyFn>) {
    if let Some(f) = hook_fn {
        let _ = BUSY_HOOK.set(f);
    }
}

pub fn driver_notify_for_rt(rt: *mut JSRuntime) -> Arc<Notify> {
    let key = rt as usize;
    let mut guard = DRIVER_NOTIFY.lock().unwrap();
    guard
        .entry(key)
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone()
}

pub fn unregister_driver_notify(rt: *mut JSRuntime) {
    DRIVER_NOTIFY.lock().unwrap().remove(&(rt as usize));
}

/// Wake any timer loop waiting on this runtime. Polling happens on the JS thread.
pub fn wake_native_drivers(rt: *mut JSRuntime) {
    if let Some(notify) = DRIVER_NOTIFY.lock().unwrap().get(&(rt as usize)) {
        notify.notify_one();
    }
}

pub fn poll_native_drivers(rt: *mut JSRuntime) {
    if let Some(f) = HOOK.get() {
        f(rt);
    }
}

/// Whether native drivers still hold work that must finish before exit.
///
/// Driver work runs as spawned runtime tasks, so it never shows up in the
/// event-loop reference count; the drain loop would otherwise stop the process
/// while an addon callback is still pending.
pub fn drivers_busy(rt: *mut JSRuntime) -> bool {
    BUSY_HOOK.get().is_some_and(|f| f(rt))
}
