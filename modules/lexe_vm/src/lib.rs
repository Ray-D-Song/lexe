// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

mod run_in_new_context;

use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    function::Func,
    module::{Declarations, Exports, ModuleDef},
    prelude::Opt,
    Ctx, Result, Value,
};

use run_in_new_context::run_in_new_context_impl;

fn run_in_new_context<'js>(
    ctx: Ctx<'js>,
    code: Value<'js>,
    context_object: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    run_in_new_context_impl(ctx, code, context_object.0, options.0)
}

pub struct VmModule;

impl ModuleDef for VmModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("runInNewContext")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            let run_in_new_context = Func::from(run_in_new_context);
            default.set("runInNewContext", run_in_new_context)?;
            Ok(())
        })
    }
}

impl From<VmModule> for ModuleInfo<VmModule> {
    fn from(val: VmModule) -> Self {
        ModuleInfo {
            name: "vm",
            module: val,
        }
    }
}
