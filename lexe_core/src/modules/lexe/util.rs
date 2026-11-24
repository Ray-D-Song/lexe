// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{
    context::EvalOptions,
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, Opt},
    Array, Ctx, Object, Result, Value,
};

use crate::libs::utils::{
    module::{export_default, ModuleInfo},
    object::ObjectExt,
};

fn dimensions(ctx: Ctx<'_>) -> Result<Array<'_>> {
    let array = Array::new(ctx.clone())?;
    match terminal_size::terminal_size() {
        Some((width, height)) => {
            array.set(0, width.0)?;
            array.set(1, height.0)?;
        },
        None => {
            array.set(0, 0)?;
            array.set(1, 0)?;
        },
    }
    Ok(array)
}

/// Unicode display width for readline cursor math (Node getStringWidth).
fn string_width(s: String) -> usize {
    unicode_width::UnicodeWidthStr::width(s.as_str())
}

fn load<'js>(ctx: Ctx<'js>, filename: String, options: Opt<Object<'js>>) -> Result<Value<'js>> {
    let mut eval_options = EvalOptions::default();
    eval_options.strict = false;
    eval_options.promise = true;

    if let Some(options) = options.0 {
        if let Some(global) = options.get_optional("global")? {
            eval_options.global = global;
        }

        if let Some(strict) = options.get_optional("strict")? {
            eval_options.strict = strict;
        }
    }

    ctx.eval_file_with_options(filename, eval_options)
}

fn print(value: String, stdout: Opt<bool>) {
    if stdout.0.unwrap_or_default() {
        println!("{value}");
    } else {
        eprintln!("{value}")
    }
}

pub struct LexeUtilModule;

impl ModuleDef for LexeUtilModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("dimensions")?;
        declare.declare("load")?;
        declare.declare("print")?;
        declare.declare("stringWidth")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            default.set("dimensions", Func::from(dimensions))?;
            default.set("load", Func::from(load))?;
            default.set("print", Func::from(print))?;
            default.set("stringWidth", Func::from(string_width))?;
            Ok(())
        })
    }
}

impl From<LexeUtilModule> for ModuleInfo<LexeUtilModule> {
    fn from(val: LexeUtilModule) -> Self {
        ModuleInfo {
            name: "lexe:util",
            module: val,
        }
    }
}
