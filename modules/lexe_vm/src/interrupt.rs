// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use rquickjs::{qjs, Ctx, Error, Result, Value};

use crate::errors::{
    throw_invalid_arg_type, throw_out_of_range, throw_script_interrupted, throw_script_timeout,
};

#[derive(Clone)]
struct InterruptEntry {
    deadline: Option<Instant>,
    timeout_ms: Option<u64>,
    break_on_sigint: bool,
}

#[derive(Default)]
struct RuntimeInterruptState {
    stack: Vec<InterruptEntry>,
    triggered_timeout_ms: Option<u64>,
    triggered_sigint: bool,
}

fn interrupt_states() -> &'static Mutex<HashMap<usize, RuntimeInterruptState>> {
    static STATES: OnceLock<Mutex<HashMap<usize, RuntimeInterruptState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

static SIGINT_FLAG: AtomicBool = AtomicBool::new(false);
static SIGINT_INSTALL_DEPTH: Mutex<usize> = Mutex::new(0);

#[cfg(unix)]
static PREV_SIGINT_HANDLER: Mutex<Option<libc::sighandler_t>> = Mutex::new(None);

#[cfg(unix)]
extern "C" fn vm_sigint_handler(_sig: libc::c_int) {
    SIGINT_FLAG.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn push_sigint_handler() {
    let mut depth = SIGINT_INSTALL_DEPTH.lock().unwrap();
    if *depth == 0 {
        SIGINT_FLAG.store(false, Ordering::SeqCst);
        let prev = unsafe { libc::signal(libc::SIGINT, vm_sigint_handler as *const () as usize) };
        *PREV_SIGINT_HANDLER.lock().unwrap() = Some(prev);
    }
    *depth += 1;
}

#[cfg(unix)]
fn pop_sigint_handler() {
    let mut depth = SIGINT_INSTALL_DEPTH.lock().unwrap();
    if *depth > 0 {
        *depth -= 1;
        if *depth == 0 {
            if let Some(prev) = PREV_SIGINT_HANDLER.lock().unwrap().take() {
                unsafe {
                    libc::signal(libc::SIGINT, prev);
                }
            }
            SIGINT_FLAG.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(not(unix))]
fn push_sigint_handler() {
    let mut depth = SIGINT_INSTALL_DEPTH.lock().unwrap();
    if *depth == 0 {
        SIGINT_FLAG.store(false, Ordering::SeqCst);
    }
    *depth += 1;
}

#[cfg(not(unix))]
fn pop_sigint_handler() {
    let mut depth = SIGINT_INSTALL_DEPTH.lock().unwrap();
    if *depth > 0 {
        *depth -= 1;
        if *depth == 0 {
            SIGINT_FLAG.store(false, Ordering::SeqCst);
        }
    }
}

unsafe extern "C" fn vm_interrupt_handler(_rt: *mut qjs::JSRuntime, opaque: *mut c_void) -> i32 {
    let rt_key = opaque as usize;
    let Ok(mut map) = interrupt_states().lock() else {
        return 0;
    };
    let Some(state) = map.get_mut(&rt_key) else {
        return 0;
    };

    let now = Instant::now();
    // Iterate outermost to innermost so an outer shorter deadline triggers cleanly.
    for entry in &state.stack {
        if entry.break_on_sigint && SIGINT_FLAG.load(Ordering::SeqCst) {
            state.triggered_sigint = true;
            return 1;
        }
        if let Some(deadline) = entry.deadline {
            if now >= deadline {
                state.triggered_timeout_ms = entry.timeout_ms;
                return 1;
            }
        }
    }
    0
}

/// Validates Node.js `timeout` option (`uint32 >= 1` and `<= 4294967295`).
pub fn validate_timeout(ctx: &Ctx<'_>, val: Option<&Value<'_>>) -> Result<Option<u64>> {
    let Some(val) = val else {
        return Ok(None);
    };
    if val.is_undefined() {
        return Ok(None);
    }
    let Some(num) = val.as_number() else {
        return Err(throw_invalid_arg_type(
            ctx,
            "options.timeout",
            "of type number",
            val,
        ));
    };
    if !num.is_finite() || num.fract() != 0.0 {
        return Err(throw_out_of_range(
            ctx,
            "options.timeout",
            "an integer",
            &format!("{num}"),
        ));
    }
    if !(1.0..=4_294_967_295.0).contains(&num) {
        return Err(throw_out_of_range(
            ctx,
            "options.timeout",
            ">= 1 && <= 4294967295",
            &format!("{num}"),
        ));
    }
    Ok(Some(num as u64))
}

/// Validates Node.js `breakOnSigint` boolean option.
pub fn validate_break_on_sigint(ctx: &Ctx<'_>, val: Option<&Value<'_>>) -> Result<bool> {
    let Some(val) = val else {
        return Ok(false);
    };
    if val.is_undefined() {
        return Ok(false);
    }
    val.as_bool()
        .ok_or_else(|| throw_invalid_arg_type(ctx, "options.breakOnSigint", "of type boolean", val))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TriggeredReason {
    Timeout(u64),
    Sigint,
}

pub struct InterruptGuard {
    rt: *mut qjs::JSRuntime,
    rt_key: usize,
    active: bool,
    installed_sigint: bool,
}

impl InterruptGuard {
    pub fn enter(ctx: &Ctx<'_>, timeout_ms: Option<u64>, break_on_sigint: bool) -> Self {
        let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        if timeout_ms.is_none() && !break_on_sigint {
            return Self {
                rt,
                rt_key: rt as usize,
                active: false,
                installed_sigint: false,
            };
        }

        let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
        if break_on_sigint {
            push_sigint_handler();
        }

        let rt_key = rt as usize;
        let was_empty = {
            let mut states = interrupt_states().lock().unwrap();
            let entry = states.entry(rt_key).or_default();
            let empty = entry.stack.is_empty();
            if empty {
                entry.triggered_timeout_ms = None;
                entry.triggered_sigint = false;
            }
            entry.stack.push(InterruptEntry {
                deadline,
                timeout_ms,
                break_on_sigint,
            });
            empty
        };

        if was_empty {
            unsafe {
                qjs::JS_SetInterruptHandler(rt, Some(vm_interrupt_handler), rt as *mut c_void);
            }
        }

        Self {
            rt,
            rt_key,
            active: true,
            installed_sigint: break_on_sigint,
        }
    }

    pub fn triggered_reason(&self) -> Option<TriggeredReason> {
        if !self.active {
            // Even without our own deadline, an outer active InterruptGuard could have interrupted.
        }
        let states = interrupt_states().lock().unwrap();
        let state = states.get(&self.rt_key)?;
        if state.triggered_sigint {
            Some(TriggeredReason::Sigint)
        } else {
            state.triggered_timeout_ms.map(TriggeredReason::Timeout)
        }
    }

    /// Clears QuickJS's uncatchable `InternalError: interrupted` when triggered and returns a
    /// catchable Node `ERR_SCRIPT_EXECUTION_TIMEOUT` / `ERR_SCRIPT_EXECUTION_INTERRUPTED`.
    ///
    /// # Safety
    /// `target_raw` and `parent_ctx` must belong to the same `JSRuntime`.
    pub unsafe fn map_triggered_error(
        &self,
        target_raw: *mut qjs::JSContext,
        parent_ctx: &Ctx<'_>,
    ) -> Option<Error> {
        let reason = self.triggered_reason()?;
        qjs::JS_ResetUncatchableError(target_raw);
        let stale = qjs::JS_GetException(target_raw);
        qjs::JS_FreeValue(target_raw, stale);
        let err = match reason {
            TriggeredReason::Timeout(ms) => throw_script_timeout(parent_ctx, ms),
            TriggeredReason::Sigint => throw_script_interrupted(parent_ctx),
        };
        Some(err)
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if self.installed_sigint {
            pop_sigint_handler();
        }
        let became_empty = {
            let mut states = interrupt_states().lock().unwrap();
            if let Some(state) = states.get_mut(&self.rt_key) {
                state.stack.pop();
                if state.stack.is_empty() {
                    states.remove(&self.rt_key);
                    true
                } else {
                    false
                }
            } else {
                true
            }
        };
        if became_empty {
            unsafe {
                qjs::JS_SetInterruptHandler(self.rt, None, ptr::null_mut());
            }
        }
    }
}

pub fn clear_runtime_interrupts(rt: *mut qjs::JSRuntime) {
    interrupt_states().lock().unwrap().remove(&(rt as usize));
    unsafe {
        qjs::JS_SetInterruptHandler(rt, None, ptr::null_mut());
    }
}
