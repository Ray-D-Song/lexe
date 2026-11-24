// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Table`: `anyfunc`/`funcref` and `externref` element tables.

use std::rc::Rc;

use rquickjs::{atom::PredefinedAtom, class::Trace, Class, Ctx, IntoJs, Object, Result, Value};
use wasmi::{TableType, ValType};

use crate::host_state::{HostState, WrapKind};

#[derive(rquickjs::JsLifetime)]
#[rquickjs::class(rename = "Table")]
pub struct WasmTable {
    pub(crate) realm_id: u64,
    pub(crate) handle: wasmi::Table,
    pub(crate) element: ValType,
}

impl<'js> Trace<'js> for WasmTable {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

fn parse_element_type<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    descriptor: &Object<'js>,
) -> Result<ValType> {
    let element_value =
        crate::descriptor::get_required(ctx, host, descriptor, "element", "WebAssembly.Table")?;
    // WebIDL `TableKind` (an enum) is read via ordinary JS `ToString`, not a
    // strict "must already be a JS string" check: e.g. `{ element: { toString()
    // { return "externref"; } } }` is legal per spec.
    let element = crate::descriptor::to_string(ctx, element_value)?;
    match element.as_str() {
        "anyfunc" | "funcref" => Ok(ValType::FuncRef),
        "externref" => Ok(ValType::ExternRef),
        other => {
            Err(host.throw_type_error(ctx, format!("unsupported table element type '{other}'")))
        },
    }
}

fn checked_table_type(
    ctx: &Ctx<'_>,
    host: &HostState,
    element: ValType,
    initial: u32,
    maximum: Option<u32>,
) -> Result<TableType> {
    if let Some(max) = maximum {
        if max < initial {
            return Err(
                host.throw_range_error(ctx, "maximum table size is smaller than initial size")
            );
        }
    }
    Ok(TableType::new(element, initial, maximum))
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
        let maximum: Option<u32> = match crate::descriptor::get_optional(&descriptor, "maximum")? {
            Some(v) => Some(crate::descriptor::to_u32_enforce_range(
                &ctx, &host, v, "maximum",
            )?),
            None => None,
        };
        let ty = checked_table_type(&ctx, &host, element, initial, maximum)?;

        let handle = crate::realm::with_context_mut(&realm, |store| -> Result<wasmi::Table> {
            let init_val = match &initial_fill_value.0 {
                Some(v) if !v.is_undefined() => {
                    crate::value_conv::js_to_val(&ctx, &host, store, v.clone(), element)?
                },
                _ => crate::value_conv::default_val(&ctx, &host, store, element)?,
            };
            wasmi::Table::new(store.as_context_mut(), ty, init_val)
                .map_err(|err| host.throw_range_error(&ctx, err.to_string()))
        })?;
        Ok(Self {
            realm_id: host.realm_id,
            handle,
            element,
        })
    }

    #[qjs(get)]
    pub fn length(&self, ctx: Ctx<'_>) -> Result<u32> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        Ok(
            crate::realm::with_context_mut(&realm, |store| self.handle.size(store.as_context()))
                as u32,
        )
    }

    pub fn get<'js>(&self, ctx: Ctx<'js>, index: u32) -> Result<Value<'js>> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        crate::realm::with_context_mut(&realm, |store| {
            let val = self
                .handle
                .get(store.as_context(), u64::from(index))
                .ok_or_else(|| host.throw_range_error(&ctx, "table index out of bounds"))?;
            crate::value_conv::val_to_js(&ctx, &host, store, &val)
        })
    }

    pub fn set<'js>(
        &self,
        ctx: Ctx<'js>,
        index: u32,
        value: rquickjs::prelude::Opt<Value<'js>>,
    ) -> Result<()> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        crate::realm::with_context_mut(&realm, |store| {
            let val = match &value.0 {
                Some(v) if !v.is_undefined() => {
                    crate::value_conv::js_to_val(&ctx, &host, store, v.clone(), self.element)?
                },
                _ => crate::value_conv::default_val(&ctx, &host, store, self.element)?,
            };
            self.handle
                .set(store.as_context_mut(), u64::from(index), val)
                .map_err(|err| host.throw_range_error(&ctx, err.to_string()))
        })
    }

    pub fn grow<'js>(
        &self,
        ctx: Ctx<'js>,
        delta: u32,
        value: rquickjs::prelude::Opt<Value<'js>>,
    ) -> Result<u32> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        crate::realm::with_context_mut(&realm, |store| {
            let init = match &value.0 {
                Some(v) if !v.is_undefined() => {
                    crate::value_conv::js_to_val(&ctx, &host, store, v.clone(), self.element)?
                },
                _ => crate::value_conv::default_val(&ctx, &host, store, self.element)?,
            };
            let old = self
                .handle
                .grow(store.as_context_mut(), u64::from(delta), init)
                .map_err(|err| host.throw_range_error(&ctx, err.to_string()))?;
            Ok(old as u32)
        })
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
    table: wasmi::Table,
    element: ValType,
) -> Result<Class<'js, WasmTable>> {
    let bits = unsafe { crate::store_access::handle_bits(table) };
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

#[cfg(test)]
mod tests {
    use lexe_test::test_sync_with;

    #[tokio::test]
    async fn funcref_table_length_get_set_grow() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const table = new WebAssembly.Table({ element: "anyfunc", initial: 2, maximum: 4 });
                    if (table.length !== 2) return false;
                    if (table.get(0) !== null) return false;
                    if (table.get(1) !== null) return false;
                    const old = table.grow(2);
                    if (old !== 2) return false;
                    if (table.length !== 4) return false;
                    return true;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn table_get_out_of_bounds_throws_range_error() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let threw: bool = ctx.eval(
                r#"
                (() => {
                    const table = new WebAssembly.Table({ element: "anyfunc", initial: 1 });
                    try {
                        table.get(5);
                        return false;
                    } catch (e) {
                        return e instanceof RangeError;
                    }
                })()
                "#,
            )?;
            assert!(threw);
            Ok(())
        })
        .await;
    }

    /// Regression test for the reviewed bug: per the JS API spec's
    /// `DefaultValue(externref)` algorithm, an omitted `externref` table
    /// slot must read back as JS `undefined`, not `null` (`null` is only
    /// `DefaultValue(funcref)`). Covers the constructor's omitted initial
    /// value, `grow`'s omitted fill value, and `set`'s omitted value.
    #[tokio::test]
    async fn omitted_externref_value_defaults_to_undefined_not_null() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const table = new WebAssembly.Table({ element: "externref", initial: 1 });
                    if (table.get(0) !== undefined) return false;

                    table.grow(1);
                    if (table.get(1) !== undefined) return false;

                    table.set(0, {});
                    table.set(0);
                    if (table.get(0) !== undefined) return false;

                    return true;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn externref_table_preserves_object_identity() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const table = new WebAssembly.Table({ element: "externref", initial: 1 });
                    const obj = { tag: "probe" };
                    table.set(0, obj);
                    return table.get(0) === obj;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// A throwing `element`/`initial`/`maximum` descriptor getter must
    /// propagate with its exact original identity, not get swallowed or
    /// replaced by a synthetic `TypeError`.
    #[tokio::test]
    async fn descriptor_getter_thrown_value_identity_is_preserved() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            for key in ["element", "initial", "maximum"] {
                let ok: bool = ctx.eval(format!(
                    r#"
                    (() => {{
                        const thrown = {{}};
                        const descriptor = {{
                            element: "anyfunc",
                            initial: 1,
                            get {key}() {{ throw thrown; }},
                        }};
                        try {{
                            new WebAssembly.Table(descriptor);
                            return false;
                        }} catch (e) {{
                            return e === thrown;
                        }}
                    }})()
                    "#
                ))?;
                assert!(
                    ok,
                    "descriptor.{key} getter's thrown value must propagate as-is"
                );
            }
            Ok(())
        })
        .await;
    }

    /// Repeatedly overwriting an externref table cell must preserve the
    /// identity of the value currently stored in it. This also exercises the
    /// conservative registry lifetime policy: IDs are retained until the
    /// whole Wasm realm is torn down, because wasmi 1.1.0 does not expose a
    /// complete root enumeration API for internal tables, globals, and live
    /// Wasm frames.
    #[tokio::test]
    async fn repeated_table_set_preserves_current_externref() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            ctx.eval::<(), _>(
                r#"
                globalThis.__table = new WebAssembly.Table({ element: "externref", initial: 1 });
                for (let i = 0; i < 5000; i++) {
                    globalThis.__table.set(0, {});
                }
                globalThis.__current = globalThis.__table.get(0);
                "#,
            )?;
            let preserved: bool = ctx.eval("globalThis.__table.get(0) === globalThis.__current")?;
            assert!(preserved);
            Ok(())
        })
        .await;
    }

    /// Regression test for the reviewed descriptor-coercion bug: `element`
    /// goes through ordinary JS `ToString` (an object with a `toString()`
    /// is legal), and `initial` goes through WebIDL `[EnforceRange]`
    /// (a numeric string coerces; `NaN` throws `TypeError` rather than
    /// silently becoming `0`).
    #[tokio::test]
    async fn element_uses_to_string_and_initial_uses_enforce_range_coercion() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const table = new WebAssembly.Table({
                        element: { toString() { return "anyfunc"; } },
                        initial: "2",
                    });
                    return table.length === 2;
                })()
                "#,
            )?;
            assert!(ok);

            let threw: bool = ctx.eval(
                r#"
                (() => {
                    try {
                        new WebAssembly.Table({ element: "anyfunc", initial: NaN });
                        return false;
                    } catch (e) {
                        return e instanceof TypeError;
                    }
                })()
                "#,
            )?;
            assert!(
                threw,
                "'initial: NaN' must throw TypeError, not be silently accepted as 0"
            );
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn table_rejects_maximum_smaller_than_initial() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let threw: bool = ctx.eval(
                r#"
                (() => {
                    try {
                        new WebAssembly.Table({ element: "anyfunc", initial: 4, maximum: 1 });
                        return false;
                    } catch (e) {
                        return e instanceof RangeError;
                    }
                })()
                "#,
            )?;
            assert!(threw);
            Ok(())
        })
        .await;
    }
}
