// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{
    atom::PredefinedAtom, class::Trace, function::Opt, object::Property, qjs, Class, Ctx,
    Exception, Function, IntoJs, Null, Result, Symbol, Value,
};

use super::file::File;

#[rquickjs::class]
#[derive(Trace, Clone, rquickjs::JsLifetime)]
pub struct FileList {
    files: Vec<File>,
}

#[rquickjs::methods]
impl FileList {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>) -> Result<Self> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    }

    #[qjs(get)]
    pub fn length(&self) -> usize {
        self.files.len()
    }

    pub fn item<'js>(&self, ctx: Ctx<'js>, index: Opt<Value<'js>>) -> Result<Value<'js>> {
        let Some(index_val) = index.0 else {
            return Err(Exception::throw_type(
                &ctx,
                "Failed to execute 'item' on 'FileList': 1 argument required, but only 0 present.",
            ));
        };
        let mut num = 0.0_f64;
        let rc = unsafe { qjs::JS_ToFloat64(ctx.as_raw().as_ptr(), &mut num, index_val.as_raw()) };
        if rc < 0 {
            return Err(rquickjs::Error::Exception);
        }
        let idx = if !num.is_finite() || num == 0.0 {
            0_usize
        } else {
            (num.trunc().rem_euclid(4_294_967_296.0)) as u32 as usize
        };
        match self.files.get(idx) {
            Some(file) => file.clone().into_js(&ctx),
            None => Null.into_js(&ctx),
        }
    }
}

impl FileList {
    pub fn from_files<'js>(ctx: &Ctx<'js>, files: Vec<File>) -> Result<Value<'js>> {
        let instance = Class::instance(
            ctx.clone(),
            Self {
                files: files.clone(),
            },
        )?;
        if let Some(obj) = instance.as_object() {
            for (i, file) in files.into_iter().enumerate() {
                let val = file.into_js(ctx)?;
                obj.prop(i.to_string(), Property::from(val).enumerable())?;
            }
        }
        Ok(instance.into_value())
    }

    pub fn install_prototype<'js>(ctx: &Ctx<'js>) -> Result<()> {
        if let Some(proto) = Class::<Self>::prototype(ctx)? {
            proto.prop(
                PredefinedAtom::SymbolToStringTag,
                Property::from("FileList").configurable(),
            )?;
            let iter_fn: Function<'js> = ctx.eval(
                r#"(function () {
                    let i = 0;
                    const self = this;
                    return {
                        next() {
                            if (i < self.length) {
                                return { value: self.item(i++), done: false };
                            }
                            return { value: undefined, done: true };
                        },
                        [Symbol.iterator]() { return this; }
                    };
                })"#,
            )?;
            proto.prop(
                Symbol::iterator(ctx.clone()),
                Property::from(iter_fn).configurable().writable(),
            )?;
        }
        Ok(())
    }
}
