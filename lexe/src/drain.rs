// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible process shutdown: drain the event loop, emit `beforeExit`,
//! drain again, then emit `exit` — the process only leaves through an explicit
//! `process.exit()`/signal default, never through `std::process::exit` here.

use std::sync::atomic::Ordering;

use lexe_core::vm::Vm;
use lexe_process::lifecycle::{
    default_exit_code_for_signal, emit_before_exit, emit_os_signal, emit_process_exit,
    signal_listener_count,
};
use lexe_process::EXIT_CODE;
use lexe_utils::{driver_poll, event_loop::quiescence_watch};
use rquickjs::{qjs, Ctx};
use tokio::time::{sleep, Duration};

#[cfg(not(target_family = "wasm"))]
use crate::signal_bridge;

/// Set when a signal arrived without a JS listener: Node terminates with
/// `128 + signum` after emitting `exit`.
static SIGNAL_EXIT_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub async fn run_until_process_exit(
    vm: &Vm,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rt_key = vm.js_runtime_key();

    loop {
        if schedule_signals(vm).await? {
            continue;
        }

        run_event_loop(vm, rt_key).await?;

        if schedule_signals(vm).await? {
            continue;
        }

        if SIGNAL_EXIT_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        if stay_alive(vm, rt_key).await? {
            continue;
        }

        vm.ctx
            .with(|ctx| emit_before_exit(&ctx))
            .await
            .map_err(|e: rquickjs::Error| e.to_string())?;

        run_event_loop(vm, rt_key).await?;

        if schedule_signals(vm).await? {
            continue;
        }

        if SIGNAL_EXIT_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        if stay_alive(vm, rt_key).await? {
            continue;
        }

        vm.ctx
            .with(|ctx| {
                let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
                lexe_timers::discard_all_timers_on_process_exit(&ctx, rt)
            })
            .await
            .map_err(|e: rquickjs::Error| e.to_string())?;

        run_event_loop(vm, rt_key).await?;
        poll_drivers(vm).await;

        vm.ctx
            .with(|ctx| emit_process_exit(&ctx))
            .await
            .map_err(|e: rquickjs::Error| e.to_string())?;

        break;
    }

    Ok(())
}

/// Let pending JS work run, then poll native drivers.
///
/// `idle` is never cancelled: dropping it mid-poll skips the runtime's pending
/// context cleanup and strands JS values, which later aborts teardown with
/// QuickJS's `gc_obj_list` assertion. It parks while spawned tasks are pending,
/// so the handles that own those tasks release the loop reference when they
/// finish; when they do, the quiescence notify wakes this wait.
async fn run_event_loop(
    vm: &Vm,
    rt_key: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    loop {
        if pending_signal() {
            return Ok(());
        }

        poll_drivers(vm).await;
        if !stay_alive(vm, rt_key).await? {
            // Nothing owns pending work any more: take one last bounded turn so
            // already-runnable JS callbacks still execute, then stop.
            idle_bounded(vm).await?;
            poll_drivers(vm).await;
            return Ok(());
        }

        idle_until_wake(vm, rt_key).await?;
        poll_drivers(vm).await;
    }
}

fn pending_signal() -> bool {
    #[cfg(target_family = "wasm")]
    {
        false
    }

    #[cfg(not(target_family = "wasm"))]
    {
        signal_bridge::has_pending_signal()
    }
}

/// Run the loop until it settles or a signal arrives.
///
/// Used only when no handle keeps the loop alive. `idle` is never cancelled:
/// dropping it mid-poll skips the runtime's pending-context cleanup and strands
/// JS values, which later aborts teardown with QuickJS's `gc_obj_list`
/// assertion.
async fn idle_bounded(vm: &Vm) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_family = "wasm")]
    {
        vm.idle().await?;
        return Ok(());
    }

    #[cfg(not(target_family = "wasm"))]
    {
        let wake = signal_bridge::wake_handle();
        tokio::select! {
            res = vm.idle() => res?,
            _ = wake.notified() => {},
        }

        Ok(())
    }
}

/// Run the loop while work remains, waking on quiescence or a signal instead of
/// polling, so `idle` is never cancelled mid-flight.
async fn idle_until_wake(
    vm: &Vm,
    rt_key: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let quiescent = quiescence_watch(rt_key);

    #[cfg(target_family = "wasm")]
    {
        let _ = quiescent;
        vm.idle().await?;
        return Ok(());
    }

    #[cfg(not(target_family = "wasm"))]
    {
        if signal_bridge::has_pending_signal() {
            return Ok(());
        }

        let _ = quiescent;
        let wake = signal_bridge::wake_handle();
        tokio::select! {
            res = vm.idle() => res?,
            _ = wake.notified() => {},
            _ = sleep(Duration::from_millis(20)) => {},
        }

        Ok(())
    }
}

/// Queue JS tasks for signals that arrived since the last turn.
///
/// The tasks are spawned into the runtime so they run inside `idle`, on the JS
/// thread and with the module's context — signal streams themselves must not
/// touch QuickJS.
async fn schedule_signals(vm: &Vm) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_family = "wasm")]
    {
        let _ = vm;
        return Ok(false);
    }

    #[cfg(not(target_family = "wasm"))]
    {
        if !signal_bridge::has_pending_signal() {
            return Ok(false);
        }

        // Dispatch on this task: `idle_turn` has returned, so the runtime lock is
        // released. A listener owns shutdown and may keep the process running;
        // without one the signal default terminates the process.
        for name in signal_bridge::take_pending_signals() {
            let _ = vm
                .ctx
                .with(|js_ctx| -> rquickjs::Result<()> {
                    if let Err(err) = dispatch_signal(&js_ctx, &name) {
                        report_signal_error(&js_ctx, err);
                    }
                    Ok(())
                })
                .await;
        }

        Ok(true)
    }
}

/// Emit `process` `"signal"`; without a listener request the default
/// termination (`128 + signum`) instead of exiting from inside the runtime.
fn dispatch_signal(ctx: &Ctx<'_>, name: &str) -> rquickjs::Result<()> {
    if emit_os_signal(ctx, name)? {
        return Ok(());
    }

    let code = default_exit_code_for_signal(name);
    EXIT_CODE.store(code, Ordering::Relaxed);
    emit_process_exit(ctx)?;
    // Pending timer callbacks would outlive runtime teardown; the process is
    // terminating by signal default, so drop them like Node's exit does.
    let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let _ = lexe_timers::discard_all_timers_on_process_exit(ctx, rt);
    // The runtime lock is held here, so unwinding is the drain loop's job.
    SIGNAL_EXIT_REQUESTED.store(true, Ordering::SeqCst);
    Ok(())
}

fn report_signal_error(ctx: &Ctx<'_>, err: rquickjs::Error) {
    let caught = ctx.catch();
    if let Some(exception) = caught.as_exception() {
        eprintln!("{}", exception.message().unwrap_or_default());
        if let Some(stack) = exception.stack() {
            eprintln!("{stack}");
        }
        return;
    }
    let value = caught.as_value();
    let text: std::result::Result<String, rquickjs::Error> = value.get();
    eprintln!("{}", text.unwrap_or_else(|_| format!("{err:?}")));
}

async fn poll_drivers(vm: &Vm) {
    let _ = vm
        .ctx
        .with(|ctx| {
            let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
            lexe_timers::resume_timer_loop_if_idle(&ctx)?;
            driver_poll::poll_native_drivers(rt);
            Ok::<_, rquickjs::Error>(())
        })
        .await;
}

async fn stay_alive(vm: &Vm, rt_key: usize) -> Result<bool, String> {
    vm.ctx
        .with(|ctx| {
            let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
            let listeners = signal_listener_total(&ctx)?;
            // Publish for the signal watcher, which cannot touch JS.
            lexe_process::lifecycle::publish_signal_listener_count(listeners);

            if event_loop_alive(rt_key, rt) {
                return Ok(true);
            }
            // Node keeps the loop alive while a signal listener is installed
            // (the signal watcher is a referenced handle).
            Ok(listeners > 0)
        })
        .await
        .map_err(|e: rquickjs::Error| e.to_string())
}

fn signal_listener_total<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<usize> {
    let process: rquickjs::Value = ctx.globals().get("process")?;
    let Some(process) = process.as_object() else {
        return Ok(0);
    };
    signal_listener_count(ctx, process)
}

/// Whether the JS loop still has work that must finish before `exit`:
/// live handle references, a referenced timer, or pending native driver work
/// (async work, referenced TSFNs, queued jobs) that runs as spawned tasks.
fn event_loop_alive(rt_key: usize, rt: *mut qjs::JSRuntime) -> bool {
    lexe_utils::event_loop::should_keep_process_alive(rt_key)
        || lexe_timers::keeps_timers_alive(rt)
        || driver_poll::drivers_busy(rt)
}
