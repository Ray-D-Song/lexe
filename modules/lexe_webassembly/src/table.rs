// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Table` backed by `wasmz::Table`.

use std::rc::Rc;

use rquickjs::{atom::PredefinedAtom, class::Trace, Class, Ctx, IntoJs, Object, Result, Value};

use crate::host_state::{HostState, WrapKind};
use crate::module::ModuleValType;

#[derive(rquickjs::JsLifetime)]
#[rquickjs::class(rename = "Table")]
pub struct WasmTable {
    pub(crate) realm_id: u64,
    pub(crate) handle: wasmz::Table,
    pub(crate) element: ModuleValType,
}

impl<'js> Trace<'js> for WasmTable {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

fn parse_element_type<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    descriptor: &Object<'js>,
) -> Result<ModuleValType> {
    let element_value =
        crate::descriptor::get_required(ctx, host, descriptor, "element", "WebAssembly.Table")?;
    let element = crate::descriptor::to_string(ctx, element_value)?;
    match element.as_str() {
        "anyfunc" | "funcref" => Ok(ModuleValType::FuncRef),
        "externref" => Ok(ModuleValType::ExternRef),
        other => {
            Err(host.throw_type_error(ctx, format!("unsupported table element type '{other}'")))
        },
    }
}

enum TableInitTarget {
    Null,
    ExternRef(usize),
    InstanceFunc(Rc<wasmz::Instance>, u32),
}

fn resolve_table_value<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    element: ModuleValType,
    value: Option<Value<'js>>,
) -> Result<TableInitTarget> {
    match (element, value) {
        (ModuleValType::FuncRef, None) => Ok(TableInitTarget::Null),
        (ModuleValType::FuncRef, Some(v)) if v.is_undefined() || v.is_null() => {
            Ok(TableInitTarget::Null)
        },
        (ModuleValType::FuncRef, Some(v)) => {
            let key = host.unwrap_func_key(&v).ok_or_else(|| {
                host.throw_type_error(
                    ctx,
                    "funcref values must be null or a WebAssembly-exported function",
                )
            })?;
            let inst = host.instance_by_id(key.instance_id).ok_or_else(|| {
                host.throw_link_error(ctx, "Table funcref owner instance is no longer alive")
            })?;
            Ok(TableInitTarget::InstanceFunc(inst, key.func_idx))
        },
        (ModuleValType::ExternRef, None) => {
            let id = host.intern_externref(&Value::new_undefined(ctx.clone()));
            Ok(TableInitTarget::ExternRef(id))
        },
        (ModuleValType::ExternRef, Some(v)) => {
            if v.is_null() {
                Ok(TableInitTarget::ExternRef(0))
            } else {
                Ok(TableInitTarget::ExternRef(host.intern_externref(&v)))
            }
        },
        _ => unreachable!("table element is always FuncRef or ExternRef"),
    }
}

#[rquickjs::methods]
impl WasmTable {
    #[qjs(constructor)]
    pub fn new<'js>(
        ctx: Ctx<'js>,
        descriptor: Object<'js>,
        initial_fill_value: rquickjs::prelude::Opt<Value<'js>>,
    ) -> Result<Self> {
        let realm = crate::realm::realm(&ctx)?;
        let host = realm.state.clone();
        let element = parse_element_type(&ctx, &host, &descriptor)?;
        let initial_value = crate::descriptor::get_required(
            &ctx,
            &host,
            &descriptor,
            "initial",
            "WebAssembly.Table",
        )?;
        let initial =
            crate::descriptor::to_u32_enforce_range(&ctx, &host, initial_value, "initial")?;
        let maximum =
            crate::descriptor::optional_u32_enforce_range(&ctx, &host, &descriptor, "maximum")?;
        if let Some(max) = maximum {
            if max < initial {
                return Err(
                    host.throw_range_error(&ctx, "maximum table size is smaller than initial size")
                );
            }
        }

        let elem_kind = match element {
            ModuleValType::ExternRef => wasmz::ValKind::ExternRef,
            _ => wasmz::ValKind::FuncRef,
        };
        let target = resolve_table_value(&ctx, &host, element, initial_fill_value.0)?;
        let handle = match target {
            TableInitTarget::Null => wasmz::Table::new(elem_kind, initial, maximum, 0, None),
            TableInitTarget::ExternRef(raw) => {
                wasmz::Table::new(elem_kind, initial, maximum, raw, None)
            },
            TableInitTarget::InstanceFunc(ref inst, func_idx) => {
                wasmz::Table::new(elem_kind, initial, maximum, 0, Some((inst, func_idx)))
            },
        }
        .map_err(|err| host.throw_range_error(&ctx, err.to_string()))?;

        Ok(Self {
            realm_id: host.realm_id,
            handle,
            element,
        })
    }

    #[qjs(get)]
    pub fn length(&self, ctx: Ctx<'_>) -> Result<u32> {
        let _realm = require_same_realm(&ctx, self.realm_id)?;
        Ok(self.handle.len())
    }

    pub fn get<'js>(&self, ctx: Ctx<'js>, index: u32) -> Result<Value<'js>> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        let entry = self
            .handle
            .get_entry(index)
            .ok_or_else(|| host.throw_range_error(&ctx, "table index out of bounds"))?;
        match entry {
            wasmz::TableEntry::Null => Ok(Value::new_null(ctx)),
            wasmz::TableEntry::ExternRef(id) => host.externref_object(&ctx, id).ok_or_else(|| {
                host.throw_runtime_error(&ctx, "externref object identity was lost")
            }),
            wasmz::TableEntry::Func {
                owner_id,
                func_idx,
                raw,
            } => {
                let key = crate::host_state::FuncKey {
                    instance_id: owner_id,
                    func_idx,
                };
                if let Some(existing) = host.cached_func_wrapper(&ctx, key) {
                    return Ok(existing);
                }
                if owner_id == 0 && raw != 0 {
                    // Imported JS callback slot stored without instance ownership
                    if let Some(existing) = host.cached_func_wrapper(
                        &ctx,
                        crate::host_state::FuncKey {
                            instance_id: 0,
                            func_idx: raw as u32,
                        },
                    ) {
                        return Ok(existing);
                    }
                }
                if let Some(inst) = host.instance_by_id(owner_id) {
                    if let Some(sig) = crate::func_wrapper::sig_from_instance(&inst, func_idx) {
                        let func =
                            crate::func_wrapper::wrap_func(&ctx, &host, inst, func_idx, sig, None)?;
                        return Ok(func.into_value());
                    }
                }
                Ok(Value::new_null(ctx))
            },
        }
    }

    pub fn set<'js>(
        &self,
        ctx: Ctx<'js>,
        index: u32,
        value: rquickjs::prelude::Opt<Value<'js>>,
    ) -> Result<()> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        let target = resolve_table_value(&ctx, &host, self.element, value.0)?;
        let ok = match target {
            TableInitTarget::Null => self.handle.set_null(index),
            TableInitTarget::ExternRef(raw) => self.handle.set_externref(index, raw),
            TableInitTarget::InstanceFunc(ref inst, func_idx) => {
                self.handle.set_instance_func(index, inst, func_idx)
            },
        };
        if !ok {
            return Err(host.throw_range_error(&ctx, "table index out of bounds"));
        }
        Ok(())
    }

    pub fn grow<'js>(
        &self,
        ctx: Ctx<'js>,
        delta: u32,
        value: rquickjs::prelude::Opt<Value<'js>>,
    ) -> Result<u32> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        let target = resolve_table_value(&ctx, &host, self.element, value.0)?;
        let prev = match target {
            TableInitTarget::Null => self.handle.grow_instance_func(delta, None, 0),
            TableInitTarget::ExternRef(raw) => self.handle.grow(delta, raw),
            TableInitTarget::InstanceFunc(ref inst, func_idx) => {
                self.handle.grow_instance_func(delta, Some(inst), func_idx)
            },
        };
        prev.ok_or_else(|| host.throw_range_error(&ctx, "failed to grow WebAssembly.Table"))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "WebAssembly.Table"
    }
}

fn require_same_realm(ctx: &Ctx<'_>, realm_id: u64) -> Result<Rc<crate::realm::WasmRealm>> {
    let realm = crate::realm::realm(ctx)?;
    if realm.state.realm_id != realm_id {
        return Err(realm
            .state
            .throw_link_error(ctx, "Table belongs to a different realm"));
    }
    Ok(realm)
}

pub fn wrap_table<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    table: wasmz::Table,
    element: ModuleValType,
) -> Result<Class<'js, WasmTable>> {
    let bits = table.identity();
    if let Some(existing) = host.cached_wrapper(ctx, WrapKind::Table, bits) {
        if let Ok(class) = Class::<WasmTable>::from_value(&existing) {
            return Ok(class);
        }
    }
    let instance = Class::instance(
        ctx.clone(),
        WasmTable {
            realm_id: host.realm_id,
            handle: table,
            element,
        },
    )?;
    host.cache_wrapper(WrapKind::Table, bits, instance.clone().into_js(ctx)?);
    Ok(instance)
}
