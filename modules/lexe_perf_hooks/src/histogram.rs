// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lexe_utils::perf::{self, HdrHistogram};
use rquickjs::class::{Trace, Tracer};
use rquickjs::{Array, BigInt, Class, Ctx, Exception, IntoJs, JsLifetime, Result, Value};

const ONE_HOUR_NANOS: u64 = 3_600_000_000_000;

struct EldSamplerGuard {
    rt_key: usize,
    sampler_id: Mutex<Option<u64>>,
}

impl Drop for EldSamplerGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.sampler_id.lock() {
            if let Some(id) = guard.take() {
                perf::unregister_eld_sampler(self.rt_key, id);
            }
        }
    }
}

#[rquickjs::class]
#[derive(Clone)]
pub struct NativeHistogram {
    inner: Arc<Mutex<HdrHistogram>>,
    resolution_ms: u64,
    sampler_guard: Arc<EldSamplerGuard>,
}

impl<'js> Trace<'js> for NativeHistogram {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

unsafe impl<'js> JsLifetime<'js> for NativeHistogram {
    type Changed<'to> = NativeHistogram;
}

impl NativeHistogram {
    pub fn new_recordable(ctx: &Ctx<'_>, lowest: f64, highest: f64, figures: f64) -> Result<Self> {
        let low = lowest as u64;
        let high = highest as u64;
        let figs = figures as u8;
        let hist =
            HdrHistogram::new(low, high, figs).map_err(|msg| Exception::throw_range(ctx, msg))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(hist)),
            resolution_ms: 0,
            sampler_guard: Arc::new(EldSamplerGuard {
                rt_key: 0,
                sampler_id: Mutex::new(None),
            }),
        })
    }

    pub fn new_eld(ctx: &Ctx<'_>, resolution_ms: f64) -> Result<Self> {
        let hist = HdrHistogram::new(1, ONE_HOUR_NANOS, 3)
            .map_err(|msg| Exception::throw_range(ctx, msg))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(hist)),
            resolution_ms: (resolution_ms as u64).max(1),
            sampler_guard: Arc::new(EldSamplerGuard {
                rt_key: perf::rt_key_for_ctx(ctx),
                sampler_id: Mutex::new(None),
            }),
        })
    }
}

fn pairs_to_array<'js>(
    ctx: &Ctx<'js>,
    pairs: Vec<(f64, u64)>,
    as_bigint: bool,
) -> Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    for (idx, (p, v)) in pairs.into_iter().enumerate() {
        let pair = Array::new(ctx.clone())?;
        pair.set(0, p)?;
        if as_bigint {
            pair.set(
                1,
                BigInt::from_i64(ctx.clone(), v.min(i64::MAX as u64) as i64)?,
            )?;
        } else {
            pair.set(1, v as f64)?;
        }
        arr.set(idx, pair)?;
    }
    Ok(arr)
}

#[rquickjs::methods(rename_all = "camelCase")]
impl NativeHistogram {
    pub fn record<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> Result<()> {
        let sample = if let Some(bi) = value.as_big_int() {
            let n = bi.clone().to_i64()?;
            if n < 1 {
                return Err(Exception::throw_range(&ctx, "value must be >= 1"));
            }
            n as u64
        } else if let Some(n) = value.as_int() {
            if n < 1 {
                return Err(Exception::throw_range(&ctx, "value must be >= 1"));
            }
            n as u64
        } else if let Some(n) = value.as_float() {
            if !n.is_finite() || n.fract() != 0.0 || n < 1.0 || n > (i64::MAX as f64) {
                return Err(Exception::throw_range(&ctx, "value out of range"));
            }
            n as u64
        } else {
            return Err(Exception::throw_type(&ctx, "val must be number or bigint"));
        };

        self.inner.lock().unwrap().record(sample);
        Ok(())
    }

    pub fn record_delta(&self) {
        self.inner.lock().unwrap().record_delta();
    }

    pub fn reset(&self) {
        self.inner.lock().unwrap().reset();
    }

    pub fn add<'js>(&self, ctx: Ctx<'js>, other: Class<'js, NativeHistogram>) -> Result<()> {
        let other_inner = other.borrow().inner.clone();
        if Arc::ptr_eq(&self.inner, &other_inner) {
            return Err(Exception::throw_range(
                &ctx,
                "Cannot add a histogram to itself",
            ));
        }
        let other_guard = other_inner.lock().unwrap();
        let mut self_guard = self.inner.lock().unwrap();
        self_guard.add(&other_guard);
        Ok(())
    }

    #[qjs(get)]
    pub fn count(&self) -> f64 {
        self.inner.lock().unwrap().count() as f64
    }

    #[qjs(get)]
    pub fn count_bigint<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let v = self.inner.lock().unwrap().count().min(i64::MAX as u64) as i64;
        BigInt::from_i64(ctx, v)
    }

    #[qjs(get)]
    pub fn exceeds(&self) -> f64 {
        self.inner.lock().unwrap().exceeds() as f64
    }

    #[qjs(get)]
    pub fn exceeds_bigint<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let v = self.inner.lock().unwrap().exceeds().min(i64::MAX as u64) as i64;
        BigInt::from_i64(ctx, v)
    }

    #[qjs(get)]
    pub fn min(&self) -> f64 {
        self.inner.lock().unwrap().min() as f64
    }

    #[qjs(get)]
    pub fn min_bigint<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let v = self.inner.lock().unwrap().min().min(i64::MAX as u64) as i64;
        BigInt::from_i64(ctx, v)
    }

    #[qjs(get)]
    pub fn max(&self) -> f64 {
        self.inner.lock().unwrap().max() as f64
    }

    #[qjs(get)]
    pub fn max_bigint<'js>(&self, ctx: Ctx<'js>) -> Result<BigInt<'js>> {
        let v = self.inner.lock().unwrap().max().min(i64::MAX as u64) as i64;
        BigInt::from_i64(ctx, v)
    }

    #[qjs(get)]
    pub fn mean(&self) -> f64 {
        self.inner.lock().unwrap().mean()
    }

    #[qjs(get)]
    pub fn stddev(&self) -> f64 {
        self.inner.lock().unwrap().stddev()
    }

    pub fn percentile(&self, p: f64) -> f64 {
        self.inner.lock().unwrap().value_at_percentile(p) as f64
    }

    pub fn percentile_bigint<'js>(&self, ctx: Ctx<'js>, p: f64) -> Result<BigInt<'js>> {
        let v = self
            .inner
            .lock()
            .unwrap()
            .value_at_percentile(p)
            .min(i64::MAX as u64) as i64;
        BigInt::from_i64(ctx, v)
    }

    pub fn percentiles_pairs<'js>(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let pairs = self.inner.lock().unwrap().percentiles_map();
        pairs_to_array(&ctx, pairs, false)
    }

    pub fn percentiles_bigint_pairs<'js>(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let pairs = self.inner.lock().unwrap().percentiles_map();
        pairs_to_array(&ctx, pairs, true)
    }

    pub fn enable(&self, ctx: Ctx<'_>) -> Result<bool> {
        let mut guard = self.sampler_guard.sampler_id.lock().unwrap();
        if guard.is_some() {
            return Ok(false);
        }
        let id = perf::register_eld_sampler(
            self.sampler_guard.rt_key,
            Duration::from_millis(self.resolution_ms.max(1)),
            self.inner.clone(),
        );
        *guard = Some(id);
        drop(guard);
        perf::resume_timer_loop(&ctx)?;
        Ok(true)
    }

    pub fn disable(&self) -> bool {
        let mut guard = self.sampler_guard.sampler_id.lock().unwrap();
        let Some(id) = guard.take() else {
            return false;
        };
        drop(guard);
        perf::unregister_eld_sampler(self.sampler_guard.rt_key, id);
        true
    }
}

pub fn into_class_value<'js>(ctx: &Ctx<'js>, hist: NativeHistogram) -> Result<Value<'js>> {
    Class::instance(ctx.clone(), hist)?.into_js(ctx)
}
