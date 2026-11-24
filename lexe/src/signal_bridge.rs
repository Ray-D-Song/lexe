// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Bridge OS termination signals into the JS `process` `"signal"` event.
//!
//! Signal streams are polled on a background task, which must never touch
//! QuickJS: `AsyncRuntime::idle` holds the runtime lock while pending. Instead
//! the signal name is queued and the drain loop is woken; [`take_pending_signals`]
//! is then consumed on the JS thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use lexe_utils::driver_poll;
use tokio::sync::{mpsc, Notify};

static SIGNAL_BRIDGE_STARTED: AtomicBool = AtomicBool::new(false);
static PENDING_SIGNALS: LazyLock<Mutex<VecDeque<String>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));
/// Wakes the drain loop out of `Vm::idle` so a queued signal is dispatched.
static SIGNAL_WAKE: LazyLock<Arc<Notify>> = LazyLock::new(|| Arc::new(Notify::new()));

/// Notified whenever an OS signal is queued. Permits are retained, so a signal
/// that arrives between loop turns is not lost.
pub fn wake_handle() -> Arc<Notify> {
    SIGNAL_WAKE.clone()
}

/// Whether a signal arrived but has not been dispatched on the JS thread yet.
pub fn has_pending_signal() -> bool {
    !PENDING_SIGNALS.lock().unwrap().is_empty()
}

/// Pop every signal that arrived since the last event-loop turn.
pub fn take_pending_signals() -> Vec<String> {
    PENDING_SIGNALS.lock().unwrap().drain(..).collect()
}

pub async fn install(
    vm: &lexe_core::vm::Vm,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if SIGNAL_BRIDGE_STARTED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let rt_addr = vm.js_runtime_key();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    tokio::spawn(async move {
        while let Some(name) = rx.recv().await {
            // Node: with no `signal` listener the OS default termination applies,
            // so the process dies by signal (exit code 128+signum) without running
            // `exit` handlers. Doing that here also avoids waiting for the runtime
            // lock, which a parked `idle` can hold indefinitely.
            if lexe_process::lifecycle::signal_listener_count_snapshot() == Some(0) {
                std::process::exit(i32::from(
                    lexe_process::lifecycle::default_exit_code_for_signal(&name),
                ));
            }
            PENDING_SIGNALS.lock().unwrap().push_back(name);
            driver_poll::wake_native_drivers(rt_addr as *mut rquickjs::qjs::JSRuntime);
            SIGNAL_WAKE.notify_one();
        }
    });

    #[cfg(unix)]
    spawn_unix_listeners(tx.clone())?;
    #[cfg(windows)]
    spawn_ctrl_c_listener(tx.clone())?;

    Ok(())
}

#[cfg(unix)]
fn spawn_unix_listeners(
    tx: mpsc::UnboundedSender<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::signal::unix::{signal, SignalKind};

    fn forward(kind: SignalKind, name: &'static str, tx: mpsc::UnboundedSender<String>) {
        tokio::spawn(async move {
            if let Ok(mut stream) = signal(kind) {
                while stream.recv().await.is_some() {
                    if tx.send(name.into()).is_err() {
                        break;
                    }
                }
            }
        });
    }

    forward(SignalKind::interrupt(), "SIGINT", tx.clone());
    forward(SignalKind::terminate(), "SIGTERM", tx.clone());
    forward(SignalKind::hangup(), "SIGHUP", tx);

    Ok(())
}

#[cfg(windows)]
fn spawn_ctrl_c_listener(
    tx: mpsc::UnboundedSender<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::spawn(async move {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                break;
            }
            if tx.send("SIGINT".into()).is_err() {
                break;
            }
        }
    });
    Ok(())
}
