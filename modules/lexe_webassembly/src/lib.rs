// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `lexe_webassembly`: the core WebAssembly JavaScript API
//! (`WebAssembly.compile`/`instantiate`/`validate`/`compileStreaming`/
//! `instantiateStreaming`, `Module`/`Instance`/`Memory`/`Table`/`Global`,
//! `CompileError`/`LinkError`/`RuntimeError`), backed by `wasmi` 1.1.0.
//!
//! See `API.md` at the workspace root for the full list of what is (and,
//! explicitly, is not) implemented, the JS/Wasm numeric value mapping, and
//! the `Memory` synchronous-mirror model.
//!
//! [`init`] is the single entry point this crate exposes to
//! `lexe_modules`/`lexe_core`: it installs a fresh,
//! independent `WebAssembly` global (and its private per-realm `wasmi::Store`
//! state, see `realm`) on the given QuickJS context.

mod buffer_source;
mod descriptor;
mod engine;
mod errors;
mod func_wrapper;
mod global;
mod host_state;
mod instance;
mod memory;
mod module;
mod realm;
mod store_access;
mod table;
mod top_level;
mod value_conv;

use std::{any::Any, sync::Arc};

use rquickjs::{
    atom::PredefinedAtom, class::Class, object::Property, Ctx, IntoJs, Object, Result, Value,
};

use global::WasmGlobal;
use instance::WasmInstance;
use memory::WasmMemory;
use module::WasmModule;
use table::WasmTable;

/// Registers native class `C` (JS-visible name `C::NAME`, e.g. `"Module"`) as
/// a non-enumerable, writable, configurable data property on `namespace`,
/// matching Node's own property descriptors for these constructors (and the
/// pattern already used for `CompileError`/`LinkError`/`RuntimeError` in
/// [`errors::install`]) rather than `Class::define`'s plain (enumerable)
/// assignment.
fn install_class<'js, C: rquickjs::class::JsClass<'js> + 'js>(
    ctx: &Ctx<'js>,
    namespace: &Object<'js>,
) -> Result<()> {
    let ctor = Class::<C>::create_constructor(ctx)?
        .expect("WebAssembly native classes always define a constructor");
    namespace.prop(C::NAME, Property::from(ctor).writable().configurable())?;
    Ok(())
}

/// Installs a fresh `WebAssembly` global (namespace object, native classes,
/// error subclasses, and top-level methods) plus this realm's private
/// `wasmi::Store` state (see [`realm::install`]) on `ctx`.
///
/// Must be called at most once per QuickJS context. Every `Instance`/
/// `Memory`/`Table`/`Global`/exported-function/callback object created
/// through the resulting `WebAssembly` is scoped to (and only usable
/// within) this one context; using one from a *different* context throws a
/// `LinkError` (see `realm::WasmRealm`'s per-object `realm_id` checks).
fn extract_wasm_module_handle(
    _ctx: &Ctx<'_>,
    value: &Value<'_>,
    _for_wire: bool,
) -> Result<Option<Arc<dyn Any + Send + Sync>>> {
    let Ok(cls) = Class::<WasmModule>::from_value(value) else {
        return Ok(None);
    };
    let borrow = cls.borrow();
    Ok(Some(Arc::new((
        Arc::clone(&borrow.inner),
        Arc::clone(&borrow.metadata),
    ))))
}

fn rebuild_wasm_module_value<'js>(
    ctx: &Ctx<'js>,
    handle: &Arc<dyn Any + Send + Sync>,
) -> Result<Option<Value<'js>>> {
    let Some(pair) = handle.downcast_ref::<(Arc<wasmi::Module>, Arc<module::ModuleMetadata>)>()
    else {
        return Ok(None);
    };
    let instance = Class::instance(
        ctx.clone(),
        WasmModule {
            inner: Arc::clone(&pair.0),
            metadata: Arc::clone(&pair.1),
        },
    )?;
    Ok(Some(instance.into_js(ctx)?))
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    lexe_utils::clone_wire::register_native_codec(lexe_utils::clone_wire::NativeCloneCodec {
        name: "WasmModule",
        transfer_only: false,
        try_extract: extract_wasm_module_handle,
        try_rebuild: rebuild_wasm_module_value,
    });

    let namespace = Object::new(ctx.clone())?;

    let error_constructors = errors::install(ctx, &namespace)?;
    let _realm = realm::install(ctx, error_constructors)?;

    install_class::<WasmModule>(ctx, &namespace)?;
    install_class::<WasmInstance<'_>>(ctx, &namespace)?;
    install_class::<WasmMemory>(ctx, &namespace)?;
    install_class::<WasmTable>(ctx, &namespace)?;
    install_class::<WasmGlobal>(ctx, &namespace)?;

    namespace.prop(
        PredefinedAtom::SymbolToStringTag,
        Property::from("WebAssembly").configurable(),
    )?;

    // `top_level::install`'s JS glue resolves `WebAssembly` by reading this
    // global, so the namespace must already be published before it runs.
    ctx.globals().set("WebAssembly", namespace.clone())?;

    top_level::install(ctx, &namespace)?;

    Ok(())
}
