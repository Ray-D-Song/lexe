// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use raster_runtime_utils::module::ModuleInfo;
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    Ctx, Function, Object, Result,
};

const ASSERT_JS: &str = include_str!("assert_impl.js");

/// Build the `assert` object (a callable function with the full method set).
fn build_assert<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let assert: Object<'js> = ctx.eval(ASSERT_JS)?;
    Ok(assert)
}

pub struct AssertModule;

impl ModuleDef for AssertModule {
    fn declare(declare: &Declarations) -> Result<()> {
        // The whole surface is produced by the JS implementation, so every name
        // it defines has to be declared here as well.
        for name in [
            "ok",
            "fail",
            "equal",
            "notEqual",
            "strictEqual",
            "notStrictEqual",
            "deepEqual",
            "notDeepEqual",
            "deepStrictEqual",
            "notDeepStrictEqual",
            "throws",
            "doesNotThrow",
            "rejects",
            "doesNotReject",
            "match",
            "doesNotMatch",
            "ifError",
            "AssertionError",
            "CallTracker",
            "strict",
            "default",
        ] {
            declare.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let assert = build_assert(ctx)?;
        for name in [
            "ok",
            "fail",
            "equal",
            "notEqual",
            "strictEqual",
            "notStrictEqual",
            "deepEqual",
            "notDeepEqual",
            "deepStrictEqual",
            "notDeepStrictEqual",
            "throws",
            "doesNotThrow",
            "rejects",
            "doesNotReject",
            "match",
            "doesNotMatch",
            "ifError",
            "AssertionError",
            "CallTracker",
            "strict",
        ] {
            let value = assert.get::<_, rquickjs::Value<'js>>(name)?;
            exports.export(name, value)?;
        }
        let as_value = assert.clone().into_value();
        exports.export("default", as_value)?;
        Ok(())
    }
}

impl From<AssertModule> for ModuleInfo<AssertModule> {
    fn from(val: AssertModule) -> Self {
        ModuleInfo {
            name: "assert",
            module: val,
        }
    }
}

pub struct AssertStrictModule;

impl ModuleDef for AssertStrictModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let assert = build_assert(ctx)?;
        let strict: Function<'js> = assert.get("strict")?;
        exports.export("default", strict)?;
        Ok(())
    }
}

impl From<AssertStrictModule> for ModuleInfo<AssertStrictModule> {
    fn from(val: AssertStrictModule) -> Self {
        ModuleInfo {
            name: "assert/strict",
            module: val,
        }
    }
}
