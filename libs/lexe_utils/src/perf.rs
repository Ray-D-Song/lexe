// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shared performance instrumentation: HdrHistogram, NodeTiming milestones,
//! event-loop utilization/delay tracking, GC hooks, and observer delivery.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rquickjs::{qjs, Ctx, Function, Object, Result, Value};

use crate::{driver_poll, time};

pub type RtKey = usize;

const UNSET_TIMESTAMP: u64 = u64::MAX;

/// Entry-type bitflags synchronized with `PerformanceObserver`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerfEntryKind {
    Mark = 1 << 0,
    Measure = 1 << 1,
    Resource = 1 << 2,
    Function = 1 << 3,
    Gc = 1 << 4,
    Http = 1 << 5,
    Http2 = 1 << 6,
    Dns = 1 << 7,
    Net = 1 << 8,
}

#[derive(Clone, Copy, Debug)]
pub struct GcEvent {
    pub start_ms: f64,
    pub duration_ms: f64,
    pub kind: i32,
    pub flags: i32,
}

#[derive(Clone, Debug)]
pub struct NodeTimingSnapshot {
    pub node_start: f64,
    pub v8_start: f64,
    pub bootstrap_complete: f64,
    pub environment: f64,
    pub loop_start: f64,
    pub loop_exit: f64,
    pub idle_time: f64,
    pub loop_count: u64,
    pub events: u64,
    pub events_waiting: u64,
}

#[derive(Clone, Debug)]
pub struct ResourceTimingRecord {
    pub name: String,
    pub initiator_type: String,
    pub next_hop_protocol: String,
    pub start_time: f64,
    pub duration: f64,
    pub worker_start: f64,
    pub redirect_start: f64,
    pub redirect_end: f64,
    pub fetch_start: f64,
    pub domain_lookup_start: f64,
    pub domain_lookup_end: f64,
    pub connect_start: f64,
    pub connect_end: f64,
    pub secure_connection_start: f64,
    pub request_start: f64,
    pub response_start: f64,
    pub response_end: f64,
    pub transfer_size: u64,
    pub encoded_body_size: u64,
    pub decoded_body_size: u64,
    pub response_status: u16,
    pub delivery_type: String,
}

struct EldSampler {
    id: u64,
    resolution: Duration,
    next_deadline: Instant,
    prev_tick_ns: u64,
    histogram: Arc<Mutex<HdrHistogram>>,
}

struct PerfRuntimeState {
    node_start_ns: AtomicU64,
    v8_start_ns: AtomicU64,
    environment_ns: AtomicU64,
    bootstrap_complete_ns: AtomicU64,
    loop_start_ns: AtomicU64,
    loop_exit_ns: AtomicU64,

    cumulative_active_ns: AtomicU64,
    cumulative_idle_ns: AtomicU64,
    active_turn_start_ns: AtomicU64,
    active_depth: AtomicUsize,

    loop_count: AtomicU64,
    events_count: AtomicU64,
    events_waiting: AtomicU64,

    observer_mask: AtomicU32,
    gc_start_ns: AtomicU64,
    has_pending_gc: AtomicBool,
    pending_gc: Mutex<Vec<GcEvent>>,

    eld_active_count: AtomicUsize,
    eld_samplers: Mutex<Vec<EldSampler>>,
}

impl PerfRuntimeState {
    fn new() -> Self {
        Self {
            node_start_ns: AtomicU64::new(UNSET_TIMESTAMP),
            v8_start_ns: AtomicU64::new(UNSET_TIMESTAMP),
            environment_ns: AtomicU64::new(UNSET_TIMESTAMP),
            bootstrap_complete_ns: AtomicU64::new(UNSET_TIMESTAMP),
            loop_start_ns: AtomicU64::new(UNSET_TIMESTAMP),
            loop_exit_ns: AtomicU64::new(UNSET_TIMESTAMP),
            cumulative_active_ns: AtomicU64::new(0),
            cumulative_idle_ns: AtomicU64::new(0),
            active_turn_start_ns: AtomicU64::new(0),
            active_depth: AtomicUsize::new(0),
            loop_count: AtomicU64::new(0),
            events_count: AtomicU64::new(0),
            events_waiting: AtomicU64::new(0),
            observer_mask: AtomicU32::new(0),
            gc_start_ns: AtomicU64::new(0),
            has_pending_gc: AtomicBool::new(false),
            pending_gc: Mutex::new(Vec::new()),
            eld_active_count: AtomicUsize::new(0),
            eld_samplers: Mutex::new(Vec::new()),
        }
    }
}

static PERF_STATES: LazyLock<Mutex<HashMap<RtKey, Arc<PerfRuntimeState>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_ELD_ID: AtomicU64 = AtomicU64::new(1);

type TimerResumeHook = fn(&Ctx<'_>) -> Result<()>;
static TIMER_RESUME_HOOK: OnceLock<TimerResumeHook> = OnceLock::new();

pub fn set_timer_loop_resume_hook(hook: TimerResumeHook) {
    let _ = TIMER_RESUME_HOOK.set(hook);
}

pub fn resume_timer_loop(ctx: &Ctx<'_>) -> Result<()> {
    if let Some(hook) = TIMER_RESUME_HOOK.get() {
        hook(ctx)?;
    }
    Ok(())
}

fn state_for(rt: RtKey) -> Arc<PerfRuntimeState> {
    let mut guard = PERF_STATES.lock().unwrap();
    guard
        .entry(rt)
        .or_insert_with(|| Arc::new(PerfRuntimeState::new()))
        .clone()
}

pub fn unregister_runtime(rt: RtKey) {
    PERF_STATES.lock().unwrap().remove(&rt);
}

pub fn rt_key_for_ctx(ctx: &Ctx<'_>) -> RtKey {
    let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    rt as RtKey
}

fn ns_to_milestone_ms(raw: u64) -> f64 {
    if raw == UNSET_TIMESTAMP {
        -1.0
    } else {
        raw as f64 / 1e6
    }
}

pub fn set_boot_milestones(
    rt: RtKey,
    node_start_ns: u64,
    v8_start_ns: u64,
    environment_ns: u64,
    bootstrap_complete_ns: u64,
) {
    let state = state_for(rt);
    state.node_start_ns.store(node_start_ns, Ordering::Release);
    state.v8_start_ns.store(v8_start_ns, Ordering::Release);
    state
        .environment_ns
        .store(environment_ns, Ordering::Release);
    state
        .bootstrap_complete_ns
        .store(bootstrap_complete_ns, Ordering::Release);
}

pub fn mark_loop_start_once(rt: RtKey) {
    let state = state_for(rt);
    if state.loop_start_ns.load(Ordering::Acquire) == UNSET_TIMESTAMP {
        let now = time::monotonic_elapsed_nanos();
        let _ = state.loop_start_ns.compare_exchange(
            UNSET_TIMESTAMP,
            now,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

pub fn mark_loop_exit(rt: RtKey) {
    let state = state_for(rt);
    state
        .loop_exit_ns
        .store(time::monotonic_elapsed_nanos(), Ordering::Release);
}

/// RAII guard attributing synchronous JS/callback execution to `active`.
pub struct ActiveTurnGuard {
    state: Arc<PerfRuntimeState>,
}

impl ActiveTurnGuard {
    pub fn enter(rt: RtKey) -> Self {
        let state = state_for(rt);
        if state.active_depth.fetch_add(1, Ordering::AcqRel) == 0 {
            state
                .active_turn_start_ns
                .store(time::monotonic_elapsed_nanos(), Ordering::Release);
        }
        Self { state }
    }
}

impl Drop for ActiveTurnGuard {
    fn drop(&mut self) {
        if self.state.active_depth.fetch_sub(1, Ordering::AcqRel) == 1 {
            let start = self.state.active_turn_start_ns.swap(0, Ordering::AcqRel);
            if start != 0 {
                let elapsed = time::monotonic_elapsed_nanos().saturating_sub(start);
                self.state
                    .cumulative_active_ns
                    .fetch_add(elapsed, Ordering::AcqRel);
            }
        }
    }
}

pub fn add_idle_ns(rt: RtKey, delta_ns: u64) {
    if delta_ns > 0 {
        state_for(rt)
            .cumulative_idle_ns
            .fetch_add(delta_ns, Ordering::AcqRel);
    }
}

pub fn inc_loop_count(rt: RtKey) {
    state_for(rt).loop_count.fetch_add(1, Ordering::Relaxed);
}

pub fn add_events(rt: RtKey, delta: u64) {
    if delta > 0 {
        state_for(rt)
            .events_count
            .fetch_add(delta, Ordering::Relaxed);
    }
}

pub fn event_loop_utilization_ms(rt: RtKey) -> (f64, f64) {
    let state = state_for(rt);
    let idle_ns = state.cumulative_idle_ns.load(Ordering::Acquire);
    let mut active_ns = state.cumulative_active_ns.load(Ordering::Acquire);
    if state.active_depth.load(Ordering::Acquire) > 0 {
        let start = state.active_turn_start_ns.load(Ordering::Acquire);
        if start != 0 {
            active_ns =
                active_ns.saturating_add(time::monotonic_elapsed_nanos().saturating_sub(start));
        }
    }
    (idle_ns as f64 / 1e6, active_ns as f64 / 1e6)
}

pub fn node_timing_snapshot(rt: RtKey) -> NodeTimingSnapshot {
    let state = state_for(rt);
    let (idle_time, _) = event_loop_utilization_ms(rt);
    NodeTimingSnapshot {
        node_start: ns_to_milestone_ms(state.node_start_ns.load(Ordering::Acquire)),
        v8_start: ns_to_milestone_ms(state.v8_start_ns.load(Ordering::Acquire)),
        bootstrap_complete: ns_to_milestone_ms(state.bootstrap_complete_ns.load(Ordering::Acquire)),
        environment: ns_to_milestone_ms(state.environment_ns.load(Ordering::Acquire)),
        loop_start: ns_to_milestone_ms(state.loop_start_ns.load(Ordering::Acquire)),
        loop_exit: ns_to_milestone_ms(state.loop_exit_ns.load(Ordering::Acquire)),
        idle_time,
        loop_count: state.loop_count.load(Ordering::Relaxed),
        events: state.events_count.load(Ordering::Relaxed),
        events_waiting: state.events_waiting.load(Ordering::Relaxed),
    }
}

pub fn set_observer_mask(rt: RtKey, mask: u32) {
    state_for(rt).observer_mask.store(mask, Ordering::Release);
}

#[inline]
pub fn has_observers(rt: RtKey, kind: PerfEntryKind) -> bool {
    let state = state_for(rt);
    (state.observer_mask.load(Ordering::Acquire) & (kind as u32)) != 0
}

unsafe extern "C" fn gc_hook_trampoline(
    rt: *mut qjs::JSRuntime,
    phase: core::ffi::c_int,
    flags: core::ffi::c_int,
    _opaque: *mut core::ffi::c_void,
) {
    let rt_key = rt as RtKey;
    let state = state_for(rt_key);
    if (state.observer_mask.load(Ordering::Acquire) & (PerfEntryKind::Gc as u32)) == 0 {
        return;
    }

    let now_ns = time::monotonic_elapsed_nanos();
    if phase == 0 {
        state.gc_start_ns.store(now_ns, Ordering::Release);
        return;
    }

    let start_ns = state.gc_start_ns.swap(0, Ordering::AcqRel);
    let effective_start = if start_ns == 0 { now_ns } else { start_ns };
    let duration_ns = now_ns.saturating_sub(effective_start);

    if let Ok(mut pending) = state.pending_gc.lock() {
        // Bound buffer size under heavy GC storms without observer polling.
        if pending.len() < 4096 {
            pending.push(GcEvent {
                start_ms: effective_start as f64 / 1e6,
                duration_ms: duration_ns as f64 / 1e6,
                kind: 4, // NODE_PERFORMANCE_GC_MAJOR (QuickJS cycle collection)
                flags,
            });
            state.has_pending_gc.store(true, Ordering::Release);
        }
    }
    driver_poll::wake_native_drivers(rt);
}

/// Register QuickJS GC prologue/epilogue callbacks for `rt`.
///
/// # Safety
/// `rt` must be a valid, live `JSRuntime` pointer for the current thread.
pub unsafe fn install_gc_hook(rt: *mut qjs::JSRuntime) {
    let _ = state_for(rt as RtKey);
    qjs::JS_SetGCHook(rt, Some(gc_hook_trampoline), std::ptr::null_mut());
}

/// Unregister QuickJS GC callbacks for `rt`.
///
/// # Safety
/// `rt` must be a valid, live `JSRuntime` pointer for the current thread.
pub unsafe fn uninstall_gc_hook(rt: *mut qjs::JSRuntime) {
    qjs::JS_SetGCHook(rt, None, std::ptr::null_mut());
}

pub fn flush_pending_gc(ctx: &Ctx<'_>) {
    let rt_key = rt_key_for_ctx(ctx);
    let state = state_for(rt_key);
    if !state.has_pending_gc.swap(false, Ordering::AcqRel) {
        return;
    }

    let events = {
        let mut guard = state.pending_gc.lock().unwrap();
        if guard.is_empty() {
            return;
        }
        std::mem::take(&mut *guard)
    };

    let Ok(Some(emit_fn)) = get_internal_fn(ctx, "emitGc") else {
        return;
    };

    for ev in events {
        let _ = emit_fn.call::<_, ()>((ev.start_ms, ev.duration_ms, ev.kind, ev.flags));
    }
}

pub fn register_eld_sampler(
    rt: RtKey,
    resolution: Duration,
    histogram: Arc<Mutex<HdrHistogram>>,
) -> u64 {
    let state = state_for(rt);
    let id = NEXT_ELD_ID.fetch_add(1, Ordering::Relaxed);
    let now_inst = Instant::now();
    let now_ns = time::monotonic_elapsed_nanos();
    let mut guard = state.eld_samplers.lock().unwrap();
    guard.push(EldSampler {
        id,
        resolution,
        next_deadline: now_inst + resolution,
        prev_tick_ns: now_ns,
        histogram,
    });
    state.eld_active_count.store(guard.len(), Ordering::Release);
    id
}

pub fn unregister_eld_sampler(rt: RtKey, id: u64) {
    let guard = PERF_STATES.lock().unwrap();
    let Some(state) = guard.get(&rt).cloned() else {
        return;
    };
    drop(guard);
    let mut samplers = state.eld_samplers.lock().unwrap();
    samplers.retain(|s| s.id != id);
    state
        .eld_active_count
        .store(samplers.len(), Ordering::Release);
}

#[inline]
pub fn has_active_eld_samplers(rt: RtKey) -> bool {
    let guard = PERF_STATES.lock().unwrap();
    guard
        .get(&rt)
        .is_some_and(|s| s.eld_active_count.load(Ordering::Acquire) > 0)
}

/// Sample any due ELD monitors and return the earliest upcoming deadline.
pub fn poll_eld_samplers(rt: RtKey, now: Instant) -> Option<Instant> {
    let state = {
        let guard = PERF_STATES.lock().unwrap();
        guard.get(&rt).cloned()?
    };
    if state.eld_active_count.load(Ordering::Acquire) == 0 {
        return None;
    }

    let now_ns = time::monotonic_elapsed_nanos();
    let mut samplers = state.eld_samplers.lock().unwrap();
    if samplers.is_empty() {
        state.eld_active_count.store(0, Ordering::Release);
        return None;
    }

    let mut earliest: Option<Instant> = None;
    for sampler in samplers.iter_mut() {
        if now >= sampler.next_deadline {
            let delta = now_ns.saturating_sub(sampler.prev_tick_ns).max(1);
            if let Ok(mut hist) = sampler.histogram.lock() {
                hist.record(delta);
            }
            sampler.prev_tick_ns = now_ns;
            let mut next = sampler.next_deadline + sampler.resolution;
            if next <= now {
                next = now + sampler.resolution;
            }
            sampler.next_deadline = next;
        }
        earliest = Some(match earliest {
            Some(prev) => prev.min(sampler.next_deadline),
            None => sampler.next_deadline,
        });
    }
    earliest
}

fn get_internal_fn<'js>(ctx: &Ctx<'js>, method: &str) -> Result<Option<Function<'js>>> {
    let sym = rquickjs::Symbol::new_global(ctx.clone(), "lexe.perfInternal")?;
    let internal: Option<Object<'js>> = ctx.globals().get(sym)?;
    let Some(internal) = internal else {
        return Ok(None);
    };
    internal.get(method)
}

pub fn emit_resource_timing(ctx: &Ctx<'_>, rec: ResourceTimingRecord) {
    let Ok(Some(emit_fn)) = get_internal_fn(ctx, "emitResource") else {
        return;
    };
    let Ok(obj) = Object::new(ctx.clone()) else {
        return;
    };
    let _ = obj.set("name", rec.name);
    let _ = obj.set("initiatorType", rec.initiator_type);
    let _ = obj.set("nextHopProtocol", rec.next_hop_protocol);
    let _ = obj.set("startTime", rec.start_time);
    let _ = obj.set("duration", rec.duration);
    let _ = obj.set("workerStart", rec.worker_start);
    let _ = obj.set("redirectStart", rec.redirect_start);
    let _ = obj.set("redirectEnd", rec.redirect_end);
    let _ = obj.set("fetchStart", rec.fetch_start);
    let _ = obj.set("domainLookupStart", rec.domain_lookup_start);
    let _ = obj.set("domainLookupEnd", rec.domain_lookup_end);
    let _ = obj.set("connectStart", rec.connect_start);
    let _ = obj.set("connectEnd", rec.connect_end);
    let _ = obj.set("secureConnectionStart", rec.secure_connection_start);
    let _ = obj.set("requestStart", rec.request_start);
    let _ = obj.set("responseStart", rec.response_start);
    let _ = obj.set("responseEnd", rec.response_end);
    let _ = obj.set("transferSize", rec.transfer_size as f64);
    let _ = obj.set("encodedBodySize", rec.encoded_body_size as f64);
    let _ = obj.set("decodedBodySize", rec.decoded_body_size as f64);
    let _ = obj.set("responseStatus", rec.response_status);
    let _ = obj.set("deliveryType", rec.delivery_type);
    let _ = emit_fn.call::<_, ()>((obj,));
}

pub fn emit_node_entry<'js, F>(
    ctx: &Ctx<'js>,
    kind: PerfEntryKind,
    name: &str,
    start_time: f64,
    duration: f64,
    build_detail: F,
) where
    F: FnOnce(&Ctx<'js>) -> Result<Value<'js>>,
{
    let rt_key = rt_key_for_ctx(ctx);
    if !has_observers(rt_key, kind) {
        return;
    }
    let Ok(Some(emit_fn)) = get_internal_fn(ctx, "emitNodeEntry") else {
        return;
    };
    let entry_type = match kind {
        PerfEntryKind::Http => "http",
        PerfEntryKind::Http2 => "http2",
        PerfEntryKind::Dns => "dns",
        PerfEntryKind::Net => "net",
        PerfEntryKind::Function => "function",
        PerfEntryKind::Gc => "gc",
        PerfEntryKind::Resource => "resource",
        PerfEntryKind::Mark => "mark",
        PerfEntryKind::Measure => "measure",
    };
    let Ok(detail) = build_detail(ctx) else {
        return;
    };
    let _ = emit_fn.call::<_, ()>((name, entry_type, start_time, duration, detail));
}

/// Pure-Rust HdrHistogram matching Node.js's `deps/histogram` log-linear bucketing.
pub struct HdrHistogram {
    lowest_discernible_value: u64,
    unit_magnitude: u32,
    sub_bucket_half_count_magnitude: u32,
    sub_bucket_half_count: usize,
    sub_bucket_mask: u64,
    counts_len: usize,
    pages: Vec<Option<Box<[u64]>>>,
    total_count: u64,
    exceeds: u64,
    min_recorded: u64,
    max_recorded: u64,
    prev_delta_ns: Option<u64>,
}

impl HdrHistogram {
    pub fn new(lowest: u64, highest: u64, figures: u8) -> std::result::Result<Self, &'static str> {
        if lowest == 0 {
            return Err("lowest must be >= 1");
        }
        let Some(min_highest) = lowest.checked_mul(2) else {
            return Err("highest must be >= 2 * lowest");
        };
        if highest < min_highest {
            return Err("highest must be >= 2 * lowest");
        }
        if !(1..=5).contains(&figures) {
            return Err("figures must be between 1 and 5");
        }

        let unit_magnitude = lowest.ilog2();
        let largest_single = 2u64 * 10u64.pow(figures as u32);
        let sub_bucket_count_magnitude = largest_single.next_power_of_two().ilog2().max(1);
        let sub_bucket_half_count_magnitude = sub_bucket_count_magnitude - 1;
        let sub_bucket_count = 1usize << sub_bucket_count_magnitude;
        let sub_bucket_half_count = sub_bucket_count >> 1;
        let sub_bucket_mask = ((sub_bucket_count as u64) - 1) << unit_magnitude;

        let mut smallest_untrackable = (sub_bucket_count as u128) << unit_magnitude;
        let mut bucket_count = 1usize;
        while smallest_untrackable <= highest as u128 {
            if smallest_untrackable > i64::MAX as u128 {
                bucket_count += 1;
                break;
            }
            smallest_untrackable <<= 1;
            bucket_count += 1;
        }

        let total_pages = bucket_count + 1;
        let counts_len = total_pages * sub_bucket_half_count;
        Ok(Self {
            lowest_discernible_value: lowest,
            unit_magnitude,
            sub_bucket_half_count_magnitude,
            sub_bucket_half_count,
            sub_bucket_mask,
            counts_len,
            pages: vec![None; total_pages],
            total_count: 0,
            exceeds: 0,
            min_recorded: u64::MAX,
            max_recorded: 0,
            prev_delta_ns: None,
        })
    }

    #[inline]
    fn bucket_and_sub_for(&self, value: u64) -> Option<(usize, usize)> {
        let pow2ceiling = 64 - (value | self.sub_bucket_mask).leading_zeros();
        let bucket_index = pow2ceiling
            .saturating_sub(self.unit_magnitude + self.sub_bucket_half_count_magnitude + 1)
            as usize;
        let shift = bucket_index as u32 + self.unit_magnitude;
        if shift >= 64 {
            return None;
        }
        let sub_bucket_index = (value >> shift) as usize;
        Some((bucket_index, sub_bucket_index))
    }

    #[inline]
    fn page_slot_for(&self, value: u64) -> Option<(usize, usize)> {
        let (bucket_index, sub_bucket_index) = self.bucket_and_sub_for(value)?;
        let counts_index = bucket_index
            .checked_mul(self.sub_bucket_half_count)?
            .checked_add(sub_bucket_index)?;
        if counts_index >= self.counts_len {
            return None;
        }
        Some((
            counts_index / self.sub_bucket_half_count,
            counts_index % self.sub_bucket_half_count,
        ))
    }

    #[inline]
    fn step_for_bucket(&self, bucket_index: usize) -> u128 {
        1u128 << (bucket_index as u32 + self.unit_magnitude)
    }

    #[inline]
    fn decode_page_slot(&self, page_idx: usize, slot_idx: usize) -> (usize, usize) {
        if page_idx < 2 {
            (0, page_idx * self.sub_bucket_half_count + slot_idx)
        } else {
            (page_idx - 1, self.sub_bucket_half_count + slot_idx)
        }
    }

    #[inline]
    fn lowest_equivalent_from(&self, bucket_index: usize, sub_bucket_index: usize) -> u64 {
        let val = (sub_bucket_index as u128) << (bucket_index as u32 + self.unit_magnitude);
        val.min(u64::MAX as u128) as u64
    }

    #[inline]
    fn highest_equivalent_from(&self, bucket_index: usize, sub_bucket_index: usize) -> u64 {
        let base = (sub_bucket_index as u128) << (bucket_index as u32 + self.unit_magnitude);
        let step = self.step_for_bucket(bucket_index);
        base.saturating_add(step)
            .saturating_sub(1)
            .min(i64::MAX as u128) as u64
    }

    #[inline]
    fn median_equivalent_from(&self, bucket_index: usize, sub_bucket_index: usize) -> f64 {
        let base = (sub_bucket_index as u128) << (bucket_index as u32 + self.unit_magnitude);
        let step = self.step_for_bucket(bucket_index);
        (base + (step >> 1)) as f64
    }

    #[inline]
    fn lowest_equivalent_value(&self, value: u64) -> u64 {
        if let Some((b, s)) = self.bucket_and_sub_for(value) {
            self.lowest_equivalent_from(b, s)
        } else {
            value
        }
    }

    #[inline]
    fn highest_equivalent_value(&self, value: u64) -> u64 {
        if let Some((b, s)) = self.bucket_and_sub_for(value) {
            self.highest_equivalent_from(b, s)
        } else {
            value
        }
    }

    pub fn record(&mut self, value: u64) -> bool {
        self.record_count(value, 1)
    }

    fn record_count(&mut self, value: u64, count: u64) -> bool {
        if count == 0 {
            return true;
        }
        let Some((page_idx, slot_idx)) = self.page_slot_for(value) else {
            self.exceeds = self.exceeds.saturating_add(count);
            return false;
        };
        let half = self.sub_bucket_half_count;
        let page = self.pages[page_idx].get_or_insert_with(|| vec![0u64; half].into_boxed_slice());
        page[slot_idx] = page[slot_idx].saturating_add(count);
        self.total_count = self.total_count.saturating_add(count);
        if value < self.min_recorded {
            self.min_recorded = value;
        }
        if value > self.max_recorded {
            self.max_recorded = value;
        }
        true
    }

    pub fn record_delta(&mut self) {
        let now = time::monotonic_elapsed_nanos();
        if let Some(prev) = self.prev_delta_ns {
            let delta = now.saturating_sub(prev).max(1);
            self.record(delta);
        }
        self.prev_delta_ns = Some(now);
    }

    pub fn reset(&mut self) {
        self.pages.fill(None);
        self.total_count = 0;
        self.exceeds = 0;
        self.min_recorded = u64::MAX;
        self.max_recorded = 0;
        self.prev_delta_ns = None;
    }

    pub fn add(&mut self, other: &Self) {
        let mut dropped = other.exceeds;
        for (page_idx, page_opt) in other.pages.iter().enumerate() {
            let Some(page) = page_opt else { continue };
            for (slot_idx, &count) in page.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let (b, s) = other.decode_page_slot(page_idx, slot_idx);
                let val = other.lowest_equivalent_from(b, s);
                if self.page_slot_for(val).is_some() {
                    self.record_count(val, count);
                } else {
                    dropped = dropped.saturating_add(count);
                }
            }
        }
        self.exceeds = self.exceeds.saturating_add(dropped);
    }

    pub fn count(&self) -> u64 {
        self.total_count
    }

    pub fn exceeds(&self) -> u64 {
        self.exceeds
    }

    pub fn lowest_discernible_value(&self) -> u64 {
        self.lowest_discernible_value
    }

    pub fn min(&self) -> u64 {
        if self.total_count == 0 {
            i64::MAX as u64
        } else {
            self.lowest_equivalent_value(self.min_recorded)
        }
    }

    pub fn max(&self) -> u64 {
        if self.total_count == 0 {
            0
        } else {
            self.highest_equivalent_value(self.max_recorded)
        }
    }

    pub fn mean(&self) -> f64 {
        if self.total_count == 0 {
            return f64::NAN;
        }
        let mut total_val = 0.0f64;
        for (page_idx, page_opt) in self.pages.iter().enumerate() {
            let Some(page) = page_opt else { continue };
            for (slot_idx, &count) in page.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let (b, s) = self.decode_page_slot(page_idx, slot_idx);
                total_val += count as f64 * self.median_equivalent_from(b, s);
            }
        }
        total_val / self.total_count as f64
    }

    pub fn stddev(&self) -> f64 {
        if self.total_count == 0 {
            return f64::NAN;
        }
        let mean = self.mean();
        let mut geom_dev_total = 0.0f64;
        for (page_idx, page_opt) in self.pages.iter().enumerate() {
            let Some(page) = page_opt else { continue };
            for (slot_idx, &count) in page.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let (b, s) = self.decode_page_slot(page_idx, slot_idx);
                let dev = self.median_equivalent_from(b, s) - mean;
                geom_dev_total += (dev * dev) * count as f64;
            }
        }
        (geom_dev_total / self.total_count as f64).sqrt()
    }

    pub fn value_at_percentile(&self, percentile: f64) -> u64 {
        if self.total_count == 0 {
            return 0;
        }
        let p = percentile.clamp(0.0, 100.0);
        let requested_count = ((p / 100.0) * self.total_count as f64).ceil() as u64;
        let requested_count = requested_count.max(1);
        let mut seen = 0u64;
        for (page_idx, page_opt) in self.pages.iter().enumerate() {
            let Some(page) = page_opt else { continue };
            for (slot_idx, &count) in page.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                seen = seen.saturating_add(count);
                if seen >= requested_count {
                    let (b, s) = self.decode_page_slot(page_idx, slot_idx);
                    return self.highest_equivalent_from(b, s);
                }
            }
        }
        self.max()
    }

    pub fn percentiles_map(&self) -> Vec<(f64, u64)> {
        if self.total_count == 0 {
            return vec![(100.0, 0)];
        }
        let mut out = Vec::new();
        let mut percentile = 0.0f64;

        let mut iter = self.pages.iter().enumerate().flat_map(|(pi, po)| {
            po.iter().flat_map(move |p| {
                p.iter()
                    .enumerate()
                    .filter(|(_, &c)| c > 0)
                    .map(move |(si, &c)| (pi, si, c))
            })
        });

        let mut running = 0u64;
        let mut cur_val = 0u64;
        if let Some((pi, si, c)) = iter.next() {
            running += c;
            let (b, s) = self.decode_page_slot(pi, si);
            cur_val = self.highest_equivalent_from(b, s);
        }

        loop {
            let target = (((percentile / 100.0) * self.total_count as f64).ceil() as u64).max(1);
            while running < target {
                let Some((pi, si, c)) = iter.next() else {
                    break;
                };
                running = running.saturating_add(c);
                let (b, s) = self.decode_page_slot(pi, si);
                cur_val = self.highest_equivalent_from(b, s);
            }
            out.push((percentile, cur_val));
            if running >= self.total_count {
                break;
            }
            let next = percentile + (100.0 - percentile) / 2.0;
            if next <= percentile {
                break;
            }
            percentile = next;
        }

        if out.last().is_none_or(|(p, _)| *p < 100.0) {
            out.push((100.0, self.max()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdr_histogram_precision_and_bounds() {
        let mut h1 = HdrHistogram::new(1, 1_000_000, 1).unwrap();
        assert!(h1.record(50_000));
        assert_eq!(h1.max(), 51_199);

        let mut h3 = HdrHistogram::new(1, 1_000_000, 3).unwrap();
        assert!(h3.record(50_000));
        assert_eq!(h3.max(), 50_015);

        let mut h_clamp = HdrHistogram::new(10, 100, 3).unwrap();
        assert!(h_clamp.record(5));
        assert!(h_clamp.record(200));
        assert_eq!(h_clamp.count(), 2);
        assert_eq!(h_clamp.min(), 0);
        assert_eq!(h_clamp.max(), 207);
        assert!(!h_clamp.record(1_000_000));
        assert_eq!(h_clamp.exceeds(), 1);
    }

    #[test]
    fn hdr_histogram_empty_percentiles_add_reset() {
        let mut h = HdrHistogram::new(1, 1_000, 3).unwrap();
        assert_eq!(h.count(), 0);
        assert_eq!(h.min(), i64::MAX as u64);
        assert_eq!(h.max(), 0);
        assert!(h.mean().is_nan());
        assert!(h.stddev().is_nan());
        assert_eq!(h.percentiles_map(), vec![(100.0, 0)]);

        for i in 1..=10 {
            assert!(h.record(i * 10));
        }
        let map = h.percentiles_map();
        assert_eq!(
            map,
            vec![
                (0.0, 10),
                (50.0, 50),
                (75.0, 80),
                (87.5, 90),
                (93.75, 100),
                (100.0, 100),
            ]
        );

        let mut other = HdrHistogram::new(1, 1_000, 3).unwrap();
        other.record(5000);
        h.add(&other);
        assert_eq!(h.exceeds(), 1);
        assert_eq!(h.count(), 10);

        h.reset();
        assert_eq!(h.count(), 0);
        assert_eq!(h.exceeds(), 0);
        assert_eq!(h.max(), 0);
    }
}
