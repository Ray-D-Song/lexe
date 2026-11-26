// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-API (N-API) implementation for lexe (QuickJS-backed).

#![allow(non_camel_case_types, clippy::missing_safety_doc)]

mod api;
mod async_work;
pub mod dlopen;
mod driver;
mod env;
mod error;
mod external;
mod finalizers;
mod gc_hook;
mod js_helpers;
mod refs;
mod scopes;
mod types;
mod value;
mod wrap;

#[cfg(feature = "v8-compat")]
pub use dlopen::capture_v8_runtime_and_shutdown_context;
#[cfg(feature = "v8-compat")]
pub use dlopen::finalize_v8_environment;
pub use dlopen::{begin_shutdown, dlopen_module, finish_shutdown, shutdown_all};
pub use env::Env;
pub use types::*;

#[cfg(feature = "v8-compat")]
pub fn init_v8_shim() {
    v8_compat::ensure_shim_linked();
}

pub fn register_async_context(ctx: std::ptr::NonNull<rquickjs::qjs::JSContext>) {
    #[cfg(feature = "v8-compat")]
    init_v8_shim();
    let _ = ctx;
    lexe_utils::driver_poll::set_driver_poll_hook(Some(crate::api::poll_pending_drivers));
    lexe_utils::driver_poll::set_driver_busy_hook(Some(crate::api::drivers_busy));
}

pub const NAPI_VERSION: u32 = 9;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rquickjs::qjs::{self};
    use rquickjs::{AsyncContext, AsyncRuntime};

    use crate::driver::DriverState;
    use crate::env::Env;
    use crate::external::{create_external_object, get_external_pointer};
    use crate::js_helpers::{define_hidden_usize_configurable, new_int32};
    use crate::refs::RefTable;
    use crate::scopes::ScopeStack;
    use crate::types::napi_status;
    use crate::value::{napi_to_value, napi_value_for_slot, value_to_napi_owned};
    use crate::NAPI_VERSION;

    /// Drive `rt` until `driver` finishes, or panic after 5s with driver counters.
    async fn wait_driver_finished_bounded(rt: &AsyncRuntime, driver: &Arc<DriverState>) {
        driver.wake_if_quiescent();
        let drive = async {
            while !driver.is_finished() {
                let _ = rt.idle().await;
                tokio::task::yield_now().await;
            }
            driver.wait_finished().await;
        };
        if tokio::time::timeout(std::time::Duration::from_secs(5), drive)
            .await
            .is_err()
        {
            panic!(
                "driver wait timed out after 5s: loop_running={} pending={} inflight={} idle_refs={}",
                driver.is_loop_running(),
                driver.pending_count(),
                driver
                    .inflight_async
                    .load(std::sync::atomic::Ordering::Acquire),
                driver.idle_refs.load(std::sync::atomic::Ordering::Acquire),
            );
        }
    }

    /// After `begin_dispose` on a registry-owned env: drive the rquickjs runtime
    /// until the driver loop exits, then `finish_shutdown`.
    async fn finish_registry_env(
        rt: &AsyncRuntime,
        actx: &AsyncContext,
        driver: Option<Arc<DriverState>>,
    ) {
        if let Some(driver) = driver {
            wait_driver_finished_bounded(rt, &driver).await;
        }
        actx.with(|ctx| {
            crate::dlopen::finish_shutdown(&ctx).expect("finish_shutdown");
            crate::dlopen::shutdown_all().expect("shutdown_all");
        })
        .await;
    }

    /// Register a heap-owned Env for tests that start the driver loop.
    /// Returns a raw pointer only — build `&mut Env` inside `ctx.with` and drop
    /// the borrow before `finish_shutdown` frees the Box.
    fn register_test_env(
        raw: std::ptr::NonNull<rquickjs::qjs::JSContext>,
    ) -> std::ptr::NonNull<Env> {
        let env_ptr = crate::dlopen::register_env(raw, Box::new(Env::new(raw)));
        std::ptr::NonNull::new(env_ptr).expect("register_env returns non-null")
    }

    /// Drain, close scopes, begin_dispose; returns driver for `finish_registry_env`.
    ///
    /// # Safety
    /// `env_ptr` must be the live registry Env for `raw` (from `register_test_env`).
    unsafe fn begin_registry_dispose(
        env_ptr: std::ptr::NonNull<Env>,
        raw: std::ptr::NonNull<rquickjs::qjs::JSContext>,
    ) -> Option<Arc<DriverState>> {
        let env = unsafe { &mut *env_ptr.as_ptr() };
        if let Some(driver) = env.driver.clone() {
            driver.drain_ready_jobs(env);
        }
        env.scopes.close(raw.as_ptr());
        env.begin_dispose();
        env.driver.clone()
    }

    #[test]
    fn napi_version_is_nine() {
        assert_eq!(NAPI_VERSION, 9);
    }

    #[test]
    fn napi_status_ok_is_zero() {
        assert_eq!(napi_status::napi_ok as i32, 0);
    }

    #[tokio::test]
    async fn napi_create_symbol_with_description() {
        use crate::api::{napi_create_string_utf8, napi_create_symbol};
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let env_ptr = env.as_napi_env();
            let mut desc_handle = std::ptr::null_mut();
            let status =
                unsafe { napi_create_string_utf8(env_ptr, c"tag".as_ptr(), 3, &mut desc_handle) };
            assert_eq!(status, napi_status::napi_ok);
            let mut sym_handle = std::ptr::null_mut();
            let status = unsafe { napi_create_symbol(env_ptr, desc_handle, &mut sym_handle) };
            assert_eq!(status, napi_status::napi_ok);
            let sym = unsafe { napi_to_value(&env, sym_handle) }.unwrap();
            assert!(unsafe { qjs::JS_IsSymbol(sym) });
            let roundtrip_handle =
                value_to_napi_owned(&mut env, unsafe { qjs::JS_DupValue(raw.as_ptr(), sym) });
            let roundtrip = unsafe { napi_to_value(&env, roundtrip_handle) }.unwrap();
            assert!(unsafe { qjs::JS_IsSymbol(roundtrip) });
            env.scopes.close(raw.as_ptr());
        })
        .await;
    }

    #[tokio::test]
    async fn value_bridge_owned_does_not_leak_extra_ref() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let s = unsafe { qjs::JS_NewStringLen(raw.as_ptr(), c"hello".as_ptr(), 5) };
            let handle = value_to_napi_owned(&mut env, s);
            let roundtrip = unsafe { napi_to_value(&env, handle) }.unwrap();
            assert!(unsafe { qjs::JS_IsString(roundtrip) });
            env.scopes.close(raw.as_ptr());
        })
        .await;
    }

    #[tokio::test]
    async fn handle_scope_frees_values_on_close() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut scopes = ScopeStack::new();
            scopes.open();
            let val = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            scopes.push_value(val);
            scopes.close_handle(raw.as_ptr());
            assert_eq!(scopes.depth(), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn ref_table_create_and_delete_once() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut refs = RefTable::new();
            let s = unsafe { qjs::JS_NewStringLen(raw.as_ptr(), c"x".as_ptr(), 1) };
            let reference = refs.create(raw.as_ptr(), s, 1, std::ptr::null_mut(), false);
            // create() dups for the strong ref; caller still owns the original `s`.
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), s) };
            assert!(!reference.is_null());
            let status = unsafe { refs.delete(raw.as_ptr(), reference) };
            assert_eq!(status, napi_status::napi_ok);
            let status2 = unsafe { refs.delete(raw.as_ptr(), reference) };
            assert_eq!(status2, napi_status::napi_invalid_arg);
        })
        .await;
    }

    #[tokio::test]
    async fn external_object_roundtrip() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        crate::external::reset_for_tests();
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let ptr = 0x1234usize as *mut std::ffi::c_void;
            let obj = create_external_object(&mut env, ptr, None, std::ptr::null_mut());
            let handle = value_to_napi_owned(&mut env, obj);
            let js_val = unsafe { napi_to_value(&env, handle) }.unwrap();
            assert_eq!(get_external_pointer(raw.as_ptr(), js_val), Some(ptr));
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn external_finalize_runs_on_dispose() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        crate::external::reset_for_tests();
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};

        static FINALIZE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn external_finalize(
            _env: crate::types::napi_env,
            _data: *mut c_void,
            _hint: *mut c_void,
        ) {
            FINALIZE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            FINALIZE_COUNT.store(0, Ordering::SeqCst);
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let obj = create_external_object(
                &mut env,
                std::ptr::null_mut(),
                Some(external_finalize),
                std::ptr::null_mut(),
            );
            let _handle = value_to_napi_owned(&mut env, obj);
            env.dispose();
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn external_finalize_runs_on_dispose_on_global() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        crate::external::reset_for_tests();
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};

        static FINALIZE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn external_finalize(
            _env: crate::types::napi_env,
            _data: *mut c_void,
            _hint: *mut c_void,
        ) {
            FINALIZE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            FINALIZE_COUNT.store(0, Ordering::SeqCst);
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let obj = create_external_object(
                &mut env,
                std::ptr::null_mut(),
                Some(external_finalize),
                std::ptr::null_mut(),
            );
            let global = unsafe { qjs::JS_GetGlobalObject(raw.as_ptr()) };
            unsafe {
                qjs::JS_SetPropertyStr(
                    raw.as_ptr(),
                    global,
                    c"__napi_external_on_global".as_ptr(),
                    obj,
                );
                qjs::JS_FreeValue(raw.as_ptr(), global);
            }
            env.scopes.close(raw.as_ptr());
            env.dispose();
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn promise_resolve_settles() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut deferred: crate::types::napi_deferred = std::ptr::null_mut();
            let mut promise: crate::types::napi_value = std::ptr::null_mut();
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let status = unsafe {
                crate::async_work::napi_create_promise(napi_env, &mut deferred, &mut promise)
            };
            assert_eq!(status, napi_status::napi_ok);
            let one = new_int32(1);
            let one_handle = value_to_napi_owned(&mut env, one);
            let status =
                unsafe { crate::async_work::napi_resolve_deferred(napi_env, deferred, one_handle) };
            assert_eq!(status, napi_status::napi_ok);
            let p = unsafe { napi_to_value(&env, promise) }.unwrap();
            assert!(unsafe { qjs::JS_IsObject(p) });
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn wrap_marker_not_enumerable() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            assert!(unsafe {
                define_hidden_usize_configurable(raw.as_ptr(), obj, c"__napi_wrap_id".as_ptr(), 42)
            });

            let flags = (qjs::JS_GPN_STRING_MASK | qjs::JS_GPN_ENUM_ONLY) as i32;
            let mut keys: *mut qjs::JSPropertyEnum = std::ptr::null_mut();
            let mut len: u32 = 0;
            let ok = unsafe {
                qjs::JS_GetOwnPropertyNames(raw.as_ptr(), &mut keys, &mut len, obj, flags)
            };
            assert!(ok >= 0);
            let wrap_atom = unsafe { qjs::JS_NewAtom(raw.as_ptr(), c"__napi_wrap_id".as_ptr()) };
            for i in 0..len {
                let atom = unsafe { (*keys.add(i as usize)).atom };
                assert_ne!(atom, wrap_atom);
            }
            unsafe { qjs::JS_FreeAtom(raw.as_ptr(), wrap_atom) };
            unsafe { qjs::JS_FreePropertyEnum(raw.as_ptr(), keys, len) };
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
        })
        .await;
    }

    #[tokio::test]
    async fn escape_handle_survives_escapable_scope_close() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            env.scopes.open_escapable();
            let inner = unsafe { qjs::JS_NewStringLen(raw.as_ptr(), c"escaped".as_ptr(), 7) };
            let inner_handle = value_to_napi_owned(&mut env, inner);
            let duped = unsafe {
                qjs::JS_DupValue(raw.as_ptr(), napi_to_value(&env, inner_handle).unwrap())
            };
            let escape_slot = env.scopes.escape_into_slot(duped).unwrap();
            let escaped = napi_value_for_slot(&mut env, escape_slot);
            env.scopes.close_escapable(raw.as_ptr());
            let v = unsafe { napi_to_value(&env, escaped) }.unwrap();
            assert!(unsafe { qjs::JS_IsString(v) });
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn nested_handle_scope_outer_handle_still_resolves() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let outer = unsafe { qjs::JS_NewStringLen(raw.as_ptr(), c"outer".as_ptr(), 5) };
            let outer_handle = value_to_napi_owned(&mut env, outer);
            env.scopes.open();
            let inner = unsafe { qjs::JS_NewStringLen(raw.as_ptr(), c"inner".as_ptr(), 5) };
            let _inner_handle = value_to_napi_owned(&mut env, inner);
            let v = unsafe { napi_to_value(&env, outer_handle) }.unwrap();
            assert!(unsafe { qjs::JS_IsString(v) });
            env.scopes.close_handle(raw.as_ptr());
            let v2 = unsafe { napi_to_value(&env, outer_handle) }.unwrap();
            assert!(unsafe { qjs::JS_IsString(v2) });
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    static GC_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn gc_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
        GC_TEST_LOCK.lock().await
    }

    #[tokio::test]
    async fn wrap_finalize_runs_once_after_gc() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};

        static FINALIZE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn finalize_cb(
            _env: crate::types::napi_env,
            _data: *mut c_void,
            _hint: *mut c_void,
        ) {
            FINALIZE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            FINALIZE_COUNT.store(0, Ordering::SeqCst);
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            crate::gc_hook::register_holder_class(rt_ptr);
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let handle = crate::value::value_to_napi_borrowed(&mut env, obj);
            let status = unsafe {
                crate::wrap::napi_wrap(
                    napi_env,
                    handle,
                    std::ptr::null_mut(),
                    Some(finalize_cb),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(status, napi_status::napi_ok);
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close_handle(raw.as_ptr());
            env.scopes.open();
            ctx.run_gc();
            crate::gc_hook::drain_pending_finalizers(&mut env);
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 1);
            ctx.run_gc();
            crate::gc_hook::drain_pending_finalizers(&mut env);
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 1);
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn remove_wrap_does_not_finalize_again_on_gc() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};

        static FINALIZE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn finalize_cb(
            _env: crate::types::napi_env,
            _data: *mut c_void,
            _hint: *mut c_void,
        ) {
            FINALIZE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            FINALIZE_COUNT.store(0, Ordering::SeqCst);
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            crate::gc_hook::register_holder_class(rt_ptr);
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let handle = crate::value::value_to_napi_borrowed(&mut env, obj);
            let status = unsafe {
                crate::wrap::napi_wrap(
                    napi_env,
                    handle,
                    std::ptr::null_mut(),
                    Some(finalize_cb),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(status, napi_status::napi_ok);
            let mut native: *mut c_void = std::ptr::null_mut();
            let status = unsafe { crate::wrap::napi_remove_wrap(napi_env, handle, &mut native) };
            assert_eq!(status, napi_status::napi_ok);
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 0);
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close_handle(raw.as_ptr());
            env.scopes.open();
            ctx.run_gc();
            crate::gc_hook::drain_pending_finalizers(&mut env);
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 0);
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn weak_reference_returns_undefined_after_gc() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            crate::gc_hook::register_holder_class(rt_ptr);
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let handle = crate::value::value_to_napi_borrowed(&mut env, obj);
            let mut weak_ref: crate::types::napi_ref = std::ptr::null_mut();
            let status =
                unsafe { crate::refs::napi_create_reference(napi_env, handle, 0, &mut weak_ref) };
            assert_eq!(status, napi_status::napi_ok);
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close_handle(raw.as_ptr());
            env.scopes.open();
            ctx.run_gc();
            crate::gc_hook::drain_pending_finalizers(&mut env);
            let mut value: crate::types::napi_value = std::ptr::null_mut();
            let status =
                unsafe { crate::refs::napi_get_reference_value(napi_env, weak_ref, &mut value) };
            assert_eq!(status, napi_status::napi_ok);
            let js_val = unsafe { napi_to_value(&env, value) }.unwrap();
            assert!(unsafe { qjs::JS_IsUndefined(js_val) });
            unsafe { crate::refs::napi_delete_reference(napi_env, weak_ref) };
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn two_weak_references_on_same_object_stay_alive() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            crate::gc_hook::register_holder_class(rt_ptr);
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let handle = crate::value::value_to_napi_borrowed(&mut env, obj);
            let mut weak1: crate::types::napi_ref = std::ptr::null_mut();
            let mut weak2: crate::types::napi_ref = std::ptr::null_mut();
            assert_eq!(
                unsafe { crate::refs::napi_create_reference(napi_env, handle, 0, &mut weak1) },
                napi_status::napi_ok
            );
            assert_eq!(
                unsafe { crate::refs::napi_create_reference(napi_env, handle, 0, &mut weak2) },
                napi_status::napi_ok
            );
            let mut value: crate::types::napi_value = std::ptr::null_mut();
            assert_eq!(
                unsafe { crate::refs::napi_get_reference_value(napi_env, weak2, &mut value) },
                napi_status::napi_ok
            );
            let js_val = unsafe { napi_to_value(&env, value) }.unwrap();
            assert!(unsafe { qjs::JS_IsObject(js_val) });
            unsafe {
                crate::refs::napi_delete_reference(napi_env, weak1);
                crate::refs::napi_delete_reference(napi_env, weak2);
                qjs::JS_FreeValue(raw.as_ptr(), obj);
            }
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn remove_wrap_allows_second_wrap() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            crate::gc_hook::register_holder_class(rt_ptr);
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let handle = crate::value::value_to_napi_borrowed(&mut env, obj);
            let native1 = 0x11usize as *mut std::ffi::c_void;
            let native2 = 0x22usize as *mut std::ffi::c_void;
            assert_eq!(
                unsafe {
                    crate::wrap::napi_wrap(
                        napi_env,
                        handle,
                        native1,
                        None,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                },
                napi_status::napi_ok
            );
            let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
            assert_eq!(
                unsafe { crate::wrap::napi_remove_wrap(napi_env, handle, &mut out) },
                napi_status::napi_ok
            );
            assert_eq!(out, native1);
            assert_eq!(
                unsafe {
                    crate::wrap::napi_wrap(
                        napi_env,
                        handle,
                        native2,
                        None,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                },
                napi_status::napi_ok
            );
            let mut unwrapped: *mut std::ffi::c_void = std::ptr::null_mut();
            assert_eq!(
                unsafe { crate::wrap::napi_unwrap(napi_env, handle, &mut unwrapped) },
                napi_status::napi_ok
            );
            assert_eq!(unwrapped, native2);
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn wrap_weak_ref_delete_still_allows_finalize() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};

        static FINALIZE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn finalize_cb(
            _env: crate::types::napi_env,
            _data: *mut c_void,
            _hint: *mut c_void,
        ) {
            FINALIZE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            FINALIZE_COUNT.store(0, Ordering::SeqCst);
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            crate::gc_hook::register_holder_class(rt_ptr);
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let handle = crate::value::value_to_napi_borrowed(&mut env, obj);
            let mut weak_ref: crate::types::napi_ref = std::ptr::null_mut();
            let status = unsafe {
                crate::wrap::napi_wrap(
                    napi_env,
                    handle,
                    std::ptr::null_mut(),
                    Some(finalize_cb),
                    std::ptr::null_mut(),
                    &mut weak_ref,
                )
            };
            assert_eq!(status, napi_status::napi_ok);
            assert_eq!(
                unsafe { crate::refs::napi_delete_reference(napi_env, weak_ref) },
                napi_status::napi_ok
            );
            assert_eq!(crate::gc_hook::entry_count(), 1);
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close_handle(raw.as_ptr());
            env.scopes.open();
            ctx.run_gc();
            crate::gc_hook::drain_pending_finalizers(&mut env);
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 1);
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn wrap_remove_cycle_does_not_leak_gc_entries() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            crate::gc_hook::register_holder_class(rt_ptr);
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let handle = crate::value::value_to_napi_borrowed(&mut env, obj);
            let native = 0x55usize as *mut std::ffi::c_void;
            for _ in 0..5 {
                assert_eq!(
                    unsafe {
                        crate::wrap::napi_wrap(
                            napi_env,
                            handle,
                            native,
                            None,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        )
                    },
                    napi_status::napi_ok
                );
                assert_eq!(crate::gc_hook::entry_count(), 1);
                let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
                assert_eq!(
                    unsafe { crate::wrap::napi_remove_wrap(napi_env, handle, &mut out) },
                    napi_status::napi_ok
                );
                assert_eq!(out, native);
                assert_eq!(crate::gc_hook::entry_count(), 0);
            }
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close_handle(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn dual_env_external_class_survives_first_dispose() {
        let _lock = gc_test_lock().await;
        crate::external::reset_for_tests();
        let rt = AsyncRuntime::new().unwrap();
        let ctx1 = AsyncContext::full(&rt).await.unwrap();
        let ctx2 = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx1 = ctx1.clone();
        let _async_ctx2 = ctx2.clone();

        ctx1.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let native = 0xDEAD_BEEFusize as *mut std::ffi::c_void;
            let obj = create_external_object(&mut env, native, None, std::ptr::null_mut());
            assert_eq!(get_external_pointer(raw.as_ptr(), obj), Some(native));
            assert!(crate::external::is_external_object(raw.as_ptr(), obj));
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;

        ctx2.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let native = 0xCAFE_BABEusize as *mut std::ffi::c_void;
            let obj = create_external_object(&mut env, native, None, std::ptr::null_mut());
            assert_eq!(get_external_pointer(raw.as_ptr(), obj), Some(native));
            assert!(crate::external::is_external_object(raw.as_ptr(), obj));
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;

        crate::external::reset_for_tests();
    }

    #[tokio::test]
    async fn dual_env_global_external_survives_first_dispose() {
        let _lock = gc_test_lock().await;
        crate::external::reset_for_tests();
        crate::gc_hook::reset_for_tests();
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};

        static FINALIZE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn external_finalize(
            _env: crate::types::napi_env,
            _data: *mut c_void,
            _hint: *mut c_void,
        ) {
            FINALIZE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx1 = AsyncContext::full(&rt).await.unwrap();
        let ctx2 = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx1 = ctx1.clone();
        let _async_ctx2 = ctx2.clone();

        ctx1.with(|ctx| {
            FINALIZE_COUNT.store(0, Ordering::SeqCst);
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let obj = create_external_object(
                &mut env,
                0x1111usize as *mut c_void,
                Some(external_finalize),
                std::ptr::null_mut(),
            );
            let global = unsafe { qjs::JS_GetGlobalObject(raw.as_ptr()) };
            unsafe {
                qjs::JS_SetPropertyStr(
                    raw.as_ptr(),
                    global,
                    c"__napi_dual_env_external".as_ptr(),
                    obj,
                );
                qjs::JS_FreeValue(raw.as_ptr(), global);
            }
            env.scopes.close(raw.as_ptr());
            env.dispose();
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 1);
        })
        .await;

        ctx2.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let obj = create_external_object(
                &mut env,
                0x2222usize as *mut c_void,
                None,
                std::ptr::null_mut(),
            );
            assert!(crate::external::is_external_object(raw.as_ptr(), obj));
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;

        ctx1.with(|ctx| {
            let raw = ctx.as_raw();
            let rt = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            let global = unsafe { qjs::JS_GetGlobalObject(raw.as_ptr()) };
            let atom =
                unsafe { qjs::JS_NewAtom(raw.as_ptr(), c"__napi_dual_env_external".as_ptr()) };
            let ext = unsafe { qjs::JS_GetProperty(raw.as_ptr(), global, atom) };
            unsafe {
                qjs::JS_DeleteProperty(raw.as_ptr(), global, atom, 0);
                qjs::JS_FreeAtom(raw.as_ptr(), atom);
                qjs::JS_FreeValue(raw.as_ptr(), ext);
                qjs::JS_FreeValue(raw.as_ptr(), global);
                qjs::JS_RunGC(rt);
                qjs::JS_RunGC(rt);
            }
            assert_eq!(FINALIZE_COUNT.load(Ordering::SeqCst), 1);
        })
        .await;

        crate::external::reset_for_tests();
    }

    #[tokio::test]
    async fn cancel_unqueued_async_work_returns_generic_failure() {
        let _lock = gc_test_lock().await;
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let mut work: crate::types::napi_async_work = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    crate::async_work::napi_create_async_work(
                        napi_env,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        None,
                        None,
                        std::ptr::null_mut(),
                        &mut work,
                    )
                },
                napi_status::napi_ok
            );
            assert_eq!(
                unsafe { crate::async_work::napi_cancel_async_work(napi_env, work) },
                napi_status::napi_generic_failure
            );
            assert_eq!(
                unsafe { crate::async_work::napi_delete_async_work(napi_env, work) },
                napi_status::napi_ok
            );
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn cancel_queued_async_work_completes_exactly_once() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        use std::thread;
        use std::time::Duration;

        static COMPLETE_COUNT: AtomicU32 = AtomicU32::new(0);
        static EXECUTE_STARTED: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" fn execute_slow(_env: crate::types::napi_env, _data: *mut c_void) {
            EXECUTE_STARTED.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(100));
        }

        unsafe extern "C" fn complete_once(
            _env: crate::types::napi_env,
            _status: napi_status,
            _data: *mut c_void,
        ) {
            COMPLETE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                COMPLETE_COUNT.store(0, Ordering::SeqCst);
                EXECUTE_STARTED.store(false, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let driver = crate::driver::ensure_driver(env);
                let mut work: crate::types::napi_async_work = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_async_work(
                            napi_env,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            Some(execute_slow),
                            Some(complete_once),
                            std::ptr::null_mut(),
                            &mut work,
                        )
                    },
                    napi_status::napi_ok
                );
                assert_eq!(
                    unsafe { crate::async_work::napi_queue_async_work(napi_env, work) },
                    napi_status::napi_ok
                );

                let mut cancelled = false;
                for _ in 0..1000 {
                    if EXECUTE_STARTED.load(Ordering::SeqCst) {
                        break;
                    }
                    let status =
                        unsafe { crate::async_work::napi_cancel_async_work(napi_env, work) };
                    if status == napi_status::napi_ok {
                        cancelled = true;
                        break;
                    }
                    thread::yield_now();
                }

                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while COMPLETE_COUNT.load(Ordering::SeqCst) == 0
                    && deadline > std::time::Instant::now()
                {
                    driver.drain_ready_jobs(env);
                    thread::sleep(Duration::from_millis(1));
                }

                if cancelled {
                    assert_eq!(COMPLETE_COUNT.load(Ordering::SeqCst), 1);
                    assert_eq!(driver.inflight_async.load(Ordering::SeqCst), 0);
                }

                assert_eq!(
                    unsafe { crate::async_work::napi_delete_async_work(napi_env, work) },
                    napi_status::napi_ok
                );
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn async_complete_dispatch_runs_at_most_once() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};

        static COMPLETE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn complete_once(
            _env: crate::types::napi_env,
            _status: napi_status,
            _data: *mut c_void,
        ) {
            COMPLETE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                COMPLETE_COUNT.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let driver = crate::driver::ensure_driver(env);
                let mut work: crate::types::napi_async_work = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_async_work(
                            napi_env,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            None,
                            Some(complete_once),
                            std::ptr::null_mut(),
                            &mut work,
                        )
                    },
                    napi_status::napi_ok
                );
                assert_eq!(
                    unsafe { crate::async_work::napi_queue_async_work(napi_env, work) },
                    napi_status::napi_ok
                );
                driver.drain_ready_jobs(env);
                driver.drain_ready_jobs(env);
                assert_eq!(COMPLETE_COUNT.load(Ordering::SeqCst), 1);
                assert_eq!(
                    unsafe { crate::async_work::napi_delete_async_work(napi_env, work) },
                    napi_status::napi_ok
                );
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn foreign_opaque_is_not_external() {
        use std::ptr;

        #[repr(C)]
        struct ForeignOpaque {
            tag: u32,
        }

        unsafe extern "C" fn foreign_finalizer(_rt: *mut qjs::JSRuntime, _val: qjs::JSValue) {}

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            let mut class_id: qjs::JSClassID = 0;
            unsafe {
                qjs::JS_NewClassID(rt_ptr, &mut class_id);
                let def = qjs::JSClassDef {
                    class_name: c"ForeignTest".as_ptr(),
                    finalizer: Some(foreign_finalizer),
                    gc_mark: None,
                    call: None,
                    exotic: ptr::null_mut(),
                };
                qjs::JS_NewClass(rt_ptr, class_id, &def);
            }
            let obj = unsafe { qjs::JS_NewObjectClass(raw.as_ptr(), class_id) };
            // Stack opaque: test only class/opaque classification, not heap ownership.
            let mut foreign = ForeignOpaque { tag: 0xABCD };
            unsafe {
                qjs::JS_SetOpaque(
                    obj,
                    (&mut foreign as *mut ForeignOpaque).cast::<std::ffi::c_void>(),
                );
            }
            assert!(!crate::external::is_external_object(raw.as_ptr(), obj));
            let mut env = Env::new(raw);
            env.scopes.open();
            let handle = value_to_napi_owned(&mut env, obj);
            let mut ty = crate::types::napi_valuetype::napi_undefined;
            assert_eq!(
                unsafe { crate::api::napi_typeof(env.as_napi_env(), handle, &mut ty) },
                napi_status::napi_ok
            );
            assert_ne!(ty, crate::types::napi_valuetype::napi_external);
            // Drop the handle that holds `obj` before `foreign` leaves scope.
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn foreign_opaque_after_prior_runtime_external_use() {
        use std::ptr;

        #[repr(C)]
        struct ForeignOpaque {
            tag: u32,
        }

        unsafe extern "C" fn foreign_finalizer(_rt: *mut qjs::JSRuntime, _val: qjs::JSValue) {}

        {
            let rt = AsyncRuntime::new().unwrap();
            let ctx = AsyncContext::full(&rt).await.unwrap();
            let _async_ctx = ctx.clone();
            ctx.with(|ctx| {
                let raw = ctx.as_raw();
                let mut env = Env::new(raw);
                env.scopes.open();
                let obj = create_external_object(
                    &mut env,
                    std::ptr::null_mut(),
                    None,
                    std::ptr::null_mut(),
                );
                unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
                env.scopes.close(raw.as_ptr());
                env.dispose();
            })
            .await;
            drop(ctx);
            drop(rt);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            let mut class_id: qjs::JSClassID = 0;
            unsafe {
                qjs::JS_NewClassID(rt_ptr, &mut class_id);
                let def = qjs::JSClassDef {
                    class_name: c"ForeignTestAfterRuntime".as_ptr(),
                    finalizer: Some(foreign_finalizer),
                    gc_mark: None,
                    call: None,
                    exotic: ptr::null_mut(),
                };
                qjs::JS_NewClass(rt_ptr, class_id, &def);
            }
            let obj = unsafe { qjs::JS_NewObjectClass(raw.as_ptr(), class_id) };
            // Stack opaque: free the JS object before `foreign` leaves scope.
            let mut foreign = ForeignOpaque { tag: 0xBEEF };
            unsafe {
                qjs::JS_SetOpaque(
                    obj,
                    (&mut foreign as *mut ForeignOpaque).cast::<std::ffi::c_void>(),
                );
            }
            assert!(!crate::external::is_external_object(raw.as_ptr(), obj));
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
        })
        .await;
    }

    #[tokio::test]
    async fn tombstoned_external_opaque_reclaimed_after_gc() {
        let _lock = gc_test_lock().await;
        crate::external::reset_for_tests();
        crate::gc_hook::reset_for_tests();
        use std::ptr;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static FINALIZE: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn count_finalize(
            _env: crate::types::napi_env,
            _ptr: *mut std::ffi::c_void,
            _hint: *mut std::ffi::c_void,
        ) {
            FINALIZE.fetch_add(1, Ordering::Relaxed);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        let rt_key = ctx
            .with(|c| unsafe { qjs::JS_GetRuntime(c.as_raw().as_ptr()) as usize })
            .await;

        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let obj = crate::external::create_external_object(
                &mut env,
                ptr::null_mut(),
                Some(count_finalize),
                ptr::null_mut(),
            );
            let global = unsafe { qjs::JS_GetGlobalObject(raw.as_ptr()) };
            unsafe {
                qjs::JS_SetPropertyStr(raw.as_ptr(), global, c"heldExternal".as_ptr(), obj);
                qjs::JS_FreeValue(raw.as_ptr(), global);
            }
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;

        assert_eq!(FINALIZE.load(Ordering::Relaxed), 1);

        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let global = unsafe { qjs::JS_GetGlobalObject(raw.as_ptr()) };
            let atom = unsafe { qjs::JS_NewAtom(raw.as_ptr(), c"heldExternal".as_ptr()) };
            let ext = unsafe { qjs::JS_GetProperty(raw.as_ptr(), global, atom) };
            unsafe {
                qjs::JS_DeleteProperty(raw.as_ptr(), global, atom, 0);
                qjs::JS_FreeAtom(raw.as_ptr(), atom);
                qjs::JS_FreeValue(raw.as_ptr(), ext);
                qjs::JS_FreeValue(raw.as_ptr(), global);
            }
            let rt_ptr = unsafe { qjs::JS_GetRuntime(raw.as_ptr()) };
            for _ in 0..5 {
                unsafe { qjs::JS_RunGC(rt_ptr) };
            }
        })
        .await;

        assert_eq!(
            crate::external::living_external_count_for_runtime(rt_key),
            0
        );
    }

    #[tokio::test]
    async fn surviving_external_outlives_prepare_registry_clear_until_runtime_free() {
        let _lock = gc_test_lock().await;
        crate::external::reset_for_tests();
        crate::gc_hook::reset_for_tests();
        use std::ptr;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static FINALIZE: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn count_finalize(
            _env: crate::types::napi_env,
            _ptr: *mut std::ffi::c_void,
            _hint: *mut std::ffi::c_void,
        ) {
            FINALIZE.fetch_add(1, Ordering::Relaxed);
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        let rt_key = ctx
            .with(|c| unsafe { qjs::JS_GetRuntime(c.as_raw().as_ptr()) as usize })
            .await;
        let rt_ptr = rt_key as *mut qjs::JSRuntime;

        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let obj = crate::external::create_external_object(
                &mut env,
                ptr::null_mut(),
                Some(count_finalize),
                ptr::null_mut(),
            );
            let global = unsafe { qjs::JS_GetGlobalObject(raw.as_ptr()) };
            unsafe {
                qjs::JS_SetPropertyStr(raw.as_ptr(), global, c"heldExternal".as_ptr(), obj);
                qjs::JS_FreeValue(raw.as_ptr(), global);
            }
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;

        assert_eq!(FINALIZE.load(Ordering::Relaxed), 1);
        assert_eq!(
            crate::external::living_external_count_for_runtime(rt_key),
            1
        );

        crate::external::clear_runtime_external_registry_for_tests(rt_ptr);

        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            for _ in 0..3 {
                unsafe { qjs::JS_RunGC(rt_key as *mut qjs::JSRuntime) };
            }
            let global = unsafe { qjs::JS_GetGlobalObject(raw.as_ptr()) };
            let atom = unsafe { qjs::JS_NewAtom(raw.as_ptr(), c"heldExternal".as_ptr()) };
            let ext = unsafe { qjs::JS_GetProperty(raw.as_ptr(), global, atom) };
            assert!(unsafe { qjs::JS_IsObject(ext) });
            unsafe {
                qjs::JS_FreeAtom(raw.as_ptr(), atom);
                qjs::JS_FreeValue(raw.as_ptr(), ext);
                qjs::JS_FreeValue(raw.as_ptr(), global);
            }
        })
        .await;

        assert_eq!(
            crate::external::living_external_count_for_runtime(rt_key),
            1
        );

        drop(ctx);
        drop(rt);

        assert_eq!(FINALIZE.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn dual_env_holder_survives_first_dispose_remove_wrap() {
        let _lock = gc_test_lock().await;
        crate::gc_hook::reset_for_tests();
        let rt = AsyncRuntime::new().unwrap();
        let ctx1 = AsyncContext::full(&rt).await.unwrap();
        let ctx2 = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx1 = ctx1.clone();
        let _async_ctx2 = ctx2.clone();

        let rt_key = ctx1
            .with(|ctx| unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) as usize })
            .await;
        let rt_ptr = rt_key as *mut qjs::JSRuntime;
        crate::gc_hook::register_holder_class(rt_ptr);

        ctx1.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;

        ctx2.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let handle = crate::value::value_to_napi_borrowed(&mut env, obj);
            let native1 = 0x11usize as *mut std::ffi::c_void;
            assert_eq!(
                unsafe {
                    crate::wrap::napi_wrap(
                        napi_env,
                        handle,
                        native1,
                        None,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                },
                napi_status::napi_ok
            );
            let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
            assert_eq!(
                unsafe { crate::wrap::napi_remove_wrap(napi_env, handle, &mut out) },
                napi_status::napi_ok
            );
            assert_eq!(out, native1);
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), obj) };
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    fn make_tsfn_js_callback(ctx: *mut qjs::JSContext) -> qjs::JSValue {
        unsafe {
            qjs::JS_Eval(
                ctx,
                c"(function(){})".as_ptr(),
                13,
                c"<tsfn-test>".as_ptr(),
                qjs::JS_EVAL_TYPE_GLOBAL as i32,
            )
        }
    }

    #[tokio::test]
    async fn env_js_thread_identity_main_vs_worker() {
        use std::sync::Arc;
        use std::thread;

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let env = Env::new(raw);
            assert!(env.is_js_thread());
            let js_thread_id = env.js_thread_id;
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let barrier_worker = Arc::clone(&barrier);
            let worker = thread::spawn(move || {
                barrier_worker.wait();
                assert_ne!(std::thread::current().id(), js_thread_id);
            });
            barrier.wait();
            worker.join().unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn tsfn_blocking_producer_wakes_after_pop() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        static PAYLOAD_COUNT: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn tsfn_call_js(
            _env: crate::types::napi_env,
            _js_callback: crate::types::napi_value,
            _context: *mut c_void,
            _data: *mut c_void,
        ) {
            PAYLOAD_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                PAYLOAD_COUNT.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let func_val = make_tsfn_js_callback(raw.as_ptr());
                let func_handle = value_to_napi_owned(env, func_val);
                let mut tsfn: crate::types::napi_threadsafe_function = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_threadsafe_function(
                            napi_env,
                            func_handle,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            1,
                            1,
                            std::ptr::null_mut(),
                            None,
                            std::ptr::null_mut(),
                            Some(tsfn_call_js),
                            &mut tsfn,
                        )
                    },
                    napi_status::napi_ok
                );
                let tsfn_arc = crate::async_work::lookup_tsfn_for_test(tsfn).unwrap();
                assert_eq!(tsfn_arc.test_queue_len(), 0);

                let tsfn_fill = tsfn as usize;
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_call_threadsafe_function(
                            tsfn_fill as crate::types::napi_threadsafe_function,
                            0xA1usize as *mut c_void,
                            crate::types::napi_threadsafe_function_call_mode::napi_tsfn_nonblocking,
                        )
                    },
                    napi_status::napi_ok
                );
                assert_eq!(tsfn_arc.test_queue_len(), 1);

                let tsfn_block = tsfn as usize;
                let barrier = Arc::new(std::sync::Barrier::new(2));
                let barrier_worker = Arc::clone(&barrier);
                let worker = thread::spawn(move || {
                    barrier_worker.wait();
                    let status = unsafe {
                        crate::async_work::napi_call_threadsafe_function(
                            tsfn_block as crate::types::napi_threadsafe_function,
                            0xA2usize as *mut c_void,
                            crate::types::napi_threadsafe_function_call_mode::napi_tsfn_blocking,
                        )
                    };
                    assert_eq!(status, napi_status::napi_ok);
                });

                thread::sleep(Duration::from_millis(20));
                assert_eq!(tsfn_arc.test_queue_len(), 1);

                let driver = crate::driver::ensure_driver(env);
                driver.drain_ready_jobs(env);
                assert_eq!(PAYLOAD_COUNT.load(Ordering::SeqCst), 1);

                barrier.wait();
                worker.join().unwrap();

                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while PAYLOAD_COUNT.load(Ordering::SeqCst) < 2
                    && deadline > std::time::Instant::now()
                {
                    driver.drain_ready_jobs(env);
                    thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(PAYLOAD_COUNT.load(Ordering::SeqCst), 2);

                unsafe {
                    crate::async_work::napi_release_threadsafe_function(
                        tsfn,
                        crate::types::napi_threadsafe_function_release_mode::napi_tsfn_release,
                    );
                }
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while crate::async_work::lookup_tsfn_for_test(tsfn).is_some()
                    && deadline > std::time::Instant::now()
                {
                    driver.drain_ready_jobs(env);
                    thread::sleep(Duration::from_millis(1));
                }
                driver.drain_ready_jobs(env);
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn tsfn_closing_wakes_blocking_producers_with_napi_closing() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        static CLOSING_COUNT: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn tsfn_call_js(
            _env: crate::types::napi_env,
            _js_callback: crate::types::napi_value,
            _context: *mut c_void,
            _data: *mut c_void,
        ) {
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        // Abort can leave driver work in flight; use two-phase dispose so the
        // loop exits before Env/JS roots are released.
        let driver = actx
            .with(|ctx| {
                CLOSING_COUNT.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let env_ptr = crate::dlopen::register_env(raw, Box::new(Env::new(raw)));
                let env = unsafe { &mut *env_ptr };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let func_val = make_tsfn_js_callback(raw.as_ptr());
                let func_handle = value_to_napi_owned(env, func_val);
                let mut tsfn: crate::types::napi_threadsafe_function = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_threadsafe_function(
                            napi_env,
                            func_handle,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            1,
                            2,
                            std::ptr::null_mut(),
                            None,
                            std::ptr::null_mut(),
                            Some(tsfn_call_js),
                            &mut tsfn,
                        )
                    },
                    napi_status::napi_ok
                );

                let tsfn_fill = tsfn as usize;
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_call_threadsafe_function(
                            tsfn_fill as crate::types::napi_threadsafe_function,
                            0xA1usize as *mut c_void,
                            crate::types::napi_threadsafe_function_call_mode::napi_tsfn_nonblocking,
                        )
                    },
                    napi_status::napi_ok
                );

                let tsfn_block = tsfn as usize;
                let barrier = Arc::new(std::sync::Barrier::new(3));
                let mut workers = Vec::new();
                for _ in 0..2 {
                    let barrier_worker = Arc::clone(&barrier);
                    let tsfn_worker = tsfn_block;
                    workers.push(thread::spawn(move || {
                        barrier_worker.wait();
                        let status = unsafe {
                            crate::async_work::napi_call_threadsafe_function(
                                tsfn_worker as crate::types::napi_threadsafe_function,
                                0xA2usize as *mut c_void,
                                crate::types::napi_threadsafe_function_call_mode::napi_tsfn_blocking,
                            )
                        };
                        if status == napi_status::napi_closing {
                            CLOSING_COUNT.fetch_add(1, Ordering::SeqCst);
                        }
                    }));
                }

                thread::sleep(Duration::from_millis(20));
                barrier.wait();
                unsafe {
                    crate::async_work::napi_release_threadsafe_function(
                        tsfn,
                        crate::types::napi_threadsafe_function_release_mode::napi_tsfn_abort,
                    );
                }

                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while CLOSING_COUNT.load(Ordering::SeqCst) < 2 && deadline > std::time::Instant::now()
                {
                    thread::sleep(Duration::from_millis(1));
                }
                for worker in workers {
                    worker.join().unwrap();
                }
                assert_eq!(CLOSING_COUNT.load(Ordering::SeqCst), 2);

                let driver = crate::driver::ensure_driver(env);
                driver.drain_ready_jobs(env);
                env.scopes.close(raw.as_ptr());
                env.begin_dispose();
                driver.clone()
            })
            .await;

        wait_driver_finished_bounded(&rt, &driver).await;

        actx.with(|ctx| {
            crate::dlopen::finish_shutdown(&ctx).expect("finish after wait");
            crate::dlopen::shutdown_all().expect("registry empty");
        })
        .await;
    }

    #[tokio::test]
    async fn tsfn_abort_from_worker_invokes_call_js_on_js_thread() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::Mutex;
        use std::thread;
        use std::thread::ThreadId;
        use std::time::Duration;

        static EXPECTED_JS_THREAD: Mutex<Option<ThreadId>> = Mutex::new(None);
        static TEARDOWN_THREAD: Mutex<Option<ThreadId>> = Mutex::new(None);

        unsafe extern "C" fn tsfn_call_js(
            env: crate::types::napi_env,
            _js_callback: crate::types::napi_value,
            _context: *mut c_void,
            _data: *mut c_void,
        ) {
            if env.is_null() {
                *TEARDOWN_THREAD.lock().unwrap() = Some(std::thread::current().id());
            }
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                *TEARDOWN_THREAD.lock().unwrap() = None;
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                *EXPECTED_JS_THREAD.lock().unwrap() = Some(env.js_thread_id);
                let func_val = make_tsfn_js_callback(raw.as_ptr());
                let func_handle = value_to_napi_owned(env, func_val);
                let mut tsfn: crate::types::napi_threadsafe_function = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_threadsafe_function(
                            napi_env,
                            func_handle,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            0,
                            1,
                            std::ptr::null_mut(),
                            None,
                            std::ptr::null_mut(),
                            Some(tsfn_call_js),
                            &mut tsfn,
                        )
                    },
                    napi_status::napi_ok
                );
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_call_threadsafe_function(
                            tsfn,
                            0x1234usize as *mut c_void,
                            crate::types::napi_threadsafe_function_call_mode::napi_tsfn_nonblocking,
                        )
                    },
                    napi_status::napi_ok
                );

                let driver = crate::driver::ensure_driver(env);
                let tsfn_addr = tsfn as usize;
                let worker = thread::spawn(move || unsafe {
                    crate::async_work::napi_release_threadsafe_function(
                        tsfn_addr as crate::types::napi_threadsafe_function,
                        crate::types::napi_threadsafe_function_release_mode::napi_tsfn_abort,
                    )
                });
                // Wait for abort to mark the TSFN and post cleanup jobs before draining,
                // so any already-queued Tsfn delivery job observes `aborted` and tears down.
                worker.join().unwrap();

                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while TEARDOWN_THREAD.lock().unwrap().is_none()
                    && deadline > std::time::Instant::now()
                {
                    driver.drain_ready_jobs(env);
                    thread::sleep(Duration::from_millis(1));
                }

                assert_eq!(
                    *TEARDOWN_THREAD.lock().unwrap(),
                    *EXPECTED_JS_THREAD.lock().unwrap()
                );
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn tsfn_admission_rollback_does_not_remove_other_payload() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static PAYLOAD_COUNT: AtomicUsize = AtomicUsize::new(0);
        static LAST_PAYLOAD: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn tsfn_call_js(
            _env: crate::types::napi_env,
            _js_callback: crate::types::napi_value,
            _context: *mut c_void,
            data: *mut c_void,
        ) {
            PAYLOAD_COUNT.fetch_add(1, Ordering::SeqCst);
            LAST_PAYLOAD.store(data as usize, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();

        // Registry-owned env so we can begin_dispose, wait_finished outside with(),
        // then finish_dispose — never force-finish an active driver loop.
        let driver = actx
            .with(|ctx| {
                PAYLOAD_COUNT.store(0, Ordering::SeqCst);
                LAST_PAYLOAD.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let env_ptr = crate::dlopen::register_env(raw, Box::new(Env::new(raw)));
                let env = unsafe { &mut *env_ptr };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let func_val = make_tsfn_js_callback(raw.as_ptr());
                let func_handle = value_to_napi_owned(env, func_val);
                let mut tsfn: crate::types::napi_threadsafe_function = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_threadsafe_function(
                            napi_env,
                            func_handle,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            0,
                            1,
                            std::ptr::null_mut(),
                            None,
                            std::ptr::null_mut(),
                            Some(tsfn_call_js),
                            &mut tsfn,
                        )
                    },
                    napi_status::napi_ok
                );
                let tsfn_arc = crate::async_work::lookup_tsfn_for_test(tsfn).unwrap();
                let driver = std::sync::Arc::clone(&tsfn_arc.driver);

                // Deterministic admission: queue BB first, then fail AA so rollback
                // cannot clear the surviving payload (no concurrent fail_posts race).
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_call_threadsafe_function(
                            tsfn,
                            0xBBusize as *mut c_void,
                            crate::types::napi_threadsafe_function_call_mode::napi_tsfn_nonblocking,
                        )
                    },
                    napi_status::napi_ok
                );
                assert_eq!(tsfn_arc.test_queue_len(), 1);

                driver.set_fail_posts(true);
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_call_threadsafe_function(
                            tsfn,
                            0xAAusize as *mut c_void,
                            crate::types::napi_threadsafe_function_call_mode::napi_tsfn_nonblocking,
                        )
                    },
                    napi_status::napi_generic_failure
                );
                driver.set_fail_posts(false);
                assert_eq!(tsfn_arc.test_queue_len(), 1);

                driver.drain_ready_jobs(env);
                assert_eq!(PAYLOAD_COUNT.load(Ordering::SeqCst), 1);
                assert_eq!(LAST_PAYLOAD.load(Ordering::SeqCst), 0xBB);

                unsafe {
                    crate::async_work::napi_release_threadsafe_function(
                        tsfn,
                        crate::types::napi_threadsafe_function_release_mode::napi_tsfn_release,
                    );
                }
                driver.drain_ready_jobs(env);
                env.scopes.close(raw.as_ptr());
                env.begin_dispose();
                // Loop may still be running — finish only after wait_finished.
                assert!(!env.finish_dispose() || driver.is_finished());
                driver.clone()
            })
            .await;

        wait_driver_finished_bounded(&rt, &driver).await;
        assert!(driver.is_finished());
        assert!(driver.is_quiescent());

        actx.with(|ctx| {
            crate::dlopen::finish_shutdown(&ctx).expect("finish after wait");
            crate::dlopen::shutdown_all().expect("registry empty");
        })
        .await;
    }

    #[tokio::test]
    async fn null_execute_cancel_race_completes_at_most_once() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::thread;

        static COMPLETE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn complete_cb(
            _env: crate::types::napi_env,
            _status: napi_status,
            _data: *mut c_void,
        ) {
            COMPLETE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                COMPLETE_COUNT.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let driver = crate::driver::ensure_driver(env);
                let mut work: crate::types::napi_async_work = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_async_work(
                            napi_env,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            None,
                            Some(complete_cb),
                            std::ptr::null_mut(),
                            &mut work,
                        )
                    },
                    napi_status::napi_ok
                );

                let work_for_cancel = work as usize;
                let cancel_env = napi_env as usize;
                let cancel_handle = thread::spawn(move || {
                    for _ in 0..500 {
                        unsafe {
                            crate::async_work::napi_cancel_async_work(
                                cancel_env as crate::types::napi_env,
                                work_for_cancel as crate::types::napi_async_work,
                            )
                        };
                        thread::yield_now();
                    }
                });
                assert_eq!(
                    unsafe { crate::async_work::napi_queue_async_work(napi_env, work) },
                    napi_status::napi_ok
                );
                cancel_handle.join().unwrap();
                driver.drain_ready_jobs(env);
                driver.drain_ready_jobs(env);
                assert_eq!(COMPLETE_COUNT.load(Ordering::SeqCst), 1);
                assert_eq!(driver.inflight_async.load(Ordering::SeqCst), 0);
                assert_eq!(
                    unsafe { crate::async_work::napi_delete_async_work(napi_env, work) },
                    napi_status::napi_ok
                );
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn many_async_works_release_keepalive() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::thread;
        use std::time::Duration;

        static COMPLETE_COUNT: AtomicU32 = AtomicU32::new(0);

        unsafe extern "C" fn execute_fast(_env: crate::types::napi_env, _data: *mut c_void) {}

        unsafe extern "C" fn complete_count(
            _env: crate::types::napi_env,
            _status: napi_status,
            _data: *mut c_void,
        ) {
            COMPLETE_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        const WORK_COUNT: u32 = 32;
        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                COMPLETE_COUNT.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let driver = crate::driver::ensure_driver(env);
                let mut works = Vec::new();
                for _ in 0..WORK_COUNT {
                    let mut work: crate::types::napi_async_work = std::ptr::null_mut();
                    assert_eq!(
                        unsafe {
                            crate::async_work::napi_create_async_work(
                                napi_env,
                                std::ptr::null_mut(),
                                std::ptr::null_mut(),
                                Some(execute_fast),
                                Some(complete_count),
                                std::ptr::null_mut(),
                                &mut work,
                            )
                        },
                        napi_status::napi_ok
                    );
                    assert_eq!(
                        unsafe { crate::async_work::napi_queue_async_work(napi_env, work) },
                        napi_status::napi_ok
                    );
                    works.push(work);
                }

                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while COMPLETE_COUNT.load(Ordering::SeqCst) < WORK_COUNT
                    && deadline > std::time::Instant::now()
                {
                    driver.drain_ready_jobs(env);
                    thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(COMPLETE_COUNT.load(Ordering::SeqCst), WORK_COUNT);
                assert_eq!(driver.inflight_async.load(Ordering::SeqCst), 0);

                for work in works {
                    assert_eq!(
                        unsafe { crate::async_work::napi_delete_async_work(napi_env, work) },
                        napi_status::napi_ok
                    );
                }
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn aborted_fallback_drain_uses_teardown() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static TEARDOWN_COUNT: AtomicUsize = AtomicUsize::new(0);
        static JS_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn tsfn_call_js(
            env: crate::types::napi_env,
            _js_callback: crate::types::napi_value,
            _context: *mut c_void,
            _data: *mut c_void,
        ) {
            if env.is_null() {
                TEARDOWN_COUNT.fetch_add(1, Ordering::SeqCst);
            } else {
                JS_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                TEARDOWN_COUNT.store(0, Ordering::SeqCst);
                JS_CALL_COUNT.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let func_val = make_tsfn_js_callback(raw.as_ptr());
                let func_handle = value_to_napi_owned(env, func_val);
                let mut tsfn: crate::types::napi_threadsafe_function = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_threadsafe_function(
                            napi_env,
                            func_handle,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            0,
                            1,
                            std::ptr::null_mut(),
                            None,
                            std::ptr::null_mut(),
                            Some(tsfn_call_js),
                            &mut tsfn,
                        )
                    },
                    napi_status::napi_ok
                );
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_call_threadsafe_function(
                            tsfn,
                            0x55usize as *mut c_void,
                            crate::types::napi_threadsafe_function_call_mode::napi_tsfn_nonblocking,
                        )
                    },
                    napi_status::napi_ok
                );
                let tsfn_arc = crate::async_work::lookup_tsfn_for_test(tsfn).unwrap();
                tsfn_arc.aborted.store(true, Ordering::SeqCst);
                tsfn_arc.queue_set_closing_for_test();

                crate::async_work::drain_threadsafe_functions(napi_env);
                assert_eq!(TEARDOWN_COUNT.load(Ordering::SeqCst), 1);
                assert_eq!(JS_CALL_COUNT.load(Ordering::SeqCst), 0);

                unsafe {
                    crate::async_work::napi_release_threadsafe_function(
                        tsfn,
                        crate::types::napi_threadsafe_function_release_mode::napi_tsfn_abort,
                    );
                }
                let driver = crate::driver::ensure_driver(env);
                driver.drain_ready_jobs(env);
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn multi_ref_abort_then_normal_release_tears_down() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};
        use std::thread;
        use std::time::Duration;

        static TEARDOWN_COUNT: AtomicUsize = AtomicUsize::new(0);
        static JS_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn tsfn_call_js(
            env: crate::types::napi_env,
            _js_callback: crate::types::napi_value,
            _context: *mut c_void,
            _data: *mut c_void,
        ) {
            if env.is_null() {
                TEARDOWN_COUNT.fetch_add(1, Ordering::SeqCst);
            } else {
                JS_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                TEARDOWN_COUNT.store(0, Ordering::SeqCst);
                JS_CALL_COUNT.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let func_val = make_tsfn_js_callback(raw.as_ptr());
                let func_handle = value_to_napi_owned(env, func_val);
                let mut tsfn: crate::types::napi_threadsafe_function = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_threadsafe_function(
                            napi_env,
                            func_handle,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            0,
                            2,
                            std::ptr::null_mut(),
                            None,
                            std::ptr::null_mut(),
                            Some(tsfn_call_js),
                            &mut tsfn,
                        )
                    },
                    napi_status::napi_ok
                );
                for i in 0..3 {
                    assert_eq!(
                        unsafe {
                            crate::async_work::napi_call_threadsafe_function(
                                tsfn,
                                (0x100 + i) as *mut c_void,
                                crate::types::napi_threadsafe_function_call_mode::napi_tsfn_nonblocking,
                            )
                        },
                        napi_status::napi_ok
                    );
                }

                let barrier = Arc::new(Barrier::new(2));
                let tsfn_abort = tsfn as usize;
                let barrier_abort = Arc::clone(&barrier);
                let abort_handle = thread::spawn(move || {
                    barrier_abort.wait();
                    unsafe {
                        crate::async_work::napi_release_threadsafe_function(
                            tsfn_abort as crate::types::napi_threadsafe_function,
                            crate::types::napi_threadsafe_function_release_mode::napi_tsfn_abort,
                        )
                    };
                });
                let tsfn_normal = tsfn as usize;
                let barrier_normal = Arc::clone(&barrier);
                let normal_handle = thread::spawn(move || {
                    barrier_normal.wait();
                    // Slight delay so abort sticky bit is usually set first; sticky aborted
                    // still wins even if this last release posts TsfnRelease first.
                    thread::sleep(Duration::from_millis(1));
                    unsafe {
                        crate::async_work::napi_release_threadsafe_function(
                            tsfn_normal as crate::types::napi_threadsafe_function,
                            crate::types::napi_threadsafe_function_release_mode::napi_tsfn_release,
                        )
                    };
                });
                abort_handle.join().unwrap();
                normal_handle.join().unwrap();

                let driver = crate::driver::ensure_driver(env);
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while TEARDOWN_COUNT.load(Ordering::SeqCst) < 3
                    && deadline > std::time::Instant::now()
                {
                    driver.drain_ready_jobs(env);
                    thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(TEARDOWN_COUNT.load(Ordering::SeqCst), 3);
                assert_eq!(JS_CALL_COUNT.load(Ordering::SeqCst), 0);
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    /// napi-rs JsDeferred: `func == null` with non-null `call_js_cb` must deliver.
    #[tokio::test]
    async fn tsfn_null_func_with_call_js_cb_delivers_payload() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        static DELIVERED: AtomicUsize = AtomicUsize::new(0);
        static LAST_DATA: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn tsfn_call_js(
            env: crate::types::napi_env,
            js_callback: crate::types::napi_value,
            _context: *mut c_void,
            data: *mut c_void,
        ) {
            if env.is_null() {
                // Teardown path
                return;
            }
            // Deferred-style: no JS callback function.
            assert!(js_callback.is_null());
            LAST_DATA.store(data as usize, Ordering::SeqCst);
            DELIVERED.fetch_add(1, Ordering::SeqCst);
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                DELIVERED.store(0, Ordering::SeqCst);
                LAST_DATA.store(0, Ordering::SeqCst);
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let mut tsfn: crate::types::napi_threadsafe_function = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_threadsafe_function(
                            napi_env,
                            std::ptr::null_mut(), // func == null
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            0,
                            1,
                            std::ptr::null_mut(),
                            None,
                            std::ptr::null_mut(),
                            Some(tsfn_call_js),
                            &mut tsfn,
                        )
                    },
                    napi_status::napi_ok
                );
                let payload = 0xC0FFEEusize as *mut c_void;
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_call_threadsafe_function(
                            tsfn,
                            payload,
                            crate::types::napi_threadsafe_function_call_mode::napi_tsfn_nonblocking,
                        )
                    },
                    napi_status::napi_ok
                );

                let driver = crate::driver::ensure_driver(env);
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while DELIVERED.load(Ordering::SeqCst) == 0 && deadline > std::time::Instant::now()
                {
                    driver.drain_ready_jobs(env);
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(DELIVERED.load(Ordering::SeqCst), 1);
                assert_eq!(LAST_DATA.load(Ordering::SeqCst), 0xC0FFEE);

                assert_eq!(
                    unsafe {
                        crate::async_work::napi_release_threadsafe_function(
                            tsfn,
                            crate::types::napi_threadsafe_function_release_mode::napi_tsfn_release,
                        )
                    },
                    napi_status::napi_ok
                );
                let deadline = std::time::Instant::now() + Duration::from_secs(1);
                while deadline > std::time::Instant::now() {
                    driver.drain_ready_jobs(env);
                    std::thread::sleep(Duration::from_millis(1));
                }
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn napi_get_node_version_is_24_3_0_node() {
        use std::ffi::CStr;
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let mut version: *const crate::types::napi_node_version = std::ptr::null();
            assert_eq!(
                unsafe { crate::api::napi_get_node_version(env.as_napi_env(), &mut version) },
                napi_status::napi_ok
            );
            assert!(!version.is_null());
            let v = unsafe { &*version };
            assert_eq!(v.major, 24);
            assert_eq!(v.minor, 3);
            assert_eq!(v.patch, 0);
            let release = unsafe { CStr::from_ptr(v.release) }.to_str().unwrap();
            assert_eq!(release, "node");
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn async_work_rejects_foreign_env() {
        let _lock = gc_test_lock().await;
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        let _async_ctx = ctx.clone();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut owner = Env::new(raw);
            let mut foreign = Env::new(raw);
            owner.scopes.open();
            foreign.scopes.open();
            let owner_env = owner.as_napi_env();
            let foreign_env = foreign.as_napi_env();
            let mut work: crate::types::napi_async_work = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    crate::async_work::napi_create_async_work(
                        owner_env,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        None,
                        None,
                        std::ptr::null_mut(),
                        &mut work,
                    )
                },
                napi_status::napi_ok
            );
            assert_eq!(
                unsafe { crate::async_work::napi_queue_async_work(foreign_env, work) },
                napi_status::napi_invalid_arg
            );
            assert_eq!(
                unsafe { crate::async_work::napi_cancel_async_work(foreign_env, work) },
                napi_status::napi_invalid_arg
            );
            assert_eq!(
                unsafe { crate::async_work::napi_delete_async_work(foreign_env, work) },
                napi_status::napi_invalid_arg
            );
            assert_eq!(
                unsafe { crate::async_work::napi_delete_async_work(owner_env, work) },
                napi_status::napi_ok
            );
            owner.scopes.close(raw.as_ptr());
            foreign.scopes.close(raw.as_ptr());
            owner.dispose();
            foreign.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn throw_type_and_range_error_use_correct_constructors() {
        use crate::api::{napi_throw_range_error, napi_throw_type_error};

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let env_ptr = env.as_napi_env();

            assert_eq!(
                unsafe { napi_throw_type_error(env_ptr, std::ptr::null(), c"type boom".as_ptr()) },
                napi_status::napi_ok
            );
            let exc = env.clear_exception().expect("pending type error");
            let val = unsafe { rquickjs::Value::from_raw(ctx.clone(), exc) };
            let obj = val.into_object().expect("error object");
            let name: String = obj.get("name").unwrap();
            assert_eq!(name, "TypeError");
            let check: rquickjs::Function = ctx
                .eval(r#"(function(e){ return e instanceof TypeError && e instanceof Error; })"#)
                .unwrap();
            assert!(check.call::<_, bool>((obj.clone(),)).unwrap());

            assert_eq!(
                unsafe {
                    napi_throw_range_error(env_ptr, std::ptr::null(), c"range boom".as_ptr())
                },
                napi_status::napi_ok
            );
            let exc = env.clear_exception().expect("pending range error");
            let val = unsafe { rquickjs::Value::from_raw(ctx.clone(), exc) };
            let obj = val.into_object().expect("error object");
            let name: String = obj.get("name").unwrap();
            assert_eq!(name, "RangeError");
            let check: rquickjs::Function = ctx
                .eval(r#"(function(e){ return e instanceof RangeError && e instanceof Error; })"#)
                .unwrap();
            assert!(check.call::<_, bool>((obj,)).unwrap());

            env.scopes.close(env.ctx_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn create_error_via_global_constructors() {
        use crate::api::{
            napi_create_error, napi_create_range_error, napi_create_string_utf8,
            napi_create_type_error, napi_get_named_property, napi_get_value_string_utf8,
            napi_typeof,
        };
        use crate::types::napi_valuetype;

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let env_ptr = env.as_napi_env();

            let mut msg = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_string_utf8(env_ptr, c"boom".as_ptr(), 4, &mut msg) },
                napi_status::napi_ok
            );
            let mut code = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_string_utf8(env_ptr, c"ERR_X".as_ptr(), 5, &mut code) },
                napi_status::napi_ok
            );

            // Error with code
            let mut err = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_error(env_ptr, code, msg, &mut err) },
                napi_status::napi_ok
            );
            let mut ty = napi_valuetype::napi_undefined;
            assert_eq!(
                unsafe { napi_typeof(env_ptr, err, &mut ty) },
                napi_status::napi_ok
            );
            assert_eq!(ty, napi_valuetype::napi_object);

            // name === "Error"
            let mut name_h = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_get_named_property(env_ptr, err, c"name".as_ptr(), &mut name_h) },
                napi_status::napi_ok
            );
            let mut buf = [0u8; 32];
            let mut written = 0usize;
            assert_eq!(
                unsafe {
                    napi_get_value_string_utf8(
                        env_ptr,
                        name_h,
                        buf.as_mut_ptr() as *mut _,
                        buf.len(),
                        &mut written,
                    )
                },
                napi_status::napi_ok
            );
            assert_eq!(&buf[..written], b"Error");

            // code === "ERR_X"
            let mut code_h = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_get_named_property(env_ptr, err, c"code".as_ptr(), &mut code_h) },
                napi_status::napi_ok
            );
            written = 0;
            assert_eq!(
                unsafe {
                    napi_get_value_string_utf8(
                        env_ptr,
                        code_h,
                        buf.as_mut_ptr() as *mut _,
                        buf.len(),
                        &mut written,
                    )
                },
                napi_status::napi_ok
            );
            assert_eq!(&buf[..written], b"ERR_X");

            // instanceof Error via QuickJS
            let err_js = unsafe { napi_to_value(&env, err) }.unwrap();
            let global = unsafe { qjs::JS_GetGlobalObject(raw.as_ptr()) };
            let error_ctor =
                unsafe { qjs::JS_GetPropertyStr(raw.as_ptr(), global, c"Error".as_ptr()) };
            let is_inst = unsafe { qjs::JS_IsInstanceOf(raw.as_ptr(), err_js, error_ctor) };
            assert_eq!(is_inst, 1);
            // prototype chain: Object.getPrototypeOf(err) === Error.prototype
            let err_proto = unsafe { qjs::JS_GetPrototype(raw.as_ptr(), err_js) };
            let error_proto =
                unsafe { qjs::JS_GetPropertyStr(raw.as_ptr(), error_ctor, c"prototype".as_ptr()) };
            assert_eq!(
                unsafe { qjs::JS_IsEqual(raw.as_ptr(), err_proto, error_proto) },
                1
            );
            unsafe {
                qjs::JS_FreeValue(raw.as_ptr(), err_proto);
                qjs::JS_FreeValue(raw.as_ptr(), error_proto);
                qjs::JS_FreeValue(raw.as_ptr(), error_ctor);
            }

            // TypeError name + instanceof
            let mut te = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_type_error(env_ptr, std::ptr::null_mut(), msg, &mut te) },
                napi_status::napi_ok
            );
            let te_js = unsafe { napi_to_value(&env, te) }.unwrap();
            let type_error_ctor =
                unsafe { qjs::JS_GetPropertyStr(raw.as_ptr(), global, c"TypeError".as_ptr()) };
            assert_eq!(
                unsafe { qjs::JS_IsInstanceOf(raw.as_ptr(), te_js, type_error_ctor) },
                1
            );
            // Also instanceof Error
            let error_ctor2 =
                unsafe { qjs::JS_GetPropertyStr(raw.as_ptr(), global, c"Error".as_ptr()) };
            assert_eq!(
                unsafe { qjs::JS_IsInstanceOf(raw.as_ptr(), te_js, error_ctor2) },
                1
            );
            let mut te_name = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_get_named_property(env_ptr, te, c"name".as_ptr(), &mut te_name) },
                napi_status::napi_ok
            );
            written = 0;
            assert_eq!(
                unsafe {
                    napi_get_value_string_utf8(
                        env_ptr,
                        te_name,
                        buf.as_mut_ptr() as *mut _,
                        buf.len(),
                        &mut written,
                    )
                },
                napi_status::napi_ok
            );
            assert_eq!(&buf[..written], b"TypeError");

            // RangeError
            let mut re = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_range_error(env_ptr, std::ptr::null_mut(), msg, &mut re) },
                napi_status::napi_ok
            );
            let re_js = unsafe { napi_to_value(&env, re) }.unwrap();
            let range_error_ctor =
                unsafe { qjs::JS_GetPropertyStr(raw.as_ptr(), global, c"RangeError".as_ptr()) };
            assert_eq!(
                unsafe { qjs::JS_IsInstanceOf(raw.as_ptr(), re_js, range_error_ctor) },
                1
            );

            // code=null → no code property (undefined)
            let mut err_no_code = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_error(env_ptr, std::ptr::null_mut(), msg, &mut err_no_code) },
                napi_status::napi_ok
            );
            let err_no_code_js = unsafe { napi_to_value(&env, err_no_code) }.unwrap();
            let code_prop =
                unsafe { qjs::JS_GetPropertyStr(raw.as_ptr(), err_no_code_js, c"code".as_ptr()) };
            assert!(unsafe { qjs::JS_IsUndefined(code_prop) });
            unsafe { qjs::JS_FreeValue(raw.as_ptr(), code_prop) };

            // non-string message → string_expected
            let num = new_int32(42);
            let num_h = value_to_napi_owned(&mut env, num);
            let mut bad = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_error(env_ptr, std::ptr::null_mut(), num_h, &mut bad) },
                napi_status::napi_string_expected
            );

            // non-string code → string_expected
            let mut bad2 = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_error(env_ptr, num_h, msg, &mut bad2) },
                napi_status::napi_string_expected
            );

            // null message → invalid_arg
            let mut bad3 = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    napi_create_error(
                        env_ptr,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        &mut bad3,
                    )
                },
                napi_status::napi_invalid_arg
            );

            unsafe {
                qjs::JS_FreeValue(raw.as_ptr(), type_error_ctor);
                qjs::JS_FreeValue(raw.as_ptr(), error_ctor2);
                qjs::JS_FreeValue(raw.as_ptr(), range_error_ctor);
                qjs::JS_FreeValue(raw.as_ptr(), global);
            }
            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn finish_dispose_refuses_while_inflight_async() {
        use crate::driver::DriverState;
        use std::sync::Arc;

        // Pure driver: inflight keepalive blocks finished even after close_sender.
        let driver = Arc::new(DriverState::new(
            std::thread::current().id(),
            std::ptr::null_mut(),
        ));
        driver.acquire_async_keepalive();
        driver.close_sender();
        driver.mark_finished_if_idle();
        assert!(
            !driver.is_finished(),
            "must not finish while inflight_async > 0"
        );

        driver.release_async_keepalive();
        driver.mark_finished_if_idle();
        assert!(driver.is_finished());
        // Already finished — still bound the wait so a regression cannot hang CI.
        tokio::time::timeout(std::time::Duration::from_secs(5), driver.wait_finished())
            .await
            .expect("wait_finished after idle mark must not hang");
    }

    #[tokio::test]
    async fn finish_dispose_refuses_while_driver_loop_active() {
        let _lock = gc_test_lock().await;
        use crate::driver::{ensure_driver, DriverJob};
        use std::sync::atomic::Ordering;

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();

        let driver = actx
            .with(|ctx| {
                let raw = ctx.as_raw();
                // Registry-owned env so we can finish after wait outside with().
                let env_ptr = crate::dlopen::register_env(raw, Box::new(Env::new(raw)));
                let env = unsafe { &mut *env_ptr };
                let driver = ensure_driver(env);
                driver.idle_refs.fetch_add(1, Ordering::SeqCst);
                assert!(driver.post(env, DriverJob::Wake));
                driver.ensure_loop(env);
                driver.drain_ready_jobs(env);
                env.begin_dispose();
                // Loop/keepalive active — finish must refuse.
                assert!(!env.finish_dispose());
                assert_eq!(env.dispose_state, crate::env::DisposeState::Beginning);
                assert!(!driver.is_finished());
                driver.clone()
            })
            .await;

        // idle_refs still held — driver must not be finished yet.
        assert!(!driver.is_finished());
        // Drop keepalive so the loop can exit once polled.
        driver.idle_refs.fetch_sub(1, Ordering::SeqCst);
        // Real wait — never force mark_finished; bounded so CI cannot hang.
        wait_driver_finished_bounded(&rt, &driver).await;
        assert!(driver.is_finished());
        assert!(driver.is_quiescent());
        assert_eq!(driver.inflight_async.load(Ordering::SeqCst), 0);

        actx.with(|ctx| {
            crate::dlopen::finish_shutdown(&ctx).expect("finish after wait");
            crate::dlopen::shutdown_all().expect("registry empty");
            assert_eq!(crate::async_work::registered_tsfn_count(), 0);
            assert_eq!(crate::async_work::registered_async_work_count(), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn closing_env_rejects_create_async_work() {
        let _lock = gc_test_lock().await;
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let registry_before = crate::async_work::registered_async_work_count();
            env.begin_dispose();
            let sentinel = 0xDEAD_BEEFusize as crate::types::napi_async_work;
            let mut result = sentinel;
            assert_eq!(
                unsafe {
                    crate::async_work::napi_create_async_work(
                        napi_env,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        None,
                        None,
                        std::ptr::null_mut(),
                        &mut result,
                    )
                },
                napi_status::napi_closing
            );
            assert_eq!(result, sentinel);
            assert_eq!(
                crate::async_work::registered_async_work_count(),
                registry_before
            );
            env.scopes.close(raw.as_ptr());
            env.finish_dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn closing_env_rejects_queue_async_work() {
        let _lock = gc_test_lock().await;
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let mut work: crate::types::napi_async_work = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    crate::async_work::napi_create_async_work(
                        napi_env,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        None,
                        None,
                        std::ptr::null_mut(),
                        &mut work,
                    )
                },
                napi_status::napi_ok
            );
            assert_eq!(
                crate::async_work::async_work_status_for_test(work),
                Some(crate::async_work::AW_CREATED_FOR_TEST)
            );
            env.begin_dispose();
            assert_eq!(
                unsafe { crate::async_work::napi_queue_async_work(napi_env, work) },
                napi_status::napi_closing
            );
            assert_eq!(
                crate::async_work::async_work_status_for_test(work),
                Some(crate::async_work::AW_CREATED_FOR_TEST)
            );
            assert!(env.driver.is_none());
            assert_eq!(
                unsafe { crate::async_work::napi_delete_async_work(napi_env, work) },
                napi_status::napi_ok
            );
            env.scopes.close(raw.as_ptr());
            env.finish_dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn closing_env_rejects_create_threadsafe_function() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;

        unsafe extern "C" fn tsfn_call_js(
            _env: crate::types::napi_env,
            _js_callback: crate::types::napi_value,
            _context: *mut c_void,
            _data: *mut c_void,
        ) {
        }

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let napi_env = env.as_napi_env();
            let tsfn_before = crate::async_work::registered_tsfn_count();
            env.begin_dispose();
            let sentinel = 0xFEED_FACEusize as crate::types::napi_threadsafe_function;
            let mut result = sentinel;
            assert_eq!(
                unsafe {
                    crate::async_work::napi_create_threadsafe_function(
                        napi_env,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        0,
                        1,
                        std::ptr::null_mut(),
                        None,
                        std::ptr::null_mut(),
                        Some(tsfn_call_js),
                        &mut result,
                    )
                },
                napi_status::napi_closing
            );
            assert_eq!(result, sentinel);
            assert_eq!(crate::async_work::registered_tsfn_count(), tsfn_before);
            assert!(env.driver.is_none());
            env.scopes.close(raw.as_ptr());
            env.finish_dispose();
        })
        .await;
    }

    #[tokio::test]
    async fn active_env_async_work_and_tsfn_still_ok() {
        let _lock = gc_test_lock().await;
        use std::os::raw::c_void;

        unsafe extern "C" fn tsfn_call_js(
            _env: crate::types::napi_env,
            _js_callback: crate::types::napi_value,
            _context: *mut c_void,
            _data: *mut c_void,
        ) {
        }

        let rt = AsyncRuntime::new().unwrap();
        let actx = AsyncContext::full(&rt).await.unwrap();
        let driver = actx
            .with(|ctx| {
                let raw = ctx.as_raw();
                let mut env_ptr = register_test_env(raw);
                let env = unsafe { env_ptr.as_mut() };
                env.scopes.open();
                let napi_env = env.as_napi_env();
                let mut work: crate::types::napi_async_work = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_async_work(
                            napi_env,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            None,
                            None,
                            std::ptr::null_mut(),
                            &mut work,
                        )
                    },
                    napi_status::napi_ok
                );
                assert_eq!(
                    unsafe { crate::async_work::napi_queue_async_work(napi_env, work) },
                    napi_status::napi_ok
                );
                let driver = crate::driver::ensure_driver(env);
                driver.drain_ready_jobs(env);
                assert_eq!(
                    unsafe { crate::async_work::napi_delete_async_work(napi_env, work) },
                    napi_status::napi_ok
                );

                let mut tsfn: crate::types::napi_threadsafe_function = std::ptr::null_mut();
                assert_eq!(
                    unsafe {
                        crate::async_work::napi_create_threadsafe_function(
                            napi_env,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            0,
                            1,
                            std::ptr::null_mut(),
                            None,
                            std::ptr::null_mut(),
                            Some(tsfn_call_js),
                            &mut tsfn,
                        )
                    },
                    napi_status::napi_ok
                );
                assert!(!tsfn.is_null());
                unsafe {
                    crate::async_work::napi_release_threadsafe_function(
                        tsfn,
                        crate::types::napi_threadsafe_function_release_mode::napi_tsfn_release,
                    );
                }
                driver.drain_ready_jobs(env);
                unsafe { begin_registry_dispose(env_ptr, raw) }
            })
            .await;
        finish_registry_env(&rt, &actx, driver).await;
    }

    #[tokio::test]
    async fn finish_dispose_leaves_no_env_js_roots() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let obj = unsafe { qjs::JS_NewObject(raw.as_ptr()) };
            let reference = env
                .refs
                .create(raw.as_ptr(), obj, 1, env.as_napi_env(), false);
            assert!(!reference.is_null());
            unsafe {
                qjs::JS_FreeValue(raw.as_ptr(), obj);
            }
            env.begin_dispose();
            assert!(env.try_finish_dispose().is_ok());
            assert_eq!(env.refs.len(), 0);
            let counts = env.scopes.counts();
            assert_eq!(counts.scopes, 0);
            assert_eq!(counts.values, 0);
            assert_eq!(counts.handles, 0);
            assert_eq!(env.wraps.len(), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn begin_dispose_is_idempotent() {
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.begin_dispose();
            assert_eq!(env.dispose_state, crate::env::DisposeState::Beginning);
            env.begin_dispose(); // second call no-op
            assert_eq!(env.dispose_state, crate::env::DisposeState::Beginning);
            assert!(env.finish_dispose());
            assert_eq!(env.dispose_state, crate::env::DisposeState::Finished);
            assert!(env.finish_dispose()); // already finished
        })
        .await;
    }

    #[tokio::test]
    async fn shutdown_all_errors_on_residual_registry() {
        let _lock = gc_test_lock().await;
        use std::ptr::NonNull;

        assert!(crate::dlopen::shutdown_all().is_ok());

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let _ptr = crate::dlopen::register_env(raw, Box::new(Env::new(raw)));
            let err = crate::dlopen::shutdown_all().expect_err("residual env");
            assert!(err.contains("shutdown incomplete"), "unexpected: {err}");
            // Proper cleanup: two-phase without force-mark.
            let driver = crate::dlopen::begin_shutdown(&ctx).unwrap();
            drop(driver);
            crate::dlopen::finish_shutdown(&ctx).unwrap();
            assert!(crate::dlopen::shutdown_all().is_ok());
            let _ = NonNull::new(raw.as_ptr());
        })
        .await;
    }

    #[tokio::test]
    async fn napi_buffer_apis_and_is_buffer_parity() {
        let _lock = gc_test_lock().await;
        use crate::api::{
            napi_create_buffer, napi_create_buffer_copy, napi_create_external_buffer,
            napi_get_buffer_info, napi_is_buffer,
        };
        use std::os::raw::c_void;
        use std::sync::atomic::{AtomicUsize, Ordering};

        unsafe extern "C" fn external_buf_finalize(
            _env: crate::types::napi_env,
            data: *mut c_void,
            hint: *mut c_void,
        ) {
            let calls = &*(hint as *const AtomicUsize);
            calls.fetch_add(1, Ordering::SeqCst);
            let _ = Box::from_raw(data as *mut [u8; 4]);
        }

        let finalize_calls = AtomicUsize::new(0);
        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            ctx.eval::<(), _>("globalThis.Buffer = class Buffer extends Uint8Array {};")
                .unwrap();
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let env_ptr = env.as_napi_env();

            // 1. napi_create_buffer + write via data pointer + napi_is_buffer + napi_get_buffer_info
            let mut data_ptr: *mut c_void = std::ptr::null_mut();
            let mut buf_val = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_buffer(env_ptr, 4, &mut data_ptr, &mut buf_val) },
                napi_status::napi_ok
            );
            assert!(!data_ptr.is_null());
            unsafe {
                std::slice::from_raw_parts_mut(data_ptr as *mut u8, 4)
                    .copy_from_slice(&[10, 20, 30, 40]);
            }
            let mut is_buf = false;
            assert_eq!(
                unsafe { napi_is_buffer(env_ptr, buf_val, &mut is_buf) },
                napi_status::napi_ok
            );
            assert!(is_buf);

            let mut info_ptr: *mut c_void = std::ptr::null_mut();
            let mut info_len = 0usize;
            assert_eq!(
                unsafe { napi_get_buffer_info(env_ptr, buf_val, &mut info_ptr, &mut info_len) },
                napi_status::napi_ok
            );
            assert_eq!(info_len, 4);
            assert_eq!(
                unsafe { std::slice::from_raw_parts(info_ptr as *const u8, 4) },
                &[10, 20, 30, 40]
            );

            // Plain Uint8Array -> is_buffer false, get_buffer_info reads slice with byteOffset
            let u8_js = ctx
                .eval::<rquickjs::Value, _>("new Uint8Array([1, 2, 3, 4]).subarray(1, 3)")
                .unwrap();
            let u8_handle = value_to_napi_owned(&mut env, unsafe {
                qjs::JS_DupValue(raw.as_ptr(), u8_js.as_raw())
            });
            drop(u8_js);
            assert_eq!(
                unsafe { napi_is_buffer(env_ptr, u8_handle, &mut is_buf) },
                napi_status::napi_ok
            );
            assert!(!is_buf);
            assert_eq!(
                unsafe { napi_get_buffer_info(env_ptr, u8_handle, &mut info_ptr, &mut info_len) },
                napi_status::napi_ok
            );
            assert_eq!(info_len, 2);
            assert_eq!(
                unsafe { std::slice::from_raw_parts(info_ptr as *const u8, 2) },
                &[2, 3]
            );

            // Non-buffer passed to napi_get_buffer_info leaves no pending JS exception.
            let num_handle = value_to_napi_owned(&mut env, new_int32(42));
            assert_ne!(
                unsafe { napi_get_buffer_info(env_ptr, num_handle, &mut info_ptr, &mut info_len) },
                napi_status::napi_ok
            );
            assert!(!unsafe { qjs::JS_HasException(raw.as_ptr()) });

            // 2. 5-arg napi_create_buffer_copy (including zero-length null src)
            let src = [7u8, 8, 9];
            let mut copy_ptr: *mut c_void = std::ptr::null_mut();
            let mut copy_val = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    napi_create_buffer_copy(
                        env_ptr,
                        src.len(),
                        src.as_ptr() as *const c_void,
                        &mut copy_ptr,
                        &mut copy_val,
                    )
                },
                napi_status::napi_ok
            );
            assert!(!copy_ptr.is_null());
            assert_ne!(copy_ptr as usize, src.as_ptr() as usize);
            assert_eq!(
                unsafe { std::slice::from_raw_parts(copy_ptr as *const u8, 3) },
                &[7, 8, 9]
            );

            let mut zero_ptr: *mut c_void = std::ptr::null_mut();
            let mut zero_val = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    napi_create_buffer_copy(
                        env_ptr,
                        0,
                        std::ptr::null(),
                        &mut zero_ptr,
                        &mut zero_val,
                    )
                },
                napi_status::napi_ok
            );
            assert!(!zero_ptr.is_null());

            // 3. napi_create_external_buffer + finalizer on dispose
            let ext_box = Box::into_raw(Box::new([11u8, 22, 33, 44]));
            let mut ext_val = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    napi_create_external_buffer(
                        env_ptr,
                        4,
                        ext_box as *mut c_void,
                        Some(external_buf_finalize),
                        &finalize_calls as *const AtomicUsize as *mut c_void,
                        &mut ext_val,
                    )
                },
                napi_status::napi_ok
            );
            assert_eq!(
                unsafe { napi_get_buffer_info(env_ptr, ext_val, &mut info_ptr, &mut info_len) },
                napi_status::napi_ok
            );
            assert_eq!(info_ptr as usize, ext_box as usize);
            assert_eq!(info_len, 4);

            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
        assert_eq!(finalize_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn napi_predicates_bigint_words_utf16_and_fatal_exception() {
        use crate::api::{
            napi_create_array_with_length, napi_create_string_utf8, napi_fatal_exception,
            napi_get_and_clear_last_exception, napi_get_array_length, napi_get_value_bigint_words,
            napi_get_value_string_utf16, napi_is_array, napi_is_error, napi_is_exception_pending,
            napi_strict_equals,
        };
        use std::os::raw::c_int;

        let rt = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&rt).await.unwrap();
        ctx.with(|ctx| {
            let raw = ctx.as_raw();
            let mut env = Env::new(raw);
            env.scopes.open();
            let env_ptr = env.as_napi_env();

            // napi_create_array_with_length & napi_is_array
            let mut arr_val = std::ptr::null_mut();
            assert_eq!(
                unsafe { napi_create_array_with_length(env_ptr, 5, &mut arr_val) },
                napi_status::napi_ok
            );
            let mut arr_len = 0u32;
            assert_eq!(unsafe { napi_get_array_length(env_ptr, arr_val, &mut arr_len) }, napi_status::napi_ok);
            assert_eq!(arr_len, 5);
            let mut flag = false;
            assert_eq!(unsafe { napi_is_array(env_ptr, arr_val, &mut flag) }, napi_status::napi_ok);
            assert!(flag);

            // napi_is_error
            let err_js = ctx.eval::<rquickjs::Value, _>("new TypeError('boom')").unwrap();
            let err_val = value_to_napi_owned(&mut env, unsafe { qjs::JS_DupValue(raw.as_ptr(), err_js.as_raw()) });
            drop(err_js);
            assert_eq!(unsafe { napi_is_error(env_ptr, err_val, &mut flag) }, napi_status::napi_ok);
            assert!(flag);
            assert_eq!(unsafe { napi_is_error(env_ptr, arr_val, &mut flag) }, napi_status::napi_ok);
            assert!(!flag);

            // napi_strict_equals (1 vs "1" false; NaN vs NaN false)
            let mut s_one = std::ptr::null_mut();
            assert_eq!(unsafe { napi_create_string_utf8(env_ptr, c"1".as_ptr(), 1, &mut s_one) }, napi_status::napi_ok);
            let n_one = value_to_napi_owned(&mut env, new_int32(1));
            assert_eq!(unsafe { napi_strict_equals(env_ptr, n_one, s_one, &mut flag) }, napi_status::napi_ok);
            assert!(!flag);
            assert_eq!(unsafe { napi_strict_equals(env_ptr, n_one, n_one, &mut flag) }, napi_status::napi_ok);
            assert!(flag);

            // napi_get_value_string_utf16 (query length + surrogate pairs + truncation)
            let s_js = ctx.eval::<rquickjs::Value, _>("'A\\u{1F680}Z'").unwrap();
            let s_val = value_to_napi_owned(&mut env, unsafe { qjs::JS_DupValue(raw.as_ptr(), s_js.as_raw()) });
            drop(s_js);
            let mut utf16_len = 0usize;
            assert_eq!(
                unsafe { napi_get_value_string_utf16(env_ptr, s_val, std::ptr::null_mut(), 0, &mut utf16_len) },
                napi_status::napi_ok
            );
            assert_eq!(utf16_len, 4); // 'A' (1) + '🚀' surrogate pair (2) + 'Z' (1)
            let mut small = [0xFFFFu16; 3];
            assert_eq!(
                unsafe { napi_get_value_string_utf16(env_ptr, s_val, small.as_mut_ptr(), small.len(), &mut utf16_len) },
                napi_status::napi_ok
            );
            assert_eq!(utf16_len, 2);
            assert_eq!(small, [0x0041, 0xD83D, 0]);

            // napi_get_value_bigint_words
            let bi_zero = ctx.eval::<rquickjs::Value, _>("0n").unwrap();
            let bi_zero_h = value_to_napi_owned(&mut env, unsafe { qjs::JS_DupValue(raw.as_ptr(), bi_zero.as_raw()) });
            drop(bi_zero);
            let mut sign: c_int = 99;
            let mut wc: usize = 4;
            let mut words = [0u64; 4];
            assert_eq!(
                unsafe { napi_get_value_bigint_words(env_ptr, bi_zero_h, &mut sign, &mut wc, words.as_mut_ptr()) },
                napi_status::napi_ok
            );
            assert_eq!(sign, 0);
            assert_eq!(wc, 0);

            let bi_neg = ctx.eval::<rquickjs::Value, _>("-((1n << 64n) + 0x123456789abcdef0n)").unwrap();
            let bi_neg_h = value_to_napi_owned(&mut env, unsafe { qjs::JS_DupValue(raw.as_ptr(), bi_neg.as_raw()) });
            drop(bi_neg);
            let mut query_wc = 0usize;
            assert_eq!(
                unsafe { napi_get_value_bigint_words(env_ptr, bi_neg_h, &mut sign, &mut query_wc, std::ptr::null_mut()) },
                napi_status::napi_ok
            );
            assert_eq!(sign, 1);
            assert_eq!(query_wc, 2);
            wc = 2;
            assert_eq!(
                unsafe { napi_get_value_bigint_words(env_ptr, bi_neg_h, &mut sign, &mut wc, words.as_mut_ptr()) },
                napi_status::napi_ok
            );
            assert_eq!(sign, 1);
            assert_eq!(wc, 2);
            assert_eq!(&words[..2], &[0x123456789abcdef0, 1]);

            // napi_fatal_exception (pending exception when unhandled, handled when process.emit returns true)
            assert_eq!(unsafe { napi_fatal_exception(env_ptr, err_val) }, napi_status::napi_ok);
            let mut pending = false;
            assert_eq!(unsafe { napi_is_exception_pending(env_ptr, &mut pending) }, napi_status::napi_ok);
            assert!(pending);
            let mut cleared = std::ptr::null_mut();
            assert_eq!(unsafe { napi_get_and_clear_last_exception(env_ptr, &mut cleared) }, napi_status::napi_ok);

            ctx.eval::<(), _>("globalThis.process = { emit(ev, e) { globalThis.__caught = [ev, e.message]; return true; } };").unwrap();
            assert_eq!(unsafe { napi_fatal_exception(env_ptr, err_val) }, napi_status::napi_ok);
            assert_eq!(unsafe { napi_is_exception_pending(env_ptr, &mut pending) }, napi_status::napi_ok);
            assert!(!pending);
            let caught: String = ctx.eval("globalThis.__caught.join(':')").unwrap();
            assert_eq!(caught, "uncaughtException:boom");

            env.scopes.close(raw.as_ptr());
            env.dispose();
        })
        .await;
    }
}
