// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, VecDeque};
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::ThreadId;

use parking_lot::{Condvar, Mutex, MutexGuard};
use rquickjs::qjs::{self, JSValue};

use crate::driver::{ensure_driver, DriverJob, DriverState};
use crate::env::{DisposeState, Env};
use crate::types::{
    napi_async_complete_callback, napi_async_execute_callback, napi_async_work, napi_deferred,
    napi_env, napi_ref, napi_status, napi_threadsafe_function, napi_threadsafe_function_call_js,
    napi_threadsafe_function_call_mode, napi_threadsafe_function_release_mode,
};

const TSFN_OPEN: u8 = 0;
const TSFN_CLOSING: u8 = 1;
const TSFN_CLOSED: u8 = 2;

const AW_CREATED: u8 = 0;
const AW_QUEUED: u8 = 1;
const AW_RUNNING: u8 = 2;
const AW_CANCELLED: u8 = 3;

pub struct AsyncWorkState {
    pub env: napi_env,
    pub execute: napi_async_execute_callback,
    pub complete: napi_async_complete_callback,
    pub data: *mut c_void,
    pub status: AtomicU8,
    pub completion_posted: AtomicBool,
}

unsafe impl Send for AsyncWorkState {}
unsafe impl Sync for AsyncWorkState {}

static ASYNC_WORK_REGISTRY: LazyLock<Mutex<HashMap<usize, Arc<AsyncWorkState>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn async_work_key(work: napi_async_work) -> usize {
    work as usize
}

fn lookup_async_work(work: napi_async_work) -> Option<Arc<AsyncWorkState>> {
    if work.is_null() {
        return None;
    }
    ASYNC_WORK_REGISTRY
        .lock()
        .get(&async_work_key(work))
        .cloned()
}

fn register_async_work(state: Arc<AsyncWorkState>) -> napi_async_work {
    let key = Arc::as_ptr(&state) as usize;
    ASYNC_WORK_REGISTRY.lock().insert(key, state);
    key as napi_async_work
}

fn unregister_async_work(work: napi_async_work) -> Option<Arc<AsyncWorkState>> {
    ASYNC_WORK_REGISTRY.lock().remove(&async_work_key(work))
}

fn post_async_complete(
    env: &mut Env,
    driver: &Arc<DriverState>,
    work: Arc<AsyncWorkState>,
    status: napi_status,
) {
    if !driver.post(env, DriverJob::AsyncComplete { work, status }) {
        driver.release_async_keepalive();
    }
}

fn post_async_complete_job(
    driver: &Arc<DriverState>,
    work: Arc<AsyncWorkState>,
    status: napi_status,
) -> bool {
    if driver.post_job(DriverJob::AsyncComplete { work, status }) {
        return true;
    }
    driver.release_async_keepalive();
    false
}

fn dispatch_async_completion(env: &mut Env, work: &Arc<AsyncWorkState>, status: napi_status) {
    if work.completion_posted.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(complete) = work.complete {
        unsafe {
            complete(work.env, status, work.data);
        }
    }
    if let Some(driver) = env.driver.as_ref() {
        driver.release_async_keepalive();
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_async_work(
    env: napi_env,
    _async_resource: crate::types::napi_value,
    _async_resource_name: crate::types::napi_value,
    execute: napi_async_execute_callback,
    complete: napi_async_complete_callback,
    data: *mut c_void,
    result: *mut napi_async_work,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let env_ref = unsafe { Env::from_napi_env(env) };
    if env_ref.dispose_state != DisposeState::Active {
        return napi_status::napi_closing;
    }
    let state = Arc::new(AsyncWorkState {
        env,
        execute,
        complete,
        data,
        status: AtomicU8::new(AW_CREATED),
        completion_posted: AtomicBool::new(false),
    });
    let work = register_async_work(state);
    unsafe {
        *result = work;
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_async_work(
    env: napi_env,
    work: napi_async_work,
) -> napi_status {
    if env.is_null() || work.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(state) = lookup_async_work(work) else {
        return napi_status::napi_invalid_arg;
    };
    if state.env != env {
        return napi_status::napi_invalid_arg;
    }
    if unregister_async_work(work).is_none() {
        return napi_status::napi_invalid_arg;
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_queue_async_work(
    env: napi_env,
    work: napi_async_work,
) -> napi_status {
    if env.is_null() || work.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(work_state) = lookup_async_work(work) else {
        return napi_status::napi_invalid_arg;
    };
    if work_state.env != env {
        return napi_status::napi_invalid_arg;
    }
    let env_ref = unsafe { Env::from_napi_env(env) };
    if env_ref.dispose_state != DisposeState::Active {
        return napi_status::napi_closing;
    }
    if work_state
        .status
        .compare_exchange(AW_CREATED, AW_QUEUED, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return napi_status::napi_invalid_arg;
    }
    let driver = ensure_driver(env_ref);
    let execute = work_state.execute;
    let data_addr = work_state.data as usize;
    let work_env_addr = work_state.env as usize;
    let driver_clone = driver.clone();
    let work_arc = Arc::clone(&work_state);

    driver.acquire_async_keepalive();
    driver.ensure_loop(env_ref);

    if let Some(exec) = execute {
        driver.runtime.spawn_blocking(move || {
            match work_arc.status.compare_exchange(
                AW_QUEUED,
                AW_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    exec(work_env_addr as napi_env, data_addr as *mut c_void);
                    let _ = post_async_complete_job(&driver_clone, work_arc, napi_status::napi_ok);
                },
                Err(AW_CANCELLED) => {
                    let _ = post_async_complete_job(
                        &driver_clone,
                        work_arc,
                        napi_status::napi_cancelled,
                    );
                },
                Err(_) => {
                    driver_clone.release_async_keepalive();
                },
            }
        });
    } else {
        match work_arc.status.compare_exchange(
            AW_QUEUED,
            AW_RUNNING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                post_async_complete(env_ref, &driver, work_arc, napi_status::napi_ok);
            },
            Err(AW_CANCELLED) => {
                post_async_complete(env_ref, &driver, work_arc, napi_status::napi_cancelled);
            },
            Err(_) => {
                driver.release_async_keepalive();
            },
        }
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_cancel_async_work(
    env: napi_env,
    work: napi_async_work,
) -> napi_status {
    if env.is_null() || work.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(state) = lookup_async_work(work) else {
        return napi_status::napi_invalid_arg;
    };
    if state.env != env {
        return napi_status::napi_invalid_arg;
    }
    loop {
        match state.status.load(Ordering::Acquire) {
            AW_CREATED => return napi_status::napi_generic_failure,
            AW_QUEUED => {
                if state
                    .status
                    .compare_exchange(AW_QUEUED, AW_CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return napi_status::napi_ok;
                }
            },
            AW_RUNNING => return napi_status::napi_generic_failure,
            AW_CANCELLED => return napi_status::napi_ok,
            _ => return napi_status::napi_invalid_arg,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum QueuePushResult {
    Ok,
    Full,
    Closing,
}

#[derive(Debug, PartialEq, Eq)]
enum QueueAdmissionResult {
    Ok,
    Full,
    Closing,
    PostFailed,
}

impl From<QueuePushResult> for QueueAdmissionResult {
    fn from(value: QueuePushResult) -> Self {
        match value {
            QueuePushResult::Ok => QueueAdmissionResult::Ok,
            QueuePushResult::Full => QueueAdmissionResult::Full,
            QueuePushResult::Closing => QueueAdmissionResult::Closing,
        }
    }
}

/// TSFN payload queue backed by `parking_lot` for cross-platform queue semantics.
struct TsfnQueueState {
    items: VecDeque<usize>,
    closed: bool,
}

struct TsfnQueue {
    state: Mutex<TsfnQueueState>,
    changed: Condvar,
    /// `0` means unlimited queue size.
    max_size: usize,
}

impl TsfnQueue {
    fn new(max_queue_size: usize) -> Self {
        let initial_cap = if max_queue_size == 0 {
            64
        } else {
            max_queue_size
        };
        Self {
            state: Mutex::new(TsfnQueueState {
                items: VecDeque::with_capacity(initial_cap),
                closed: false,
            }),
            changed: Condvar::new(),
            max_size: max_queue_size,
        }
    }

    fn set_closing(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        self.changed.notify_all();
    }

    fn pop_front(&self) -> Option<usize> {
        let mut state = self.state.lock();
        let item = state.items.pop_front();
        if item.is_some() {
            self.changed.notify_all();
        }
        item
    }

    fn drain_all(&self) -> Vec<usize> {
        let mut state = self.state.lock();
        let drained: Vec<usize> = state.items.drain(..).collect();
        if !drained.is_empty() {
            self.changed.notify_all();
        }
        drained
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.state.lock().items.len()
    }

    fn try_push_locked(
        changed: &Condvar,
        state: &mut MutexGuard<'_, TsfnQueueState>,
        max_size: usize,
        item: usize,
    ) -> QueuePushResult {
        if state.closed {
            return QueuePushResult::Closing;
        }
        if max_size > 0 && state.items.len() >= max_size {
            return QueuePushResult::Full;
        }
        state.items.push_back(item);
        changed.notify_all();
        QueuePushResult::Ok
    }

    fn push_blocking_locked(
        changed: &Condvar,
        state: &mut MutexGuard<'_, TsfnQueueState>,
        max_size: usize,
        item: usize,
    ) -> QueuePushResult {
        loop {
            if state.closed {
                return QueuePushResult::Closing;
            }
            if max_size == 0 || state.items.len() < max_size {
                state.items.push_back(item);
                changed.notify_all();
                return QueuePushResult::Ok;
            }
            changed.wait(state);
        }
    }

    fn pop_back_locked(
        changed: &Condvar,
        state: &mut MutexGuard<'_, TsfnQueueState>,
    ) -> Option<usize> {
        let item = state.items.pop_back();
        if item.is_some() {
            changed.notify_all();
        }
        item
    }

    fn admit_and_post(
        &self,
        driver: &Arc<DriverState>,
        tsfn: Arc<ThreadsafeFunction>,
        item: usize,
        blocking: bool,
    ) -> QueueAdmissionResult {
        let mut state = self.state.lock();
        let push_result = if blocking {
            Self::push_blocking_locked(&self.changed, &mut state, self.max_size, item)
        } else {
            Self::try_push_locked(&self.changed, &mut state, self.max_size, item)
        };
        if push_result != QueuePushResult::Ok {
            return push_result.into();
        }
        if !driver.post_job(DriverJob::Tsfn { tsfn, done: None }) {
            Self::pop_back_locked(&self.changed, &mut state);
            return QueueAdmissionResult::PostFailed;
        }
        QueueAdmissionResult::Ok
    }
}

pub struct ThreadsafeFunction {
    pub handle: usize,
    pub env: napi_env,
    pub js_thread_id: ThreadId,
    pub driver: Arc<DriverState>,
    pub call_js: napi_threadsafe_function_call_js,
    pub context: *mut c_void,
    pub func_ref: napi_ref,
    queue: TsfnQueue,
    pub refs: AtomicUsize,
    pub idle_refs: AtomicUsize,
    pub state: AtomicU8,
    pub thread_finalize_cb: crate::types::napi_finalize,
    pub thread_finalize_data: *mut c_void,
    pub finalize_called: AtomicBool,
    pub finish_started: AtomicBool,
    /// When true, queued payloads must be torn down (NULL env/callback), never delivered to JS.
    pub aborted: AtomicBool,
}

#[cfg(test)]
impl ThreadsafeFunction {
    pub(crate) fn test_queue_len(&self) -> usize {
        self.queue.len()
    }

    pub(crate) fn queue_set_closing_for_test(&self) {
        self.queue.set_closing();
    }
}

unsafe impl Send for ThreadsafeFunction {}
unsafe impl Sync for ThreadsafeFunction {}

static TSFN_REGISTRY: LazyLock<Mutex<HashMap<usize, Arc<ThreadsafeFunction>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static TSFN_NEXT_HANDLE: AtomicUsize = AtomicUsize::new(1);

pub(crate) fn has_pending_tsfn() -> bool {
    !TSFN_REGISTRY.lock().is_empty()
}

/// Whether any TSFN is still referenced, i.e. keeps the event loop alive.
///
/// An unref'd TSFN must not hold the process open: Node lets it exit while the
/// callback is still pending.
pub(crate) fn has_refed_tsfn() -> bool {
    TSFN_REGISTRY.lock().values().any(|tsfn| {
        tsfn.idle_refs.load(Ordering::SeqCst) != 0 && tsfn.refs.load(Ordering::SeqCst) != 0
    })
}

/// Whether any async work item is still queued or running.
pub(crate) fn has_pending_async_work() -> bool {
    !ASYNC_WORK_REGISTRY.lock().is_empty()
}

fn tsfn_snapshot_all() -> Vec<Arc<ThreadsafeFunction>> {
    TSFN_REGISTRY.lock().values().cloned().collect()
}

fn lookup_tsfn(handle: napi_threadsafe_function) -> Option<Arc<ThreadsafeFunction>> {
    if handle.is_null() {
        return None;
    }
    TSFN_REGISTRY.lock().get(&(handle as usize)).cloned()
}

#[cfg(test)]
pub(crate) fn lookup_tsfn_for_test(
    handle: napi_threadsafe_function,
) -> Option<Arc<ThreadsafeFunction>> {
    lookup_tsfn(handle)
}

fn register_tsfn(tsfn: Arc<ThreadsafeFunction>) -> napi_threadsafe_function {
    let handle = tsfn.handle;
    TSFN_REGISTRY.lock().insert(handle, tsfn);
    handle as napi_threadsafe_function
}

fn unregister_tsfn(handle: napi_threadsafe_function) {
    TSFN_REGISTRY.lock().remove(&(handle as usize));
}

pub(crate) fn drain_threadsafe_functions(env: napi_env) {
    let list = tsfn_snapshot_all();
    for tsfn in &list {
        if tsfn.env != env {
            continue;
        }
        let pending: Vec<*mut c_void> = tsfn
            .queue
            .drain_all()
            .into_iter()
            .map(|v| v as *mut c_void)
            .collect();
        for data in pending {
            consume_tsfn_payload(tsfn, data);
        }
    }
}

pub(crate) fn close_all_tsfn_for_env(env: napi_env) {
    let targets: Vec<Arc<ThreadsafeFunction>> = TSFN_REGISTRY
        .lock()
        .values()
        .filter(|tsfn| tsfn.env == env)
        .cloned()
        .collect();
    for tsfn in targets {
        force_close_tsfn(&tsfn);
        unregister_tsfn(tsfn.handle as napi_threadsafe_function);
    }
}

/// Emergency path only — not used by normal prepare_shutdown / shutdown_all.
#[allow(dead_code)]
pub(crate) fn shutdown_all_tsfn() {
    let list = tsfn_snapshot_all();
    for tsfn in list {
        force_close_tsfn(&tsfn);
        unregister_tsfn(tsfn.handle as napi_threadsafe_function);
    }
    TSFN_REGISTRY.lock().clear();
}

/// Count of TSFNs still registered (postcondition for `shutdown_all`).
pub(crate) fn registered_tsfn_count() -> usize {
    TSFN_REGISTRY.lock().len()
}

/// Count of async works still registered.
#[cfg(test)]
pub(crate) fn registered_async_work_count() -> usize {
    ASYNC_WORK_REGISTRY.lock().len()
}

#[cfg(test)]
pub(crate) fn async_work_status_for_test(work: napi_async_work) -> Option<u8> {
    lookup_async_work(work).map(|s| s.status.load(Ordering::Acquire))
}

#[cfg(test)]
pub(crate) const AW_CREATED_FOR_TEST: u8 = AW_CREATED;

fn tsfn_is_open(tsfn: &ThreadsafeFunction) -> bool {
    tsfn.state.load(Ordering::Acquire) == TSFN_OPEN
}

fn tsfn_begin_closing(tsfn: &ThreadsafeFunction) -> bool {
    tsfn.state
        .compare_exchange(TSFN_OPEN, TSFN_CLOSING, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn tsfn_mark_closed(tsfn: &ThreadsafeFunction) {
    tsfn.state.store(TSFN_CLOSED, Ordering::Release);
}

fn force_close_tsfn(tsfn: &ThreadsafeFunction) {
    if tsfn.state.load(Ordering::Acquire) == TSFN_CLOSED {
        return;
    }
    let _ = tsfn_begin_closing(tsfn);
    tsfn.aborted.store(true, Ordering::Release);
    tsfn.queue.set_closing();
    finish_tsfn_release(tsfn);
    tsfn_mark_closed(tsfn);
}

pub(crate) unsafe fn process_driver_job(env_ptr: *mut Env, job: DriverJob) {
    let env = &mut *env_ptr;
    match job {
        DriverJob::AsyncComplete { work, status } => {
            dispatch_async_completion(env, &work, status);
        },
        DriverJob::Tsfn { tsfn, done } => {
            let data = tsfn.queue.pop_front();
            if let Some(data) = data {
                consume_tsfn_payload(&tsfn, data as *mut c_void);
            }
            if let Some(done) = done {
                let _ = done.send(());
            }
        },
        DriverJob::TsfnAbort { tsfn } => {
            abort_pending_payloads(&tsfn);
        },
        DriverJob::TsfnRelease { tsfn } => {
            finish_tsfn_release(&tsfn);
            unregister_tsfn(tsfn.handle as napi_threadsafe_function);
        },
        DriverJob::Wake => {},
    }
}

fn invoke_thread_finalize(tsfn: &ThreadsafeFunction) {
    if tsfn
        .finalize_called
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    if let Some(cb) = tsfn.thread_finalize_cb {
        unsafe {
            cb(tsfn.env, tsfn.thread_finalize_data, tsfn.context);
        }
    }
}

fn finish_tsfn_release(tsfn: &ThreadsafeFunction) {
    if tsfn
        .finish_started
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    tsfn.queue.set_closing();
    for data in tsfn.queue.drain_all() {
        consume_tsfn_payload(tsfn, data as *mut c_void);
    }
    if !tsfn.func_ref.is_null() {
        unsafe {
            crate::refs::napi_delete_reference(tsfn.env, tsfn.func_ref);
        }
    }
    invoke_thread_finalize(tsfn);
    if tsfn
        .idle_refs
        .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        tsfn.driver.idle_refs.fetch_sub(1, Ordering::SeqCst);
    }
    tsfn.driver.wake_if_quiescent();
    tsfn_mark_closed(tsfn);
}

fn abort_pending_payloads(tsfn: &ThreadsafeFunction) {
    for data in tsfn.queue.drain_all() {
        consume_tsfn_payload(tsfn, data as *mut c_void);
    }
}

fn consume_tsfn_payload(tsfn: &ThreadsafeFunction, data: *mut c_void) {
    if tsfn.aborted.load(Ordering::Acquire) {
        invoke_tsfn_teardown(tsfn, data);
    } else {
        invoke_tsfn_call(tsfn, data);
    }
}

fn invoke_tsfn_teardown(tsfn: &ThreadsafeFunction, data: *mut c_void) {
    if let Some(call_js) = tsfn.call_js {
        unsafe {
            call_js(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                tsfn.context,
                data,
            );
        }
    }
}

fn invoke_tsfn_call(tsfn: &ThreadsafeFunction, data: *mut c_void) {
    // Node allows `func == null` with a non-null `call_js_cb` (napi-rs JsDeferred).
    // Resolve the JS callback only when a function reference was registered.
    let js_callback = if tsfn.func_ref.is_null() {
        std::ptr::null_mut()
    } else {
        let mut value = std::ptr::null_mut();
        if unsafe { crate::refs::napi_get_reference_value(tsfn.env, tsfn.func_ref, &mut value) }
            != napi_status::napi_ok
        {
            // Reference gone — still must release the payload exactly once.
            invoke_tsfn_teardown(tsfn, data);
            return;
        }
        value
    };

    if let Some(call_js) = tsfn.call_js {
        unsafe {
            call_js(tsfn.env, js_callback, tsfn.context, data);
        }
        return;
    }

    // Default path: call the registered JS function with no args.
    if js_callback.is_null() {
        return;
    }
    let env_ref = unsafe { Env::from_napi_env(tsfn.env) };
    let ctx = env_ref.ctx_ptr();
    let func_js = match unsafe { crate::value::napi_to_value_dup(env_ref, js_callback) } {
        Some(v) => v,
        None => {
            invoke_tsfn_teardown(tsfn, data);
            return;
        },
    };
    unsafe {
        let _ = qjs::JS_Call(ctx, func_js, qjs::JS_UNDEFINED, 0, std::ptr::null_mut());
        qjs::JS_FreeValue(ctx, func_js);
    }
}

fn schedule_last_release(tsfn: Arc<ThreadsafeFunction>, on_js_thread: bool) {
    if on_js_thread {
        finish_tsfn_release(&tsfn);
        unregister_tsfn(tsfn.handle as napi_threadsafe_function);
    } else {
        let _ = tsfn.driver.post_job(DriverJob::TsfnRelease {
            tsfn: Arc::clone(&tsfn),
        });
    }
}

fn schedule_tsfn_abort_cleanup(tsfn: &Arc<ThreadsafeFunction>, on_js_thread: bool) {
    if on_js_thread {
        abort_pending_payloads(tsfn);
    } else {
        let _ = tsfn.driver.post_job(DriverJob::TsfnAbort {
            tsfn: Arc::clone(tsfn),
        });
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_threadsafe_function(
    env: napi_env,
    func: crate::types::napi_value,
    async_resource: crate::types::napi_value,
    async_resource_name: crate::types::napi_value,
    max_queue_size: usize,
    initial_thread_count: usize,
    thread_finalize_data: *mut c_void,
    thread_finalize_cb: crate::types::napi_finalize,
    context: *mut c_void,
    call_js_cb: napi_threadsafe_function_call_js,
    result: *mut napi_threadsafe_function,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    if initial_thread_count == 0 {
        return napi_status::napi_invalid_arg;
    }
    if func.is_null() && call_js_cb.is_none() {
        return napi_status::napi_invalid_arg;
    }
    let env_ref = unsafe { Env::from_napi_env(env) };
    if env_ref.dispose_state != DisposeState::Active {
        return napi_status::napi_closing;
    }
    let driver = ensure_driver(env_ref);
    let mut func_ref = std::ptr::null_mut();
    if !func.is_null()
        && unsafe { crate::refs::napi_create_reference(env, func, 1, &mut func_ref) }
            != napi_status::napi_ok
    {
        return napi_status::napi_generic_failure;
    }
    let _ = (async_resource, async_resource_name);
    let handle = TSFN_NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    let tsfn = Arc::new(ThreadsafeFunction {
        handle,
        env,
        js_thread_id: env_ref.js_thread_id,
        driver: driver.clone(),
        call_js: call_js_cb,
        context,
        func_ref,
        queue: TsfnQueue::new(max_queue_size),
        refs: AtomicUsize::new(initial_thread_count),
        idle_refs: AtomicUsize::new(1),
        state: AtomicU8::new(TSFN_OPEN),
        thread_finalize_cb,
        thread_finalize_data,
        finalize_called: AtomicBool::new(false),
        finish_started: AtomicBool::new(false),
        aborted: AtomicBool::new(false),
    });
    driver.idle_refs.fetch_add(1, Ordering::SeqCst);
    driver.ensure_loop(env_ref);
    let handle_out = register_tsfn(tsfn);
    unsafe {
        *result = handle_out;
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_threadsafe_function(
    func: napi_threadsafe_function,
    data: *mut c_void,
    mode: napi_threadsafe_function_call_mode,
) -> napi_status {
    if func.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(tsfn) = lookup_tsfn(func) else {
        return napi_status::napi_invalid_arg;
    };
    if !tsfn_is_open(&tsfn) {
        return napi_status::napi_closing;
    }

    let admission = if mode == napi_threadsafe_function_call_mode::napi_tsfn_blocking {
        tsfn.queue
            .admit_and_post(&tsfn.driver, tsfn.clone(), data as usize, true)
    } else {
        tsfn.queue
            .admit_and_post(&tsfn.driver, tsfn.clone(), data as usize, false)
    };
    let status = match admission {
        QueueAdmissionResult::Ok => napi_status::napi_ok,
        QueueAdmissionResult::Full => {
            // Non-blocking full queue: caller owns `data`; do not free here.
            napi_status::napi_queue_full
        },
        QueueAdmissionResult::PostFailed => {
            // Admission was rolled back (payload not owned by TSFN). Caller retains `data`.
            // Do not call call_js teardown here — that would free caller-owned memory.
            napi_status::napi_generic_failure
        },
        QueueAdmissionResult::Closing => napi_status::napi_closing,
    };
    if status != napi_status::napi_ok {
        return status;
    }

    if tsfn.js_thread_id == std::thread::current().id() {
        let env_ref = unsafe { Env::from_napi_env(tsfn.env) };
        tsfn.driver.ensure_loop(env_ref);
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_acquire_threadsafe_function(
    func: napi_threadsafe_function,
) -> napi_status {
    if func.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(tsfn) = lookup_tsfn(func) else {
        return napi_status::napi_invalid_arg;
    };
    if !tsfn_is_open(&tsfn) {
        return napi_status::napi_closing;
    }
    tsfn.refs.fetch_add(1, Ordering::Relaxed);
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_release_threadsafe_function(
    func: napi_threadsafe_function,
    mode: napi_threadsafe_function_release_mode,
) -> napi_status {
    if func.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(tsfn) = lookup_tsfn(func) else {
        return napi_status::napi_invalid_arg;
    };
    let on_js_thread = tsfn.js_thread_id == std::thread::current().id();

    let abort_mode = mode == napi_threadsafe_function_release_mode::napi_tsfn_abort;
    if abort_mode {
        tsfn.aborted.store(true, Ordering::Release);
        let _ = tsfn_begin_closing(&tsfn);
        tsfn.queue.set_closing();
        schedule_tsfn_abort_cleanup(&tsfn, on_js_thread);
    }

    let prev = tsfn
        .refs
        .try_update(Ordering::AcqRel, Ordering::Acquire, |v| {
            if v == 0 {
                None
            } else {
                Some(v - 1)
            }
        });
    match prev {
        Err(0) => napi_status::napi_invalid_arg,
        Ok(1) => {
            let _ = tsfn_begin_closing(&tsfn);
            tsfn.queue.set_closing();
            schedule_last_release(tsfn, on_js_thread);
            napi_status::napi_ok
        },
        Ok(_) => napi_status::napi_ok,
        Err(_) => napi_status::napi_invalid_arg,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_unref_threadsafe_function(
    env: napi_env,
    func: napi_threadsafe_function,
) -> napi_status {
    if env.is_null() || func.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(tsfn) = lookup_tsfn(func) else {
        return napi_status::napi_invalid_arg;
    };
    if tsfn.js_thread_id != std::thread::current().id() {
        return napi_status::napi_generic_failure;
    }
    let env_ref = unsafe { Env::from_napi_env(env) };
    if env_ref.as_napi_env() != tsfn.env {
        return napi_status::napi_invalid_arg;
    }
    if tsfn
        .idle_refs
        .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        tsfn.driver.idle_refs.fetch_sub(1, Ordering::SeqCst);
        tsfn.driver.wake_if_quiescent();
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_ref_threadsafe_function(
    env: napi_env,
    func: napi_threadsafe_function,
) -> napi_status {
    if env.is_null() || func.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(tsfn) = lookup_tsfn(func) else {
        return napi_status::napi_invalid_arg;
    };
    if tsfn.js_thread_id != std::thread::current().id() {
        return napi_status::napi_generic_failure;
    }
    let env_ref = unsafe { Env::from_napi_env(env) };
    if env_ref.as_napi_env() != tsfn.env {
        return napi_status::napi_invalid_arg;
    }
    if tsfn
        .idle_refs
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        tsfn.driver.idle_refs.fetch_add(1, Ordering::SeqCst);
        let env_ref = unsafe { Env::from_napi_env(env) };
        tsfn.driver.ensure_loop(env_ref);
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_threadsafe_function_context(
    func: napi_threadsafe_function,
    result: *mut *mut c_void,
) -> napi_status {
    if func.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(tsfn) = lookup_tsfn(func) else {
        return napi_status::napi_invalid_arg;
    };
    unsafe {
        *result = tsfn.context;
    }
    napi_status::napi_ok
}

// Promises

struct NapiDeferred {
    resolve_fn: JSValue,
    reject_fn: JSValue,
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_promise(
    env: napi_env,
    deferred: *mut napi_deferred,
    promise: *mut crate::types::napi_value,
) -> napi_status {
    if env.is_null() || deferred.is_null() || promise.is_null() {
        return napi_status::napi_invalid_arg;
    }
    with_env_promise(env, deferred, promise)
}

fn with_env_promise(
    env: napi_env,
    deferred: *mut napi_deferred,
    promise: *mut crate::types::napi_value,
) -> napi_status {
    let e = unsafe { Env::from_napi_env(env) };
    let ctx = e.ctx_ptr();
    let mut resolving: [JSValue; 2] = [qjs::JS_UNDEFINED, qjs::JS_UNDEFINED];
    let p = unsafe { qjs::JS_NewPromiseCapability(ctx, resolving.as_mut_ptr()) };
    if unsafe { qjs::JS_IsException(p) } {
        return napi_status::napi_generic_failure;
    }
    let boxed = Box::new(NapiDeferred {
        resolve_fn: resolving[0],
        reject_fn: resolving[1],
    });
    unsafe {
        *deferred = Box::into_raw(boxed) as napi_deferred;
        *promise = crate::value::value_to_napi_owned(e, p);
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_resolve_deferred(
    env: napi_env,
    deferred: napi_deferred,
    resolution: crate::types::napi_value,
) -> napi_status {
    if env.is_null() || deferred.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let e = unsafe { Env::from_napi_env(env) };
    let ctx = e.ctx_ptr();
    let def = unsafe { Box::from_raw(deferred as *mut NapiDeferred) };
    let mut val = if resolution.is_null() {
        qjs::JS_UNDEFINED
    } else {
        match unsafe { crate::value::napi_to_value_dup(e, resolution) } {
            Some(v) => v,
            None => {
                unsafe {
                    qjs::JS_FreeValue(ctx, def.resolve_fn);
                    qjs::JS_FreeValue(ctx, def.reject_fn);
                }
                return napi_status::napi_invalid_arg;
            },
        }
    };
    let result = unsafe { qjs::JS_Call(ctx, def.resolve_fn, qjs::JS_UNDEFINED, 1, &mut val) };
    if !resolution.is_null() {
        unsafe { qjs::JS_FreeValue(ctx, val) };
    }
    if unsafe { qjs::JS_IsException(result) } {
        unsafe {
            qjs::JS_FreeValue(ctx, def.resolve_fn);
            qjs::JS_FreeValue(ctx, def.reject_fn);
            qjs::JS_FreeValue(ctx, result);
        }
        return napi_status::napi_pending_exception;
    }
    unsafe {
        qjs::JS_FreeValue(ctx, result);
        qjs::JS_FreeValue(ctx, def.resolve_fn);
        qjs::JS_FreeValue(ctx, def.reject_fn);
    }
    napi_status::napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_reject_deferred(
    env: napi_env,
    deferred: napi_deferred,
    rejection: crate::types::napi_value,
) -> napi_status {
    if env.is_null() || deferred.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let e = unsafe { Env::from_napi_env(env) };
    let ctx = e.ctx_ptr();
    let def = unsafe { Box::from_raw(deferred as *mut NapiDeferred) };
    let mut val = if rejection.is_null() {
        qjs::JS_UNDEFINED
    } else {
        match unsafe { crate::value::napi_to_value_dup(e, rejection) } {
            Some(v) => v,
            None => {
                unsafe {
                    qjs::JS_FreeValue(ctx, def.resolve_fn);
                    qjs::JS_FreeValue(ctx, def.reject_fn);
                }
                return napi_status::napi_invalid_arg;
            },
        }
    };
    let result = unsafe { qjs::JS_Call(ctx, def.reject_fn, qjs::JS_UNDEFINED, 1, &mut val) };
    if !rejection.is_null() {
        unsafe { qjs::JS_FreeValue(ctx, val) };
    }
    if unsafe { qjs::JS_IsException(result) } {
        unsafe {
            qjs::JS_FreeValue(ctx, def.resolve_fn);
            qjs::JS_FreeValue(ctx, def.reject_fn);
            qjs::JS_FreeValue(ctx, result);
        }
        return napi_status::napi_pending_exception;
    }
    unsafe {
        qjs::JS_FreeValue(ctx, result);
        qjs::JS_FreeValue(ctx, def.resolve_fn);
        qjs::JS_FreeValue(ctx, def.reject_fn);
    }
    napi_status::napi_ok
}
