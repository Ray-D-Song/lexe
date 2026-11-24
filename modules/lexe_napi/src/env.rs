// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_void;
use std::ptr;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use rquickjs::qjs::{self, JSContext, JSValue};
use std::thread::ThreadId;

use crate::driver::DriverState;
use crate::error::LastError;
use crate::finalizers::FinalizerTable;
use crate::refs::RefTable;
use crate::scopes::ScopeStack;
use crate::types::{napi_env, napi_extended_error_info, napi_finalize, napi_status};

pub type EnvCleanupHook = unsafe extern "C" fn(*mut c_void);
use crate::wrap::WrapTable;

static ENV_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisposeState {
    Active,
    Beginning,
    Finished,
}

pub struct Env {
    pub id: u64,
    pub ctx: NonNull<JSContext>,
    pub js_thread_id: ThreadId,
    pub scopes: ScopeStack,
    pub refs: RefTable,
    pub wraps: WrapTable,
    pub finalizers: FinalizerTable,
    pub driver: Option<std::sync::Arc<DriverState>>,
    pub last_error: LastError,
    pub last_error_info: napi_extended_error_info,
    pub last_error_message: Option<CString>,
    pub instance_data: HashMap<u32, *mut c_void>,
    pub instance_finalize: Option<(napi_finalize, *mut c_void)>,
    pub cleanup_hooks: Vec<(EnvCleanupHook, *mut c_void)>,
    pub external_memory: i64,
    pub external_class_acquired: bool,
    pub dispose_state: DisposeState,
    /// JS roots released (idempotent gate for begin/finish).
    roots_released: bool,
}

impl Env {
    pub fn new(ctx: NonNull<JSContext>) -> Self {
        Self {
            id: ENV_ID.fetch_add(1, Ordering::Relaxed),
            ctx,
            js_thread_id: std::thread::current().id(),
            scopes: ScopeStack::new(),
            refs: RefTable::new(),
            wraps: WrapTable::new(),
            finalizers: FinalizerTable::new(),
            driver: None,
            last_error: LastError::default(),
            last_error_info: napi_extended_error_info {
                error_message: ptr::null(),
                engine_reserved: ptr::null_mut(),
                error_code: napi_status::napi_ok,
                engine_error_code: 0,
            },
            last_error_message: None,
            instance_data: HashMap::new(),
            instance_finalize: None,
            cleanup_hooks: Vec::new(),
            external_memory: 0,
            external_class_acquired: false,
            dispose_state: DisposeState::Active,
            roots_released: false,
        }
    }

    pub fn is_js_thread(&self) -> bool {
        std::thread::current().id() == self.js_thread_id
    }

    pub fn ctx_ptr(&self) -> *mut JSContext {
        self.ctx.as_ptr()
    }

    pub fn as_napi_env(&self) -> napi_env {
        self as *const Env as *mut c_void
    }

    pub unsafe fn from_napi_env(env: napi_env) -> &'static mut Env {
        unsafe { &mut *(env as *mut Env) }
    }

    pub fn clear_exception(&mut self) -> Option<JSValue> {
        let ctx = self.ctx_ptr();
        unsafe {
            if !qjs::JS_HasException(ctx) {
                return None;
            }
            let exc = qjs::JS_GetException(ctx);
            if qjs::JS_IsNull(exc) || qjs::JS_IsUndefined(exc) {
                qjs::JS_FreeValue(ctx, exc);
                return None;
            }
            Some(exc)
        }
    }

    pub fn ensure_external_class(&mut self) -> rquickjs::qjs::JSClassID {
        let rt = unsafe { qjs::JS_GetRuntime(self.ctx_ptr()) };
        if self.external_class_acquired {
            return crate::external::class_id_for_runtime(rt)
                .expect("external class missing after acquire");
        }
        let class_id = crate::external::acquire_external_class_for_env(rt);
        self.external_class_acquired = true;
        class_id
    }

    pub fn set_pending_exception(&mut self, value: JSValue) {
        let ctx = self.ctx_ptr();
        unsafe {
            qjs::JS_Throw(ctx, value);
        }
    }

    pub fn status_from_throw(&mut self) -> napi_status {
        if self.has_pending_exception() {
            napi_status::napi_pending_exception
        } else {
            napi_status::napi_generic_failure
        }
    }

    pub fn has_pending_exception(&self) -> bool {
        let ctx = self.ctx_ptr();
        unsafe { qjs::JS_HasException(ctx) }
    }

    pub fn set_last_error(&mut self, status: napi_status, message: Option<&str>) {
        self.last_error.status = status;
        self.last_error.message = message.map(|m| CString::new(m).unwrap_or_default());
        self.last_error_message = self.last_error.message.clone();
        self.last_error_info.error_code = status;
        self.last_error_info.error_message = self
            .last_error_message
            .as_ref()
            .map(|m| m.as_ptr())
            .unwrap_or(ptr::null());
    }

    pub fn last_error_info_ptr(&self) -> *const napi_extended_error_info {
        &self.last_error_info
    }

    /// Release all handle-scope roots so GC objects can be collected on shutdown.
    pub fn close_all_scopes(&mut self) {
        self.scopes.close_all(self.ctx_ptr());
    }

    /// Phase 1: cleanup hooks, close TSFNs, close driver sender.
    ///
    /// JS roots are released here only when `inflight_async == 0` (no worker
    /// may still complete into this Env). If async work is still in flight,
    /// roots stay until [`finish_dispose`] after `wait_finished`.
    pub fn begin_dispose(&mut self) {
        if self.dispose_state != DisposeState::Active {
            return;
        }
        self.dispose_state = DisposeState::Beginning;

        // LIFO: last-registered cleanup hook runs first.
        while let Some((hook, arg)) = self.cleanup_hooks.pop() {
            unsafe { hook(arg) };
        }
        crate::async_work::close_all_tsfn_for_env(self.as_napi_env());

        let inflight = self
            .driver
            .as_ref()
            .map(|d| d.inflight_async.load(std::sync::atomic::Ordering::Acquire))
            .unwrap_or(0);

        if let Some(ref driver) = self.driver {
            driver.close_sender();
        }

        // Safe to free roots when no in-flight async work remains; idle can GC.
        if inflight == 0 {
            self.release_js_roots();
        }
    }

    /// Release JS roots and GC. Idempotent.
    fn release_js_roots(&mut self) {
        if self.roots_released {
            return;
        }
        self.roots_released = true;

        let ctx = self.ctx_ptr();
        if let Some((finalize, hint)) = self.instance_finalize.take() {
            let data = self
                .instance_data
                .remove(&0)
                .unwrap_or(std::ptr::null_mut());
            if let Some(cb) = finalize {
                unsafe { cb(self.as_napi_env(), data, hint) };
            }
        } else {
            self.instance_data.clear();
        }
        self.refs.release_all(ctx);
        self.close_all_scopes();
        self.wraps.clear();
        let rt = unsafe { qjs::JS_GetRuntime(ctx) };
        const MAX_GC_ROUNDS: usize = 32;
        for round in 0..MAX_GC_ROUNDS {
            unsafe {
                qjs::JS_RunGC(rt);
            }
            crate::gc_hook::drain_pending_finalizers(self);
            crate::gc_hook::run_all_remaining(self);
            crate::external::finalize_surviving_externals(self.as_napi_env());
            if !crate::gc_hook::has_pending_finalizers() {
                if round > 0 {
                    tracing::trace!(round, "napi env dispose GC stabilized");
                }
                break;
            }
            if round + 1 == MAX_GC_ROUNDS {
                tracing::warn!(
                    round,
                    "napi env dispose: finalizers still pending after GC cap"
                );
            }
        }
        if self.external_class_acquired {
            crate::external::release_external_class_for_env(rt);
            self.external_class_acquired = false;
        }
    }

    /// Phase 2: only after driver finished — release JS roots (if deferred), take driver.
    ///
    /// Returns `Ok(())` when finished, or `Err(reason)` describing why not.
    pub fn try_finish_dispose(&mut self) -> Result<(), String> {
        if self.dispose_state == DisposeState::Finished {
            return Ok(());
        }
        if self.dispose_state != DisposeState::Beginning {
            return Err(format!(
                "N-API Env {} dispose_state={:?}",
                self.id, self.dispose_state
            ));
        }
        if let Some(driver) = self.driver.clone() {
            // Drain any jobs left in the channel before checking finished.
            driver.drain_ready_jobs(self);
            driver.mark_finished_if_idle();
            if !driver.is_finished() {
                return Err(format!(
                    "N-API Env {} driver has not finished (loop_running={} pending={} inflight={} idle_refs={})",
                    self.id,
                    driver.is_loop_running(),
                    driver.pending_count(),
                    driver.inflight_async.load(std::sync::atomic::Ordering::Acquire),
                    driver.idle_refs.load(std::sync::atomic::Ordering::Acquire),
                ));
            }
            if !driver.is_quiescent() {
                return Err(format!(
                    "N-API Env {} driver finished flag set but work remains \
                     (pending={} inflight={} idle_refs={})",
                    self.id,
                    driver.pending_count(),
                    driver
                        .inflight_async
                        .load(std::sync::atomic::Ordering::Acquire),
                    driver.idle_refs.load(std::sync::atomic::Ordering::Acquire),
                ));
            }
        }
        self.release_js_roots();
        crate::driver::shutdown_driver(self);

        let scope_counts = self.scopes.counts();
        let refs = self.refs.len();
        let wraps = self.wraps.len();
        let finalizers = self.finalizers.len();

        if refs != 0
            || scope_counts.scopes != 0
            || scope_counts.values != 0
            || scope_counts.handles != 0
            || wraps != 0
        {
            return Err(format!(
                "N-API Env {} retained JS owners after dispose: \
                 refs={refs} scopes={} values={} handles={} wraps={wraps} finalizers={finalizers}",
                self.id, scope_counts.scopes, scope_counts.values, scope_counts.handles,
            ));
        }
        if finalizers != 0 {
            tracing::debug!(
                env_id = self.id,
                finalizers,
                "N-API metadata finalizers remain after JS roots were released"
            );
        }

        self.dispose_state = DisposeState::Finished;
        Ok(())
    }

    pub fn finish_dispose(&mut self) -> bool {
        self.try_finish_dispose().is_ok()
    }

    /// Unit-test helper for stack-owned envs that never started a driver loop.
    ///
    /// If `is_loop_running()`, panics: use a registry-owned Env and
    /// `begin_dispose` → drive runtime / `wait_finished` → `finish_shutdown`.
    #[cfg(test)]
    pub fn dispose(&mut self) {
        self.begin_dispose();
        if let Some(driver) = self.driver.clone() {
            driver.drain_ready_jobs(self);
            driver.mark_finished_if_idle();
            if !driver.is_finished() {
                assert!(
                    !driver.is_loop_running(),
                    "napi dispose: driver loop still running; use registry Env and \
                     begin_dispose → wait_finished → finish_shutdown"
                );
                if driver.pending_count() == 0
                    && driver
                        .inflight_async
                        .load(std::sync::atomic::Ordering::Acquire)
                        == 0
                {
                    // Loop never started (or already exited); no work remains.
                    driver.mark_finished();
                }
            }
        }
        assert!(
            self.finish_dispose(),
            "napi dispose: driver not finished; wait_finished before free when work remains"
        );
    }
}
