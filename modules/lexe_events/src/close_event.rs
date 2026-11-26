// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{atom::PredefinedAtom, prelude::Opt, Coerced, Ctx, Exception, Result, Value};

use lexe_utils::object::ObjectExt;

#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct CloseEvent {
    event_type: String,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    was_clean: bool,
    code: u16,
    reason: String,
}

#[rquickjs::methods]
impl CloseEvent {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, event_type: Coerced<String>, options: Opt<Value<'_>>) -> Result<Self> {
        let mut bubbles = false;
        let mut cancelable = false;
        let mut composed = false;
        let mut was_clean = false;
        let mut code = 0_u16;
        let mut reason = String::new();

        if let Some(options) = options.0 {
            if !options.is_undefined() && !options.is_null() {
                if !options.is_object() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "Failed to construct 'CloseEvent': parameter 2 is not of type 'Object'.",
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
                if let Some(opt) = options.get_optional("wasClean")? {
                    was_clean = opt;
                }
                if let Some(opt) = options.get_optional::<_, f64>("code")? {
                    code = to_uint16(opt);
                }
                if let Some(opt) = options.get_optional::<_, Coerced<String>>("reason")? {
                    reason = opt.0;
                }
            }
        }

        Ok(Self {
            event_type: event_type.0,
            bubbles,
            cancelable,
            composed,
            was_clean,
            code,
            reason,
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

    #[qjs(get, rename = "wasClean")]
    pub fn was_clean(&self) -> bool {
        self.was_clean
    }

    #[qjs(get)]
    pub fn code(&self) -> u16 {
        self.code
    }

    #[qjs(get)]
    pub fn reason(&self) -> String {
        self.reason.clone()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(CloseEvent)
    }
}

fn to_uint16(n: f64) -> u16 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    (n.trunc().rem_euclid(65_536.0)) as u16
}
