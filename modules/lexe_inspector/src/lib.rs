// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Minimal Node `inspector` compatibility surface.
//!
//! Lexe does **not** implement the Node Inspector debugging protocol.
//! `url()` always returns `undefined` so callers can probe whether an
//! inspector is listening. No `Session`, `open`, `close`, or
//! `waitForDebugger` exports are provided — those names would falsely
//! imply protocol support.

use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Ctx, Result, Undefined, Value,
};

/// Always `undefined`: Lexe has not started a Node Inspector endpoint.
fn url(ctx: Ctx<'_>) -> Result<Value<'_>> {
    Ok(Undefined.into_value(ctx))
}

pub struct InspectorModule;

impl ModuleDef for InspectorModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("url")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            default.set("url", Func::from(url))?;
            Ok(())
        })?;
        Ok(())
    }
}

impl From<InspectorModule> for ModuleInfo<InspectorModule> {
    fn from(val: InspectorModule) -> Self {
        ModuleInfo {
            name: "inspector",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    use super::*;

    #[tokio::test]
    async fn test_inspector_url_undefined() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<InspectorModule>(ctx.clone(), "inspector")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test_inspector",
                    r#"
                        import inspector, { url } from 'inspector';
                        export async function test() {
                          if (typeof url !== 'function') throw new Error('url missing');
                          if (url() !== undefined) throw new Error('url should be undefined');
                          if (inspector.url !== url) throw new Error('default/named url mismatch');
                          if ('Session' in inspector) throw new Error('must not expose Session');
                          return true;
                        }
                    "#,
                )
                .await
                .unwrap();
                let result = call_test::<bool, _>(&ctx, &module, ()).await;
                assert!(result);
            })
        })
        .await;
    }
}
