// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Legacy Node.js `constants` module.
//!
//! Exports a frozen flat object containing `fs.constants` (`F_OK`/`R_OK`/`W_OK`/`X_OK`,
//! `O_*`, `S_*`, `COPYFILE_*`, `UV_*`), flattened `os.constants` (`errno`, `signals`,
//! `dlopen`, `priority`), and OpenSSL/crypto constants.

use lexe_fs::visit_fs_constants;
use lexe_os::constants::visit_flat_os_constants;
use lexe_utils::module::ModuleInfo;
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    Ctx, Function, IntoJs, Object, Result,
};

mod crypto;

use self::crypto::visit_crypto_constants;

pub struct ConstantsModule;

fn declare_const(declare: &Declarations, name: &'static str) -> Result<()> {
    declare.declare(name)?;
    Ok(())
}

fn export_const<'js, V: IntoJs<'js> + Copy>(
    constants: &Object<'js>,
    exports: &Exports<'js>,
    name: &'static str,
    val: V,
) -> Result<()> {
    constants.set(name, val)?;
    exports.export(name, val)?;
    Ok(())
}

impl ModuleDef for ConstantsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        visit_fs_constants(|name, _| declare_const(declare, name))?;
        visit_flat_os_constants(|name, _| declare_const(declare, name))?;
        visit_crypto_constants(
            |name, _| declare_const(declare, name),
            |name, _| declare_const(declare, name),
            |name, _| declare_const(declare, name),
        )?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let constants = Object::new(ctx.clone())?;

        visit_fs_constants(|name, val| export_const(&constants, exports, name, val))?;
        visit_flat_os_constants(|name, val| export_const(&constants, exports, name, val))?;
        visit_crypto_constants(
            |name, val| export_const(&constants, exports, name, val),
            |name, val| export_const(&constants, exports, name, val),
            |name, val| export_const(&constants, exports, name, val),
        )?;

        // Node freezes the legacy constants object.
        let object_ctor: Object = ctx.globals().get("Object")?;
        let freeze: Function = object_ctor.get("freeze")?;
        let frozen: Object = freeze.call((constants,))?;

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
                        import constants, {
                          F_OK, R_OK, W_OK, X_OK,
                          O_RDONLY, S_IFREG, ENOENT, SIGINT,
                          PRIORITY_LOW, RSA_PKCS1_PADDING,
                          SSL_OP_NO_TLSv1_2, SSL_OP_CRYPTOPRO_TLSEXT_BUG,
                          defaultCoreCipherList
                        } from 'constants';

                        export async function test() {
                          if (F_OK !== 0 || R_OK !== 4 || W_OK !== 2 || X_OK !== 1) {
                            throw new Error('named access export values wrong');
                          }
                          if (
                            constants.F_OK !== F_OK ||
                            constants.O_RDONLY !== O_RDONLY ||
                            constants.S_IFREG !== S_IFREG ||
                            constants.ENOENT !== ENOENT ||
                            constants.SIGINT !== SIGINT ||
                            constants.PRIORITY_LOW !== PRIORITY_LOW ||
                            constants.RSA_PKCS1_PADDING !== RSA_PKCS1_PADDING ||
                            constants.SSL_OP_NO_TLSv1_2 !== SSL_OP_NO_TLSv1_2 ||
                            constants.defaultCoreCipherList !== defaultCoreCipherList
                          ) {
                            throw new Error('default vs named mismatch');
                          }
                          if (SSL_OP_CRYPTOPRO_TLSEXT_BUG !== 0x80000000) {
                            throw new Error('u32 SSL_OP constant mismatch');
                          }
                          if (typeof constants.RTLD_NOW !== 'number' && typeof process === 'undefined') {
                            // Unix targets expose RTLD_NOW; checked in JS unit tests per platform.
                          }
                          if (!Object.isFrozen(constants)) {
                            throw new Error('constants should be frozen');
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
