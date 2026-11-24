// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use rquickjs::{function::Args, Ctx, Function, Object, Result, Value};

use crate::{emit_exit, EXIT_CODE};

pub fn emit_before_exit<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let process: Object = ctx.globals().get("process")?;
    let code = EXIT_CODE.load(Ordering::Relaxed);
    let emit: Function = process.get("emit")?;
    let mut args = Args::new(ctx.clone(), 2);
    args.this(process.clone())?;
    args.push_arg("beforeExit")?;
    args.push_arg(code)?;
    let _ = emit.call_arg::<Value>(args)?;
    Ok(())
}

pub fn emit_process_exit<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let process: Object = ctx.globals().get("process")?;
    let code = EXIT_CODE.load(Ordering::Relaxed);
    emit_exit(ctx, &process, code)
}

pub fn signal_listener_count<'js>(ctx: &Ctx<'js>, process: &Object<'js>) -> Result<usize> {
    let listener_count: Value = process.get("listenerCount")?;
    let Some(listener_count) = listener_count.as_function() else {
        return Ok(0);
    };
    // `Function::call` leaves `this` undefined, but the EventEmitter method
    // reads its listener registry from `this`.
    let mut args = Args::new(ctx.clone(), 1);
    args.this(process.clone())?;
    args.push_arg("signal")?;
    listener_count.call_arg(args)
}

/// Emit a `process` `"signal"` event. Returns `true` when listeners were registered
/// (Node removes default termination when a listener exists).
pub fn emit_os_signal<'js>(ctx: &Ctx<'js>, signal_name: &str) -> Result<bool> {
    let process: Object = ctx.globals().get("process")?;
    let count = signal_listener_count(ctx, &process)?;
    if count == 0 {
        return Ok(false);
    }
    let emit: Function = process.get("emit")?;
    let mut args = Args::new(ctx.clone(), 2);
    args.this(process.clone())?;
    args.push_arg("signal")?;
    args.push_arg(signal_name)?;
    let _ = emit.call_arg::<Value>(args)?;
    Ok(true)
}

/// Last observed count of `signal` listeners, published by the drain loop.
///
/// The signal watcher cannot consult JS while `Vm::idle` holds the runtime lock,
/// so it reads this snapshot: with no listener the default termination applies
/// and the process can exit immediately.
static SIGNAL_LISTENERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Whether [`SIGNAL_LISTENERS`] holds a real observation yet.
static SIGNAL_LISTENERS_PUBLISHED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Publish the current `signal` listener count for the signal watcher.
pub fn publish_signal_listener_count(count: usize) {
    SIGNAL_LISTENERS.store(count, Ordering::SeqCst);
    SIGNAL_LISTENERS_PUBLISHED.store(true, Ordering::SeqCst);
}

/// Snapshot of the listener count, or `None` before the loop ever published one.
pub fn signal_listener_count_snapshot() -> Option<usize> {
    if SIGNAL_LISTENERS_PUBLISHED.load(Ordering::SeqCst) {
        Some(SIGNAL_LISTENERS.load(Ordering::SeqCst))
    } else {
        None
    }
}

#[cfg(unix)]
pub fn default_exit_code_for_signal(signal_name: &str) -> u8 {
    use lexe_utils::signals::signal_from_str;
    let Some(sig) = signal_from_str(signal_name) else {
        return 128;
    };
    (128 + sig).rem_euclid(256) as u8
}

#[cfg(not(unix))]
pub fn default_exit_code_for_signal(_signal_name: &str) -> u8 {
    1
}
