use once_cell::sync::Lazy;
use parking_lot::Mutex;

static ABI_TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn abi_test_lock() -> parking_lot::MutexGuard<'static, ()> {
    ABI_TEST_LOCK.lock()
}

#[test]
fn abi_profile_constants() {
    assert_eq!(crate::NODE_MODULE_VERSION, 137);
    assert_eq!(crate::NODE_VERSION, "24.3.0");
    assert_eq!(crate::ABI_PROFILE_LABEL, "node24-abi137");
}

#[test]
fn runtime_context_lifecycle_counts_match() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    crate::runtime_state::reset_lifecycle_counters_for_tests();
    let (ci0, di0, cc0, dc0) = crate::runtime_state::lifecycle_counts();
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ctx_ptr = context.as_raw().as_ptr();
    crate::bind_bridge(ctx_ptr);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx_ptr) };
    let _isolate = crate::ensure_isolate_for_runtime(qrt);
    let _context = crate::ensure_context_for_ctx(ctx_ptr);
    unsafe { crate::shutdown_context(ctx_ptr).unwrap() };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
    let (ci1, di1, cc1, dc1) = crate::runtime_state::lifecycle_counts();
    assert_eq!(ci1 - ci0, di1 - di0);
    assert_eq!(cc1 - cc0, dc1 - dc0);
    assert!(ci1 > ci0);
    assert!(cc1 > cc0);
}

#[test]
fn per_context_tables_do_not_leak_across_contexts() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let ctx_a = Context::full(&runtime).expect("ctx_a");
    let ctx_b = Context::full(&runtime).expect("ctx_b");
    let ptr_a = ctx_a.as_raw().as_ptr();
    let ptr_b = ctx_b.as_raw().as_ptr();
    crate::bind_bridge(ptr_a);

    let key_a = unsafe {
        let obj = rquickjs::qjs::JS_NewObject(ptr_a);
        let key = rquickjs::qjs::JS_VALUE_GET_PTR(obj) as usize;
        rquickjs::qjs::JS_FreeValue(ptr_a, obj);
        key
    };
    crate::context_tables::with_context_tables(ptr_a, |tables| {
        tables.internal_fields.insert(key_a, vec![42]);
    });

    let count_b =
        crate::context_tables::with_context_tables(ptr_b, |tables| tables.internal_fields.len());
    assert_eq!(count_b, 0);

    unsafe { crate::shutdown_context(ptr_a).unwrap() };
    unsafe { crate::shutdown_context(ptr_b).unwrap() };
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ptr_a) };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
}

#[test]
fn cleanup_hook_runs_for_context_scope() {
    let _lock = abi_test_lock();
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn hook(_arg: *mut std::ffi::c_void) {
        CALLED.fetch_add(1, Ordering::Relaxed);
    }

    let context_ptr = 0xCAFE_BABEusize;
    crate::runtime_state::context_key(context_ptr);
    crate::runtime_state::add_cleanup_hook(context_ptr, hook, std::ptr::null_mut());
    crate::runtime_state::forget_context(context_ptr);
    assert_eq!(CALLED.load(Ordering::Relaxed), 1);
}

#[test]
fn pending_weak_queue_is_per_context() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let ctx_a = Context::full(&runtime).expect("ctx_a");
    let ctx_b = Context::full(&runtime).expect("ctx_b");
    let ptr_a = ctx_a.as_raw().as_ptr();
    let ptr_b = ctx_b.as_raw().as_ptr();

    crate::context_tables::with_context_tables(ptr_a, |tables| {
        tables
            .pending_weak
            .push(crate::context_tables::PendingWeak {
                callback: 1,
                parameter: 2,
                object_key: 3,
                pass: crate::context_tables::WeakPass::First,
            });
    });

    let pending_b = crate::context_tables::take_pending_weak_callbacks(ptr_b);
    assert!(pending_b.is_empty());
    let pending_a = crate::context_tables::take_pending_weak_callbacks(ptr_a);
    assert_eq!(pending_a.len(), 1);

    unsafe {
        crate::shutdown_context(ptr_a).unwrap();
        crate::shutdown_context(ptr_b).unwrap();
    }

    assert!(!crate::context_tables::has_context_tables(ptr_a));
    assert!(!crate::context_tables::has_context_tables(ptr_b));

    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ptr_a) };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
}

#[test]
fn cleanup_hook_add_and_remove() {
    let _lock = abi_test_lock();
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn hook(_arg: *mut std::ffi::c_void) {
        CALLED.fetch_add(1, Ordering::Relaxed);
    }

    let isolate_ptr = 0xDEAD_BEEFusize;
    crate::runtime_state::add_cleanup_hook(isolate_ptr, hook, std::ptr::null_mut());
    crate::runtime_state::remove_cleanup_hook(isolate_ptr, hook, std::ptr::null_mut());
    crate::runtime_state::run_cleanup_hooks(isolate_ptr);
    assert_eq!(CALLED.load(Ordering::Relaxed), 0);

    crate::runtime_state::add_cleanup_hook(isolate_ptr, hook, std::ptr::null_mut());
    crate::runtime_state::run_cleanup_hooks(isolate_ptr);
    assert_eq!(CALLED.load(Ordering::Relaxed), 1);
}

#[test]
fn bridge_roots_are_per_context() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let ctx_a = Context::full(&runtime).expect("ctx_a");
    let ctx_b = Context::full(&runtime).expect("ctx_b");
    let ptr_a = ctx_a.as_raw().as_ptr();
    let ptr_b = ctx_b.as_raw().as_ptr();
    crate::bind_bridge(ptr_a);
    crate::bind_bridge(ptr_b);

    crate::bridge::set_active_bridge_context(ptr_a);
    let root_a = crate::bridge::with_bridge_roots(|ctx, roots| {
        let obj = unsafe { rquickjs::qjs::JS_NewObject(ctx) };
        let root = roots.insert_borrowed(ctx, obj);
        unsafe { rquickjs::qjs::JS_FreeValue(ctx, obj) };
        root
    });

    let visible_in_b =
        crate::bridge::with_state_ref_for_ctx(ptr_b, |state| state.roots.get(root_a).is_some());
    assert!(!visible_in_b);

    unsafe { crate::shutdown_context(ptr_a).unwrap() };
    unsafe { crate::shutdown_context(ptr_b).unwrap() };
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ptr_a) };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
}

#[test]
fn context_tables_survive_sibling_shutdown() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let ctx_a = Context::full(&runtime).expect("ctx_a");
    let ctx_b = Context::full(&runtime).expect("ctx_b");
    let ptr_a = ctx_a.as_raw().as_ptr();
    let ptr_b = ctx_b.as_raw().as_ptr();
    crate::bind_bridge(ptr_a);
    crate::bind_bridge(ptr_b);

    let key_b = unsafe {
        let obj = rquickjs::qjs::JS_NewObject(ptr_b);
        let key = rquickjs::qjs::JS_VALUE_GET_PTR(obj) as usize;
        rquickjs::qjs::JS_FreeValue(ptr_b, obj);
        key
    };
    crate::context_tables::with_context_tables(ptr_b, |tables| {
        tables.internal_fields.insert(key_b, vec![99]);
    });

    unsafe { crate::bridge::prepare_shutdown(ptr_a).unwrap() };

    let still_there = crate::context_tables::with_context_tables(ptr_b, |tables| {
        tables.internal_fields.get(&key_b).cloned()
    });
    assert_eq!(still_there, Some(vec![99]));

    unsafe { crate::shutdown_context(ptr_a).unwrap() };
    unsafe { crate::shutdown_context(ptr_b).unwrap() };
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ptr_a) };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
}

#[test]
fn dual_context_shutdown_preserves_sibling_until_runtime_free() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let ctx_a = Context::full(&runtime).expect("ctx_a");
    let ctx_b = Context::full(&runtime).expect("ctx_b");
    let ptr_a = ctx_a.as_raw().as_ptr();
    let ptr_b = ctx_b.as_raw().as_ptr();
    crate::bind_bridge(ptr_a);
    crate::bind_bridge(ptr_b);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ptr_a) };

    let obj_a = crate::js_ops::new_v8_object(ptr_a);
    unsafe { rquickjs::qjs::JS_FreeValue(ptr_a, obj_a) };
    assert!(crate::js_ops::v8_object_class_for_runtime(qrt).is_some());

    unsafe { crate::shutdown_context(ptr_a).unwrap() };

    crate::bridge::set_active_bridge_context(ptr_b);
    let root_b = crate::bridge::with_bridge_roots(|ctx, roots| {
        let obj = unsafe { rquickjs::qjs::JS_NewObject(ctx) };
        let root = roots.insert_borrowed(ctx, obj);
        unsafe { rquickjs::qjs::JS_FreeValue(ctx, obj) };
        root
    });
    assert!(crate::bridge::with_state_ref_for_ctx(ptr_b, |state| state
        .roots
        .get(root_b)
        .is_some()));

    let obj_b = crate::js_ops::new_v8_object(ptr_b);
    assert!(!unsafe { rquickjs::qjs::JS_IsException(obj_b) });
    unsafe { rquickjs::qjs::JS_FreeValue(ptr_b, obj_b) };

    unsafe { crate::shutdown_context(ptr_b).unwrap() };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
    assert!(crate::js_ops::v8_object_class_for_runtime(qrt).is_none());
}

#[test]
fn v8_object_class_cleared_after_dual_context_shutdown() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let ctx_a = Context::full(&runtime).expect("ctx_a");
    let ctx_b = Context::full(&runtime).expect("ctx_b");
    let ptr_a = ctx_a.as_raw().as_ptr();
    let ptr_b = ctx_b.as_raw().as_ptr();
    crate::bind_bridge(ptr_a);
    crate::bind_bridge(ptr_b);

    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ptr_a) };
    let obj_a = crate::js_ops::new_v8_object(ptr_a);
    let obj_b = crate::js_ops::new_v8_object(ptr_b);
    assert!(crate::js_ops::v8_object_class_for_runtime(qrt).is_some());
    unsafe {
        rquickjs::qjs::JS_FreeValue(ptr_a, obj_a);
        rquickjs::qjs::JS_FreeValue(ptr_b, obj_b);
    }

    unsafe { crate::shutdown_context(ptr_a).unwrap() };
    unsafe { crate::shutdown_context(ptr_b).unwrap() };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
    assert!(crate::js_ops::v8_object_class_for_runtime(qrt).is_none());
}

#[test]
fn buffer_copy_zero_length_allows_null_data() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ctx_ptr = context.as_raw().as_ptr();
    crate::bind_bridge(ctx_ptr);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx_ptr) };
    // Teardown activation requires the same isolate/context pairing production uses.
    let _isolate = crate::ensure_isolate_for_runtime(qrt);
    let _state = crate::ensure_context_for_ctx(ctx_ptr);
    crate::bridge::set_active_bridge_context(ctx_ptr);

    let global = unsafe { rquickjs::qjs::JS_GetGlobalObject(ctx_ptr) };
    let has_buffer = unsafe {
        !rquickjs::qjs::JS_IsUndefined(rquickjs::qjs::JS_GetPropertyStr(
            ctx_ptr,
            global,
            c"Buffer".as_ptr(),
        ))
    };
    unsafe { rquickjs::qjs::JS_FreeValue(ctx_ptr, global) };
    if !has_buffer {
        unsafe { crate::shutdown_context(ctx_ptr).unwrap() };
        unsafe { crate::shutdown_runtime(qrt).unwrap() };
        return;
    }

    let mut root: u64 = 0;
    let status = unsafe {
        crate::value_ops::buffer_new_copy(std::ptr::null_mut(), std::ptr::null(), 0, &mut root)
    };
    assert!(matches!(status, crate::bridge::LexeV8Status::Ok));
    assert_ne!(root, 0);

    unsafe { crate::shutdown_context(ctx_ptr).unwrap() };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
}

#[test]
fn current_thread_locals_cleared_after_teardown() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    extern "C" {
        fn lexe_v8_current_context() -> *mut crate::bridge::LexeV8ContextState;
        fn lexe_v8_current_isolate() -> *mut crate::bridge::LexeV8IsolateState;
        fn lexe_v8_set_current_context(ctx: *mut crate::bridge::LexeV8ContextState);
        fn lexe_v8_set_current_isolate(isolate: *mut crate::bridge::LexeV8IsolateState);
    }

    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ctx_ptr = context.as_raw().as_ptr();
    crate::bind_bridge(ctx_ptr);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx_ptr) };
    let isolate = crate::ensure_isolate_for_runtime(qrt);
    let context_state = crate::ensure_context_for_ctx(ctx_ptr);

    unsafe {
        lexe_v8_set_current_context(context_state as *mut crate::bridge::LexeV8ContextState);
        lexe_v8_set_current_isolate(isolate as *mut crate::bridge::LexeV8IsolateState);
    }
    assert!(!unsafe { lexe_v8_current_context().is_null() });
    assert!(!unsafe { lexe_v8_current_isolate().is_null() });

    unsafe { crate::shutdown_context(ctx_ptr).unwrap() };
    assert!(unsafe { lexe_v8_current_context().is_null() });

    unsafe { crate::shutdown_runtime(qrt).unwrap() };
    assert!(unsafe { lexe_v8_current_isolate().is_null() });
}

#[test]
fn isolate_cleanup_hook_runs_only_at_runtime_shutdown() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn hook(_arg: *mut std::ffi::c_void) {
        CALLED.fetch_add(1, Ordering::Relaxed);
    }

    let runtime = Runtime::new().expect("runtime");
    let ctx_a = Context::full(&runtime).expect("ctx_a");
    let ctx_b = Context::full(&runtime).expect("ctx_b");
    let ptr_a = ctx_a.as_raw().as_ptr();
    let ptr_b = ctx_b.as_raw().as_ptr();
    // Create each ContextState while its own bridge is active so context roots
    // land in the owner root table (with_bridge_roots uses active_ctx_key).
    crate::bind_bridge(ptr_a);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ptr_a) };
    let isolate = crate::ensure_isolate_for_runtime(qrt);
    let _ctx_a = crate::ensure_context_for_ctx(ptr_a);
    crate::bind_bridge(ptr_b);
    let _ctx_b = crate::ensure_context_for_ctx(ptr_b);

    crate::runtime_state::add_cleanup_hook(isolate as usize, hook, std::ptr::null_mut());
    CALLED.store(0, Ordering::Relaxed);

    unsafe { crate::shutdown_context(ptr_a).unwrap() };
    assert_eq!(CALLED.load(Ordering::Relaxed), 0);

    unsafe { crate::shutdown_context(ptr_b).unwrap() };
    assert_eq!(CALLED.load(Ordering::Relaxed), 0);

    unsafe { crate::shutdown_runtime(qrt).unwrap() };
    assert_eq!(CALLED.load(Ordering::Relaxed), 1);
}

#[test]
fn shutdown_context_clears_bridge_and_context_registry() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ctx_ptr = context.as_raw().as_ptr();
    crate::bind_bridge(ctx_ptr);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx_ptr) };
    let _isolate = crate::ensure_isolate_for_runtime(qrt);
    let _ctx_state = crate::ensure_context_for_ctx(ctx_ptr);

    unsafe { crate::shutdown_context(ctx_ptr).unwrap() };
    assert!(!crate::bridge::has_bridge_for_ctx(ctx_ptr));
    assert_eq!(crate::contexts_for_runtime(qrt), 0);
    assert_eq!(crate::bridge::residual_root_count_for_runtime(qrt), 0);

    unsafe { crate::shutdown_runtime(qrt).unwrap() };
}

#[test]
fn repeated_shutdown_is_idempotent() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ctx_ptr = context.as_raw().as_ptr();
    crate::bind_bridge(ctx_ptr);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx_ptr) };
    let _isolate = crate::ensure_isolate_for_runtime(qrt);
    let _ctx_state = crate::ensure_context_for_ctx(ctx_ptr);

    unsafe { crate::shutdown_context(ctx_ptr).unwrap() };
    unsafe { crate::shutdown_context(ctx_ptr).unwrap() };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
    unsafe { crate::shutdown_runtime(qrt).unwrap() };
}

#[test]
fn cleanup_hooks_run_lifo_once_per_scope() {
    let _lock = abi_test_lock();
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ORDER: AtomicUsize = AtomicUsize::new(0);
    static LOG: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

    unsafe extern "C" fn hook_a(_arg: *mut std::ffi::c_void) {
        LOG.lock().unwrap().push(1);
        ORDER.fetch_add(1, Ordering::SeqCst);
    }
    unsafe extern "C" fn hook_b(_arg: *mut std::ffi::c_void) {
        LOG.lock().unwrap().push(2);
        ORDER.fetch_add(1, Ordering::SeqCst);
    }

    let context_ptr = 0xDEAD_BEEFusize;
    crate::runtime_state::context_key(context_ptr);
    crate::runtime_state::add_cleanup_hook(context_ptr, hook_a, std::ptr::null_mut());
    crate::runtime_state::add_cleanup_hook(context_ptr, hook_b, std::ptr::null_mut());
    crate::runtime_state::forget_context(context_ptr);

    let log = LOG.lock().unwrap().clone();
    assert_eq!(log, vec![2, 1]);
    assert_eq!(ORDER.load(Ordering::SeqCst), 2);
}

#[test]
fn objectwrap_fixture_teardown_clears_all_counts() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    extern "C" {
        fn lexe_v8_test_objectwrap_fixture_setup(
            ctx: *mut crate::bridge::LexeV8ContextState,
        ) -> *mut std::ffi::c_void;
        fn lexe_v8_test_objectwrap_fixture_release_bridge_roots(
            ctx: *mut crate::bridge::LexeV8ContextState,
            counters: *mut std::ffi::c_void,
        );
        fn lexe_v8_test_objectwrap_fixture_cleanup_hook(arg: *mut std::ffi::c_void);
        fn lexe_v8_test_objectwrap_fixture_read_counts(
            counters: *const std::ffi::c_void,
            cleanup_out: *mut i32,
            weak_out: *mut i32,
            destructor_out: *mut i32,
        );
        fn lexe_v8_test_objectwrap_fixture_destroy(counters: *mut std::ffi::c_void);
    }

    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ctx_ptr = context.as_raw().as_ptr();
    crate::bind_bridge(ctx_ptr);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx_ptr) };
    let _isolate = crate::ensure_isolate_for_runtime(qrt);
    let context_state = crate::ensure_context_for_ctx(ctx_ptr);
    crate::bridge::set_active_bridge_context(ctx_ptr);

    extern "C" {
        fn lexe_v8_set_current_context(ctx: *mut crate::bridge::LexeV8ContextState);
        fn lexe_v8_set_current_isolate(isolate: *mut crate::bridge::LexeV8IsolateState);
    }
    unsafe {
        lexe_v8_set_current_context(context_state as *mut crate::bridge::LexeV8ContextState);
        lexe_v8_set_current_isolate(_isolate as *mut crate::bridge::LexeV8IsolateState);
    }

    let counters = unsafe {
        lexe_v8_test_objectwrap_fixture_setup(
            context_state as *mut crate::bridge::LexeV8ContextState,
        )
    };
    assert!(!counters.is_null());
    unsafe {
        crate::runtime_state::add_cleanup_hook(
            context_state as usize,
            lexe_v8_test_objectwrap_fixture_cleanup_hook,
            counters,
        );
        lexe_v8_test_objectwrap_fixture_release_bridge_roots(
            context_state as *mut crate::bridge::LexeV8ContextState,
            counters,
        );
    }

    for _ in 0..8 {
        unsafe {
            rquickjs::qjs::JS_RunGC(qrt);
        }
        crate::js_ops::dispatch_pending_weak_callbacks_for_ctx(ctx_ptr);
    }

    unsafe { crate::shutdown_context(ctx_ptr).unwrap() };

    let mut cleanup = 0;
    let mut weak = 0;
    let mut dtor = 0;
    unsafe {
        lexe_v8_test_objectwrap_fixture_read_counts(counters, &mut cleanup, &mut weak, &mut dtor);
    }
    assert_eq!(cleanup, 1, "cleanup hook should run once");
    assert_eq!(weak, 1, "weak ObjectWrap callback should run once");
    assert_eq!(dtor, 1, "native destructor should run once");
    assert!(
        crate::bridge::teardown_counts_for_ctx(ctx_ptr).is_zero(),
        "teardown counts should be zero"
    );

    unsafe {
        lexe_v8_test_objectwrap_fixture_destroy(counters);
        crate::shutdown_runtime(qrt).unwrap();
        lexe_v8_set_current_context(std::ptr::null_mut());
        lexe_v8_set_current_isolate(std::ptr::null_mut());
    }
    drop(context);
    drop(runtime);
}

/// better-sqlite3 teardown risk: ObjectWrap weak callbacks need C++ TLS at
/// DisposeGlobal time. Teardown must reinstall provenance, not inherit it.
#[test]
fn objectwrap_is_destroyed_when_teardown_starts_without_cpp_tls() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    extern "C" {
        fn lexe_v8_test_objectwrap_shutdown_counters_new() -> *mut std::ffi::c_void;
        fn lexe_v8_test_objectwrap_shutdown_counters_read(
            counters: *const std::ffi::c_void,
            constructed_out: *mut i32,
            destroyed_out: *mut i32,
        );
        fn lexe_v8_test_objectwrap_shutdown_counters_destroy(counters: *mut std::ffi::c_void);
        fn lexe_v8_test_setup_shutdown_object_wrap(
            ctx: *mut crate::bridge::LexeV8ContextState,
            counters: *mut std::ffi::c_void,
        ) -> i32;
        fn lexe_v8_set_current_context(ctx: *mut crate::bridge::LexeV8ContextState);
        fn lexe_v8_set_current_isolate(isolate: *mut crate::bridge::LexeV8IsolateState);
    }

    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ctx_ptr = context.as_raw().as_ptr();
    crate::bind_bridge(ctx_ptr);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx_ptr) };
    let isolate = crate::ensure_isolate_for_runtime(qrt);
    let context_state = crate::ensure_context_for_ctx(ctx_ptr);
    crate::bridge::set_active_bridge_context(ctx_ptr);
    unsafe {
        lexe_v8_set_current_context(context_state as *mut crate::bridge::LexeV8ContextState);
        lexe_v8_set_current_isolate(isolate as *mut crate::bridge::LexeV8IsolateState);
    }

    let counters = unsafe { lexe_v8_test_objectwrap_shutdown_counters_new() };
    assert!(!counters.is_null());
    let setup_ok = unsafe {
        lexe_v8_test_setup_shutdown_object_wrap(
            context_state as *mut crate::bridge::LexeV8ContextState,
            counters,
        )
    };
    assert_eq!(setup_ok, 1, "ObjectWrap setup must succeed");

    let mut constructed = 0;
    let mut destroyed = 0;
    unsafe {
        lexe_v8_test_objectwrap_shutdown_counters_read(counters, &mut constructed, &mut destroyed);
    }
    assert_eq!(constructed, 1);
    assert_eq!(destroyed, 0);

    // Exact CI risk: teardown must not rely on TLS from a prior callback.
    unsafe {
        lexe_v8_set_current_context(std::ptr::null_mut());
        lexe_v8_set_current_isolate(std::ptr::null_mut());
    }
    crate::bridge::clear_active_bridge_context();

    unsafe {
        crate::run_pre_bridge_teardown_gc(ctx_ptr).unwrap();
    }

    unsafe {
        lexe_v8_test_objectwrap_shutdown_counters_read(counters, &mut constructed, &mut destroyed);
    }
    assert_eq!(
        destroyed, 1,
        "ObjectWrap dtor must run via forced weak invoke"
    );
    assert_eq!(
        crate::bridge::teardown_counts_for_ctx(ctx_ptr).weak_callbacks,
        0
    );

    unsafe {
        crate::shutdown_context(ctx_ptr).unwrap();
        crate::shutdown_runtime(qrt).unwrap();
        lexe_v8_test_objectwrap_shutdown_counters_destroy(counters);
        lexe_v8_set_current_context(std::ptr::null_mut());
        lexe_v8_set_current_isolate(std::ptr::null_mut());
    }
    drop(context);
    drop(runtime);
}

#[test]
fn teardown_activation_uses_the_owner_context_and_isolate() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    extern "C" {
        fn lexe_v8_test_objectwrap_shutdown_counters_new() -> *mut std::ffi::c_void;
        fn lexe_v8_test_objectwrap_shutdown_counters_read(
            counters: *const std::ffi::c_void,
            constructed_out: *mut i32,
            destroyed_out: *mut i32,
        );
        fn lexe_v8_test_objectwrap_shutdown_counters_destroy(counters: *mut std::ffi::c_void);
        fn lexe_v8_test_setup_shutdown_object_wrap(
            ctx: *mut crate::bridge::LexeV8ContextState,
            counters: *mut std::ffi::c_void,
        ) -> i32;
        fn lexe_v8_set_current_context(ctx: *mut crate::bridge::LexeV8ContextState);
        fn lexe_v8_set_current_isolate(isolate: *mut crate::bridge::LexeV8IsolateState);
    }

    let runtime = Runtime::new().expect("runtime");
    let ctx_a = Context::full(&runtime).expect("ctx_a");
    let ctx_b = Context::full(&runtime).expect("ctx_b");
    let ptr_a = ctx_a.as_raw().as_ptr();
    let ptr_b = ctx_b.as_raw().as_ptr();
    // Create each ContextState while its own bridge is active so context roots
    // land in the owner root table (with_bridge_roots uses active_ctx_key).
    crate::bind_bridge(ptr_a);
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ptr_a) };
    let isolate = crate::ensure_isolate_for_runtime(qrt);
    let state_a = crate::ensure_context_for_ctx(ptr_a);

    crate::bind_bridge(ptr_b);
    let state_b = crate::ensure_context_for_ctx(ptr_b);

    // Wire A and create ObjectWrap on A.
    crate::bridge::set_active_bridge_context(ptr_a);
    unsafe {
        lexe_v8_set_current_context(state_a as *mut crate::bridge::LexeV8ContextState);
        lexe_v8_set_current_isolate(isolate as *mut crate::bridge::LexeV8IsolateState);
    }
    let counters_a = unsafe { lexe_v8_test_objectwrap_shutdown_counters_new() };
    assert_eq!(
        unsafe {
            lexe_v8_test_setup_shutdown_object_wrap(
                state_a as *mut crate::bridge::LexeV8ContextState,
                counters_a,
            )
        },
        1
    );

    // Wire B and create ObjectWrap on B.
    crate::bridge::set_active_bridge_context(ptr_b);
    unsafe {
        lexe_v8_set_current_context(state_b as *mut crate::bridge::LexeV8ContextState);
        lexe_v8_set_current_isolate(isolate as *mut crate::bridge::LexeV8IsolateState);
    }
    let counters_b = unsafe { lexe_v8_test_objectwrap_shutdown_counters_new() };
    assert_eq!(
        unsafe {
            lexe_v8_test_setup_shutdown_object_wrap(
                state_b as *mut crate::bridge::LexeV8ContextState,
                counters_b,
            )
        },
        1
    );

    // Leave TLS pointing at B, then teardown A — must destroy only A's wrap.
    let mut destroyed_a = 0;
    let mut destroyed_b = 0;
    let mut constructed = 0;
    unsafe {
        crate::run_pre_bridge_teardown_gc(ptr_a).unwrap();
        lexe_v8_test_objectwrap_shutdown_counters_read(
            counters_a,
            &mut constructed,
            &mut destroyed_a,
        );
        lexe_v8_test_objectwrap_shutdown_counters_read(
            counters_b,
            &mut constructed,
            &mut destroyed_b,
        );
    }
    assert_eq!(
        destroyed_a, 1,
        "owner context A ObjectWrap must be destroyed"
    );
    assert_eq!(
        destroyed_b, 0,
        "sibling context B ObjectWrap must survive A teardown"
    );
    assert_eq!(
        crate::bridge::teardown_counts_for_ctx(ptr_a).weak_callbacks,
        0,
        "A weak callbacks must clear after pre-bridge GC"
    );
    assert_eq!(
        crate::bridge::teardown_counts_for_ctx(ptr_b).weak_callbacks,
        1,
        "B weak callbacks must remain until B teardown"
    );

    unsafe {
        crate::shutdown_context(ptr_a).unwrap();
        crate::run_pre_bridge_teardown_gc(ptr_b).unwrap();
        lexe_v8_test_objectwrap_shutdown_counters_read(
            counters_b,
            &mut constructed,
            &mut destroyed_b,
        );
    }
    assert_eq!(
        destroyed_b, 1,
        "context B ObjectWrap must be destroyed on B teardown"
    );
    assert_eq!(
        crate::bridge::teardown_counts_for_ctx(ptr_b).weak_callbacks,
        0,
        "B weak callbacks must clear after pre-bridge GC"
    );

    unsafe {
        crate::shutdown_context(ptr_b).unwrap();
    }

    // Full bridge teardown zero only after shutdown_context (context roots live until then).
    assert!(
        crate::bridge::teardown_counts_for_ctx(ptr_a).is_zero(),
        "A bridge roots/weak/persistents must be zero after full shutdown"
    );
    assert!(
        crate::bridge::teardown_counts_for_ctx(ptr_b).is_zero(),
        "B bridge roots/weak/persistents must be zero after full shutdown"
    );

    unsafe {
        crate::shutdown_runtime(qrt).unwrap();
        lexe_v8_test_objectwrap_shutdown_counters_destroy(counters_a);
        lexe_v8_test_objectwrap_shutdown_counters_destroy(counters_b);
        lexe_v8_set_current_context(std::ptr::null_mut());
        lexe_v8_set_current_isolate(std::ptr::null_mut());
    }
    drop(ctx_a);
    drop(ctx_b);
    drop(runtime);
}

#[test]
fn objectwrap_wrapped_from_callback_receiver_registers_weak_callback() {
    let _lock = abi_test_lock();
    let mut fixture = WiredTestContext::new();
    extern "C" {
        fn lexe_v8_test_objectwrap_shutdown_counters_new() -> *mut std::ffi::c_void;
        fn lexe_v8_test_objectwrap_shutdown_counters_read(
            counters: *const std::ffi::c_void,
            constructed_out: *mut i32,
            destroyed_out: *mut i32,
        );
        fn lexe_v8_test_objectwrap_shutdown_counters_destroy(counters: *mut std::ffi::c_void);
        fn lexe_v8_test_register_objectwrap_ctor_template(counters: *mut std::ffi::c_void) -> u32;
    }
    let counters = unsafe { lexe_v8_test_objectwrap_shutdown_counters_new() };
    let function_id = unsafe { lexe_v8_test_register_objectwrap_ctor_template(counters) };
    let func_root = fixture.get_function(function_id);

    let mut instance_root = 0u64;
    let status = unsafe {
        crate::value_ops::function_new_instance(
            fixture.context_state,
            func_root,
            0,
            std::ptr::null(),
            &mut instance_root,
        )
    };
    assert!(matches!(status, crate::bridge::LexeV8Status::Ok));
    assert_ne!(instance_root, 0);

    let mut instance_ptr = std::ptr::null_mut();
    let ptr_status = unsafe {
        crate::js_ops::object_ptr_for_root(fixture.context_state, instance_root, &mut instance_ptr)
    };
    assert!(matches!(ptr_status, crate::bridge::LexeV8Status::Ok));
    assert!(!instance_ptr.is_null());
    assert!(
        crate::context_tables::with_context_tables(fixture.ctx_ptr, |t| {
            t.weak_callbacks.contains_key(&(instance_ptr as usize))
        }),
        "weak callback must be keyed to the instance, not a stale materialized layout"
    );

    assert_eq!(
        crate::bridge::teardown_counts_for_ctx(fixture.ctx_ptr).weak_callbacks,
        1,
        "Wrap(info.This()) must register the ObjectWrap weak callback"
    );

    unsafe {
        crate::run_pre_bridge_teardown_gc(fixture.ctx_ptr).unwrap();
    }
    let mut constructed = 0;
    let mut destroyed = 0;
    unsafe {
        lexe_v8_test_objectwrap_shutdown_counters_read(counters, &mut constructed, &mut destroyed);
    }
    assert_eq!(constructed, 1);
    assert_eq!(
        destroyed, 1,
        "receiver ObjectWrap must be destroyed at teardown"
    );

    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }
    assert!(crate::bridge::teardown_counts_for_ctx(fixture.ctx_ptr).is_zero());
    fixture.shutdown_bridge_and_drop_runtime();
    unsafe {
        lexe_v8_test_objectwrap_shutdown_counters_destroy(counters);
    }
}

#[test]
fn teardown_activation_rejects_registered_context_without_isolate() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    extern "C" {
        fn lexe_v8_current_context() -> *mut crate::bridge::LexeV8ContextState;
        fn lexe_v8_current_isolate() -> *mut crate::bridge::LexeV8IsolateState;
        fn lexe_v8_set_current_context(ctx: *mut crate::bridge::LexeV8ContextState);
        fn lexe_v8_set_current_isolate(isolate: *mut crate::bridge::LexeV8IsolateState);
    }

    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let ctx = context.as_raw().as_ptr();
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx) };

    crate::bind_bridge(ctx);
    crate::ensure_context_for_ctx(ctx);

    unsafe {
        lexe_v8_set_current_context(std::ptr::null_mut());
        lexe_v8_set_current_isolate(std::ptr::null_mut());
    }

    let error = unsafe { crate::module_loader::activate_v8_context_for_teardown(ctx) }.unwrap_err();

    assert!(
        error.contains("isolate is not registered"),
        "unexpected error: {error}"
    );
    // Failed activation must not mutate C++ TLS (all fallible lookups first).
    assert!(unsafe { lexe_v8_current_context().is_null() });
    assert!(unsafe { lexe_v8_current_isolate().is_null() });

    crate::ensure_isolate_for_runtime(qrt);
    unsafe {
        crate::shutdown_context(ctx).unwrap();
        crate::shutdown_runtime(qrt).unwrap();
    }

    drop(context);
    drop(runtime);
}

#[test]
fn try_activate_rejects_cpp_owned_resources_without_context_state() {
    let _lock = abi_test_lock();
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let ctx = context.as_raw().as_ptr();
    let qrt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx) };

    crate::bind_bridge(ctx);
    // No ensure_context_for_ctx — simulate weak slot registered via the
    // with_state_for_object_root fallback path.
    crate::context_tables::with_context_tables(ctx, |tables| {
        tables.weak_callbacks.insert(
            0xDEAD_BEEFusize,
            crate::context_tables::WeakSlot {
                callback: 1,
                parameter: 0,
                phase: crate::context_tables::WeakPhase::Registered,
            },
        );
    });

    let error =
        unsafe { crate::module_loader::try_activate_v8_context_for_teardown(ctx) }.unwrap_err();
    assert!(
        error.contains("C++-owned resources without a registered context"),
        "unexpected error: {error}"
    );

    // Clean the synthetic slot before shutdown so bridge-only teardown succeeds.
    crate::context_tables::with_context_tables(ctx, |tables| {
        tables.weak_callbacks.clear();
    });

    unsafe {
        crate::shutdown_context(ctx).unwrap();
        crate::shutdown_runtime(qrt).unwrap();
    }
    drop(context);
    drop(runtime);
}

struct WiredTestContext {
    runtime: rquickjs::Runtime,
    context: rquickjs::Context,
    ctx_ptr: *mut rquickjs::qjs::JSContext,
    rt_ptr: *mut rquickjs::qjs::JSRuntime,
    context_state: *mut crate::bridge::LexeV8ContextState,
    pending_missing_key: bool,
}

impl WiredTestContext {
    fn new() -> Self {
        let runtime = rquickjs::Runtime::new().expect("runtime");
        let context = rquickjs::Context::full(&runtime).expect("context");
        let ctx_ptr = context.as_raw().as_ptr();
        crate::bind_bridge(ctx_ptr);
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx_ptr) };
        let _isolate = crate::ensure_isolate_for_runtime(rt_ptr);
        let context_state =
            crate::ensure_context_for_ctx(ctx_ptr) as *mut crate::bridge::LexeV8ContextState;
        crate::bridge::set_active_bridge_context(ctx_ptr);
        extern "C" {
            fn lexe_v8_set_current_context(ctx: *mut crate::bridge::LexeV8ContextState);
            fn lexe_v8_set_current_isolate(isolate: *mut crate::bridge::LexeV8IsolateState);
        }
        unsafe {
            lexe_v8_set_current_context(context_state);
            lexe_v8_set_current_isolate(_isolate as *mut crate::bridge::LexeV8IsolateState);
        }
        Self {
            runtime,
            context,
            ctx_ptr,
            rt_ptr,
            context_state,
            pending_missing_key: false,
        }
    }

    fn register_template_property_with_missing_key(&mut self) {
        self.pending_missing_key = true;
    }

    fn register_function_template(&mut self) -> u32 {
        extern "C" {
            fn lexe_v8_test_register_function_template() -> u32;
            fn lexe_v8_test_register_template_property_with_missing_key(function_id: u32);
        }
        let function_id = unsafe { lexe_v8_test_register_function_template() };
        if self.pending_missing_key {
            unsafe { lexe_v8_test_register_template_property_with_missing_key(function_id) };
            self.pending_missing_key = false;
        }
        function_id
    }

    fn get_function(&mut self, function_id: u32) -> u64 {
        extern "C" {
            fn lexe_v8_test_function_template_get_function(
                ctx_state: *mut crate::bridge::LexeV8ContextState,
                function_id: u32,
                out_root: *mut u64,
            ) -> crate::bridge::LexeV8Status;
        }
        let mut root = 0u64;
        let status = unsafe {
            lexe_v8_test_function_template_get_function(self.context_state, function_id, &mut root)
        };
        assert!(matches!(status, crate::bridge::LexeV8Status::Ok));
        root
    }

    fn function_root(&self, function_id: u32) -> Option<u64> {
        crate::bridge::with_state_ref_for_ctx(self.ctx_ptr, |state| {
            crate::bridge::function_root_for_id(state, function_id)
        })
    }

    fn template_prototype(&self, function_id: u32) -> rquickjs::qjs::JSValue {
        extern "C" {
            fn lexe_v8_function_template_id(function_id: u32) -> u32;
            fn lexe_v8_function_template_prototype_root(template_id: u32) -> u64;
        }
        let template_id = unsafe { lexe_v8_function_template_id(function_id) };
        assert_ne!(
            template_id, 0,
            "function_id must map to a function template"
        );
        let proto_root = unsafe { lexe_v8_function_template_prototype_root(template_id) };
        assert_ne!(proto_root, 0);
        crate::bridge::with_state_ref_for_ctx(self.ctx_ptr, |state| {
            state.roots.get(proto_root).expect("prototype root")
        })
    }

    fn bridge_strong_root_count(&self) -> usize {
        crate::bridge::teardown_counts_for_ctx(self.ctx_ptr).strong_roots
    }

    /// Runs bridge `shutdown_runtime`, clears current context/isolate TLS,
    /// then drops the rquickjs `Context`/`Runtime` (real `JS_FreeRuntime`).
    ///
    /// Must not `mem::forget` handles: ASan CI runs with `detect_leaks=1`.
    fn shutdown_bridge_and_drop_runtime(self) {
        extern "C" {
            fn lexe_v8_set_current_context(ctx: *mut crate::bridge::LexeV8ContextState);
            fn lexe_v8_set_current_isolate(isolate: *mut crate::bridge::LexeV8IsolateState);
        }
        unsafe {
            crate::shutdown_runtime(self.rt_ptr).unwrap();
            lexe_v8_set_current_context(std::ptr::null_mut());
            lexe_v8_set_current_isolate(std::ptr::null_mut());
        }
        drop(self.context);
        drop(self.runtime);
    }
}

#[test]
fn function_template_constructor_has_one_bridge_owner() {
    let _lock = abi_test_lock();
    let mut fixture = WiredTestContext::new();

    let function_id = fixture.register_function_template();
    let root = fixture.get_function(function_id);

    assert_ne!(root, 0);
    assert_eq!(fixture.function_root(function_id), Some(root));

    let proto = fixture.template_prototype(function_id);
    let roots_before = fixture.bridge_strong_root_count();
    unsafe {
        assert!(crate::bridge::prototype_has_own_constructor(fixture.ctx_ptr, proto).unwrap());
        assert_eq!(fixture.bridge_strong_root_count(), roots_before);
    }

    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }

    assert!(crate::bridge::teardown_counts_for_ctx(fixture.ctx_ptr).is_zero());

    fixture.shutdown_bridge_and_drop_runtime();
}

#[test]
fn prototype_install_does_not_leak_when_key_is_missing() {
    let _lock = abi_test_lock();
    let mut fixture = WiredTestContext::new();

    fixture.register_template_property_with_missing_key();
    let function_id = fixture.register_function_template();
    let _ = fixture.get_function(function_id);

    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }

    assert!(crate::bridge::teardown_counts_for_ctx(fixture.ctx_ptr).is_zero());

    fixture.shutdown_bridge_and_drop_runtime();
}

#[test]
fn shutdown_context_releases_all_registered_root_kinds() {
    let _lock = abi_test_lock();
    let mut fixture = WiredTestContext::new();

    let function_id = fixture.register_function_template();
    let _ = fixture.get_function(function_id);

    crate::bridge::with_bridge_roots(|ctx, roots| {
        let obj = unsafe { rquickjs::qjs::JS_NewObject(ctx) };
        let root = roots.insert_borrowed(ctx, obj);
        unsafe { rquickjs::qjs::JS_FreeValue(ctx, obj) };
        roots.drop_root(ctx, root);
    });

    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }

    assert!(crate::bridge::teardown_counts_for_ctx(fixture.ctx_ptr).is_zero());

    fixture.shutdown_bridge_and_drop_runtime();
}

#[test]
fn module_init_drops_temporary_roots_on_success() {
    let _lock = abi_test_lock();
    use std::ffi::c_void;

    unsafe extern "C" fn noop_register(
        _exports: *mut c_void,
        _module: *mut c_void,
        _priv_data: *mut c_void,
    ) {
    }

    let fixture = WiredTestContext::new();
    let exports = unsafe { rquickjs::qjs::JS_NewObject(fixture.ctx_ptr) };
    let mut module = crate::module_loader::NodeModule {
        nm_version: crate::NODE_MODULE_VERSION,
        nm_flags: 0,
        nm_dso_handle: std::ptr::null_mut(),
        nm_filename: std::ptr::null(),
        nm_register_func: Some(noop_register),
        nm_context_register_func: None,
        nm_modname: std::ptr::null(),
        nm_priv: std::ptr::null_mut(),
        nm_link: std::ptr::null_mut(),
    };

    let result =
        unsafe { crate::run_v8_module_init(fixture.ctx_ptr, &mut module, exports) }.unwrap();
    unsafe {
        // `result` is a fresh ref from module.exports; caller still owns `exports`.
        rquickjs::qjs::JS_FreeValue(fixture.ctx_ptr, result);
        rquickjs::qjs::JS_FreeValue(fixture.ctx_ptr, exports);
    }

    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }
    assert!(crate::bridge::teardown_counts_for_ctx(fixture.ctx_ptr).is_zero());
    fixture.shutdown_bridge_and_drop_runtime();
}

#[test]
fn module_init_drops_temporary_roots_on_failure() {
    let _lock = abi_test_lock();

    let fixture = WiredTestContext::new();
    let exports = unsafe { rquickjs::qjs::JS_NewObject(fixture.ctx_ptr) };
    let mut module = crate::module_loader::NodeModule {
        nm_version: crate::NODE_MODULE_VERSION,
        nm_flags: 0,
        nm_dso_handle: std::ptr::null_mut(),
        nm_filename: std::ptr::null(),
        nm_register_func: None,
        nm_context_register_func: None,
        nm_modname: std::ptr::null(),
        nm_priv: std::ptr::null_mut(),
        nm_link: std::ptr::null_mut(),
    };

    let result = unsafe { crate::run_v8_module_init(fixture.ctx_ptr, &mut module, exports) };
    assert!(result.is_err(), "module init should fail");
    unsafe {
        rquickjs::qjs::JS_FreeValue(fixture.ctx_ptr, exports);
    }
    let err = result.err().unwrap();
    assert!(err.contains("failed"), "unexpected: {err}");

    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }
    assert!(crate::bridge::teardown_counts_for_ctx(fixture.ctx_ptr).is_zero());
    fixture.shutdown_bridge_and_drop_runtime();
}

#[test]
fn runtime_teardown_does_not_leave_process_global_owners() {
    let _lock = abi_test_lock();

    let mut fixture_a = WiredTestContext::new();
    let function_id = fixture_a.register_function_template();
    let _ = fixture_a.get_function(function_id);
    unsafe {
        crate::shutdown_context(fixture_a.ctx_ptr).unwrap();
    }
    assert!(crate::bridge::teardown_counts_for_ctx(fixture_a.ctx_ptr).is_zero());
    fixture_a.shutdown_bridge_and_drop_runtime();

    let mut fixture_b = WiredTestContext::new();
    let function_id = fixture_b.register_function_template();
    let root = fixture_b.get_function(function_id);
    assert_ne!(root, 0);
    let proto = fixture_b.template_prototype(function_id);
    unsafe {
        assert!(crate::bridge::prototype_has_own_constructor(fixture_b.ctx_ptr, proto).unwrap());
    }
    unsafe {
        crate::shutdown_context(fixture_b.ctx_ptr).unwrap();
    }
    assert!(crate::bridge::teardown_counts_for_ctx(fixture_b.ctx_ptr).is_zero());
    fixture_b.shutdown_bridge_and_drop_runtime();
}

#[test]
fn persistent_binds_to_requested_local_not_last_materialized() {
    let _lock = abi_test_lock();

    let fixture = WiredTestContext::new();
    extern "C" {
        fn lexe_v8_test_persistent_binds_to_requested_local(
            ctx_state: *mut crate::bridge::LexeV8ContextState,
        ) -> i32;
    }
    let ok = unsafe { lexe_v8_test_persistent_binds_to_requested_local(fixture.context_state) };
    assert_eq!(
        ok, 1,
        "Persistent::Reset must globalize the requested Local, not g_last_materialized_layout"
    );
    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }
    fixture.shutdown_bridge_and_drop_runtime();
}

#[test]
fn persistent_reset_scrubs_layout_index_maps() {
    let _lock = abi_test_lock();

    let fixture = WiredTestContext::new();
    extern "C" {
        fn lexe_v8_test_objectwrap_strong_reset_scrubs_layout_maps(
            ctx_state: *mut crate::bridge::LexeV8ContextState,
        ) -> i32;
    }
    let ok =
        unsafe { lexe_v8_test_objectwrap_strong_reset_scrubs_layout_maps(fixture.context_state) };
    assert_eq!(
        ok, 1,
        "Persistent::Reset should scrub layout_to_root/layout_to_function_id"
    );
    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }
    fixture.shutdown_bridge_and_drop_runtime();
}

#[test]
fn resolve_root_repr_smi_to_number_and_known_layouts() {
    let _lock = abi_test_lock();

    let fixture = WiredTestContext::new();
    extern "C" {
        fn lexe_v8_test_resolve_root_repr_smi_and_tagged(
            ctx_state: *mut crate::bridge::LexeV8ContextState,
        ) -> i32;
    }
    let ok = unsafe { lexe_v8_test_resolve_root_repr_smi_and_tagged(fixture.context_state) };
    assert_eq!(
        ok, 1,
        "general resolver rejects Smi/unknown addrs; return-value path materializes Smi"
    );
    unsafe {
        crate::shutdown_context(fixture.ctx_ptr).unwrap();
    }
    fixture.shutdown_bridge_and_drop_runtime();
}
