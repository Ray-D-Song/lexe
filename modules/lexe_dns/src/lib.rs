// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Ctx, Object, Result,
};

use crate::lookup::{lookup, shared_promises_lookup};

mod lookup;

pub struct DnsModule;

impl ModuleDef for DnsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("lookup")?;
        declare.declare("promises")?;

        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            let promises = Object::new(ctx.clone())?;
            let promises_lookup = shared_promises_lookup(ctx)?;
            promises.set("lookup", promises_lookup)?;

            default.set("lookup", Func::from(lookup))?;
            default.set("promises", promises)?;
            Ok(())
        })?;

        Ok(())
    }
}

impl From<DnsModule> for ModuleInfo<DnsModule> {
    fn from(val: DnsModule) -> Self {
        ModuleInfo {
            name: "dns",
            module: val,
        }
    }
}

pub struct DnsPromisesModule;

impl ModuleDef for DnsPromisesModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("lookup")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            let promises_lookup = shared_promises_lookup(ctx)?;
            default.set("lookup", promises_lookup)?;
            Ok(())
        })?;

        Ok(())
    }
}

impl From<DnsPromisesModule> for ModuleInfo<DnsPromisesModule> {
    fn from(val: DnsPromisesModule) -> Self {
        ModuleInfo {
            name: "dns/promises",
            module: val,
        }
    }
}
