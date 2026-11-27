// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Module`: compile-time validation, binary metadata extraction,
//! the native `Module` class, and `Module.imports`/`exports`/`customSections`.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use rquickjs::{
    atom::PredefinedAtom, class::Trace, Array, ArrayBuffer, Class, Ctx, Object, Result, Value,
};

use crate::host_state::HostState;

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

/// Full WebAssembly value type extracted from the binary type section,
/// including `V128` so JS/Wasm boundary checks throw `TypeError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleValType {
    I32,
    I64,
    F32,
    F64,
    V128,
    FuncRef,
    ExternRef,
}

impl ModuleValType {
    pub fn to_wasmz_kind(self) -> Option<wasmz::ValKind> {
        match self {
            Self::I32 => Some(wasmz::ValKind::I32),
            Self::I64 => Some(wasmz::ValKind::I64),
            Self::F32 => Some(wasmz::ValKind::F32),
            Self::F64 => Some(wasmz::ValKind::F64),
            Self::FuncRef => Some(wasmz::ValKind::FuncRef),
            Self::ExternRef => Some(wasmz::ValKind::ExternRef),
            Self::V128 => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FuncSigMeta {
    pub params: Vec<ModuleValType>,
    pub results: Vec<ModuleValType>,
}

#[derive(Debug, Clone)]
pub struct ImportDescriptor {
    pub module: String,
    pub name: String,
    pub kind: ExternKind,
    pub func_sig: Option<FuncSigMeta>,
}

#[derive(Debug, Clone)]
pub struct ExportDescriptor {
    pub name: String,
    pub kind: ExternKind,
    pub index: u32,
    pub func_sig: Option<FuncSigMeta>,
    pub table_elem: Option<ModuleValType>,
    pub global_type: Option<(ModuleValType, bool)>,
}

#[derive(Debug, Default)]
pub struct ModuleMetadata {
    pub imports: Vec<ImportDescriptor>,
    pub exports: Vec<ExportDescriptor>,
    pub custom_sections: Vec<(String, Vec<u8>)>,
    pub imported_func_count: u32,
    func_sigs: Vec<FuncSigMeta>,
    table_elems: Vec<ModuleValType>,
    global_types: Vec<(ModuleValType, bool)>,
}

fn map_val_type(ty: wasmparser::ValType) -> ModuleValType {
    match ty {
        wasmparser::ValType::I32 => ModuleValType::I32,
        wasmparser::ValType::I64 => ModuleValType::I64,
        wasmparser::ValType::F32 => ModuleValType::F32,
        wasmparser::ValType::F64 => ModuleValType::F64,
        wasmparser::ValType::V128 => ModuleValType::V128,
        wasmparser::ValType::Ref(rt) => {
            if rt.is_extern_ref() {
                ModuleValType::ExternRef
            } else {
                ModuleValType::FuncRef
            }
        },
    }
}

fn map_func_type(ft: &wasmparser::FuncType) -> FuncSigMeta {
    FuncSigMeta {
        params: ft.params().iter().copied().map(map_val_type).collect(),
        results: ft.results().iter().copied().map(map_val_type).collect(),
    }
}

/// Validates raw bytes via `wasmparser` feature gating + `wasmz::Engine`.
pub fn precheck(bytes: &[u8]) -> std::result::Result<(), String> {
    let mut validator = wasmparser::Validator::new_with_features(crate::engine::wasm_features());
    validator
        .validate_all(bytes)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn extract_module_metadata(bytes: &[u8]) -> std::result::Result<ModuleMetadata, String> {
    let mut metadata = ModuleMetadata::default();
    let mut type_sigs: HashMap<usize, FuncSigMeta> = HashMap::new();
    let mut next_type_idx = 0usize;

    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|err| err.to_string())?;
        match payload {
            wasmparser::Payload::TypeSection(reader) => {
                for rec_group in reader {
                    let rec_group = rec_group.map_err(|err| err.to_string())?;
                    for sub_type in rec_group.into_types() {
                        if let wasmparser::CompositeInnerType::Func(ft) = &sub_type.composite_type.inner {
                            type_sigs.insert(next_type_idx, map_func_type(ft));
                        }
                        next_type_idx += 1;
                    }
                }
            },
            wasmparser::Payload::ImportSection(reader) => {
                for import in reader {
                    let import = import.map_err(|err| err.to_string())?;
                    let mut func_sig = None;
                    let kind = match import.ty {
                        wasmparser::TypeRef::Func(type_idx) => {
                            let sig = type_sigs
                                .get(&(type_idx as usize))
                                .cloned()
                                .unwrap_or_default();
                            metadata.func_sigs.push(sig.clone());
                            metadata.imported_func_count += 1;
                            func_sig = Some(sig);
                            ExternKind::Function
                        },
                        wasmparser::TypeRef::Table(tt) => {
                            let elem = map_val_type(wasmparser::ValType::Ref(tt.element_type));
                            metadata.table_elems.push(elem);
                            ExternKind::Table
                        },
                        wasmparser::TypeRef::Memory(_) => ExternKind::Memory,
                        wasmparser::TypeRef::Global(gt) => {
                            let pair = (map_val_type(gt.content_type), gt.mutable);
                            metadata.global_types.push(pair);
                            ExternKind::Global
                        },
                        wasmparser::TypeRef::Tag(_) => ExternKind::Function,
                    };
                    metadata.imports.push(ImportDescriptor {
                        module: import.module.to_string(),
                        name: import.name.to_string(),
                        kind,
                        func_sig,
                    });
                }
            },
            wasmparser::Payload::FunctionSection(reader) => {
                for type_idx in reader {
                    let type_idx = type_idx.map_err(|err| err.to_string())?;
                    let sig = type_sigs
                        .get(&(type_idx as usize))
                        .cloned()
                        .unwrap_or_default();
                    metadata.func_sigs.push(sig);
                }
            },
            wasmparser::Payload::TableSection(reader) => {
                for table in reader {
                    let table = table.map_err(|err| err.to_string())?;
                    let elem = map_val_type(wasmparser::ValType::Ref(table.ty.element_type));
                    metadata.table_elems.push(elem);
                }
            },
            wasmparser::Payload::GlobalSection(reader) => {
                for global in reader {
                    let global = global.map_err(|err| err.to_string())?;
                    metadata
                        .global_types
                        .push((map_val_type(global.ty.content_type), global.ty.mutable));
                }
            },
            wasmparser::Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.map_err(|err| err.to_string())?;
                    let (kind, func_sig, table_elem, global_type) = match export.kind {
                        wasmparser::ExternalKind::Func => (
                            ExternKind::Function,
                            metadata.func_sigs.get(export.index as usize).cloned(),
                            None,
                            None,
                        ),
                        wasmparser::ExternalKind::Table => (
                            ExternKind::Table,
                            None,
                            metadata.table_elems.get(export.index as usize).copied(),
                            None,
                        ),
                        wasmparser::ExternalKind::Memory => (ExternKind::Memory, None, None, None),
                        wasmparser::ExternalKind::Global => (
                            ExternKind::Global,
                            None,
                            None,
                            metadata.global_types.get(export.index as usize).copied(),
                        ),
                        wasmparser::ExternalKind::Tag => (ExternKind::Function, None, None, None),
                    };
                    metadata.exports.push(ExportDescriptor {
                        name: export.name.to_string(),
                        kind,
                        index: export.index,
                        func_sig,
                        table_elem,
                        global_type,
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

#[derive(rquickjs::JsLifetime)]
#[rquickjs::class(rename = "Module")]
pub struct WasmModule {
    pub(crate) inner: Rc<wasmz::Module>,
    pub(crate) bytes: Arc<Vec<u8>>,
    pub(crate) metadata: Arc<ModuleMetadata>,
}

impl<'js> Trace<'js> for WasmModule {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

pub fn compile_module<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    source: &Value<'js>,
) -> Result<WasmModule> {
    let bytes = crate::buffer_source::extract_buffer_source(ctx, host, source)?;
    precheck(&bytes).map_err(|message| host.throw_compile_error(ctx, message))?;
    let metadata = extract_module_metadata(&bytes)
        .map_err(|message| host.throw_compile_error(ctx, message))?;
    let realm = crate::realm::realm(ctx)?;
    let inner = wasmz::Module::compile(&realm.engine, &bytes)
        .map_err(|err| host.throw_compile_error(ctx, err.to_string()))?;
    Ok(WasmModule {
        inner: Rc::new(inner),
        bytes: Arc::new(bytes),
        metadata: Arc::new(metadata),
    })
}

pub fn validate_bytes<'js>(ctx: &Ctx<'js>, host: &HostState, source: &Value<'js>) -> Result<bool> {
    let bytes = crate::buffer_source::extract_buffer_source(ctx, host, source)?;
    if precheck(&bytes).is_err() {
        return Ok(false);
    }
    let Ok(realm) = crate::realm::realm(ctx) else {
        return Ok(wasmz::validate(&bytes).is_ok());
    };
    Ok(realm.engine.validate(&bytes).is_ok())
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
    async fn precheck_accepts_wasmz_proposals_and_rejects_malformed() {
        let shared = wat::parse_str(r#"(module (memory 1 1 shared))"#).unwrap();
        assert!(precheck(&shared).is_ok());
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
            let inner = wasmz::Module::compile(&realm.engine, &bytes).unwrap();
            let metadata = extract_module_metadata(&bytes).unwrap();
            let module = WasmModule {
                inner: Rc::new(inner),
                bytes: Arc::new(bytes),
                metadata: Arc::new(metadata),
            };
            let class = Class::instance(ctx.clone(), module)?;
            let value: Value = class.clone().into_js(&ctx)?;

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
