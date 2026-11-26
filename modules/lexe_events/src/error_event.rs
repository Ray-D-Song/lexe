// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{
    atom::PredefinedAtom,
    class::{Trace, Tracer},
    prelude::Opt,
    Coerced, Ctx, Exception, JsLifetime, Result, Undefined, Value,
};

use lexe_utils::object::ObjectExt;

#[rquickjs::class]
#[derive(JsLifetime)]
pub struct ErrorEvent<'js> {
    event_type: String,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    message: String,
    filename: String,
    lineno: u32,
    colno: u32,
    error: Value<'js>,
}

impl<'js> Trace<'js> for ErrorEvent<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        tracer.mark(&self.error);
    }
}

#[rquickjs::methods]
impl<'js> ErrorEvent<'js> {
    #[qjs(constructor)]
    pub fn new(
        ctx: Ctx<'js>,
        event_type: Coerced<String>,
        options: Opt<Value<'js>>,
    ) -> Result<Self> {
        let mut bubbles = false;
        let mut cancelable = false;
        let mut composed = false;
        let mut message = String::new();
        let mut filename = String::new();
        let mut lineno = 0_u32;
        let mut colno = 0_u32;
        let mut error = Undefined.into_value(ctx.clone());

        if let Some(options) = options.0 {
            if !options.is_undefined() && !options.is_null() {
                if !options.is_object() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "Failed to construct 'ErrorEvent': parameter 2 is not of type 'Object'.",
                    ));
                }
                if let Some(opt) = options.get_optional("bubbles")? {
                    bubbles = opt;
                }
                if let Some(opt) = options.get_optional("cancelable")? {
                    cancelable = opt;
                }
                if let Some(opt) = options.get_optional("composed")? {
                    composed = opt;
                }
                if let Some(opt) = options.get_optional::<_, Coerced<String>>("message")? {
                    message = opt.0;
                }
                if let Some(opt) = options.get_optional::<_, Coerced<String>>("filename")? {
                    filename = opt.0;
                }
                if let Some(opt) = options.get_optional::<_, f64>("lineno")? {
                    lineno = to_uint32(opt);
                }
                if let Some(opt) = options.get_optional::<_, f64>("colno")? {
                    colno = to_uint32(opt);
                }
                if let Some(opt) = options.get_optional::<_, Value<'js>>("error")? {
                    error = opt;
                }
            }
        }

        Ok(Self {
            event_type: event_type.0,
            bubbles,
            cancelable,
            composed,
            message,
            filename,
            lineno,
            colno,
            error,
        })
    }

    #[qjs(get)]
    pub fn bubbles(&self) -> bool {
        self.bubbles
    }

    #[qjs(get)]
    pub fn cancelable(&self) -> bool {
        self.cancelable
    }

    #[qjs(get)]
    pub fn composed(&self) -> bool {
        self.composed
    }

    #[qjs(get, rename = "type")]
    pub fn event_type(&self) -> String {
        self.event_type.clone()
    }

    #[qjs(get)]
    pub fn message(&self) -> String {
        self.message.clone()
    }

    #[qjs(get)]
    pub fn filename(&self) -> String {
        self.filename.clone()
    }

    #[qjs(get)]
    pub fn lineno(&self) -> u32 {
        self.lineno
    }

    #[qjs(get)]
    pub fn colno(&self) -> u32 {
        self.colno
    }

    #[qjs(get)]
    pub fn error(&self) -> Value<'js> {
        self.error.clone()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(ErrorEvent)
    }
}

fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    (n.trunc().rem_euclid(4_294_967_296.0)) as u32
}
