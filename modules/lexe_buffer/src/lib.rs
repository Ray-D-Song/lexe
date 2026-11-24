// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::{
    module::{export_default, ModuleInfo},
    primordials::{BasePrimordials, Primordial},
};
use rquickjs::{
    atom::PredefinedAtom,
    function::Constructor,
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Class, Ctx, Function, IntoJs, Object, Result,
};

pub use self::array_buffer_view::*;
pub use self::blob::*;
pub use self::buffer::*;
pub use self::file::*;

mod array_buffer_view;
mod blob;
mod buffer;
mod codec;
mod extras;
mod file;
mod natives;

const EXTRA_EXPORTS: [&str; 10] = [
    "SlowBuffer",
    "transcode",
    "isUtf8",
    "isAscii",
    "kMaxLength",
    "kStringMaxLength",
    "Blob",
    "File",
    "resolveObjectURL",
    "INSPECT_MAX_BYTES",
];

/// Largest `ArrayBuffer` QuickJS allows (`INT32_MAX`), which bounds every Buffer.
const MAX_LENGTH: u32 = i32::MAX as u32;
const MAX_STRING_LENGTH: u32 = (1 << 30) - 1;

pub struct BufferModule;

impl ModuleDef for BufferModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare(stringify!(Buffer))?;
        declare.declare("atob")?;
        declare.declare("btoa")?;
        declare.declare("constants")?;
        for name in EXTRA_EXPORTS {
            declare.declare(name)?;
        }
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let globals = ctx.globals();
        let buf: Constructor = globals.get(stringify!(Buffer))?;

        let constants = Object::new(ctx.clone())?;
        constants.set("MAX_LENGTH", MAX_LENGTH)?; // For QuickJS
        constants.set("MAX_STRING_LENGTH", MAX_STRING_LENGTH)?; // For QuickJS

        let atob: Function = ctx.globals().get("atob")?;
        let btoa: Function = ctx.globals().get("btoa")?;

        export_default(ctx, exports, |default| {
            default.set(stringify!(Buffer), buf)?;
            default.set("atob", atob.into_js(ctx)?)?;
            default.set("btoa", btoa.into_js(ctx)?)?;
            default.set("constants", constants)?;
            install_extras(ctx, default)?;
            Ok(())
        })?;

        Ok(())
    }
}

fn install_extras<'js>(ctx: &Ctx<'js>, target: &Object<'js>) -> Result<()> {
    let natives = Object::new(ctx.clone())?;
    natives.set("transcodeNative", Func::from(extras::transcode_native))?;
    natives.set("isUtf8Native", Func::from(extras::is_utf8_native))?;
    natives.set("isAsciiNative", Func::from(extras::is_ascii_native))?;
    let install: Function = ctx.eval(include_str!("extras.js"))?;
    install.call::<_, ()>((natives, target.clone(), MAX_LENGTH, MAX_STRING_LENGTH))
}

impl From<BufferModule> for ModuleInfo<BufferModule> {
    fn from(val: BufferModule) -> Self {
        ModuleInfo {
            name: "buffer",
            module: val,
        }
    }
}

pub fn init<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let globals = ctx.globals();
    BasePrimordials::init(ctx)?;

    // `class Buffer extends Uint8Array {}` introduces a *global lexical*
    // binding, so evaluating it a second time throws
    // "redeclaration of 'Buffer'". init() can run more than once (e.g. the
    // http module also calls it from its module evaluation, in addition to the
    // startup global attachment), so make the setup idempotent.
    if ctx.eval::<bool, &str>(concat!("typeof ", stringify!(Buffer), " !== \"undefined\""))? {
        return Ok(());
    }

    // Buffer
    let buffer = ctx.eval::<Object<'js>, &str>(concat!(
        "class ",
        stringify!(Buffer),
        " extends Uint8Array {}\n",
        stringify!(Buffer),
    ))?;
    set_prototype(ctx, buffer)?;

    BufferPrimordials::init(ctx)?;

    // Blob
    if let Some(constructor) = Class::<Blob>::create_constructor(ctx)? {
        constructor.prop(
            PredefinedAtom::SymbolHasInstance,
            Func::from(Blob::has_instance),
        )?;
        globals.set(stringify!(Blob), constructor)?;
    }

    // File
    Class::<File>::define(&globals)?;

    //init primordials
    let _ = BufferPrimordials::get(ctx)?;

    Ok(())
}
