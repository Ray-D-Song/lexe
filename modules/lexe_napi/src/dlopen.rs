// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use libloading::Library;
use once_cell::sync::Lazy;
use rquickjs::qjs::{self, JSContext, JSValue};
use rquickjs::Ctx;
use tracing::debug;

use crate::api::take_pending_module;
use crate::driver::DriverState;
use crate::env::{DisposeState, Env};
use crate::types::{napi_addon_register_func, napi_env, napi_value};
use crate::value::value_to_napi_borrowed;

struct EnvPtr(*mut Env);
unsafe impl Send for EnvPtr {}
unsafe impl Sync for EnvPtr {}

static ENV_REGISTRY: Lazy<Mutex<HashMap<usize, EnvPtr>>> = Lazy::new(|| Mutex::new(HashMap::new()));

thread_local! {
    static CURRENT_ENV: RefCell<Option<*mut Env>> = const { RefCell::new(None) };
}

pub fn register_env(ctx: NonNull<JSContext>, env: Box<Env>) -> *mut Env {
    let ptr = Box::into_raw(env);
    let key = ctx.as_ptr() as usize;
    ENV_REGISTRY.lock().unwrap().insert(key, EnvPtr(ptr));
    ptr
}

/// Postcondition check after the two-phase main shutdown: registry must be empty.
///
/// Does **not** drain, free, or leak envs. Residual state is a hard error.
pub fn shutdown_all() -> Result<(), String> {
    let env_count = ENV_REGISTRY.lock().unwrap().len();
    let tsfn_count = crate::async_work::registered_tsfn_count();
    if env_count != 0 || tsfn_count != 0 {
        return Err(format!(
            "N-API shutdown incomplete: envs={env_count}, tsfns={tsfn_count}"
        ));
    }
    Ok(())
}

pub(crate) fn env_ptrs_for_runtime(rt: *mut qjs::JSRuntime) -> Vec<*mut Env> {
    ENV_REGISTRY
        .lock()
        .unwrap()
        .values()
        .map(|EnvPtr(ptr)| *ptr)
        .filter(|ptr| unsafe { qjs::JS_GetRuntime((**ptr).ctx_ptr()) } == rt)
        .collect()
}

fn clear_native_require_cache<'js>(ctx: &Ctx<'js>) {
    let remaining: usize = ctx
        .eval(
            r#"
            (() => {
                const req = globalThis.require;
                if (!req || !req.cache) return 0;
                const keys = Object.keys(req.cache);
                for (const key of keys) {
                    const entry = req.cache[key];
                    if (entry) {
                        entry.exports = null;
                        try { entry.children = []; } catch (_err) {}
                        try { entry.parent = null; } catch (_err) {}
                    }
                    delete req.cache[key];
                }
                return keys.length;
            })()
            "#,
        )
        .unwrap_or(0);
    if remaining > 0 {
        tracing::debug!(
            "napi begin_shutdown: cleared {} require cache entries",
            remaining
        );
    }
}

/// Final GC, N-API external drain, and V8 bridge/context teardown.
#[cfg(feature = "v8-compat")]
pub fn finalize_v8_environment<'js>(ctx: &Ctx<'js>) -> Result<usize, String> {
    let raw_ctx = ctx.as_raw().as_ptr();
    let raw_runtime = unsafe { qjs::JS_GetRuntime(raw_ctx) };
    run_final_gc(ctx, raw_runtime, None)?;
    crate::external::drain_runtime_externals(raw_runtime);
    run_final_gc(ctx, raw_runtime, None)?;
    let shutdown_result = unsafe { v8_compat::shutdown_environment(raw_ctx) };
    for _ in 0..8 {
        run_final_gc(ctx, raw_runtime, None)?;
    }
    shutdown_result?;
    run_final_gc(ctx, raw_runtime, None)?;
    Ok(raw_runtime as usize)
}

/// After N-API shutdown completes, tear down V8 context state and return the
/// QuickJS runtime pointer for a subsequent [`v8_compat::shutdown_runtime`].
#[cfg(feature = "v8-compat")]
pub fn capture_v8_runtime_and_shutdown_context<'js>(ctx: &Ctx<'js>) -> Result<usize, String> {
    finalize_v8_environment(ctx)
}

fn free_script_modules<'js>(ctx: &Ctx<'js>) {
    unsafe {
        extern "C" {
            fn JS_FreeAllModules(ctx: *mut qjs::JSContext);
        }
        JS_FreeAllModules(ctx.as_raw().as_ptr());
    }
    let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    for _ in 0..4 {
        ctx.run_gc();
        unsafe {
            qjs::JS_RunGC(rt);
        }
    }
}

/// Clear require-cache native roots and begin env dispose (phase 1).
///
/// Returns the env's driver Arc (if any) so the caller can `wait_finished()`
/// before [`finish_shutdown`]. Env stays registered and alive.
pub fn begin_shutdown<'js>(ctx: &Ctx<'js>) -> Result<Option<Arc<DriverState>>, String> {
    clear_native_require_cache(ctx);
    free_script_modules(ctx);
    crate::api::clear_function_callbacks();
    let Some(env_ptr) = env_for_ctx(ctx.as_raw().as_ptr()) else {
        return Ok(None);
    };
    let env = unsafe { &mut *env_ptr };
    env.begin_dispose();
    Ok(env.driver.clone())
}

/// Phase 2: free Env only when dispose finished (driver complete).
///
/// Does **not** remove the env from the registry until `finish_dispose` succeeds.
pub fn finish_shutdown<'js>(ctx: &Ctx<'js>) -> Result<(), String> {
    let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let key = ctx.as_raw().as_ptr() as usize;

    let ptr = {
        let registry = ENV_REGISTRY.lock().unwrap();
        registry.get(&key).map(|EnvPtr(p)| *p)
    };

    let Some(ptr) = ptr else {
        // No env registered — nothing to free (no N-API was used).
        run_final_gc(ctx, rt, None)?;
        unregister_runtime_hooks_if_unused(rt);
        return Ok(());
    };

    unsafe {
        if (*ptr).dispose_state == DisposeState::Active {
            (*ptr).begin_dispose();
        }
        (*ptr).try_finish_dispose()?;
        run_final_gc(ctx, rt, Some(ptr))?;
        // Only remove after finish succeeds.
        let removed = ENV_REGISTRY.lock().unwrap().remove(&key);
        debug_assert!(removed.is_some());
        let _ = Box::from_raw(ptr);
    }

    run_final_gc(ctx, rt, None)?;
    unregister_runtime_hooks_if_unused(rt);
    Ok(())
}

fn run_final_gc<'js>(
    ctx: &Ctx<'js>,
    rt: *mut qjs::JSRuntime,
    env: Option<*mut Env>,
) -> Result<(), String> {
    const MAX_PASSES: usize = 32;
    for _ in 0..MAX_PASSES {
        if let Some(env_ptr) = env {
            unsafe {
                crate::gc_hook::drain_pending_finalizers(&mut *env_ptr);
                crate::gc_hook::run_all_remaining(&mut *env_ptr);
            }
        } else {
            crate::gc_hook::compact_stale_pending();
        }
        ctx.run_gc();
        unsafe {
            qjs::JS_RunGC(rt);
        }
        if let Some(env_ptr) = env {
            unsafe {
                crate::gc_hook::drain_pending_finalizers(&mut *env_ptr);
            }
        } else {
            crate::gc_hook::compact_stale_pending();
        }
        if !crate::gc_hook::has_pending_finalizers() {
            return Ok(());
        }
    }
    crate::gc_hook::compact_stale_pending();
    if crate::gc_hook::has_pending_finalizers() {
        return Err(format!(
            "teardown incomplete: pending N-API finalizers after {MAX_PASSES} GC passes"
        ));
    }
    Ok(())
}

fn unregister_runtime_hooks_if_unused(rt: *mut qjs::JSRuntime) {
    if !env_ptrs_for_runtime(rt).is_empty() {
        return;
    }
    lexe_utils::driver_poll::unregister_driver_notify(rt);
    crate::gc_hook::unregister_holder_class(rt);
}

pub fn env_for_ctx(ctx: *mut JSContext) -> Option<*mut Env> {
    ENV_REGISTRY
        .lock()
        .unwrap()
        .get(&(ctx as usize))
        .map(|EnvPtr(p)| *p)
}

pub fn current_env() -> Option<&'static mut Env> {
    CURRENT_ENV.with(|cell| {
        let ptr = *cell.borrow();
        ptr.map(|p| unsafe { &mut *p })
    })
}

pub fn with_current_env_for_ctx<F, R>(ctx: *mut JSContext, f: F) -> Option<R>
where
    F: FnOnce(&mut Env) -> R,
{
    let ptr = env_for_ctx(ctx)?;
    CURRENT_ENV.with(|cell| {
        *cell.borrow_mut() = Some(ptr);
    });
    let result = f(unsafe { &mut *ptr });
    CURRENT_ENV.with(|cell| {
        *cell.borrow_mut() = None;
    });
    Some(result)
}

struct EnvActivation {
    _ptr: *mut Env,
}

impl EnvActivation {
    fn enter(ptr: *mut Env) -> Self {
        CURRENT_ENV.with(|cell| {
            *cell.borrow_mut() = Some(ptr);
        });
        Self { _ptr: ptr }
    }
}

impl Drop for EnvActivation {
    fn drop(&mut self) {
        CURRENT_ENV.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

/// Load a native addon and populate `module.exports`.
pub fn dlopen_module(ctx_ptr: NonNull<JSContext>, exports_obj: JSValue) -> Result<JSValue, String> {
    let filename = resolve_addon_path(exports_obj, ctx_ptr)?;
    debug!("dlopen: loading {}", filename);

    #[cfg(target_env = "musl")]
    {
        let _ = (ctx_ptr, exports_obj);
        return Err(
            "Native addon loading requires a dynamically linked lexe build (not musl+static)"
                .to_string(),
        );
    }

    #[cfg(not(target_env = "musl"))]
    {
        dlopen_impl(ctx_ptr, exports_obj, &filename)
    }
}

fn resolve_addon_path(exports_obj: JSValue, ctx_ptr: NonNull<JSContext>) -> Result<String, String> {
    let _ = (exports_obj, ctx_ptr);
    Err("dlopen path resolution requires module context".to_string())
}

pub fn dlopen_file(ctx_ptr: NonNull<JSContext>, filename: &str) -> Result<JSValue, String> {
    debug!("dlopen_file: {}", filename);

    #[cfg(target_env = "musl")]
    {
        let _ = (ctx_ptr, filename);
        return Err(
            "Native addon loading requires a dynamically linked lexe build (not musl+static)"
                .to_string(),
        );
    }

    #[cfg(not(target_env = "musl"))]
    {
        let exports = unsafe {
            let ctx = ctx_ptr.as_ptr();
            qjs::JS_NewObject(ctx)
        };
        dlopen_impl(ctx_ptr, exports, filename)
    }
}

#[cfg(not(target_env = "musl"))]
fn dlopen_impl(
    ctx_ptr: NonNull<JSContext>,
    exports_obj: JSValue,
    filename: &str,
) -> Result<JSValue, String> {
    if !Path::new(filename).exists() {
        return Err(format!("Cannot load addon '{}': file not found", filename));
    }

    let env_ptr = if let Some(existing) = env_for_ctx(ctx_ptr.as_ptr()) {
        existing
    } else {
        register_env(ctx_ptr, Box::new(Env::new(ctx_ptr)))
    };
    unsafe {
        (*env_ptr).ensure_external_class();
        (*env_ptr).scopes.open();
    }
    let _activation = EnvActivation::enter(env_ptr);

    let napi_env = unsafe { (*env_ptr).as_napi_env() };
    let exports_napi = value_to_napi_borrowed(unsafe { &mut *env_ptr }, exports_obj);

    #[cfg(feature = "v8-compat")]
    let _v8_guard = {
        v8_compat::bind_bridge(ctx_ptr.as_ptr());
        let rt = unsafe { qjs::JS_GetRuntime(ctx_ptr.as_ptr()) };
        let isolate = v8_compat::ensure_isolate_for_runtime(rt);
        let context_state = v8_compat::ensure_context_for_ctx(ctx_ptr.as_ptr());
        Some(v8_compat::push_native_load(
            ctx_ptr.as_ptr(),
            isolate,
            context_state,
        ))
    };

    #[cfg(not(feature = "v8-compat"))]
    let _v8_guard: Option<()> = None;

    let library = unsafe { Library::new(filename) }
        .map_err(|e| format!("dlopen failed for '{}': {}", filename, e))?;

    #[cfg(feature = "v8-compat")]
    {
        let pending_start = _v8_guard
            .as_ref()
            .map(|g| g.pending_modules_start())
            .unwrap_or(0);
        let v8_modules = v8_compat::drain_pending_v8_modules_since(pending_start);
        if v8_modules.len() == 1 {
            let module = v8_modules[0];
            let actual_exports =
                unsafe { v8_compat::run_v8_module_init(ctx_ptr.as_ptr(), module, exports_obj)? };
            std::mem::forget(library);
            unsafe {
                (*env_ptr).close_all_scopes();
            }
            return Ok(actual_exports);
        }
        if !v8_modules.is_empty() {
            v8_compat::clear_pending_v8_modules();
            return Err(format!(
                "Expected exactly one V8 node_module registration in '{}', found {}",
                filename,
                v8_modules.len()
            ));
        }
    }

    if let Some(module) = take_pending_module() {
        let final_exports = run_register_func(
            napi_env,
            module.nm_register_func,
            exports_napi,
            ctx_ptr,
            exports_obj,
        )?;
        std::mem::forget(library);
        unsafe {
            (*env_ptr).close_all_scopes();
        }
        return Ok(final_exports);
    }

    type RegisterV1Fn = unsafe extern "C" fn(env: napi_env, exports: napi_value) -> napi_value;

    let register: libloading::Symbol<RegisterV1Fn> = unsafe {
        library.get(b"napi_register_module_v1\0").map_err(|_| {
            format!(
                "No napi_register_module_v1 symbol in '{}'. Is this an N-API addon?",
                filename
            )
        })?
    };

    let result = unsafe { register(napi_env, exports_napi) };
    let final_exports = if !result.is_null() {
        unsafe { crate::value::napi_to_value_dup(&*env_ptr, result) }
            .ok_or_else(|| "Invalid return value from addon init".to_string())?
    } else {
        unsafe { qjs::JS_DupValue(ctx_ptr.as_ptr(), exports_obj) }
    };

    std::mem::forget(library);
    unsafe {
        (*env_ptr).close_all_scopes();
    }
    Ok(final_exports)
}

fn run_register_func(
    napi_env: napi_env,
    register: napi_addon_register_func,
    exports: napi_value,
    ctx_ptr: NonNull<JSContext>,
    exports_obj: JSValue,
) -> Result<JSValue, String> {
    let result = unsafe { register(napi_env, exports) };
    if !result.is_null() {
        let env = unsafe { Env::from_napi_env(napi_env) };
        if let Some(js_result) = unsafe { crate::value::napi_to_value_dup(env, result) } {
            return Ok(js_result);
        }
    }
    Ok(unsafe { qjs::JS_DupValue(ctx_ptr.as_ptr(), exports_obj) })
}

/// Rust entry for `process.dlopen(module, filename[, flags])`.
/// Returns the final `module.exports` value (caller must install it on the module).
pub fn process_dlopen(
    ctx_ptr: NonNull<JSContext>,
    module_exports: JSValue,
    filename: &str,
    _flags: u32,
) -> Result<JSValue, String> {
    #[cfg(target_env = "musl")]
    {
        let _ = (ctx_ptr, module_exports, filename);
        Err(
            "Native addon loading requires a dynamically linked lexe build (not musl+static)"
                .to_string(),
        )
    }

    #[cfg(not(target_env = "musl"))]
    {
        dlopen_impl(ctx_ptr, module_exports, filename)
    }
}
