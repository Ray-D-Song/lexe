// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Instance`: import resolution (ordered JS `Get`s matching the
//! module's binary import order), instantiation (which also runs the Wasm
//! `start` function), the `exports` object, and the two call paths that
//! cross the JS/Wasm boundary at runtime:
//!
//! - [`call_exported_func`]: JS calls into a Wasm (or re-exported host) function.
//! - [`call_js_import`]: Wasm calls into a JS function import (the host
//!   function trampoline installed by [`build_dynamic_host_func`]).
//!
//! Both paths perform the synchronous memory-mirror sync described in
//! `crate::memory` immediately before/after crossing the boundary, and route
//! all `Store`/`Caller` access through [`crate::realm::with_context_mut`] so
//! that a JS import callback which reenters the same instance (e.g. calls an
//! exported function while it is running) reuses the currently active
//! `wasmi::Caller` instead of re-borrowing the realm's `Store` -- see
//! `crate::store_access` for the full reentrancy design.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use rquickjs::function::{IntoArgs, Rest};
use rquickjs::{
    atom::PredefinedAtom, class::Trace, Array, Class, Ctx, JsLifetime, Object, Result, Value,
};
use wasmi::{AsContextMut, ExternType, Func, FuncType, Val};

use crate::host_state::HostState;
use crate::module::WasmModule;
use crate::realm::WasmRealm;

/// `exports` is an ordinary `'js`-scoped `Object` (not a
/// `Persistent<Object<'static>>`): a `WasmInstance` is routinely reachable
/// from its own context's `globalThis` (assigned to some variable by the
/// script that constructed it), and per `crate::realm`'s module docs, any
/// `Persistent` stashed in a class field reachable that way is not
/// guaranteed to be dropped before `Runtime` teardown -- `Persistent` must
/// live only inside the `Runtime`-userdata-backed realm registry. Keeping
/// `exports` as a plain, lifetime-tracked `Object<'js>` sidesteps the issue
/// entirely: it participates in ordinary `Trace`/GC exactly like any other
/// JS-facing class field (see `lexe_events::EventEmitter<'js>` for
/// the same pattern elsewhere in this codebase).
#[rquickjs::class(rename = "Instance")]
pub struct WasmInstance<'js> {
    exports: Object<'js>,
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

/// Resolves `module`'s imports against `imports` (ordinary JS `Get`s, in the
/// module's binary import order), instantiates it (which also runs the Wasm
/// `start` function, if any), and builds the JS-visible `exports` object.
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

    // `module.metadata.imports` preserves the binary's declaration order
    // (required so that `imports[moduleName][fieldName]` `Get`s -- which can
    // observably run through user `Proxy` traps/getters/exceptions -- happen
    // in that same order); `wasmi::Module::imports()` does not document (or,
    // empirically, keep) that order, but it does carry the precise
    // `ExternType` (e.g. exact `FuncType`) each import needs to be resolved
    // against. Build a `(module, name) -> ExternType` multimap from the
    // latter (a `VecDeque` per key to correctly handle the legal-but-rare
    // case of the same `module.name` pair imported more than once) and drain
    // it in metadata order.
    let mut by_key: HashMap<(String, String), VecDeque<ExternType>> = HashMap::new();
    for import in module.inner.imports() {
        by_key
            .entry((import.module().to_string(), import.name().to_string()))
            .or_default()
            .push_back(import.ty().clone());
    }

    // Resolve every import's JS `Get` (in metadata/binary order, as required
    // above) into a `wasmi::Extern`, but keep each resolved value keyed by
    // `(module, name)` rather than in a single positional `Vec` yet:
    // `wasmi::Instance::new`'s `externals` argument must line up
    // positionally with `wasmi::Module::imports()`'s *own* iteration order
    // (see `wasmi::module::instantiate::InstancePre::extract_imports`, which
    // simply `zip()`s the two), and that order does not necessarily match
    // `module.metadata.imports`'s binary order -- confirmed empirically to
    // diverge for modules that mix import kinds (e.g. memory + table +
    // global together), even though it happened to coincide for every
    // single-kind-only fixture used while this was first implemented. Using
    // `module.metadata.imports`'s order directly as `wasmi::Instance::new`'s
    // `externals` would then silently zip each resolved extern against the
    // *wrong* import declaration, surfacing as a spurious `LinkError` type
    // mismatch (or worse, a wrong-but-type-compatible extern silently
    // linked to the wrong import).
    let mut resolved_by_key: HashMap<(String, String), VecDeque<wasmi::Extern>> = HashMap::new();
    for import in &module.metadata.imports {
        let module_name = import.module.as_str();
        let field_name = import.name.as_str();
        // `imports` is `Some` here: either it was checked above, or
        // `module.metadata.imports` is empty and this loop body never runs.
        //
        // These two `Get`s are "ordinary JS `Get`"s per the implementation
        // plan: if `namespace_obj`/`namespace` is (or is wrapped by) a
        // `Proxy`, or the property is an accessor, whatever it throws must
        // propagate to the caller with its exact original identity, *not*
        // get reclassified into a `LinkError`. Using `?` here (rather than
        // `map_err`) is what preserves that: `rquickjs::Object::get`'s only
        // failure mode for a `Value`-typed destination is the JS `Get`
        // itself throwing, in which case the thrown value is already the
        // QuickJS context's pending exception and `?` simply lets it
        // propagate unmodified. A `LinkError` is only synthesized below,
        // *after* a `Get` has already succeeded, once the resulting value
        // is observed to have the wrong shape (missing namespace object,
        // missing/mistyped import).
        let namespace_obj = imports.as_ref().expect("checked above");
        let namespace_val: Value = namespace_obj.get(module_name)?;
        let namespace = namespace_val.as_object().ok_or_else(|| {
            host.throw_link_error(
                ctx,
                format!("import namespace '{module_name}' is not an object"),
            )
        })?;
        let value: Value = namespace.get(field_name)?;

        let ty = by_key
            .get_mut(&(module_name.to_string(), field_name.to_string()))
            .and_then(VecDeque::pop_front)
            .expect("wasmparser and wasmi must agree on the module's import set");
        let resolved = resolve_import(ctx, &host, realm, &ty, &value, module_name, field_name)?;
        resolved_by_key
            .entry((module_name.to_string(), field_name.to_string()))
            .or_default()
            .push_back(resolved);
    }

    // Now re-derive the externs `Vec` in `wasmi::Module::imports()`'s own
    // order (matching each entry back up by `(module, name)`, draining the
    // same per-key `VecDeque`s so a repeated `module.name` pair still
    // resolves its N JS-side values to its N wasmi-side import slots in a
    // stable, consistent order) for the actual `wasmi::Instance::new` call.
    let mut externs: Vec<wasmi::Extern> = Vec::with_capacity(module.metadata.imports.len());
    for import in module.inner.imports() {
        let resolved = resolved_by_key
            .get_mut(&(import.module().to_string(), import.name().to_string()))
            .and_then(VecDeque::pop_front)
            .expect("every wasmi-reported import was resolved from module.metadata.imports above");
        externs.push(resolved);
    }

    let handle = crate::realm::with_context_mut(realm, |store| {
        wasmi::Instance::new(store.as_context_mut(), &module.inner, &externs)
    })
    .map_err(|err| crate::errors::throw_for_wasmi_error_or_sentinel(ctx, &host, err))?;

    let exports_obj = build_exports_object(ctx, &host, realm, module, handle)?;
    Ok(WasmInstance {
        exports: exports_obj,
    })
}

fn resolve_import<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    realm: &Rc<WasmRealm>,
    ty: &ExternType,
    value: &Value<'js>,
    module_name: &str,
    field_name: &str,
) -> Result<wasmi::Extern> {
    match ty {
        ExternType::Func(func_ty) => {
            if let Some(existing) = crate::func_wrapper::unwrap_func(ctx, host, value)? {
                return Ok(wasmi::Extern::Func(existing));
            }
            let js_func = value.as_function().cloned().ok_or_else(|| {
                host.throw_link_error(
                    ctx,
                    format!("import '{module_name}.{field_name}' is not a function"),
                )
            })?;
            let func = build_dynamic_host_func(realm, host, func_ty.clone(), js_func);
            Ok(wasmi::Extern::Func(func))
        },
        ExternType::Memory(_) => {
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
            let handle = borrow.handle;
            drop(borrow);
            register_wrapper_provenance(host, crate::host_state::WrapKind::Memory, handle, value);
            Ok(wasmi::Extern::Memory(handle))
        },
        ExternType::Table(_) => {
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
            let handle = borrow.handle;
            drop(borrow);
            register_wrapper_provenance(host, crate::host_state::WrapKind::Table, handle, value);
            Ok(wasmi::Extern::Table(handle))
        },
        ExternType::Global(_) => {
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
            let handle = borrow.handle;
            drop(borrow);
            register_wrapper_provenance(host, crate::host_state::WrapKind::Global, handle, value);
            Ok(wasmi::Extern::Global(handle))
        },
    }
}

/// Records that `value` (the exact JS `Memory`/`Table`/`Global` object the
/// caller passed as an import) is the canonical wrapper for `handle` in the
/// realm-wide wrapper cache (`crate::host_state::HostState::cache_wrapper`),
/// *before* instantiation runs.
///
/// Without this, re-exporting an imported Memory/Table/Global (e.g.
/// `(export "aliasName" (global $imported))`) would build its `exports`
/// entry via `wrap_memory`/`wrap_table`/`wrap_global`, which only *consults*
/// the wrapper cache -- it has no way to discover that this exact
/// `wasmi` handle already has a JS wrapper unless that wrapper registered
/// itself first. Registering here (using the same `(WrapKind, handle_bits)`
/// key `wrap_*` looks up) guarantees `instance.exports.aliasName === value`,
/// matching Node/browsers and the implementation plan's "导入/再导出的
/// Global 保持 wrapper identity" requirement.
fn register_wrapper_provenance<'js, T: Copy>(
    host: &HostState,
    kind: crate::host_state::WrapKind,
    handle: T,
    value: &Value<'js>,
) {
    let bits = unsafe { crate::store_access::handle_bits(handle) };
    host.cache_wrapper(kind, bits, value.clone());
}

/// Creates the exported members object for a freshly instantiated `handle`:
/// a null-prototype, non-extensible object whose own properties are
/// enumerable, non-writable, non-configurable data properties, one per Wasm
/// export.
///
/// Properties are defined by iterating `module.metadata.exports` (parsed
/// directly from the binary in `crate::module::extract_module_metadata`,
/// preserving binary declaration order) and resolving each one against
/// `handle` via `Instance::get_export`, rather than by iterating
/// `wasmi::Module::exports()` directly -- that iterator's order does *not*
/// match the binary's declaration order (verified empirically: it is
/// neither binary order nor a simple per-kind grouping of it).
fn build_exports_object<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    realm: &Rc<WasmRealm>,
    module: &WasmModule,
    handle: wasmi::Instance,
) -> Result<Object<'js>> {
    let exports = Object::new(ctx.clone())?;
    exports.set_prototype(None)?;

    crate::realm::with_context_mut(realm, |store| -> Result<()> {
        for export in &module.metadata.exports {
            let name = export.name.as_str();
            let extern_ = handle
                .get_export(store.as_context(), name)
                .expect("export declared by the Module must be resolvable on its own Instance");
            let value: Value = match extern_ {
                wasmi::Extern::Func(func) => {
                    crate::func_wrapper::wrap_func(ctx, host, store, func)?.into_value()
                },
                wasmi::Extern::Memory(memory) => {
                    crate::memory::wrap_memory(ctx, host, memory)?.into_value()
                },
                wasmi::Extern::Table(table) => {
                    let element = table.ty(store.as_context()).element();
                    crate::table::wrap_table(ctx, host, table, element)?.into_value()
                },
                wasmi::Extern::Global(global) => {
                    let ty = global.ty(store.as_context());
                    let mutable = matches!(ty.mutability(), wasmi::Mutability::Var);
                    crate::global::wrap_global(ctx, host, global, ty.content(), mutable)?
                        .into_value()
                },
            };
            use rquickjs::object::Property;
            exports.prop(name, Property::from(value).enumerable())?;
        }
        Ok(())
    })?;

    // SAFETY: `exports` is a live `Object` owned by `ctx`, per
    // `store_access::prevent_extensions`'s documented invariant.
    unsafe { crate::store_access::prevent_extensions(ctx, &exports) }.map_err(|_| {
        host.throw_runtime_error(ctx, "failed to make Instance.exports non-extensible")
    })?;
    Ok(exports)
}

/// Creates a `wasmi::Func` host trampoline for a JS function import. The
/// closure passed to `wasmi::Func::new` captures only `realm_id` and
/// `callback_id` (both trivially `Send + Sync + 'static`), per the
/// implementation plan; the actual JS `Function` is looked up through
/// [`HostState::callback`] each time the trampoline runs, using the `Ctx`
/// reconstructed from `Caller::data()`.
fn build_dynamic_host_func(
    realm: &Rc<WasmRealm>,
    host: &HostState,
    ty: FuncType,
    js_func: rquickjs::Function<'_>,
) -> Func {
    let callback_id = host.register_callback(js_func);
    let realm_id = host.realm_id;
    crate::realm::with_context_mut(realm, |store| {
        wasmi::Func::new(
            store.as_context_mut(),
            ty,
            move |caller, inputs, outputs| {
                call_js_import(realm_id, callback_id, caller, inputs, outputs)
            },
        )
    })
}

/// The host function trampoline invoked by `wasmi` whenever Wasm code calls
/// a JS function import created by [`build_dynamic_host_func`].
fn call_js_import(
    realm_id: u64,
    callback_id: u32,
    mut caller: wasmi::Caller<'_, Rc<HostState>>,
    inputs: &[Val],
    outputs: &mut [Val],
) -> std::result::Result<(), wasmi::Error> {
    let host = caller.data().clone();
    // SAFETY: this trampoline only ever runs synchronously on the thread
    // that owns `host`'s realm, for the dynamic extent of the `Func::call`
    // that invoked it (which itself only happens while that realm's context
    // is alive), per `HostState::ctx`'s documented invariant.
    let ctx = unsafe { host.ctx() };

    if let Err(err) = crate::memory::sync_all_wasm_to_js(&ctx, &host, &mut caller) {
        return Err(crate::errors::to_wasmi_error(&ctx, &host, err));
    }

    // SAFETY: `guard` is dropped (popping the stack) before this function
    // returns on every path below, and `caller` is not moved or dropped
    // while it is alive.
    let guard = unsafe { crate::store_access::ActiveCallerGuard::push(realm_id, &mut caller) };
    let outcome = run_js_import_callback(&ctx, &host, callback_id, &mut caller, inputs, outputs);
    drop(guard);

    match outcome {
        Ok(()) => crate::memory::sync_all_js_to_wasm(&ctx, &host, &mut caller)
            .map_err(|err| crate::errors::to_wasmi_error(&ctx, &host, err)),
        Err(err) => {
            // Capture (and clear, via `to_wasmi_error`'s internal
            // `ctx.catch()`) the *original* callback exception/error
            // immediately, before running any further JS code below.
            // `ctx.catch()` only ever returns whatever exception is
            // *currently* pending on the context -- if the best-effort
            // write-back ran first and it also threw (e.g. because the
            // mirror's `ArrayBuffer` was concurrently detached), that would
            // silently replace the original exception in the QuickJS
            // context's pending-exception slot before we ever got a chance
            // to read it, exactly the "同步错误不得覆盖原始 callback 异常"
            // case the implementation plan calls out.
            let wasmi_err = crate::errors::to_wasmi_error(&ctx, &host, err);
            // Best-effort write-back; its own result is deliberately
            // discarded from this point on: the original callback
            // error/exception (already captured above) must always win.
            let _ = crate::memory::sync_all_js_to_wasm(&ctx, &host, &mut caller);
            Err(wasmi_err)
        },
    }
}

fn run_js_import_callback<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    callback_id: u32,
    caller: &mut wasmi::Caller<'_, Rc<HostState>>,
    inputs: &[Val],
    outputs: &mut [Val],
) -> Result<()> {
    let js_func = host
        .callback(ctx, callback_id)
        .ok_or_else(|| host.throw_runtime_error(ctx, "missing import callback registration"))?;

    let mut js_args = Vec::with_capacity(inputs.len());
    for v in inputs {
        js_args.push(crate::value_conv::val_to_js(ctx, host, caller, v)?);
    }
    let result: Value<'js> = (Rest(js_args),).apply(&js_func)?;

    if outputs.is_empty() {
        // Per the WebAssembly JS API, a zero-result import's JS return value
        // is ignored entirely.
    } else if outputs.len() == 1 {
        let ty = outputs[0].ty();
        outputs[0] = crate::value_conv::js_to_val(ctx, host, caller, result, ty)?;
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
            let ty = slot.ty();
            let element: Value<'js> = array.get(i)?;
            *slot = crate::value_conv::js_to_val(ctx, host, caller, element, ty)?;
        }
    }
    Ok(())
}

/// Called by [`crate::func_wrapper::wrap_func`]'s JS-visible wrapper whenever
/// JS code calls an exported (or re-exported/imported-then-exported) Wasm
/// function.
pub fn call_exported_func<'js>(
    ctx: &Ctx<'js>,
    realm_id: u64,
    func: Func,
    ty: &FuncType,
    args: Vec<Value<'js>>,
) -> Result<Value<'js>> {
    let realm = crate::realm::realm(ctx)?;
    if realm.state.realm_id != realm_id {
        return Err(realm
            .state
            .throw_link_error(ctx, "function belongs to a different realm"));
    }
    let host = realm.state.clone();

    crate::realm::with_context_mut(&realm, |store| -> Result<Value<'js>> {
        crate::memory::sync_all_js_to_wasm(ctx, &host, store)?;

        // Per the JS API spec, a Wasm-exported function is called exactly
        // like any other variadic JS function: missing trailing arguments
        // are simply `undefined` (which each type's own JS-value coercion
        // already knows how to handle -- e.g. `ToInt32(undefined)` is `0`),
        // not a `TypeError`; extra arguments beyond `param_types.len()` are
        // silently ignored. Iterate `param_types` (not `args`) so a call
        // with too *few* arguments still produces one `undefined`-coerced
        // input per missing parameter instead of a short `inputs`.
        let param_types = ty.params();
        let mut inputs = Vec::with_capacity(param_types.len());
        for (i, pty) in param_types.iter().enumerate() {
            let arg = args
                .get(i)
                .cloned()
                .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
            inputs.push(crate::value_conv::js_to_val(ctx, &host, store, arg, *pty)?);
        }
        let result_types = ty.results();
        let mut outputs: Vec<Val> = result_types.iter().map(|t| Val::default(*t)).collect();

        let call_result = func.call(store.as_context_mut(), &inputs, &mut outputs);
        // Best-effort write-back; must not clobber a trap/callback error below.
        let sync_result = crate::memory::sync_all_wasm_to_js(ctx, &host, store);

        match call_result {
            Ok(()) => {
                sync_result?;
                to_js_results(ctx, &host, store, &outputs)
            },
            Err(err) => {
                // `throw_for_wasmi_error_or_sentinel`'s sentinel path
                // restores the original callback exception straight out of
                // `HostState`'s own field (no further JS execution
                // involved), so it is unaffected by whatever `sync_result`
                // did. Its *non*-sentinel path, though, throws a fresh
                // `CompileError`/`LinkError`/`RuntimeError` by constructing
                // one via the captured primordial constructor -- which is
                // itself a JS call, and must not run while `sync_result`'s
                // own failure (a real thrown exception; see
                // `sync_all_wasm_to_js`'s error paths) is still sitting
                // uncaught on `ctx`. Drain and discard it first so `err`
                // (the original trap/link/instantiation failure this
                // function call actually failed with) is what gets
                // classified and thrown below.
                if sync_result.is_err() {
                    let _ = ctx.catch();
                }
                Err(crate::errors::throw_for_wasmi_error_or_sentinel(
                    ctx, &host, err,
                ))
            },
        }
    })
}

fn to_js_results<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    store: &mut dyn AsContextMut<Data = Rc<HostState>>,
    outputs: &[Val],
) -> Result<Value<'js>> {
    match outputs.len() {
        0 => Ok(Value::new_undefined(ctx.clone())),
        1 => crate::value_conv::val_to_js(ctx, host, store, &outputs[0]),
        _ => {
            let array = Array::new(ctx.clone())?;
            for (idx, v) in outputs.iter().enumerate() {
                array.set(idx, crate::value_conv::val_to_js(ctx, host, store, v)?)?;
            }
            Ok(array.into_value())
        },
    }
}

#[cfg(test)]
mod tests {
    use lexe_test::test_sync_with;
    use rquickjs::{Class, Object, Value};

    fn set_wasm_bytes<'js>(
        ctx: &rquickjs::Ctx<'js>,
        name: &str,
        wat: &str,
    ) -> rquickjs::Result<()> {
        let bytes = wat::parse_str(wat).unwrap();
        let buffer = rquickjs::ArrayBuffer::new_copy(ctx.clone(), &bytes)?;
        ctx.globals().set(name, buffer)?;
        Ok(())
    }

    /// A JS import callback (running inside `call_js_import`'s host
    /// trampoline, i.e. while the outer `wasmi::Store` borrow is already
    /// held for that `Func::call`) reentrantly calls back into one of the
    /// *same instance's* exported functions. This must be served by
    /// `crate::store_access`'s `ActiveCallerGuard` stack (via
    /// `crate::realm::with_context_mut`) rather than re-borrowing the
    /// realm's `Store`, which would panic on the double `RefCell` borrow.
    #[tokio::test]
    async fn js_import_callback_can_reenter_and_call_an_export() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"
                (module
                    (import "env" "cb" (func $cb (result i32)))
                    (func (export "add") (param i32 i32) (result i32)
                        local.get 0
                        local.get 1
                        i32.add)
                    (func (export "callIntoAdd") (result i32)
                        call $cb)
                )
                "#,
            )?;
            let result: i32 = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    let instance;
                    instance = new WebAssembly.Instance(module, {
                        env: { cb: () => instance.exports.add(40, 2) },
                    });
                    return instance.exports.callIntoAdd();
                })()
                "#,
            )?;
            assert_eq!(result, 42);
            Ok(())
        })
        .await;
    }

    /// Per the implementation plan: "JS import callback 抛出的值：保持原对
    /// 象身份原样重新抛出，不包装为 RuntimeError". The exact thrown object
    /// (not a copy, not a `RuntimeError` wrapper) must surface at the call
    /// site of the exported function that (transitively) invoked the
    /// import.
    #[tokio::test]
    async fn js_import_callback_thrown_value_identity_is_preserved() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"
                (module
                    (import "env" "cb" (func $cb))
                    (func (export "run") call $cb)
                )
                "#,
            )?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const thrown = { marker: "distinctive-probe" };
                    const instance = new WebAssembly.Instance(module, {
                        env: { cb() { throw thrown; } },
                    });
                    try {
                        instance.exports.run();
                        return false;
                    } catch (e) {
                        return e === thrown;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// Regression test for the P2 fix in `call_js_import`'s error path: a
    /// JS import callback's original thrown value must win over a failure
    /// in the best-effort Wasm-boundary memory resync that runs after it,
    /// not get silently replaced by whatever that resync itself throws.
    ///
    /// The corruption (via `crate::memory::corrupt_mirror_for_test`) must be
    /// injected from *inside* the JS import callback, immediately before it
    /// throws -- not up front, before `run()` is even called: `run()`'s own
    /// `call_exported_func` performs an unconditional JS->Wasm resync of
    /// every materialized mirror *before* invoking the Wasm function at all,
    /// which would itself fail (for an unrelated reason -- nothing has
    /// thrown yet) if the mirror were already corrupted at that point,
    /// short-circuiting before the callback ever runs. Exposing a tiny
    /// native `corrupt()` import that the JS callback calls just before its
    /// `throw` reproduces the exact ordering the P2 fix is about: callback
    /// throws, *then* both the resync inside `call_js_import` and the one
    /// `call_exported_func` runs on its own error path fail. Asserts the
    /// exact original thrown object -- not a `RuntimeError` about the
    /// corrupted mirror -- is what the caller observes.
    #[tokio::test]
    async fn callback_exception_identity_wins_over_injected_sync_failure() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"
                (module
                    (import "env" "cb" (func $cb))
                    (import "env" "corrupt" (func $corrupt))
                    (memory (export "mem") 1)
                    (func (export "run") call $cb)
                )
                "#,
            )?;

            // Deliberately does *not* capture `host`/`realm.state` (an
            // `Rc<HostState>`) by value: doing so would give this native
            // `Function`'s JS-reachable closure its own extra, independent
            // strong reference to the very `HostState` that `RealmRegistry`
            // (`Runtime`-userdata) owns. `RawRuntime::drop` only drops
            // `Runtime`-userdata (dropping the registry's `Rc<WasmRealm>`,
            // and, if that were the *last* reference, `HostState` and every
            // `Persistent` it owns) *before* calling `JS_FreeRuntime` -- but
            // if this closure's own clone keeps that same `Rc<HostState>`'s
            // count above zero at that point, `HostState` (and its
            // `Persistent`s) only actually drops *later*, when
            // `JS_FreeRuntime` itself finalizes this native `Function`
            // object during its own object-freeing sweep -- i.e. reentrantly,
            // from inside the engine's own teardown, reproducing the exact
            // `gc_obj_list` hazard this crate's `WasmRealmHolder` is
            // designed to avoid. `crate::func_wrapper::wrap_func` (the
            // production equivalent of a JS-reachable closure tied to this
            // realm) avoids this correctly by capturing only a plain
            // `realm_id: u64` and looking `HostState` up fresh from `ctx` on
            // every call; do the same here rather than capturing `host`.
            let corrupt_fn = rquickjs::Function::new(
                ctx.clone(),
                |ctx: rquickjs::Ctx<'_>| -> rquickjs::Result<()> {
                    let instance: Object = ctx.globals().get("__instance")?;
                    let exports: Object = instance.get("exports")?;
                    let mem_value: Value = exports.get("mem")?;
                    let mem_class = Class::<crate::memory::WasmMemory>::from_value(&mem_value)
                        .expect("exports.mem must be a WebAssembly.Memory");
                    let handle = mem_class.borrow().handle;
                    let host = crate::realm::realm(&ctx)?.state.clone();
                    crate::memory::corrupt_mirror_for_test(&ctx, &host, handle);
                    Ok(())
                },
            )?;
            ctx.globals().set("__corrupt", corrupt_fn)?;

            ctx.eval::<(), _>(
                r#"
                (() => {
                    globalThis.__thrown = {};
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const instance = new WebAssembly.Instance(module, {
                        env: {
                            cb() {
                                globalThis.__corrupt();
                                throw globalThis.__thrown;
                            },
                            corrupt() {},
                        },
                    });
                    // Materialize the memory mirror so it participates in
                    // the boundary-crossing sync passes below.
                    void instance.exports.mem.buffer;
                    globalThis.__instance = instance;
                })()
                "#,
            )?;

            let ok: bool = ctx.eval(
                r#"
                (() => {
                    try {
                        globalThis.__instance.exports.run();
                        return false;
                    } catch (e) {
                        return e === globalThis.__thrown;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// A trapping `start` function must surface as instantiation failing
    /// with a `RuntimeError` (not a `LinkError`/`CompileError`, and not an
    /// uncatchable abort).
    #[tokio::test]
    async fn trapping_start_function_throws_runtime_error() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"
                (module
                    (func $boom unreachable)
                    (start $boom)
                )
                "#,
            )?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    try {
                        new WebAssembly.Instance(module, {});
                        return false;
                    } catch (e) {
                        return e instanceof WebAssembly.RuntimeError;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// A multi-result exported function's return value must be a JS
    /// `Array` with one element per Wasm result, in result order.
    #[tokio::test]
    async fn multi_value_export_returns_array_of_results() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"
                (module
                    (func (export "divmod") (param i32 i32) (result i32 i32)
                        local.get 0
                        local.get 1
                        i32.div_u
                        local.get 0
                        local.get 1
                        i32.rem_u)
                )
                "#,
            )?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const instance = new WebAssembly.Instance(module, {});
                    const result = instance.exports.divmod(17, 5);
                    return Array.isArray(result) && result.length === 2 && result[0] === 3 && result[1] === 2;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// Regression test for the reviewed bug: calling an exported WebAssembly
    /// function with fewer arguments than it declares parameters must *not*
    /// throw -- per the JS API spec it is called like any other variadic JS
    /// function, so missing trailing arguments are simply `undefined`,
    /// coerced the same way an explicit `undefined` argument would be
    /// (`ToInt32(undefined) === 0` for `i32`). Extra arguments beyond the
    /// declared parameter count are silently ignored.
    #[tokio::test]
    async fn exported_function_treats_missing_arguments_as_undefined() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"
                (module
                    (func (export "add") (param i32 i32) (result i32)
                        local.get 0
                        local.get 1
                        i32.add)
                )
                "#,
            )?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const instance = new WebAssembly.Instance(module, {});
                    const { add } = instance.exports;
                    // `ToInt32(undefined) === 0` for every missing argument.
                    if (add() !== 0) return false;
                    if (add(1) !== 1) return false;
                    // Extra arguments are simply ignored.
                    if (add(1, 2, 3, 4) !== 3) return false;
                    return true;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// Regression test: a `Memory`/`Table`/`Global` *constructed directly
    /// in JS* (as opposed to one obtained by wrapping a `wasmi` handle via
    /// `crate::memory::wrap_memory`/`crate::table::wrap_table`/
    /// `crate::global::wrap_global`, e.g. as another export), then passed
    /// in as an import and re-exported under an alias, must come back out
    /// of `Instance.exports` as the exact same JS object
    /// (`instance.exports.alias === theOriginalObject`), matching
    /// Node/browsers and the implementation plan's "导入/再导出的 Global
    /// 保持 wrapper identity" requirement.
    ///
    /// Before `resolve_import` registered the imported value into
    /// `HostState`'s wrapper cache (keyed by `store_access::handle_bits`),
    /// this would spuriously construct a *second*, distinct `WasmGlobal`/
    /// `WasmMemory`/`WasmTable` wrapping the same underlying `wasmi`
    /// handle -- because `WasmGlobal::new` et al. (the JS-visible
    /// constructors) never populate that cache themselves, only
    /// `wrap_*` consult it.
    #[tokio::test]
    async fn imported_then_reexported_memory_table_global_preserve_wrapper_identity() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"
                (module
                    (import "env" "mem" (memory 1))
                    (import "env" "tbl" (table 1 funcref))
                    (import "env" "g" (global $g (mut i32)))
                    (export "memAlias" (memory 0))
                    (export "tblAlias" (table 0))
                    (export "gAlias" (global $g))
                )
                "#,
            )?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const mem = new WebAssembly.Memory({ initial: 1 });
                    const tbl = new WebAssembly.Table({ element: "anyfunc", initial: 1 });
                    const g = new WebAssembly.Global({ value: "i32", mutable: true }, 7);
                    const instance = new WebAssembly.Instance(module, { env: { mem, tbl, g } });
                    return (
                        instance.exports.memAlias === mem &&
                        instance.exports.tblAlias === tbl &&
                        instance.exports.gAlias === g
                    );
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// If `imports[moduleName]` is a getter (or the `imports` object itself
    /// is a `Proxy`) that throws, the thrown value must propagate to the
    /// `new WebAssembly.Instance(...)` call site with its exact original
    /// identity -- it must *not* be replaced by a `LinkError`.
    #[tokio::test]
    async fn import_namespace_getter_thrown_value_identity_is_preserved() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(&ctx, "__wasmBytes", r#"(module (import "env" "f" (func)))"#)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const thrown = { marker: "namespace-getter-probe" };
                    const imports = {
                        get env() { throw thrown; },
                    };
                    try {
                        new WebAssembly.Instance(module, imports);
                        return false;
                    } catch (e) {
                        return e === thrown;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// Same as above, but the throwing getter is on the import *namespace*
    /// object (`imports.env.f`) rather than on `imports` itself.
    #[tokio::test]
    async fn import_property_getter_thrown_value_identity_is_preserved() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(&ctx, "__wasmBytes", r#"(module (import "env" "f" (func)))"#)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const thrown = { marker: "property-getter-probe" };
                    const imports = {
                        env: {
                            get f() { throw thrown; },
                        },
                    };
                    try {
                        new WebAssembly.Instance(module, imports);
                        return false;
                    } catch (e) {
                        return e === thrown;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// Same as above, but via a `Proxy`'s `get` trap wrapping the whole
    /// `imports` object, matching the report's exact repro.
    #[tokio::test]
    async fn import_proxy_get_trap_thrown_value_identity_is_preserved() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(&ctx, "__wasmBytes", r#"(module (import "env" "f" (func)))"#)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const thrown = {};
                    const imports = new Proxy({}, {
                        get() { throw thrown; },
                    });
                    try {
                        new WebAssembly.Instance(module, imports);
                        return false;
                    } catch (e) {
                        return e === thrown;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// Once a `Get` actually succeeds but the resulting value has the wrong
    /// shape (namespace present but not an object), a genuine `LinkError`
    /// must still be thrown -- the fix for the getter-identity issue above
    /// must not turn this case into something else (or swallow it).
    #[tokio::test]
    async fn import_namespace_not_an_object_throws_link_error() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(&ctx, "__wasmBytes", r#"(module (import "env" "f" (func)))"#)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    try {
                        new WebAssembly.Instance(module, { env: 42 });
                        return false;
                    } catch (e) {
                        return e instanceof WebAssembly.LinkError;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// A `Memory` created under one realm must be rejected with a
    /// `LinkError` (not silently accepted, and not a panic/UB from mixing
    /// `wasmi::Store`s) when used as an import while instantiating a
    /// module in a *different* realm -- the scenario the plan calls out as
    /// arising from `vm.runInNewContext`-created child contexts sharing a
    /// `JSRuntime` (and therefore able to pass object references) with the
    /// realm that created them.
    #[tokio::test]
    async fn cross_realm_memory_import_is_rejected_with_link_error() {
        test_sync_with(|ctx| {
            // realm A: install a second, independent `WasmRealm` directly
            // (mirroring `realm::tests::multiple_realms_on_same_runtime_do_not_collide`)
            // and build a `Memory` under it.
            let namespace_a = rquickjs::Object::new(ctx.clone())?;
            let errors_a = crate::errors::install(&ctx, &namespace_a)?;
            let realm_a = std::rc::Rc::new(crate::realm::WasmRealm::new(&ctx, errors_a));
            let host_a = realm_a.state.clone();
            let memory_a = crate::realm::with_context_mut(&realm_a, |store| {
                wasmi::Memory::new(store.as_context_mut(), wasmi::MemoryType::new(1, None)).unwrap()
            });
            let memory_a_obj = crate::memory::wrap_memory(&ctx, &host_a, memory_a)?;
            crate::realm::insert_into_registry(&ctx, realm_a);

            // realm B: this context's *actual* installed `WebAssembly` (via
            // `crate::init`), the one `imports.env.mem` will be checked
            // against.
            crate::init(&ctx)?;
            let namespace_b: rquickjs::Object = ctx.globals().get("WebAssembly")?;
            let _ = namespace_b;

            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"(module (import "env" "mem" (memory 1)))"#,
            )?;
            let imports = rquickjs::Object::new(ctx.clone())?;
            let env = rquickjs::Object::new(ctx.clone())?;
            env.set("mem", memory_a_obj)?;
            imports.set("env", env)?;
            ctx.globals().set("__imports", imports)?;

            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    try {
                        new WebAssembly.Instance(module, globalThis.__imports);
                        return false;
                    } catch (e) {
                        return e instanceof WebAssembly.LinkError;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// An externref held only by an unexported Wasm table is still live. A
    /// registry compaction pass must therefore never discard its JS identity
    /// merely because the table is not exposed as `instance.exports`.
    #[tokio::test]
    async fn unexported_wasm_table_preserves_externref_identity() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            set_wasm_bytes(
                &ctx,
                "__wasmBytes",
                r#"
                (module
                    (table $table 1 externref)
                    (func (export "save") (param externref)
                        (table.set $table (i32.const 0) (local.get 0)))
                    (func (export "load") (result externref)
                        (table.get $table (i32.const 0)))
                )
                "#,
            )?;
            let preserved: bool = ctx.eval(
                r#"
                (() => {
                    const module = new WebAssembly.Module(globalThis.__wasmBytes);
                    const instance = new WebAssembly.Instance(module);
                    const saved = { saved: true };
                    instance.exports.save(saved);

                    // Exercise the old sweep threshold through a separate,
                    // JS-visible table. The value in the Wasm-private table
                    // must remain recoverable afterwards.
                    const churn = new WebAssembly.Table({ element: "externref", initial: 1 });
                    for (let i = 0; i < 300; i++) churn.set(0, { i });

                    return instance.exports.load() === saved;
                })()
                "#,
            )?;
            assert!(preserved);
            Ok(())
        })
        .await;
    }
}
