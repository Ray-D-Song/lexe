// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Global` backed by `wasmz::Global`.

use std::cell::Cell;
use std::rc::Rc;

use rquickjs::{atom::PredefinedAtom, class::Trace, Class, Ctx, IntoJs, Object, Result, Value};

use crate::host_state::{HostState, WrapKind};
use crate::module::ModuleValType;

#[derive(rquickjs::JsLifetime)]
#[rquickjs::class(rename = "Global")]
pub struct WasmGlobal {
    pub(crate) realm_id: u64,
    pub(crate) handle: wasmz::Global,
    pub(crate) value_type: ModuleValType,
    pub(crate) mutable: bool,
    pub(crate) funcref_owner: Cell<Option<u64>>,
}

impl<'js> Trace<'js> for WasmGlobal {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

fn parse_value_type<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    descriptor: &Object<'js>,
) -> Result<ModuleValType> {
    let value_value =
        crate::descriptor::get_required(ctx, host, descriptor, "value", "WebAssembly.Global")?;
    let value = crate::descriptor::to_string(ctx, value_value)?;
    match value.as_str() {
        "i32" => Ok(ModuleValType::I32),
        "i64" => Ok(ModuleValType::I64),
        "f32" => Ok(ModuleValType::F32),
        "f64" => Ok(ModuleValType::F64),
        "v128" => Err(host.throw_type_error(ctx, "v128 globals are not accessible from JS")),
        "anyfunc" | "funcref" => Ok(ModuleValType::FuncRef),
        "externref" => Ok(ModuleValType::ExternRef),
        other => {
            Err(host.throw_type_error(ctx, format!("unsupported global value type '{other}'")))
        },
    }
}

#[rquickjs::methods]
impl WasmGlobal {
    #[qjs(constructor)]
    pub fn new<'js>(
        ctx: Ctx<'js>,
        descriptor: Object<'js>,
        value: rquickjs::prelude::Opt<Value<'js>>,
    ) -> Result<Self> {
        let realm = crate::realm::realm(&ctx)?;
        let host = realm.state.clone();
        let value_type = parse_value_type(&ctx, &host, &descriptor)?;
        let mutable = crate::descriptor::optional_bool(&ctx, &descriptor, "mutable", false)?;

        let mut funcref_owner = None;
        let init = match &value.0 {
            Some(v) if !v.is_undefined() => {
                if value_type == ModuleValType::FuncRef && !(v.is_null() || v.is_undefined()) {
                    if let Some(key) = host.unwrap_func_key(v) {
                        funcref_owner = Some(key.instance_id);
                    }
                }
                crate::value_conv::js_to_val(&ctx, &host, v.clone(), value_type)?
            },
            _ => crate::value_conv::default_val(&ctx, &host, value_type)?,
        };
        let kind = value_type
            .to_wasmz_kind()
            .ok_or_else(|| host.throw_type_error(&ctx, "v128 globals are not supported"))?;
        let handle = wasmz::Global::new(kind, mutable, init)
            .map_err(|err| host.throw_type_error(&ctx, err.to_string()))?;

        Ok(Self {
            realm_id: host.realm_id,
            handle,
            value_type,
            mutable,
            funcref_owner: Cell::new(funcref_owner),
        })
    }

    #[qjs(get)]
    pub fn value<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        let val = self.handle.get();
        crate::value_conv::val_to_js(&ctx, &host, self.funcref_owner.get(), &val)
    }

    #[qjs(set, rename = "value")]
    pub fn set_value<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> Result<()> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        if !self.mutable {
            return Err(host.throw_type_error(
                &ctx,
                "cannot set the value of an immutable WebAssembly.Global",
            ));
        }
        if self.value_type == ModuleValType::FuncRef && !(value.is_null() || value.is_undefined()) {
            if let Some(key) = host.unwrap_func_key(&value) {
                self.funcref_owner.set(Some(key.instance_id));
            }
        }
        let val = crate::value_conv::js_to_val(&ctx, &host, value, self.value_type)?;
        self.handle
            .set(val)
            .map_err(|err| host.throw_type_error(&ctx, err.to_string()))
    }

    #[qjs(rename = "valueOf")]
    pub fn value_of<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.value(ctx)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "WebAssembly.Global"
    }
}

fn require_same_realm(ctx: &Ctx<'_>, realm_id: u64) -> Result<Rc<crate::realm::WasmRealm>> {
    let realm = crate::realm::realm(ctx)?;
    if realm.state.realm_id != realm_id {
        return Err(realm
            .state
            .throw_link_error(ctx, "Global belongs to a different realm"));
    }
    Ok(realm)
}

pub fn wrap_global<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    global: wasmz::Global,
    value_type: ModuleValType,
    mutable: bool,
) -> Result<Class<'js, WasmGlobal>> {
    let bits = global.identity();
    if let Some(existing) = host.cached_wrapper(ctx, WrapKind::Global, bits) {
        if let Ok(class) = Class::<WasmGlobal>::from_value(&existing) {
            return Ok(class);
        }
    }
    let instance = Class::instance(
        ctx.clone(),
        WasmGlobal {
            realm_id: host.realm_id,
            handle: global,
            value_type,
            mutable,
            funcref_owner: Cell::new(None),
        },
    )?;
    host.cache_wrapper(WrapKind::Global, bits, instance.clone().into_js(ctx)?);
    Ok(instance)
}
