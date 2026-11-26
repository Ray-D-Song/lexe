// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{atom::PredefinedAtom, prelude::Opt, Coerced, Ctx, IntoJs, Null, Result, Value};

use lexe_utils::object::ObjectExt;

#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct CustomEvent<'js> {
    event_type: String,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    detail: Option<Value<'js>>,
}

#[rquickjs::methods]
impl<'js> CustomEvent<'js> {
    #[qjs(constructor)]
    pub fn new(event_type: Coerced<String>, options: Opt<Value<'js>>) -> Result<Self> {
        let mut bubbles = false;
        let mut cancelable = false;
        let mut composed = false;
        let mut detail = None;
        if let Some(options) = options.0 {
            if let Some(opt) = options.get_optional("bubbles")? {
                bubbles = opt;
            }
            if let Some(opt) = options.get_optional("cancelable")? {
                cancelable = opt;
            }
            if let Some(opt) = options.get_optional("composed")? {
                composed = opt;
            }
            if let Some(opt) = options.get_optional("detail")? {
                detail = opt;
            }
        }
        Ok(Self {
            event_type: event_type.0,
            bubbles,
            cancelable,
            composed,
            detail,
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

    #[qjs(get)]
    pub fn detail(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        if let Some(detail) = &self.detail {
            return Ok(detail.clone());
        }
        Null.into_js(&ctx)
    }

    #[qjs(get, rename = "type")]
    pub fn event_type(&self) -> String {
        self.event_type.clone()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(CustomEvent)
    }
}
