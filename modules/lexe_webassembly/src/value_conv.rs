// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! JS <-> Wasm value conversions.
//!
//! - `i32`/`f32`/`f64` use plain JS `Number` semantics (`ToInt32` for `i32`).
//! - `i64` only ever accepts/returns a JS `BigInt` (never coerces from/to
//!   `Number`), matching the core WebAssembly JS API spec.
//! - `externref` round-trips through [`HostState`]'s per-realm registry so JS
//!   object identity is preserved.
//! - `funcref` only accepts `null` or a Lexe-wrapped Wasm function; it never
//!   auto-wraps an arbitrary JS function (that would require a full
//!   `WebAssembly.Function`-style adapter, which is out of scope for this
//!   batch).
//! - `v128` cannot cross the JS/Wasm boundary and always raises `TypeError`.
//!
//! Every conversion below takes a `&mut dyn wasmi::AsContextMut<Data =
//! Rc<HostState>>` context (see `crate::realm::with_context_mut`) rather than
//! a concrete `Store`/`Caller`, so the same conversion code works whether it
//! runs at the top level or reentrantly from inside a host import callback.

use std::rc::Rc;

use rquickjs::{BigInt, Coerced, Ctx, FromJs, IntoJs, Result, Value};
use wasmi::{AsContextMut, Ref, Val, ValType};

use crate::host_state::HostState;

pub fn val_to_js<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    value: &Val,
) -> Result<Value<'js>> {
    match value {
        Val::I32(v) => v.into_js(ctx),
        Val::I64(v) => Ok(BigInt::from_i64(ctx.clone(), *v)?.into_value()),
        Val::F32(v) => f64::from(f32::from(*v)).into_js(ctx),
        Val::F64(v) => f64::from(*v).into_js(ctx),
        Val::FuncRef(r) => match r.val() {
            None => Ok(Value::new_null(ctx.clone())),
            Some(func) => Ok(crate::func_wrapper::wrap_func(ctx, host, store, *func)?.into_value()),
        },
        Val::ExternRef(r) => match r.val() {
            None => Ok(Value::new_null(ctx.clone())),
            Some(externref) => host
                .externref_object(ctx, store, *externref)
                .ok_or_else(|| host.throw_runtime_error(ctx, "externref object identity was lost")),
        },
        Val::V128(_) => {
            Err(host.throw_type_error(ctx, "v128 values cannot cross the JS/Wasm boundary"))
        },
    }
}

pub fn js_to_val<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    value: Value<'js>,
    ty: ValType,
) -> Result<Val> {
    match ty {
        ValType::I32 => {
            // `Coerced<i32>` goes through QuickJS's own `JS_ToInt32` (the
            // spec's `ToInt32`: `ToNumber` followed by 2**32 modulo
            // wraparound into a signed 32-bit range), matching the plan's
            // "`i32` 使用 JS Number 的 `ToInt32` 语义" -- unlike plain
            // `i32::from_js`, which only special-cases already-`Int`/`Float`
            // JS values and otherwise saturates a Rust `as i32` cast instead
            // of wrapping (e.g. it maps `2**32 + 1` to `i32::MAX`, not `1`).
            let n = Coerced::<i32>::from_js(ctx, value)
                .map_err(|_| {
                    host.throw_type_error(ctx, "expected a value convertible to a 32-bit integer")
                })?
                .0;
            Ok(Val::I32(n))
        },
        ValType::I64 => {
            let big = BigInt::from_js(ctx, value)
                .map_err(|_| host.throw_type_error(ctx, "expected a BigInt for an i64 value"))?;
            Ok(Val::I64(big.to_i64().map_err(|_| {
                host.throw_type_error(ctx, "BigInt value is out of i64 range")
            })?))
        },
        ValType::F32 => {
            let n = Coerced::<f64>::from_js(ctx, value)
                .map_err(|_| host.throw_type_error(ctx, "expected a Number"))?
                .0;
            Ok(Val::F32((n as f32).into()))
        },
        ValType::F64 => {
            let n = Coerced::<f64>::from_js(ctx, value)
                .map_err(|_| host.throw_type_error(ctx, "expected a Number"))?
                .0;
            Ok(Val::F64(n.into()))
        },
        ValType::FuncRef => {
            if value.is_null() || value.is_undefined() {
                return Ok(Val::FuncRef(Ref::Null));
            }
            let func = crate::func_wrapper::unwrap_func(ctx, host, &value)?.ok_or_else(|| {
                host.throw_type_error(
                    ctx,
                    "funcref values must be null or a WebAssembly-exported function",
                )
            })?;
            Ok(Val::FuncRef(Ref::Val(func)))
        },
        ValType::ExternRef => {
            // Only JS `null` maps to the Wasm-side null `externref`. `undefined`
            // is a normal, distinct JS value and must round-trip as itself
            // (`table.get(i) === undefined`, not `=== null`) -- see
            // `HostState::intern_externref`'s doc comment for how every other
            // JS value (including `undefined`) is interned.
            if value.is_null() {
                return Ok(Val::ExternRef(Ref::Null));
            }
            let handle = host.intern_externref(store, &value);
            Ok(Val::ExternRef(Ref::Val(handle)))
        },
        ValType::V128 => {
            Err(host.throw_type_error(ctx, "v128 values cannot cross the JS/Wasm boundary"))
        },
    }
}

/// The Wasm-side "default value" for a `Table`/`Global` constructor's
/// omitted initial value, `Table.prototype.set`/`grow`'s omitted fill
/// value, etc -- i.e. the JS API spec's own `DefaultValue(type)` algorithm.
///
/// For every type except `externref` this is exactly `Val::default(ty)`
/// (numeric zero, or `ref.null` for `funcref`), which already matches
/// `DefaultValue`. `externref`'s `DefaultValue` is specified as
/// `ToWebAssemblyValue(undefined, externref)` -- a *non-null* externref
/// that wraps the JS value `undefined` -- not `ref.null externref`
/// (`Val::default(ValType::ExternRef)`'s actual value). Getting this wrong
/// is observable: `new WebAssembly.Table({ element: "externref", initial: 1
/// }).get(0)` must be JS `undefined`, not `null`, and likewise for
/// `WebAssembly.Global({ value: "externref" }).value`.
pub fn default_val<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    ty: ValType,
) -> Result<Val> {
    if ty == ValType::ExternRef {
        return js_to_val(
            ctx,
            host,
            store,
            Value::new_undefined(ctx.clone()),
            ValType::ExternRef,
        );
    }
    Ok(Val::default(ty))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_sync_with;

    #[tokio::test]
    async fn i32_round_trips_with_to_int32_semantics() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let host = realm.state.clone();
            let value: Value = ctx.eval("4294967297")?; // 2**32 + 1 -> wraps to 1
            let val = crate::realm::with_context_mut(&realm, |store| {
                js_to_val(&ctx, &host, store, value, ValType::I32)
            })?;
            assert_eq!(val.i32(), Some(1));
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn i64_requires_bigint_not_number() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let host = realm.state.clone();
            let value: Value = ctx.eval("42")?;
            let err = crate::realm::with_context_mut(&realm, |store| {
                js_to_val(&ctx, &host, store, value, ValType::I64)
            });
            assert!(err.is_err());
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn v128_cannot_cross_boundary() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let host = realm.state.clone();
            let val = Val::V128(0u128.into());
            let err =
                crate::realm::with_context_mut(&realm, |store| val_to_js(&ctx, &host, store, &val));
            assert!(err.is_err());
            Ok(())
        })
        .await;
    }
}
