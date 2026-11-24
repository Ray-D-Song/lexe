// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Legacy Node.js `constants` module.
//!
//! Flat object with fs access-mode bits only (`F_OK`/`R_OK`/`W_OK`/`X_OK`).
//! Does not expose open flags, errno, crypto, or signal constants that Lexe
//! does not fully support.

use lexe_fs::{create_constants, CONSTANT_F_OK, CONSTANT_R_OK, CONSTANT_W_OK, CONSTANT_X_OK};
use lexe_utils::module::ModuleInfo;
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    Ctx, Function, Object, Result,
};

pub struct ConstantsModule;

impl ModuleDef for ConstantsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("F_OK")?;
        declare.declare("R_OK")?;
        declare.declare("W_OK")?;
        declare.declare("X_OK")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let constants = create_constants(ctx)?;

        // Node freezes the legacy constants object.
        let object_ctor: Object = ctx.globals().get("Object")?;
        let freeze: Function = object_ctor.get("freeze")?;
        let frozen: Object = freeze.call((constants.clone(),))?;

        exports.export("F_OK", CONSTANT_F_OK)?;
        exports.export("R_OK", CONSTANT_R_OK)?;
        exports.export("W_OK", CONSTANT_W_OK)?;
        exports.export("X_OK", CONSTANT_X_OK)?;
        // Prefer the frozen object for default / CJS require shape.
        exports.export("default", frozen)?;
        Ok(())
    }
}

impl From<ConstantsModule> for ModuleInfo<ConstantsModule> {
    fn from(val: ConstantsModule) -> Self {
        ModuleInfo {
            name: "constants",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    #[tokio::test]
    async fn test_constants_module_shape() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<ConstantsModule>(ctx.clone(), "constants")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import constants, { F_OK, R_OK, W_OK, X_OK } from 'constants';

                        export async function test() {
                          if (F_OK !== 0 || R_OK !== 4 || W_OK !== 2 || X_OK !== 1) {
                            throw new Error('named export values wrong');
                          }
                          if (constants.F_OK !== F_OK || constants.R_OK !== R_OK) {
                            throw new Error('default vs named mismatch');
                          }
                          if (!Object.isFrozen(constants)) {
                            throw new Error('constants should be frozen');
                          }
                          if ('O_SYMLINK' in constants) {
                            throw new Error('must not expose O_SYMLINK');
                          }
                          try {
                            constants.F_OK = 99;
                          } catch (_) {
                            // freeze may throw
                          }
                          if (constants.F_OK !== 0) {
                            throw new Error('F_OK must remain 0 after write attempt');
                          }
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
