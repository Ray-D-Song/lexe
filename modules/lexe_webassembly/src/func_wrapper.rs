// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Wraps a `wasmi::Func` (an exported or re-exported Wasm function) as a
//! plain, callable JS `Function`, and the inverse: recovering the original
//! `wasmi::Func` handle from such a wrapper (used when a funcref value flows
//! back into a table/global/import).
//!
//! Wrapping goes through [`HostState`]'s wrapper-identity cache keyed by
//! [`crate::store_access::handle_bits`], so re-exporting (or importing then
//! re-exporting) the same underlying `wasmi::Func` always yields the exact
//! same JS `Function` object (`instance.exports.foo === instance.exports.alias`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::prelude::Rest;
use rquickjs::{qjs, Ctx, Function, IntoJs, Result, Value};
use wasmi::Func;

use crate::host_state::{HostState, WrapKind};

thread_local! {
    /// Reverse mapping from a wrapped `Function`'s JS object identity back to
    /// its `wasmi::Func`, scoped per realm via the outer key. This lives
    /// alongside (not inside) `HostState` because it must be keyed by raw JS
    /// object pointer identity, which is a `store_access`-style raw detail.
    static FUNC_BY_OBJECT: RefCell<HashMap<(u64, usize), Func>> = RefCell::new(HashMap::new());
}

pub fn wrap_func<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    store: &mut dyn wasmi::AsContextMut<Data = Rc<HostState>>,
    func: Func,
) -> Result<Function<'js>> {
    let bits = unsafe { crate::store_access::handle_bits(func) };
    if let Some(existing) = host.cached_wrapper(ctx, WrapKind::Func, bits) {
        return Function::from_value(existing).map_err(|_| {
            host.throw_runtime_error(ctx, "cached function wrapper was not a function")
        });
    }

    let ty = func.ty(store.as_context());
    let realm_id = host.realm_id;

    let js_func = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> Result<Value<'js>> {
            crate::instance::call_exported_func(&ctx, realm_id, func, &ty, args.0)
        },
    )?;
    let _ = js_func.set_name(format!("wasm_func_{bits:x}"));

    let value = js_func.clone().into_js(ctx)?;
    let ptr = unsafe { qjs::JS_VALUE_GET_PTR(value.as_raw()) } as usize;
    FUNC_BY_OBJECT.with(|map| map.borrow_mut().insert((realm_id, ptr), func));
    host.cache_wrapper(WrapKind::Func, bits, value);

    Ok(js_func)
}

/// Recovers the `wasmi::Func` a JS value was wrapped from by [`wrap_func`] for
/// `host`'s realm, if any. Returns `Ok(None)` (not an error) for values that
/// are simply not a WebAssembly function wrapper, so callers can produce a
/// precise `TypeError`/`LinkError` message themselves.
pub fn unwrap_func(_ctx: &Ctx<'_>, host: &HostState, value: &Value<'_>) -> Result<Option<Func>> {
    if !value.is_function() {
        return Ok(None);
    }
    let ptr = unsafe { qjs::JS_VALUE_GET_PTR(value.as_raw()) } as usize;
    Ok(FUNC_BY_OBJECT.with(|map| map.borrow().get(&(host.realm_id, ptr)).copied()))
}

/// Cleans up every entry this realm registered in the reverse map. Called
/// from `WasmRealm`'s `Drop` impl so a long-lived process (many realms
/// created and destroyed over its lifetime, e.g. repeated `vm` usage) does
/// not leak `(realm_id, ptr)` entries for realms that no longer exist.
pub fn clear_realm(realm_id: u64) {
    FUNC_BY_OBJECT.with(|map| {
        map.borrow_mut().retain(|(id, _), _| *id != realm_id);
    });
}
