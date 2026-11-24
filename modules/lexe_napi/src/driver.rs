// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::thread::ThreadId;

use lexe_context::CtxExtension;
use parking_lot::Mutex;
use rquickjs::qjs::JSRuntime;
use rquickjs::Ctx;
use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::Notify;

use crate::env::Env;
use crate::types::{napi_env, napi_status};

pub enum DriverJob {
    AsyncComplete {
        work: Arc<crate::async_work::AsyncWorkState>,
        status: napi_status,
    },
    Tsfn {
        tsfn: Arc<crate::async_work::ThreadsafeFunction>,
        done: Option<SyncSender<()>>,
    },
    TsfnRelease {
        tsfn: Arc<crate::async_work::ThreadsafeFunction>,
    },
    TsfnAbort {
        tsfn: Arc<crate::async_work::ThreadsafeFunction>,
    },
    /// Wakes a parked `run_loop` when it should observe quiescence and exit.
    Wake,
}

pub struct DriverState {
    tx: Mutex<Option<mpsc::UnboundedSender<DriverJob>>>,
    rx: AsyncMutex<mpsc::UnboundedReceiver<DriverJob>>,
    pub pending: AtomicUsize,
    pub idle_refs: AtomicUsize,
    /// In-flight async work items that must keep the driver loop alive until complete.
    pub inflight_async: AtomicUsize,
    loop_running: AtomicBool,
    /// Set true when the driver has fully finished (loop exited or never started
    /// and confirmed idle after sender close).
    finished: AtomicBool,
    /// Multi-waiter notification when `finished` becomes true.
    finished_notify: Notify,
    pub js_thread_id: ThreadId,
    pub runtime: tokio::runtime::Handle,
    pub rt_ptr: *mut JSRuntime,
    #[cfg(test)]
    fail_posts: AtomicBool,
}

unsafe impl Send for DriverState {}
unsafe impl Sync for DriverState {}

impl DriverState {
    pub fn new(js_thread_id: ThreadId, rt_ptr: *mut JSRuntime) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx: Mutex::new(Some(tx)),
            rx: AsyncMutex::new(rx),
            pending: AtomicUsize::new(0),
            idle_refs: AtomicUsize::new(0),
            inflight_async: AtomicUsize::new(0),
            loop_running: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            finished_notify: Notify::new(),
            js_thread_id,
            runtime: tokio::runtime::Handle::current(),
            rt_ptr,
            #[cfg(test)]
            fail_posts: AtomicBool::new(false),
        }
    }

    /// Mark the driver finished and wake all waiters. Idempotent.
    pub fn mark_finished(&self) {
        if !self.finished.swap(true, Ordering::AcqRel) {
            self.finished_notify.notify_waiters();
        }
    }

    /// Wait until the driver is finished (loop exited or idle-never-started).
    pub async fn wait_finished(&self) {
        loop {
            let notified = self.finished_notify.notified();
            if self.finished.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn is_loop_running(&self) -> bool {
        self.loop_running.load(Ordering::Acquire)
    }

    /// After `close_sender()`, if the loop never started (or has already set
    /// `loop_running=false`) and there is no outstanding work, mark finished.
    ///
    /// When the loop is still running, only `run_loop`'s exit path calls
    /// `mark_finished()` — callers must `wait_finished()`.
    pub fn is_quiescent(&self) -> bool {
        self.pending.load(Ordering::Acquire) == 0
            && self.inflight_async.load(Ordering::Acquire) == 0
            && self.idle_refs.load(Ordering::Acquire) == 0
    }

    pub fn mark_finished_if_idle(&self) {
        if self.is_loop_running() {
            return;
        }
        if self.tx.lock().is_some() {
            return;
        }
        if !self.is_quiescent() {
            return;
        }
        self.mark_finished();
    }

    #[cfg(test)]
    pub fn set_fail_posts(&self, fail: bool) {
        self.fail_posts.store(fail, Ordering::SeqCst);
    }

    /// Thread-safe: enqueue a job without touching `Env` or QuickJS.
    pub fn post_job(&self, job: DriverJob) -> bool {
        #[cfg(test)]
        if self.fail_posts.load(Ordering::SeqCst) {
            return false;
        }
        let guard = self.tx.lock();
        let Some(tx) = guard.as_ref() else {
            return false;
        };
        self.pending.fetch_add(1, Ordering::SeqCst);
        if tx.send(job).is_err() {
            drop(guard);
            self.dec_pending();
            return false;
        }
        drop(guard);
        if !self.rt_ptr.is_null() {
            lexe_utils::driver_poll::wake_native_drivers(self.rt_ptr);
        }
        true
    }

    pub fn acquire_async_keepalive(&self) {
        self.inflight_async.fetch_add(1, Ordering::SeqCst);
    }

    pub fn release_async_keepalive(&self) {
        let _ = self
            .inflight_async
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                if v == 0 {
                    None
                } else {
                    Some(v - 1)
                }
            });
        self.wake_if_quiescent();
    }

    pub fn should_keep_loop_alive(&self) -> bool {
        !self.is_quiescent()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    /// If the driver loop is parked on an empty queue with no ref'd work, nudge it
    /// so it can observe the new quiescent state and exit.
    pub fn wake_if_quiescent(&self) {
        if !self.should_keep_loop_alive() {
            self.post_job(DriverJob::Wake);
        }
    }

    /// JS-thread only: enqueue and ensure the driver loop is running.
    pub fn post(&self, env: &mut Env, job: DriverJob) -> bool {
        if !self.post_job(job) {
            return false;
        }
        self.ensure_loop(env);
        true
    }

    pub fn should_ensure_loop(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0 && !self.loop_running.load(Ordering::SeqCst)
    }

    pub fn ensure_loop(&self, env: &mut Env) {
        let driver = env.driver.clone().expect("driver");
        driver.spawn_loop(env as *mut Env as usize, env.ctx_ptr());
    }

    fn spawn_loop(self: &Arc<Self>, env_addr: usize, ctx_ptr: *mut rquickjs::qjs::JSContext) {
        if self.loop_running.load(Ordering::SeqCst) {
            return;
        }
        let on_js_thread = std::thread::current().id() == self.js_thread_id;
        if !on_js_thread {
            debug_assert!(
                on_js_thread,
                "spawn_loop must run on the Env's registered JS thread"
            );
            tracing::debug!("spawn_loop skipped: current thread does not match Env JS thread");
            return;
        }
        if self.loop_running.swap(true, Ordering::SeqCst) {
            return;
        }
        // A previous wait_finished may have seen finished=true for an idle env;
        // starting a new loop requires unfinished state.
        self.finished.store(false, Ordering::Release);

        let driver = Arc::clone(self);
        let ctx = unsafe { Ctx::from_raw(NonNull::new_unchecked(ctx_ptr)) };
        ctx.spawn_exit_simple(async move {
            driver.run_loop(env_addr).await;
            Ok(())
        });
    }

    async fn run_loop(self: Arc<Self>, env_addr: usize) {
        let _napi_env = env_addr as napi_env;
        loop {
            if !self.should_keep_loop_alive() {
                let mut rx = self.rx.lock().await;
                match rx.try_recv() {
                    Ok(job) => {
                        drop(rx);
                        self.dispatch_job(env_addr, job);
                        continue;
                    },
                    Err(mpsc::error::TryRecvError::Empty) => {
                        // Channel empty and no keepalive — but inflight may race.
                        drop(rx);
                        if self.is_quiescent() {
                            break;
                        }
                        // Wait briefly for worker completion to post.
                        tokio::task::yield_now().await;
                        continue;
                    },
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        drop(rx);
                        // Sender closed: drain remaining and wait for quiescence.
                        self.wait_until_quiescent(env_addr).await;
                        break;
                    },
                }
            }

            let job = self.rx.lock().await.recv().await;
            let Some(job) = job else {
                // Disconnected while waiting — never finish until quiescent.
                self.wait_until_quiescent(env_addr).await;
                break;
            };
            self.dispatch_job(env_addr, job);
        }
        self.loop_running.store(false, Ordering::Release);

        while !self.is_quiescent() {
            self.drain_ready_jobs_from_ptr(env_addr);
            tokio::task::yield_now().await;
        }

        self.mark_finished();
    }

    /// After sender disconnect, drain the queue and wait until fully quiescent.
    async fn wait_until_quiescent(&self, env_addr: usize) {
        loop {
            self.drain_ready_jobs_from_ptr(env_addr);

            if self.is_quiescent() {
                return;
            }

            tokio::task::yield_now().await;
        }
    }

    fn drain_ready_jobs_from_ptr(&self, env_addr: usize) {
        let Ok(mut rx) = self.rx.try_lock() else {
            return;
        };
        while let Ok(job) = rx.try_recv() {
            self.dispatch_job(env_addr, job);
        }
    }

    fn dispatch_job(&self, env_addr: usize, job: DriverJob) {
        unsafe {
            crate::async_work::process_driver_job(env_addr as *mut Env, job);
        }
        self.dec_pending();
        unsafe {
            crate::gc_hook::drain_pending_finalizers(&mut *(env_addr as *mut Env));
        }
    }

    pub fn sender_clone(&self) -> Option<mpsc::UnboundedSender<DriverJob>> {
        self.tx.lock().clone()
    }

    pub fn close_sender(&self) {
        // Only drop the sender — do **not** mark finished here.
        // Premature finished (before pending hits 0) races with jobs already
        // enqueued by force-close TSFN. finished is set only by:
        // - run_loop exit (after pending/inflight are zero), or
        // - mark_finished_if_idle() once the loop is not running and queue is empty.
        self.tx.lock().take();
    }

    fn dec_pending(&self) {
        let _ = self
            .pending
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.saturating_sub(1))
            });
    }

    #[cfg(test)]
    fn drain_jobs_for_test(&self) {
        let Ok(mut rx) = self.rx.try_lock() else {
            return;
        };
        while rx.try_recv().is_ok() {
            self.dec_pending();
        }
    }

    /// JS-thread only: process any jobs already in the driver queue.
    pub fn drain_ready_jobs(&self, env: &mut Env) {
        let env_ptr = env as *mut Env;
        let Ok(mut rx) = self.rx.try_lock() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(job) => {
                    unsafe {
                        crate::async_work::process_driver_job(env_ptr, job);
                    }
                    self.dec_pending();
                    crate::gc_hook::drain_pending_finalizers(env);
                },
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
    }
}

pub fn ensure_driver(env: &mut Env) -> Arc<DriverState> {
    if env.driver.is_none() {
        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(env.ctx_ptr()) };
        lexe_utils::driver_poll::driver_notify_for_rt(rt_ptr);
        env.driver = Some(Arc::new(DriverState::new(env.js_thread_id, rt_ptr)));
    }
    env.driver.clone().unwrap()
}

pub fn shutdown_driver(env: &mut Env) {
    if let Some(driver) = env.driver.take() {
        driver.close_sender();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;

    use crate::async_work::AsyncWorkState;
    use crate::types::napi_status;
    use std::sync::atomic::AtomicU8;

    #[test]
    fn post_job_pending_reaches_zero_after_concurrent_dispatch() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let driver = Arc::new(DriverState::new(
                std::thread::current().id(),
                std::ptr::null_mut(),
            ));
            let work = Arc::new(AsyncWorkState {
                env: std::ptr::null_mut(),
                execute: None,
                complete: None,
                data: std::ptr::null_mut(),
                status: AtomicU8::new(0),
                completion_posted: AtomicBool::new(false),
            });
            let barrier = Arc::new(Barrier::new(3));
            let mut handles = Vec::new();
            for _ in 0..2 {
                let driver = Arc::clone(&driver);
                let work = Arc::clone(&work);
                let barrier = Arc::clone(&barrier);
                handles.push(thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..50 {
                        let _ = driver.post_job(DriverJob::AsyncComplete {
                            work: Arc::clone(&work),
                            status: napi_status::napi_ok,
                        });
                    }
                }));
            }
            barrier.wait();
            while driver.pending_count() > 0 {
                driver.drain_jobs_for_test();
                thread::yield_now();
            }
            for handle in handles {
                handle.join().unwrap();
            }
            driver.drain_jobs_for_test();
            assert_eq!(driver.pending_count(), 0);
        });
    }

    #[tokio::test]
    async fn wait_finished_after_close_sender_when_idle() {
        let driver = Arc::new(DriverState::new(
            std::thread::current().id(),
            std::ptr::null_mut(),
        ));
        assert!(!driver.is_finished());
        driver.close_sender();
        // close_sender alone does not mark finished; idle path must.
        driver.mark_finished_if_idle();
        driver.wait_finished().await;
        assert!(driver.is_finished());
    }

    #[tokio::test]
    async fn mark_finished_notifies_waiters() {
        let driver = Arc::new(DriverState::new(
            std::thread::current().id(),
            std::ptr::null_mut(),
        ));
        let d2 = Arc::clone(&driver);
        let join = tokio::spawn(async move {
            d2.wait_finished().await;
        });
        tokio::task::yield_now().await;
        driver.mark_finished();
        join.await.unwrap();
        assert!(driver.is_finished());
    }

    #[tokio::test]
    async fn mark_finished_if_idle_refuses_when_pending() {
        let driver = DriverState::new(std::thread::current().id(), std::ptr::null_mut());
        driver.close_sender();
        driver.pending.store(1, Ordering::SeqCst);
        driver.mark_finished_if_idle();
        assert!(!driver.is_finished());
        driver.pending.store(0, Ordering::SeqCst);
        driver.mark_finished_if_idle();
        assert!(driver.is_finished());
    }

    #[tokio::test]
    async fn mark_finished_if_idle_refuses_when_inflight() {
        let driver = DriverState::new(std::thread::current().id(), std::ptr::null_mut());
        driver.close_sender();
        driver.acquire_async_keepalive();
        driver.mark_finished_if_idle();
        assert!(!driver.is_finished());
        driver.release_async_keepalive();
        driver.mark_finished_if_idle();
        assert!(driver.is_finished());
    }

    #[tokio::test]
    async fn idle_refs_prevents_finished_until_released() {
        let driver = DriverState::new(std::thread::current().id(), std::ptr::null_mut());
        driver.idle_refs.fetch_add(1, Ordering::SeqCst);
        driver.close_sender();
        driver.mark_finished_if_idle();

        assert!(!driver.is_quiescent());
        assert!(!driver.is_finished());

        driver.idle_refs.fetch_sub(1, Ordering::SeqCst);
        driver.mark_finished_if_idle();

        assert!(driver.is_quiescent());
        assert!(driver.is_finished());
        driver.wait_finished().await;
    }
}
