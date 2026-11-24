// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::module::ModuleInfo;
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    Ctx, Object, Result, Value,
};

const ASSERT_JS: &str = include_str!("assert_impl.js");

const ASSERT_EXPORTS: &[&str] = &[
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
    "partialDeepStrictEqual",
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
];

/// Build the `assert` object (a callable function with the full method set).
fn build_assert<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let assert: Object<'js> = ctx.eval(ASSERT_JS)?;
    Ok(assert)
}

fn declare_assert_exports(declare: &Declarations) -> Result<()> {
    for name in ASSERT_EXPORTS {
        declare.declare(*name)?;
    }
    declare.declare("default")?;
    Ok(())
}

fn export_assert_object<'js>(exports: &Exports<'js>, obj: Object<'js>) -> Result<()> {
    for name in ASSERT_EXPORTS {
        let value: Value<'js> = obj.get(*name)?;
        exports.export(*name, value)?;
    }
    exports.export("default", obj.into_value())?;
    Ok(())
}

pub struct AssertModule;

impl ModuleDef for AssertModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare_assert_exports(declare)
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let assert = build_assert(ctx)?;
        export_assert_object(exports, assert)
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
        declare_assert_exports(declare)
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let assert = build_assert(ctx)?;
        let strict: Object<'js> = assert.get("strict")?;
        export_assert_object(exports, strict)
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
