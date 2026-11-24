// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{cell::RefCell, rc::Rc};

use lexe_utils::io::BYTECODE_FILE_EXT;
use rquickjs::{Ctx, Object, Result, Value};
use tracing::trace;

use crate::CJS_IMPORT_PREFIX;

use super::extensions::load_via_extensions;
use super::facade::{call_resolve_filename, get_or_create_module_record};
use super::import_load::load_via_import;
use super::{ModuleNames, RequireState};

pub fn require_from_module<'js>(
    ctx: Ctx<'js>,
    parent: Object<'js>,
    specifier: String,
    link_parent: bool,
) -> Result<Value<'js>> {
    let ctx_for_names = ctx.clone();
    let module_names = ctx_for_names.userdata::<ModuleNames>();

    let record_parent = if link_parent {
        Some(parent.clone())
    } else {
        None
    };

    // Match pre-facade require: `__cjs:` is an internal loader key, not a resolve input.
    let is_cjs_import = specifier.starts_with(CJS_IMPORT_PREFIX);

    // Node accepts a trailing separator on bare specifiers
    // (`require("string_decoder/")`) and bundled CommonJS helpers emit exactly
    // that. Normalize it away for bare names before resolving; paths keep their
    // separator because it is meaningful there.
    let specifier = if !is_cjs_import
        && !specifier.starts_with('.')
        && !specifier.starts_with('/')
        && !specifier.starts_with("file://")
    {
        specifier.trim_end_matches('/').to_string()
    } else {
        specifier
    };

    let import_name: Rc<str>;
    let import_specifier: Rc<str>;
    let is_builtin;
    let is_bytecode;

    if is_cjs_import {
        is_builtin = false;
        is_bytecode = false;
        import_name = specifier[CJS_IMPORT_PREFIX.len()..].into();
        import_specifier = specifier.clone().into();
    } else {
        if let Some(names) = module_names.as_ref() {
            let trimmed = specifier.trim_end_matches('/');
            if let Some(request) = names.disabled_node_builtin_error_request(trimmed) {
                super::facade::unknown_builtin_module(&ctx, &request)?;
            }
        }

        let resolved = call_resolve_filename(&ctx, &specifier, Some(parent.clone()), None)?;
        is_bytecode = resolved.ends_with(BYTECODE_FILE_EXT);

        if is_bytecode {
            import_name = resolved.clone().into();
            import_specifier = import_name.clone();
            is_builtin = false;
        } else if let Some(names) = module_names.as_ref() {
            let resolved_trimmed = resolved.trim_end_matches('/');
            if let Some(request) = names.disabled_node_builtin_error_request(resolved_trimmed) {
                super::facade::unknown_builtin_module(&ctx, &request)?;
            }
            // Builtins must be looked up with the trailing separator removed:
            // bundled CommonJS helpers `require("string_decoder/")`, and Node
            // accepts that form. Looking up the untrimmed name falls through to
            // the package resolver and fails.
            if let Some(builtin_name) = names.resolve_builtin(resolved_trimmed) {
                is_builtin = true;
                import_name = builtin_name.into();
                import_specifier = import_name.clone();
            } else {
                is_builtin = false;
                import_name = resolved.clone().into();
                import_specifier = import_name.clone();
            }
        } else {
            is_builtin = false;
            import_name = resolved.clone().into();
            import_specifier = import_name.clone();
        }
    };

    trace!("Require resolved: {} -> {}", specifier, import_specifier);

    {
        let facade = ctx
            .userdata::<RefCell<super::facade::ModuleFacadeState>>()
            .unwrap();
        let js_cache = facade.borrow().cache.clone();
        if let Ok(record) = js_cache.get::<_, Object>(import_name.as_ref()) {
            if record.get::<_, bool>("loaded").unwrap_or(false) {
                return record.get("exports");
            }
        }
    }

    {
        let binding = ctx.userdata::<RefCell<RequireState>>().unwrap();
        let mut state = binding.borrow_mut();

        if let Some(cached_value) = state.cache.get(import_name.as_ref()) {
            if ctx
                .userdata::<RefCell<super::facade::ModuleFacadeState>>()
                .unwrap()
                .borrow()
                .cache
                .get::<_, Object>(import_name.as_ref())
                .is_err()
            {
                state.cache.remove(import_name.as_ref());
            } else {
                return Ok(cached_value.clone());
            }
        }

        if let Some(obj) = state.progress.get(&import_name) {
            if let Some(record) = obj.as_object() {
                if let Ok(exports) = record.get::<_, Value>("exports") {
                    return Ok(exports);
                }
            }
            return Ok(obj.clone().into_value());
        }
    }

    if is_builtin || is_bytecode || is_cjs_import {
        let record = get_or_create_module_record(&ctx, import_name.as_ref(), record_parent)?;
        record.set("loaded", false)?;
        return load_via_import(ctx, import_name, import_specifier, record);
    }

    let record = get_or_create_module_record(&ctx, import_name.as_ref(), record_parent)?;
    load_via_extensions(ctx, import_name, import_specifier, record)
}
