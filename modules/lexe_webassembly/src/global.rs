// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Global`.

use std::rc::Rc;

use rquickjs::{atom::PredefinedAtom, class::Trace, Class, Ctx, IntoJs, Object, Result, Value};
use wasmi::{Mutability, ValType};

use crate::host_state::{HostState, WrapKind};

#[derive(rquickjs::JsLifetime)]
#[rquickjs::class(rename = "Global")]
pub struct WasmGlobal {
    pub(crate) realm_id: u64,
    pub(crate) handle: wasmi::Global,
    pub(crate) value_type: ValType,
    pub(crate) mutable: bool,
}

impl<'js> Trace<'js> for WasmGlobal {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

fn parse_value_type<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    descriptor: &Object<'js>,
) -> Result<ValType> {
    let value_value =
        crate::descriptor::get_required(ctx, host, descriptor, "value", "WebAssembly.Global")?;
    // Ordinary JS `ToString`, matching the WebIDL `ValueType` enum's
    // conversion (not a strict "must already be a JS string" check).
    let value = crate::descriptor::to_string(ctx, value_value)?;
    match value.as_str() {
        "i32" => Ok(ValType::I32),
        "i64" => Ok(ValType::I64),
        "f32" => Ok(ValType::F32),
        "f64" => Ok(ValType::F64),
        "v128" => Err(host.throw_type_error(ctx, "v128 globals are not accessible from JS")),
        "anyfunc" | "funcref" => Ok(ValType::FuncRef),
        "externref" => Ok(ValType::ExternRef),
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
        let mutability = if mutable {
            Mutability::Var
        } else {
            Mutability::Const
        };

        let handle = crate::realm::with_context_mut(&realm, |store| -> Result<wasmi::Global> {
            let init = match &value.0 {
                Some(v) if !v.is_undefined() => {
                    crate::value_conv::js_to_val(&ctx, &host, store, v.clone(), value_type)?
                },
                _ => crate::value_conv::default_val(&ctx, &host, store, value_type)?,
            };
            Ok(wasmi::Global::new(store.as_context_mut(), init, mutability))
        })?;
        Ok(Self {
            realm_id: host.realm_id,
            handle,
            value_type,
            mutable,
        })
    }

    #[qjs(get)]
    pub fn value<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let host = realm.state.clone();
        crate::realm::with_context_mut(&realm, |store| {
            let val = self.handle.get(store.as_context());
            crate::value_conv::val_to_js(&ctx, &host, store, &val)
        })
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
        crate::realm::with_context_mut(&realm, |store| {
            let val = crate::value_conv::js_to_val(&ctx, &host, store, value, self.value_type)?;
            self.handle
                .set(store.as_context_mut(), val)
                .map_err(|err| host.throw_type_error(&ctx, err.to_string()))
        })
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
    global: wasmi::Global,
    value_type: ValType,
    mutable: bool,
) -> Result<Class<'js, WasmGlobal>> {
    let bits = unsafe { crate::store_access::handle_bits(global) };
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
        },
    )?;
    host.cache_wrapper(WrapKind::Global, bits, instance.clone().into_js(ctx)?);
    Ok(instance)
}

#[cfg(test)]
mod tests {
    use lexe_test::test_sync_with;

    #[tokio::test]
    async fn mutable_i32_global_gets_and_sets() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const g = new WebAssembly.Global({ value: "i32", mutable: true }, 41);
                    if (g.value !== 41) return false;
                    if (g.valueOf() !== 41) return false;
                    g.value = 100;
                    return g.value === 100;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// Regression test for the reviewed descriptor-coercion bug: `mutable`
    /// goes through `ToBoolean` (any truthy/falsy value, not just an
    /// actual JS boolean), and `value` goes through ordinary `ToString`.
    #[tokio::test]
    async fn mutable_uses_to_boolean_and_value_uses_to_string_coercion() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const g = new WebAssembly.Global(
                        { value: { toString() { return "i32"; } }, mutable: 1 },
                        5,
                    );
                    g.value = 6; // must not throw: `mutable: 1` (truthy) means mutable.
                    return g.value === 6;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn immutable_global_setter_throws_type_error() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let threw: bool = ctx.eval(
                r#"
                (() => {
                    const g = new WebAssembly.Global({ value: "i32" }, 7);
                    try {
                        g.value = 8;
                        return false;
                    } catch (e) {
                        return e instanceof TypeError;
                    }
                })()
                "#,
            )?;
            assert!(threw);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn i64_global_round_trips_bigint() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const g = new WebAssembly.Global({ value: "i64", mutable: true }, 10n);
                    if (typeof g.value !== "bigint") return false;
                    if (g.value !== 10n) return false;
                    g.value = 20n;
                    return g.value === 20n;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// A throwing `value`/`mutable` descriptor getter must propagate with
    /// its exact original identity, not get swallowed or replaced by a
    /// synthetic `TypeError`.
    #[tokio::test]
    async fn descriptor_getter_thrown_value_identity_is_preserved() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            for key in ["value", "mutable"] {
                let ok: bool = ctx.eval(format!(
                    r#"
                    (() => {{
                        const thrown = {{}};
                        const descriptor = {{
                            value: "i32",
                            get {key}() {{ throw thrown; }},
                        }};
                        try {{
                            new WebAssembly.Global(descriptor, 0);
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

    /// Regression test for the reviewed bug: per the JS API spec's
    /// `DefaultValue(externref)` algorithm, an `externref` global
    /// constructed with no explicit initial value must read back as JS
    /// `undefined`, not `null` (`null` is only `DefaultValue(funcref)`).
    #[tokio::test]
    async fn omitted_externref_global_value_defaults_to_undefined_not_null() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const g = new WebAssembly.Global({ value: "externref" });
                    return g.value === undefined;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn f64_global_default_value_is_zero() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const g = new WebAssembly.Global({ value: "f64" });
                    return g.value === 0;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }
}
