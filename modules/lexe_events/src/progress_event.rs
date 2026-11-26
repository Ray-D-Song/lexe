// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{atom::PredefinedAtom, prelude::Opt, Coerced, Ctx, Exception, Result, Value};

use lexe_utils::object::ObjectExt;

#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct ProgressEvent {
    event_type: String,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    length_computable: bool,
    loaded: f64,
    total: f64,
}

#[rquickjs::methods]
impl ProgressEvent {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, event_type: Coerced<String>, options: Opt<Value<'_>>) -> Result<Self> {
        let mut bubbles = false;
        let mut cancelable = false;
        let mut composed = false;
        let mut length_computable = false;
        let mut loaded = 0.0_f64;
        let mut total = 0.0_f64;

        if let Some(options) = options.0 {
            if !options.is_undefined() && !options.is_null() {
                if !options.is_object() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "Failed to construct 'ProgressEvent': parameter 2 is not of type 'Object'.",
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
                if let Some(opt) = options.get_optional("lengthComputable")? {
                    length_computable = opt;
                }
                if let Some(opt) = options.get_optional::<_, f64>("loaded")? {
                    if !opt.is_finite() {
                        return Err(Exception::throw_type(
                            &ctx,
                            "Failed to construct 'ProgressEvent': Failed to read the 'loaded' property from 'ProgressEventInit': The provided double value is non-finite.",
                        ));
                    }
                    loaded = opt;
                }
                if let Some(opt) = options.get_optional::<_, f64>("total")? {
                    if !opt.is_finite() {
                        return Err(Exception::throw_type(
                            &ctx,
                            "Failed to construct 'ProgressEvent': Failed to read the 'total' property from 'ProgressEventInit': The provided double value is non-finite.",
                        ));
                    }
                    total = opt;
                }
            }
        }

        Ok(Self {
            event_type: event_type.0,
            bubbles,
            cancelable,
            composed,
            length_computable,
            loaded,
            total,
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

    #[qjs(get, rename = "lengthComputable")]
    pub fn length_computable(&self) -> bool {
        self.length_computable
    }

    #[qjs(get)]
    pub fn loaded(&self) -> f64 {
        self.loaded
    }

    #[qjs(get)]
    pub fn total(&self) -> f64 {
        self.total
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "ProgressEvent"
    }
}

impl ProgressEvent {
    pub fn with_values(
        event_type: impl Into<String>,
        length_computable: bool,
        loaded: f64,
        total: f64,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            bubbles: false,
            cancelable: false,
            composed: false,
            length_computable,
            loaded,
            total,
        }
    }
}
