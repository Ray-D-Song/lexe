// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! JS <-> `wasmz::Val` conversions.

use rquickjs::{BigInt, Coerced, Ctx, FromJs, IntoJs, Result, Value};

use crate::host_state::{FuncKey, HostState};
use crate::module::ModuleValType;

pub fn val_to_js<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    funcref_owner: Option<u64>,
    value: &wasmz::Val,
) -> Result<Value<'js>> {
    match value {
        wasmz::Val::I32(v) => v.into_js(ctx),
        wasmz::Val::I64(v) => Ok(BigInt::from_i64(ctx.clone(), *v)?.into_value()),
        wasmz::Val::F32(v) => f64::from(*v).into_js(ctx),
        wasmz::Val::F64(v) => v.into_js(ctx),
        wasmz::Val::FuncRef(idx_opt) => match idx_opt {
            None => Ok(Value::new_null(ctx.clone())),
            Some(func_idx) => {
                if let Some(owner_id) = funcref_owner {
                    if let Some(existing) = host.cached_func_wrapper(
                        ctx,
                        FuncKey {
                            instance_id: owner_id,
                            func_idx: *func_idx,
                        },
                    ) {
                        return Ok(existing);
                    }
                    if let Some(inst) = host.instance_by_id(owner_id) {
                        if let Some(sig) = crate::func_wrapper::sig_from_instance(&inst, *func_idx)
                        {
                            let wrapped = crate::func_wrapper::wrap_func(
                                ctx, host, inst, *func_idx, sig, None,
                            )?;
                            return Ok(wrapped.into_value());
                        }
                    }
                }
                Ok(Value::new_null(ctx.clone()))
            },
        },
        wasmz::Val::ExternRef(id) => {
            if *id == 0 {
                Ok(Value::new_null(ctx.clone()))
            } else {
                host.externref_object(ctx, *id).ok_or_else(|| {
                    host.throw_runtime_error(ctx, "externref object identity was lost")
                })
            }
        },
    }
}

pub fn js_to_val<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    value: Value<'js>,
    ty: ModuleValType,
) -> Result<wasmz::Val> {
    match ty {
        ModuleValType::I32 => {
            let n = Coerced::<i32>::from_js(ctx, value)
                .map_err(|_| {
                    host.throw_type_error(ctx, "expected a value convertible to a 32-bit integer")
                })?
                .0;
            Ok(wasmz::Val::I32(n))
        },
        ModuleValType::I64 => {
            let big = BigInt::from_js(ctx, value)
                .map_err(|_| host.throw_type_error(ctx, "expected a BigInt for an i64 value"))?;
            Ok(wasmz::Val::I64(big.to_i64().map_err(|_| {
                host.throw_type_error(ctx, "BigInt value is out of i64 range")
            })?))
        },
        ModuleValType::F32 => {
            let n = Coerced::<f64>::from_js(ctx, value)
                .map_err(|_| host.throw_type_error(ctx, "expected a Number"))?
                .0;
            Ok(wasmz::Val::F32(n as f32))
        },
        ModuleValType::F64 => {
            let n = Coerced::<f64>::from_js(ctx, value)
                .map_err(|_| host.throw_type_error(ctx, "expected a Number"))?
                .0;
            Ok(wasmz::Val::F64(n))
        },
        ModuleValType::FuncRef => {
            if value.is_null() || value.is_undefined() {
                return Ok(wasmz::Val::FuncRef(None));
            }
            let key = host.unwrap_func_key(&value).ok_or_else(|| {
                host.throw_type_error(
                    ctx,
                    "funcref values must be null or a WebAssembly-exported function",
                )
            })?;
            Ok(wasmz::Val::FuncRef(Some(key.func_idx)))
        },
        ModuleValType::ExternRef => {
            if value.is_null() {
                return Ok(wasmz::Val::ExternRef(0));
            }
            let id = host.intern_externref(&value);
            Ok(wasmz::Val::ExternRef(id))
        },
        ModuleValType::V128 => {
            Err(host.throw_type_error(ctx, "v128 values cannot cross the JS/Wasm boundary"))
        },
    }
}

pub fn default_val<'js>(ctx: &Ctx<'js>, host: &HostState, ty: ModuleValType) -> Result<wasmz::Val> {
    match ty {
        ModuleValType::I32 => Ok(wasmz::Val::I32(0)),
        ModuleValType::I64 => Ok(wasmz::Val::I64(0)),
        ModuleValType::F32 => Ok(wasmz::Val::F32(0.0)),
        ModuleValType::F64 => Ok(wasmz::Val::F64(0.0)),
        ModuleValType::FuncRef => Ok(wasmz::Val::FuncRef(None)),
        ModuleValType::ExternRef => {
            let id = host.intern_externref(&Value::new_undefined(ctx.clone()));
            Ok(wasmz::Val::ExternRef(id))
        },
        ModuleValType::V128 => {
            Err(host.throw_type_error(ctx, "v128 values cannot cross the JS/Wasm boundary"))
        },
    }
}
