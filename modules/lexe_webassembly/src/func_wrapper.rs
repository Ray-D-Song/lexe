// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Wraps exported `wasmz::Instance` functions as plain, callable JS `Function`s
//! preserving identity across multiple references / re-exports / `Table.get`.

use std::rc::Rc;

use rquickjs::prelude::Rest;
use rquickjs::{Ctx, Function, IntoJs, Result, Value};

use crate::host_state::{ExportedFuncEntry, FuncKey, HostState};
use crate::module::{FuncSigMeta, ModuleValType};

fn map_val_kind(k: wasmz::ValKind) -> ModuleValType {
    match k {
        wasmz::ValKind::I32 => ModuleValType::I32,
        wasmz::ValKind::I64 => ModuleValType::I64,
        wasmz::ValKind::F32 => ModuleValType::F32,
        wasmz::ValKind::F64 => ModuleValType::F64,
        wasmz::ValKind::FuncRef => ModuleValType::FuncRef,
        wasmz::ValKind::ExternRef => ModuleValType::ExternRef,
    }
}

pub fn sig_from_instance(inst: &wasmz::Instance, func_idx: u32) -> Option<FuncSigMeta> {
    let (params, results) = inst.func_sig(func_idx)?;
    Some(FuncSigMeta {
        params: params.into_iter().map(map_val_kind).collect(),
        results: results.into_iter().map(map_val_kind).collect(),
    })
}

pub fn wrap_func<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    instance: Rc<wasmz::Instance>,
    func_idx: u32,
    sig: FuncSigMeta,
    canonical_alias: Option<FuncKey>,
) -> Result<Function<'js>> {
    let key = canonical_alias.unwrap_or(FuncKey {
        instance_id: instance.id(),
        func_idx,
    });
    if let Some(existing) = host.cached_func_wrapper(ctx, key) {
        return Function::from_value(existing).map_err(|_| {
            host.throw_runtime_error(ctx, "cached function wrapper was not a function")
        });
    }

    let realm_id = host.realm_id;
    let target_instance = instance.clone();
    let call_sig = sig.clone();
    let js_func = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> Result<Value<'js>> {
            crate::instance::call_exported_func(
                &ctx,
                realm_id,
                &target_instance,
                func_idx,
                &call_sig,
                args.0,
            )
        },
    )?;
    let _ = js_func.set_name(format!("wasm_func_{}_{}", key.instance_id, key.func_idx));

    let entry = ExportedFuncEntry {
        instance: Rc::downgrade(&instance),
        func_idx,
        sig,
    };
    let value = js_func.clone().into_js(ctx)?;
    host.cache_func_wrapper(key, entry, value);

    Ok(js_func)
}
