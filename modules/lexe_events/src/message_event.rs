// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::object::ObjectExt;
use rquickjs::{
    atom::PredefinedAtom,
    class::{Trace, Tracer},
    prelude::Opt,
    Array, Coerced, Ctx, Exception, Function, IntoJs, JsLifetime, Null, Object, Result, Value,
};

#[rquickjs::class]
#[derive(JsLifetime)]
pub struct MessageEvent<'js> {
    event_type: String,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    data: Value<'js>,
    origin: String,
    last_event_id: String,
    source: Value<'js>,
    ports: Value<'js>,
}

impl<'js> Trace<'js> for MessageEvent<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        tracer.mark(&self.data);
        tracer.mark(&self.source);
        tracer.mark(&self.ports);
    }
}

fn freeze_empty_array<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let arr = Array::new(ctx.clone())?;
    let freeze: Function = ctx.eval("Object.freeze")?;
    freeze.call((arr,))
}

#[rquickjs::methods]
impl<'js> MessageEvent<'js> {
    #[qjs(constructor)]
    pub fn new(
        ctx: Ctx<'js>,
        event_type: Coerced<String>,
        options: Opt<Value<'js>>,
    ) -> Result<Self> {
        let mut bubbles = false;
        let mut cancelable = false;
        let mut composed = false;
        let mut data = Null.into_value(ctx.clone());
        let mut origin = String::new();
        let mut last_event_id = String::new();
        let mut source = Null.into_value(ctx.clone());
        let mut ports = freeze_empty_array(&ctx)?;

        if let Some(options) = options.0 {
            if !options.is_undefined() && !options.is_null() {
                if !options.is_object() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "Failed to construct 'MessageEvent': parameter 2 is not of type 'Object'.",
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
                if let Some(opt) = options.get_optional::<_, Value<'js>>("data")? {
                    data = opt;
                }
                if let Some(opt) = options.get_optional::<_, Coerced<String>>("origin")? {
                    origin = opt.0;
                }
                if let Some(opt) = options.get_optional::<_, Coerced<String>>("lastEventId")? {
                    last_event_id = opt.0;
                }
                if let Some(opt) = options.get_optional::<_, Value<'js>>("source")? {
                    source = opt;
                }
                if let Some(opt) = options.get_optional::<_, Value<'js>>("ports")? {
                    if let Some(arr) = opt.as_array() {
                        let copy = Array::new(ctx.clone())?;
                        for (i, item) in arr.iter::<Value<'js>>().enumerate() {
                            copy.set(i, item?)?;
                        }
                        let freeze: Function = ctx.eval("Object.freeze")?;
                        ports = freeze.call((copy,))?;
                    } else if !opt.is_undefined() {
                        return Err(Exception::throw_type(
                            &ctx,
                            "Failed to construct 'MessageEvent': 'ports' must be an iterable.",
                        ));
                    }
                }
            }
        }

        Ok(Self {
            event_type: event_type.0,
            bubbles,
            cancelable,
            composed,
            data,
            origin,
            last_event_id,
            source,
            ports,
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
    pub fn data(&self) -> Value<'js> {
        self.data.clone()
    }

    #[qjs(get)]
    pub fn origin(&self) -> String {
        self.origin.clone()
    }

    #[qjs(get, rename = "lastEventId")]
    pub fn last_event_id(&self) -> String {
        self.last_event_id.clone()
    }

    #[qjs(get)]
    pub fn source(&self) -> Value<'js> {
        self.source.clone()
    }

    #[qjs(get)]
    pub fn ports(&self) -> Value<'js> {
        self.ports.clone()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "MessageEvent"
    }
}

impl<'js> MessageEvent<'js> {
    pub fn with_data(
        ctx: &Ctx<'js>,
        event_type: impl Into<String>,
        data: Value<'js>,
    ) -> Result<Self> {
        let obj = Object::new(ctx.clone())?;
        obj.set("data", data)?;
        Self::new(
            ctx.clone(),
            Coerced(event_type.into()),
            Opt(Some(obj.into_js(ctx)?)),
        )
    }
}
