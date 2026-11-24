// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Node-compatible `v8` surface backed by QuickJS heap statistics.
//!
//! Values come from QuickJS `JS_ComputeMemoryUsage`, not from Google V8.
//! Unsupported V8 features (serialization, snapshots, profilers) are not exposed.
//! `setFlagsFromString` is a compatibility no-op.

use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, Opt},
    qjs, Array, Ctx, Exception, Object, Result, Value,
};

/// ECMAScript `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Convert a non-negative size into a finite JS number, clamping to MAX_SAFE_INTEGER.
fn safe_number_u64(value: u64) -> f64 {
    value.min(MAX_SAFE_INTEGER) as f64
}

/// Convert a QuickJS signed counter: negative values become 0, then clamp.
fn safe_number_i64(value: i64) -> f64 {
    if value <= 0 {
        0.0
    } else {
        safe_number_u64(value as u64)
    }
}

// SAFETY:
// - The associated QuickJS runtime must stay alive for the duration of this call.
// - QuickJS runtimes must not be accessed concurrently.
// - The runtime is not destroyed while this function runs.
unsafe fn compute_memory_usage(ctx: &Ctx<'_>) -> qjs::JSMemoryUsage {
    let mut usage = std::mem::zeroed();
    let runtime = qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
    qjs::JS_ComputeMemoryUsage(runtime, &mut usage);
    usage
}

fn heap_limit(usage: &qjs::JSMemoryUsage) -> f64 {
    // QuickJS uses malloc_limit == 0 when no limit is configured.
    if usage.malloc_limit > 0 {
        safe_number_i64(usage.malloc_limit)
    } else {
        MAX_SAFE_INTEGER as f64
    }
}

fn get_heap_statistics(ctx: Ctx<'_>) -> Result<Object<'_>> {
    let usage = unsafe { compute_memory_usage(&ctx) };
    let malloc_size = safe_number_i64(usage.malloc_size);
    let used = safe_number_i64(usage.memory_used_size);
    let limit = heap_limit(&usage);
    let available = (limit - used).max(0.0);

    let obj = Object::new(ctx.clone())?;
    obj.set("total_heap_size", malloc_size)?;
    obj.set(
        "total_heap_size_executable",
        safe_number_i64(usage.js_func_code_size),
    )?;
    obj.set("total_physical_size", malloc_size)?;
    obj.set("total_available_size", available)?;
    obj.set("used_heap_size", used)?;
    obj.set("heap_size_limit", limit)?;
    obj.set("malloced_memory", malloc_size)?;
    obj.set("peak_malloced_memory", malloc_size)?;
    obj.set("does_zap_garbage", 0)?;
    obj.set("number_of_native_contexts", 1)?;
    obj.set("number_of_detached_contexts", 0)?;
    obj.set("total_global_handles_size", 0)?;
    obj.set("used_global_handles_size", 0)?;
    obj.set("external_memory", safe_number_i64(usage.binary_object_size))?;
    obj.set("total_allocated_bytes", malloc_size)?;
    Ok(obj)
}

fn get_heap_space_statistics(ctx: Ctx<'_>) -> Result<Array<'_>> {
    let usage = unsafe { compute_memory_usage(&ctx) };
    let malloc_size = safe_number_i64(usage.malloc_size);
    let used = safe_number_i64(usage.memory_used_size);
    let limit = heap_limit(&usage);
    let available = (limit - used).max(0.0);

    let space = Object::new(ctx.clone())?;
    space.set("space_name", "quickjs")?;
    space.set("space_size", malloc_size)?;
    space.set("space_used_size", used)?;
    space.set("space_available_size", available)?;
    space.set("physical_space_size", malloc_size)?;

    let arr = Array::new(ctx)?;
    arr.set(0, space)?;
    Ok(arr)
}

fn get_heap_code_statistics(ctx: Ctx<'_>) -> Result<Object<'_>> {
    let usage = unsafe { compute_memory_usage(&ctx) };
    let bytecode = usage
        .js_func_code_size
        .saturating_add(usage.js_func_pc2line_size);

    let obj = Object::new(ctx.clone())?;
    obj.set(
        "code_and_metadata_size",
        safe_number_i64(usage.js_func_size),
    )?;
    obj.set("bytecode_and_metadata_size", safe_number_i64(bytecode))?;
    obj.set("external_script_source_size", 0)?;
    obj.set("cpu_profiler_metadata_size", 0)?;
    Ok(obj)
}

/// Compatibility no-op: Lexe does not run V8 and does not apply flags to QuickJS.
fn set_flags_from_string(ctx: Ctx<'_>, flags: Opt<Value<'_>>) -> Result<()> {
    let Some(value) = flags.0 else {
        return Err(Exception::throw_type(
            &ctx,
            "The \"flags\" argument must be of type string",
        ));
    };
    if value.as_string().is_none() {
        return Err(Exception::throw_type(
            &ctx,
            "The \"flags\" argument must be of type string",
        ));
    }
    Ok(())
}

pub struct V8Module;

impl ModuleDef for V8Module {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("getHeapStatistics")?;
        declare.declare("getHeapSpaceStatistics")?;
        declare.declare("getHeapCodeStatistics")?;
        declare.declare("setFlagsFromString")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            default.set("getHeapStatistics", Func::from(get_heap_statistics))?;
            default.set(
                "getHeapSpaceStatistics",
                Func::from(get_heap_space_statistics),
            )?;
            default.set(
                "getHeapCodeStatistics",
                Func::from(get_heap_code_statistics),
            )?;
            default.set("setFlagsFromString", Func::from(set_flags_from_string))?;
            Ok(())
        })
    }
}

impl From<V8Module> for ModuleInfo<V8Module> {
    fn from(val: V8Module) -> Self {
        ModuleInfo {
            name: "v8",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    #[tokio::test]
    async fn test_v8_heap_statistics() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<V8Module>(ctx.clone(), "v8")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import {
                          getHeapStatistics,
                          getHeapSpaceStatistics,
                          getHeapCodeStatistics,
                          setFlagsFromString,
                        } from 'v8';

                        export async function test() {
                          const stats = getHeapStatistics();
                          const spaces = getHeapSpaceStatistics();
                          const code = getHeapCodeStatistics();
                          const flagsResult = setFlagsFromString('--detect-ineffective-gcs-near-heap-limit');

                          const numberFields = [
                            'total_heap_size',
                            'total_heap_size_executable',
                            'total_physical_size',
                            'total_available_size',
                            'used_heap_size',
                            'heap_size_limit',
                            'malloced_memory',
                            'peak_malloced_memory',
                            'does_zap_garbage',
                            'number_of_native_contexts',
                            'number_of_detached_contexts',
                            'total_global_handles_size',
                            'used_global_handles_size',
                            'external_memory',
                            'total_allocated_bytes',
                          ];

                          for (const key of numberFields) {
                            if (!(key in stats)) throw new Error('missing ' + key);
                            const v = stats[key];
                            if (!Number.isFinite(v) || v < 0 || v > Number.MAX_SAFE_INTEGER) {
                              throw new Error('bad value for ' + key + ': ' + v);
                            }
                          }

                          if (stats.heap_size_limit < stats.used_heap_size) {
                            throw new Error('heap_size_limit < used_heap_size');
                          }
                          if (stats.total_available_size < 0) {
                            throw new Error('total_available_size < 0');
                          }

                          if (!Array.isArray(spaces) || spaces.length !== 1) {
                            throw new Error('expected one heap space');
                          }
                          if (spaces[0].space_name !== 'quickjs') {
                            throw new Error('expected space_name quickjs');
                          }
                          for (const key of [
                            'space_size',
                            'space_used_size',
                            'space_available_size',
                            'physical_space_size',
                          ]) {
                            const v = spaces[0][key];
                            if (!Number.isFinite(v) || v < 0 || v > Number.MAX_SAFE_INTEGER) {
                              throw new Error('bad space field ' + key + ': ' + v);
                            }
                          }

                          for (const key of [
                            'code_and_metadata_size',
                            'bytecode_and_metadata_size',
                            'external_script_source_size',
                            'cpu_profiler_metadata_size',
                          ]) {
                            if (!(key in code)) throw new Error('missing code field ' + key);
                            const v = code[key];
                            if (!Number.isFinite(v) || v < 0 || v > Number.MAX_SAFE_INTEGER) {
                              throw new Error('bad code field ' + key + ': ' + v);
                            }
                          }

                          if (flagsResult !== undefined) {
                            throw new Error('setFlagsFromString should return undefined');
                          }

                          let typeError = false;
                          try {
                            setFlagsFromString(42);
                          } catch (e) {
                            typeError = e instanceof TypeError;
                          }
                          if (!typeError) throw new Error('expected TypeError for non-string flags');

                          return true;
                        }
                    "#,
                )
                .await
                .unwrap();

                let ok = call_test::<bool, _>(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }
}
