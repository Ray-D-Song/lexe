// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Instance`: binary-ordered import resolution, `wasmz`
//! linking & instantiation (including running start section), null-prototype
//! non-extensible `exports`, and JS <-> Wasm boundary call dispatch.

use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use rquickjs::function::{IntoArgs, Rest};
use rquickjs::object::Property;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, Array, Class, Ctx, JsLifetime, Object, Result, Value,
};

use crate::host_state::{FuncKey, HostState, WrapKind};
use crate::module::{ExternKind, FuncSigMeta, ImportDescriptor, ModuleValType, WasmModule};
use crate::realm::WasmRealm;

#[rquickjs::class(rename = "Instance")]
pub struct WasmInstance<'js> {
    pub(crate) realm_id: u64,
    pub(crate) handle: Rc<wasmz::Instance>,
    pub(crate) exports: Object<'js>,
}

unsafe impl<'js> JsLifetime<'js> for WasmInstance<'js> {
    type Changed<'to> = WasmInstance<'to>;
}

impl<'js> Trace<'js> for WasmInstance<'js> {
    fn trace<'a>(&self, tracer: rquickjs::class::Tracer<'a, 'js>) {
        tracer.mark(self.exports.as_value());
    }
}

#[rquickjs::methods]
impl<'js> WasmInstance<'js> {
    #[qjs(constructor)]
    pub fn new(
        ctx: Ctx<'js>,
        module: Value<'js>,
        imports: rquickjs::prelude::Opt<Object<'js>>,
    ) -> Result<Self> {
        let realm = crate::realm::realm(&ctx)?;
        let host = realm.state.clone();
        let module_class = Class::<WasmModule>::from_value(&module)
            .map_err(|_| host.throw_type_error(&ctx, "expected a WebAssembly.Module"))?;
        let module_ref = module_class.borrow();
        instantiate_module(&ctx, &realm, &module_ref, imports.0)
    }

    #[qjs(get)]
    pub fn exports(&self) -> Object<'js> {
        self.exports.clone()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "WebAssembly.Instance"
    }
}

pub fn instantiate_module<'js>(
    ctx: &Ctx<'js>,
    realm: &Rc<WasmRealm>,
    module: &WasmModule,
    imports: Option<Object<'js>>,
) -> Result<WasmInstance<'js>> {
    let host = realm.state.clone();

    if !module.metadata.imports.is_empty() && imports.is_none() {
        return Err(host.throw_type_error(
            ctx,
            "module requires an imports object but none was provided",
        ));
    }

    let mut linker =
        wasmz::Linker::new().map_err(|err| host.throw_runtime_error(ctx, err.to_string()))?;
    let mut added_wasi_hosts: HashSet<u64> = HashSet::new();
    let mut imported_func_aliases: HashMap<u32, FuncKey> = HashMap::new();
    let mut imported_func_idx: u32 = 0;

    if let Some(imports_obj) = imports.as_ref() {
        let mut by_key_namespaces: HashMap<String, Option<u64>> = HashMap::new();
        for import in &module.metadata.imports {
            let module_name = import.module.as_str();
            let field_name = import.name.as_str();

            let namespace_val: Value = imports_obj.get(module_name)?;
            let namespace = namespace_val.as_object().ok_or_else(|| {
                host.throw_link_error(
                    ctx,
                    format!("import namespace '{module_name}' is not an object"),
                )
            })?;

            let wasi_id = if let Some(cached) = by_key_namespaces.get(module_name) {
                *cached
            } else {
                let id = crate::wasi::extract_wasi_host_id(namespace)?;
                by_key_namespaces.insert(module_name.to_string(), id);
                id
            };

            let value: Value = namespace.get(field_name)?;

            if import.kind == ExternKind::Function {
                if let Some(wid) = wasi_id {
                    if !added_wasi_hosts.contains(&wid) {
                        if let Some(wasi_rc) = host.wasi_state(wid) {
                            let mut state = wasi_rc.borrow_mut();
                            linker
                                .add_wasi(&mut state.host)
                                .map_err(|err| host.throw_link_error(ctx, err.to_string()))?;
                            added_wasi_hosts.insert(wid);
                        }
                    }
                    // If caller left the WASI stub function intact (didn't override it with a custom JS function), keep the native WASI host implementation.
                    if host.is_wasi_stub(&value) {
                        imported_func_idx += 1;
                        continue;
                    }
                }
            }

            if let Some(alias) = define_import_on_linker(
                ctx,
                &host,
                &mut linker,
                import,
                &value,
                module_name,
                field_name,
            )? {
                imported_func_aliases.insert(imported_func_idx, alias);
            }
            if import.kind == ExternKind::Function {
                imported_func_idx += 1;
            }
        }
    }

    let instance = Rc::new(
        wasmz::Instance::new(&realm.store, &module.inner, Some(&linker))
            .map_err(|err| crate::errors::throw_wasmz_error(ctx, &host, err))?,
    );
    host.register_instance(&instance);

    crate::memory::sync_all_js_to_wasm(ctx, &host)?;
    let start_res = instance.run_start();
    let sync_res = crate::memory::sync_all_wasm_to_js(ctx, &host);
    if let Err(err) = start_res {
        if sync_res.is_err() {
            let _ = ctx.catch();
        }
        return Err(crate::errors::throw_wasmz_error(ctx, &host, err));
    }
    sync_res?;

    let exports_obj = build_exports_object(ctx, &host, module, &instance, &imported_func_aliases)?;
    Ok(WasmInstance {
        realm_id: host.realm_id,
        handle: instance,
        exports: exports_obj,
    })
}

fn define_import_on_linker<'js>(
    ctx: &Ctx<'js>,
    host: &Rc<HostState>,
    linker: &mut wasmz::Linker,
    import: &ImportDescriptor,
    value: &Value<'js>,
    module_name: &str,
    field_name: &str,
) -> Result<Option<FuncKey>> {
    match import.kind {
        ExternKind::Function => {
            let sig = import
                .func_sig
                .as_ref()
                .ok_or_else(|| host.throw_link_error(ctx, "missing function signature"))?;

            if let Some(key) = host.unwrap_func_key(value) {
                if let Some(entry) = host.func_entry(key) {
                    if entry.sig != *sig {
                        return Err(host.throw_link_error(
                            ctx,
                            format!("import '{module_name}.{field_name}' signature mismatch"),
                        ));
                    }
                    let param_kinds: Vec<wasmz::ValKind> = sig
                        .params
                        .iter()
                        .map(|t| t.to_wasmz_kind())
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| {
                            host.throw_link_error(ctx, "v128 in JS import signature unsupported")
                        })?;
                    let result_kinds: Vec<wasmz::ValKind> = sig
                        .results
                        .iter()
                        .map(|t| t.to_wasmz_kind())
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| {
                            host.throw_link_error(ctx, "v128 in JS import signature unsupported")
                        })?;
                    let target_inst = entry.instance.clone();
                    let target_idx = entry.func_idx;
                    let r_kinds = result_kinds.clone();
                    linker
                        .define_trappable_func(
                            module_name,
                            field_name,
                            &param_kinds,
                            &result_kinds,
                            Box::new(move |_caller, params, results| {
                                let inst = target_inst
                                    .upgrade()
                                    .ok_or_else(|| "imported instance dropped".to_string())?;
                                let out = inst
                                    .call_index(target_idx, params, &r_kinds)
                                    .map_err(|e| e.to_string())?;
                                results.copy_from_slice(&out);
                                Ok(())
                            }),
                        )
                        .map_err(|err| host.throw_link_error(ctx, err.to_string()))?;
                    return Ok(Some(key));
                }
            }

            let js_func = value.as_function().cloned().ok_or_else(|| {
                host.throw_link_error(
                    ctx,
                    format!("import '{module_name}.{field_name}' is not a function"),
                )
            })?;

            let callback_id = host.register_callback(js_func);
            let host_weak = Rc::downgrade(host);
            let sig_clone = sig.clone();

            let param_kinds: Vec<wasmz::ValKind> = sig
                .params
                .iter()
                .map(|t| t.to_wasmz_kind())
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    host.throw_link_error(ctx, "v128 parameters in host import unsupported")
                })?;
            let result_kinds: Vec<wasmz::ValKind> = sig
                .results
                .iter()
                .map(|t| t.to_wasmz_kind())
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    host.throw_link_error(ctx, "v128 results in host import unsupported")
                })?;

            linker
                .define_trappable_func(
                    module_name,
                    field_name,
                    &param_kinds,
                    &result_kinds,
                    Box::new(move |_caller, inputs, outputs| {
                        let host = host_weak
                            .upgrade()
                            .ok_or_else(|| "WebAssembly realm was dropped".to_string())?;
                        call_js_import(&host, callback_id, &sig_clone, inputs, outputs)
                    }),
                )
                .map_err(|err| host.throw_link_error(ctx, err.to_string()))?;
            Ok(None)
        },
        ExternKind::Memory => {
            let class = Class::<crate::memory::WasmMemory>::from_value(value).map_err(|_| {
                host.throw_link_error(
                    ctx,
                    format!("import '{module_name}.{field_name}' expected a WebAssembly.Memory"),
                )
            })?;
            let borrow = class.borrow();
            if borrow.realm_id != host.realm_id {
                return Err(host.throw_link_error(ctx, "cross-realm Memory import is not allowed"));
            }
            let mem = borrow.handle.borrow().clone();
            let bits = mem.identity();
            drop(borrow);
            host.cache_wrapper(WrapKind::Memory, bits, value.clone());
            linker
                .define_memory(module_name, field_name, &mem)
                .map_err(|err| host.throw_link_error(ctx, err.to_string()))?;
            Ok(None)
        },
        ExternKind::Table => {
            let class = Class::<crate::table::WasmTable>::from_value(value).map_err(|_| {
                host.throw_link_error(
                    ctx,
                    format!("import '{module_name}.{field_name}' expected a WebAssembly.Table"),
                )
            })?;
            let borrow = class.borrow();
            if borrow.realm_id != host.realm_id {
                return Err(host.throw_link_error(ctx, "cross-realm Table import is not allowed"));
            }
            let tbl = borrow.handle.clone();
            let bits = tbl.identity();
            drop(borrow);
            host.cache_wrapper(WrapKind::Table, bits, value.clone());
            linker
                .define_table(module_name, field_name, &tbl)
                .map_err(|err| host.throw_link_error(ctx, err.to_string()))?;
            Ok(None)
        },
        ExternKind::Global => {
            let class = Class::<crate::global::WasmGlobal>::from_value(value).map_err(|_| {
                host.throw_link_error(
                    ctx,
                    format!("import '{module_name}.{field_name}' expected a WebAssembly.Global"),
                )
            })?;
            let borrow = class.borrow();
            if borrow.realm_id != host.realm_id {
                return Err(host.throw_link_error(ctx, "cross-realm Global import is not allowed"));
            }
            let glob = borrow.handle.clone();
            let bits = glob.identity();
            drop(borrow);
            host.cache_wrapper(WrapKind::Global, bits, value.clone());
            linker
                .define_global(module_name, field_name, &glob)
                .map_err(|err| host.throw_link_error(ctx, err.to_string()))?;
            Ok(None)
        },
    }
}

fn build_exports_object<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    module: &WasmModule,
    instance: &Rc<wasmz::Instance>,
    imported_func_aliases: &HashMap<u32, FuncKey>,
) -> Result<Object<'js>> {
    let exports = Object::new(ctx.clone())?;
    exports.set_prototype(None)?;

    for export in &module.metadata.exports {
        let name = export.name.as_str();
        let value: Value = match export.kind {
            ExternKind::Function => {
                let sig = export
                    .func_sig
                    .clone()
                    .or_else(|| crate::func_wrapper::sig_from_instance(instance, export.index))
                    .ok_or_else(|| {
                        host.throw_runtime_error(ctx, "missing exported function sig")
                    })?;
                let alias = imported_func_aliases.get(&export.index).copied();
                crate::func_wrapper::wrap_func(
                    ctx,
                    host,
                    instance.clone(),
                    export.index,
                    sig,
                    alias,
                )?
                .into_value()
            },
            ExternKind::Memory => {
                let mem = instance
                    .export_memory(name)
                    .ok_or_else(|| host.throw_runtime_error(ctx, "missing exported memory"))?;
                crate::memory::wrap_memory(ctx, host, mem)?.into_value()
            },
            ExternKind::Table => {
                let tbl = instance
                    .export_table(name)
                    .ok_or_else(|| host.throw_runtime_error(ctx, "missing exported table"))?;
                let element = export.table_elem.unwrap_or(ModuleValType::FuncRef);
                crate::table::wrap_table(ctx, host, tbl, element)?.into_value()
            },
            ExternKind::Global => {
                let glob = instance
                    .export_global(name)
                    .ok_or_else(|| host.throw_runtime_error(ctx, "missing exported global"))?;
                let (vt, mutable) = export
                    .global_type
                    .unwrap_or((ModuleValType::I32, glob.is_mutable()));
                crate::global::wrap_global(ctx, host, glob, vt, mutable)?.into_value()
            },
        };
        exports.prop(name, Property::from(value).enumerable())?;
    }

    unsafe { crate::store_access::prevent_extensions(ctx, &exports) }.map_err(|_| {
        host.throw_runtime_error(ctx, "failed to make Instance.exports non-extensible")
    })?;
    Ok(exports)
}

fn call_js_import(
    host: &HostState,
    callback_id: u32,
    sig: &FuncSigMeta,
    inputs: &[wasmz::Val],
    outputs: &mut [wasmz::Val],
) -> std::result::Result<(), String> {
    let ctx = unsafe { host.ctx() };

    if let Err(err) = crate::memory::sync_all_wasm_to_js(&ctx, host) {
        return Err(crate::errors::capture_js_error_for_wasmz(&ctx, host, err));
    }

    let outcome = run_js_import_callback(&ctx, host, callback_id, sig, inputs, outputs);
    match outcome {
        Ok(()) => crate::memory::sync_all_js_to_wasm(&ctx, host)
            .map_err(|err| crate::errors::capture_js_error_for_wasmz(&ctx, host, err)),
        Err(err) => {
            let msg = crate::errors::capture_js_error_for_wasmz(&ctx, host, err);
            let _ = crate::memory::sync_all_js_to_wasm(&ctx, host);
            Err(msg)
        },
    }
}

fn run_js_import_callback<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    callback_id: u32,
    sig: &FuncSigMeta,
    inputs: &[wasmz::Val],
    outputs: &mut [wasmz::Val],
) -> Result<()> {
    let js_func = host
        .callback(ctx, callback_id)
        .ok_or_else(|| host.throw_runtime_error(ctx, "missing import callback registration"))?;

    let mut js_args = Vec::with_capacity(inputs.len());
    for v in inputs {
        js_args.push(crate::value_conv::val_to_js(ctx, host, None, v)?);
    }
    let result: Value<'js> = (Rest(js_args),).apply(&js_func)?;

    if outputs.is_empty() {
        Ok(())
    } else if outputs.len() == 1 {
        let ty = sig.results[0];
        outputs[0] = crate::value_conv::js_to_val(ctx, host, result, ty)?;
        Ok(())
    } else {
        let array = Array::from_value(result).map_err(|_| {
            host.throw_type_error(ctx, "a multi-result import callback must return an array")
        })?;
        if array.len() < outputs.len() {
            return Err(
                host.throw_type_error(ctx, "import callback returned too few result values")
            );
        }
        for (i, slot) in outputs.iter_mut().enumerate() {
            let ty = sig.results[i];
            let element: Value<'js> = array.get(i)?;
            *slot = crate::value_conv::js_to_val(ctx, host, element, ty)?;
        }
        Ok(())
    }
}

pub fn call_exported_func<'js>(
    ctx: &Ctx<'js>,
    realm_id: u64,
    instance: &wasmz::Instance,
    func_idx: u32,
    sig: &FuncSigMeta,
    args: Vec<Value<'js>>,
) -> Result<Value<'js>> {
    let realm = crate::realm::realm(ctx)?;
    if realm.state.realm_id != realm_id {
        return Err(realm
            .state
            .throw_link_error(ctx, "function belongs to a different realm"));
    }
    let host = realm.state.clone();

    if sig.params.contains(&ModuleValType::V128) || sig.results.contains(&ModuleValType::V128) {
        return Err(host.throw_type_error(ctx, "v128 values cannot cross the JS/Wasm boundary"));
    }

    crate::memory::sync_all_js_to_wasm(ctx, &host)?;

    let mut inputs = Vec::with_capacity(sig.params.len());
    for (i, pty) in sig.params.iter().enumerate() {
        let arg = args
            .get(i)
            .cloned()
            .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        inputs.push(crate::value_conv::js_to_val(ctx, &host, arg, *pty)?);
    }

    let result_kinds: Vec<wasmz::ValKind> = sig
        .results
        .iter()
        .map(|t| t.to_wasmz_kind())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            host.throw_type_error(ctx, "v128 values cannot cross the JS/Wasm boundary")
        })?;

    let call_result = instance.call_index(func_idx, &inputs, &result_kinds);
    let sync_result = crate::memory::sync_all_wasm_to_js(ctx, &host);

    match call_result {
        Ok(outputs) => {
            sync_result?;
            to_js_results(ctx, &host, Some(instance.id()), &outputs)
        },
        Err(err) => {
            if sync_result.is_err() {
                let _ = ctx.catch();
            }
            Err(crate::errors::throw_wasmz_error(ctx, &host, err))
        },
    }
}

fn to_js_results<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    funcref_owner: Option<u64>,
    outputs: &[wasmz::Val],
) -> Result<Value<'js>> {
    match outputs.len() {
        0 => Ok(Value::new_undefined(ctx.clone())),
        1 => crate::value_conv::val_to_js(ctx, host, funcref_owner, &outputs[0]),
        _ => {
            let array = Array::new(ctx.clone())?;
            for (idx, v) in outputs.iter().enumerate() {
                array.set(
                    idx,
                    crate::value_conv::val_to_js(ctx, host, funcref_owner, v)?,
                )?;
            }
            Ok(array.into_value())
        },
    }
}

// Keep unused-import-free cleanly
#[allow(dead_code)]
fn _unused_placeholder(_: VecDeque<()>) {}
