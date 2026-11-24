// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_events::Emitter;
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    Class, Ctx, Result,
};

use crate::performance::Performance;

mod observer;
mod performance;

pub struct PerfHooksModule;

impl ModuleDef for PerfHooksModule {
    fn declare(declare: &Declarations<'_>) -> Result<()> {
        for name in [
            "performance",
            "Performance",
            "PerformanceEntry",
            "PerformanceMark",
            "PerformanceMeasure",
            "PerformanceObserver",
            "PerformanceObserverEntryList",
            "PerformanceResourceTiming",
            "constants",
            "createHistogram",
            "eventLoopUtilization",
            "monitorEventLoopDelay",
            "timerify",
            "default",
        ] {
            declare.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            observer::install(ctx, default)?;
            Ok(())
        })
    }
}

impl From<PerfHooksModule> for ModuleInfo<PerfHooksModule> {
    fn from(val: PerfHooksModule) -> Self {
        ModuleInfo {
            name: "perf_hooks",
            module: val,
        }
    }
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();

    // The observation layer reads the monotonic clock through this hook; it is
    // the same clock `performance.now()` uses, so entry timestamps and `now()`
    // are always on one timeline.
    globals.set(
        "__lexePerfNow",
        rquickjs::Function::new(ctx.clone(), || -> f64 { Performance::now() })?,
    )?;

    let instance = Class::instance(ctx.clone(), Performance::new())?;
    Performance::add_event_target_prototype(ctx)?;

    globals.set("performance", instance)?;
    Ok(())
}

// #[cfg(test)]
// mod tests {
//     use lexe_test::{call_test, test_async_with, ModuleEvaluator};
//     use rquickjs::CatchResultExt;

//     use super::*;

//     #[tokio::test]
//     async fn test_now() {
//         time::init();
//         test_async_with(|ctx| {
//             Box::pin(async move {
//                 ModuleEvaluator::eval_rust::<PerfHooksModule>(ctx.clone(), "perf_hooks")
//                     .await
//                     .catch(&ctx)
//                     .unwrap();

//                 let module = ModuleEvaluator::eval_js(
//                     ctx.clone(),
//                     "test",
//                     r#"
//                         import { performance } from 'perf_hooks';

//                         export async function test() {
//                             const now = performance.now()
//                             // TODO: Delaying with setTimeout
//                             for(let i=0; i < (1<<20); i++){}

//                             return performance.now() - now
//                         }
//                     "#,
//                 )
//                 .await
//                 .unwrap();
//                 let result = call_test::<u32, _>(&ctx, &module, ()).await;
//                 assert!(result > 0)
//             })
//         })
//         .await;
//     }

//     #[tokio::test]
//     async fn test_time_origin() {
//         time::init();
//         test_async_with(|ctx| {
//             Box::pin(async move {
//                 ModuleEvaluator::eval_rust::<PerfHooksModule>(ctx.clone(), "perf_hooks")
//                     .await
//                     .catch(&ctx)
//                     .unwrap();

//                 let module = ModuleEvaluator::eval_js(
//                     ctx.clone(),
//                     "test",
//                     r#"
//                         import { performance } from 'perf_hooks';

//                         export async function test() {
//                             return performance.timeOrigin
//                         }
//                     "#,
//                 )
//                 .await
//                 .catch(&ctx)
//                 .unwrap();
//                 let result = call_test::<f64, _>(&ctx, &module, ()).await;
//                 assert!(result > 0.0);
//             })
//         })
//         .await;
//     }

//     #[tokio::test]
//     async fn test_to_json() {
//         time::init();
//         test_async_with(|ctx| {
//             Box::pin(async move {
//                 ModuleEvaluator::eval_rust::<PerfHooksModule>(ctx.clone(), "perf_hooks")
//                     .await
//                     .unwrap();

//                 let module = ModuleEvaluator::eval_js(
//                     ctx.clone(),
//                     "test",
//                     r#"
//                         import { performance } from 'perf_hooks';

//                         export async function test() {
//                             return performance.toJSON()
//                         }
//                     "#,
//                 )
//                 .await
//                 .unwrap();
//                 let result = call_test::<Object, _>(&ctx, &module, ()).await;
//                 let time_origin = result.get::<_, f64>("timeOrigin").unwrap();
//                 assert!(time_origin > 0.);
//             })
//         })
//         .await;
//     }
// }
