// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Module`: compile-time precheck, the native `Module` class,
//! and `Module.imports`/`exports`/`customSections`.
//!
//! A compiled [`WasmModule`] holds only an (engine-scoped, `Store`-free)
//! `wasmi::Module` plus its original bytes; per the implementation plan this
//! makes it independent of any particular realm's `Store` and safe to keep
//! alive across `Instance` creation/destruction.

use std::sync::Arc;

use rquickjs::{
    atom::PredefinedAtom, class::Trace, Array, ArrayBuffer, Class, Ctx, Object, Result, Value,
};
use wasmi::Module as WasmiModule;

use crate::host_state::HostState;

/// The four kinds of WebAssembly extern, as exposed by the JS-visible
/// `{module, name, kind}` / `{name, kind}` import/export descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternKind {
    Function,
    Table,
    Memory,
    Global,
}

impl ExternKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExternKind::Function => "function",
            ExternKind::Table => "table",
            ExternKind::Memory => "memory",
            ExternKind::Global => "global",
        }
    }
}

/// Runs a `wasmparser::Validator` pass configured with exactly the feature
/// set [`crate::engine::wasm_features`] mirrors, ahead of handing the bytes
/// to `wasmi::Module::new`. This is a defensive, belt-and-suspenders check:
/// it guarantees any input containing an out-of-scope proposal (shared
/// memory, Memory64, tags, GC types, function-references, ...) is rejected
/// with a clean `CompileError` and a descriptive message before those bytes
/// ever reach wasmi's own compilation pipeline.
pub fn precheck(bytes: &[u8]) -> std::result::Result<(), String> {
    let mut validator = wasmparser::Validator::new_with_features(crate::engine::wasm_features());
    validator
        .validate_all(bytes)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// One `{module, name, kind}` entry, in the exact order it appears in the
/// binary's import section.
#[derive(Debug, Clone)]
pub struct ImportDescriptor {
    pub module: String,
    pub name: String,
    pub kind: ExternKind,
}

/// One `{name, kind}` entry, in the exact order it appears in the binary's
/// export section.
#[derive(Debug, Clone)]
pub struct ExportDescriptor {
    pub name: String,
    pub kind: ExternKind,
}

/// Import/export descriptors and custom sections, captured directly from the
/// binary (in binary order) rather than from `wasmi::Module`'s own
/// `imports()`/`exports()` iterators.
///
/// `wasmi::Module` groups its exports/imports by extern kind internally, so
/// its iteration order does *not* match the module's binary declaration
/// order; the spec (and Node/browsers) require `Module.imports`/`exports` to
/// preserve binary order, so this metadata is parsed once, up front, at
/// compile time instead.
#[derive(Debug, Default)]
pub struct ModuleMetadata {
    pub imports: Vec<ImportDescriptor>,
    pub exports: Vec<ExportDescriptor>,
    pub custom_sections: Vec<(String, Vec<u8>)>,
}

fn extract_module_metadata(bytes: &[u8]) -> std::result::Result<ModuleMetadata, String> {
    let mut metadata = ModuleMetadata::default();
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|err| err.to_string())?;
        match payload {
            wasmparser::Payload::ImportSection(reader) => {
                for import in reader {
                    let import = import.map_err(|err| err.to_string())?;
                    metadata.imports.push(ImportDescriptor {
                        module: import.module.to_string(),
                        name: import.name.to_string(),
                        kind: extern_kind_from_type_ref(&import.ty),
                    });
                }
            },
            wasmparser::Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.map_err(|err| err.to_string())?;
                    metadata.exports.push(ExportDescriptor {
                        name: export.name.to_string(),
                        kind: extern_kind_from_external_kind(export.kind),
                    });
                }
            },
            wasmparser::Payload::CustomSection(reader) => {
                metadata
                    .custom_sections
                    .push((reader.name().to_string(), reader.data().to_vec()));
            },
            _ => {},
        }
    }
    Ok(metadata)
}

fn extern_kind_from_type_ref(ty: &wasmparser::TypeRef) -> ExternKind {
    match ty {
        wasmparser::TypeRef::Func(_) => ExternKind::Function,
        wasmparser::TypeRef::Table(_) => ExternKind::Table,
        wasmparser::TypeRef::Memory(_) => ExternKind::Memory,
        wasmparser::TypeRef::Global(_) => ExternKind::Global,
        // Tags are out of scope (exception-handling proposal); `precheck`
        // rejects any module containing one before this ever runs.
        wasmparser::TypeRef::Tag(_) => ExternKind::Function,
    }
}

fn extern_kind_from_external_kind(kind: wasmparser::ExternalKind) -> ExternKind {
    match kind {
        wasmparser::ExternalKind::Func => ExternKind::Function,
        wasmparser::ExternalKind::Table => ExternKind::Table,
        wasmparser::ExternalKind::Memory => ExternKind::Memory,
        wasmparser::ExternalKind::Global => ExternKind::Global,
        // Same rationale as above: unreachable for a module that passed
        // `precheck`.
        wasmparser::ExternalKind::Tag => ExternKind::Function,
    }
}

#[derive(rquickjs::JsLifetime)]
#[rquickjs::class(rename = "Module")]
pub struct WasmModule {
    pub(crate) inner: Arc<WasmiModule>,
    pub(crate) metadata: Arc<ModuleMetadata>,
}

impl<'js> Trace<'js> for WasmModule {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

/// Compiles `source` (a `BufferSource`) into a [`WasmModule`]. Shared by the
/// `new WebAssembly.Module(source)` constructor and the top-level
/// `WebAssembly.compile`/`instantiate` helpers.
pub fn compile_module<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    source: &Value<'js>,
) -> Result<WasmModule> {
    let bytes = crate::buffer_source::extract_buffer_source(ctx, host, source)?;
    precheck(&bytes).map_err(|message| host.throw_compile_error(ctx, message))?;
    let metadata = extract_module_metadata(&bytes)
        .map_err(|message| host.throw_compile_error(ctx, message))?;
    let engine = crate::engine::shared_engine();
    let inner = WasmiModule::new(&engine, &bytes[..])
        .map_err(|err| host.throw_compile_error(ctx, err.to_string()))?;
    Ok(WasmModule {
        inner: Arc::new(inner),
        metadata: Arc::new(metadata),
    })
}

/// `WebAssembly.validate(source)`: same precheck + compile pipeline as
/// [`compile_module`], but reports failure as `false` instead of throwing
/// (except for an illegal, non-`BufferSource` argument, which is still a
/// `TypeError`).
pub fn validate_bytes<'js>(ctx: &Ctx<'js>, host: &HostState, source: &Value<'js>) -> Result<bool> {
    let bytes = crate::buffer_source::extract_buffer_source(ctx, host, source)?;
    if precheck(&bytes).is_err() {
        return Ok(false);
    }
    let engine = crate::engine::shared_engine();
    Ok(WasmiModule::new(&engine, &bytes[..]).is_ok())
}

fn descriptor_object<'js>(
    ctx: &Ctx<'js>,
    module: Option<&str>,
    name: &str,
    kind: ExternKind,
) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    if let Some(module) = module {
        obj.set("module", module)?;
    }
    obj.set("name", name)?;
    obj.set("kind", kind.as_str())?;
    Ok(obj)
}

fn require_module<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    value: &Value<'js>,
) -> Result<Class<'js, WasmModule>> {
    Class::<WasmModule>::from_value(value)
        .map_err(|_| host.throw_type_error(ctx, "expected a WebAssembly.Module"))
}

#[rquickjs::methods]
impl WasmModule {
    #[qjs(constructor)]
    pub fn new<'js>(ctx: Ctx<'js>, source: Value<'js>) -> Result<Self> {
        let host = crate::realm::realm(&ctx)?.state.clone();
        compile_module(&ctx, &host, &source)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "WebAssembly.Module"
    }

    #[qjs(static)]
    pub fn imports<'js>(ctx: Ctx<'js>, module: Value<'js>) -> Result<Array<'js>> {
        let host = crate::realm::realm(&ctx)?.state.clone();
        let class = require_module(&ctx, &host, &module)?;
        let borrow = class.borrow();
        let array = Array::new(ctx.clone())?;
        for (idx, import) in borrow.metadata.imports.iter().enumerate() {
            let descriptor =
                descriptor_object(&ctx, Some(&import.module), &import.name, import.kind)?;
            array.set(idx, descriptor)?;
        }
        Ok(array)
    }

    #[qjs(static)]
    pub fn exports<'js>(ctx: Ctx<'js>, module: Value<'js>) -> Result<Array<'js>> {
        let host = crate::realm::realm(&ctx)?.state.clone();
        let class = require_module(&ctx, &host, &module)?;
        let borrow = class.borrow();
        let array = Array::new(ctx.clone())?;
        for (idx, export) in borrow.metadata.exports.iter().enumerate() {
            let descriptor = descriptor_object(&ctx, None, &export.name, export.kind)?;
            array.set(idx, descriptor)?;
        }
        Ok(array)
    }

    #[qjs(rename = "customSections", static)]
    pub fn custom_sections<'js>(
        ctx: Ctx<'js>,
        module: Value<'js>,
        name: String,
    ) -> Result<Array<'js>> {
        let host = crate::realm::realm(&ctx)?.state.clone();
        let class = require_module(&ctx, &host, &module)?;
        let borrow = class.borrow();
        let array = Array::new(ctx.clone())?;
        let mut idx = 0usize;
        for (section_name, data) in borrow.metadata.custom_sections.iter() {
            if section_name == &name {
                let buffer = ArrayBuffer::new_copy(ctx.clone(), data)?;
                array.set(idx, buffer)?;
                idx += 1;
            }
        }
        Ok(array)
    }
}

/// Wraps an already-compiled `wasmi::Module` (e.g. the module half of
/// `WebAssembly.instantiate(bytes, imports)`'s `{module, instance}` result)
/// as a JS-visible [`WasmModule`].
pub fn wrap_module<'js>(ctx: &Ctx<'js>, module: WasmModule) -> Result<Class<'js, WasmModule>> {
    Class::instance(ctx.clone(), module)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_sync_with;
    use rquickjs::IntoJs;

    const ADD_WAT: &str = r#"
        (module
            (import "env" "log" (func $log (param i32)))
            (memory (export "memory") 1)
            (func (export "add") (param i32 i32) (result i32)
                local.get 0
                local.get 1
                i32.add)
            (export "unused" (func $log))
        )
    "#;

    #[tokio::test]
    async fn precheck_rejects_shared_memory() {
        let wasm = wat::parse_str(r#"(module (memory 1 1 shared))"#).unwrap();
        assert!(precheck(&wasm).is_err());
    }

    #[tokio::test]
    async fn precheck_rejects_malformed_bytes() {
        let bytes = vec![0x00, 0x01, 0x02, 0x03];
        assert!(precheck(&bytes).is_err());
    }

    #[tokio::test]
    async fn imports_and_exports_preserve_binary_order() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let bytes = wat::parse_str(ADD_WAT).unwrap();
            let engine = crate::engine::shared_engine();
            let inner = WasmiModule::new(&engine, &bytes[..]).unwrap();
            let metadata = extract_module_metadata(&bytes).unwrap();
            let module = WasmModule {
                inner: Arc::new(inner),
                metadata: Arc::new(metadata),
            };
            let class = Class::instance(ctx.clone(), module)?;
            let value: Value = class.clone().into_js(&ctx)?;
            let _ = &realm;

            let imports = WasmModule::imports(ctx.clone(), value.clone())?;
            assert_eq!(imports.len(), 1);
            let first: Object = imports.get(0)?;
            let module_name: String = first.get("module")?;
            assert_eq!(module_name, "env");

            let exports = WasmModule::exports(ctx.clone(), value)?;
            assert_eq!(exports.len(), 3);
            let e0: Object = exports.get(0)?;
            let name0: String = e0.get("name")?;
            assert_eq!(name0, "memory");
            Ok(())
        })
        .await;
    }
}
