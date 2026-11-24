// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{cell::RefCell, rc::Rc};

use lexe_hooking::{invoke_async_hook, register_finalization_registry, HookType};
use lexe_utils::{provider::ProviderType, result::ResultExt};
use rquickjs::{atom::PredefinedAtom, qjs, Ctx, Module, Object, Result, Value};
use tokio::time::Instant;
use tracing::trace;

use crate::modules::timers::poll_timers;
use crate::package::loader::prepend_cjs_dirname_filename;

use super::current_module::CurrentModuleGuard;
use super::facade::ModuleFacadeState;
use super::{ModuleCache, ModuleNames, RequireState};

fn is_builtin_import_name(ctx: &Ctx<'_>, import_name: &str) -> bool {
    ctx.userdata::<ModuleNames>()
        .is_some_and(|names| names.is_builtin(import_name.trim_end_matches('/')))
}

fn collect_imported_exports<'js>(
    ctx: &Ctx<'js>,
    import_name: &Rc<str>,
    record: &Object<'js>,
    progress: &Object<'js>,
    imported_object: Object<'js>,
) -> Result<Value<'js>> {
    let binding = ctx.userdata::<RefCell<RequireState>>().unwrap();
    let exports_obj = binding.borrow().exports.get(import_name).cloned();

    if let Some(exports_obj) = exports_obj {
        if exports_obj.type_of() == rquickjs::Type::Object {
            let exports = unsafe { exports_obj.as_object().unwrap_unchecked() };
            for prop in exports
                .own_props::<Value, Value>(rquickjs::Filter::new().private().string().symbol())
            {
                let (key, value) = prop?;
                progress.set(key, value)?;
            }
        } else {
            record.set("exports", exports_obj.clone())?;
            return Ok(exports_obj);
        }
    }

    let props = imported_object.props::<String, Value>();
    let has_default = imported_object.contains_key(PredefinedAtom::Default)?;
    let default_export: Option<Value> = if has_default {
        Some(imported_object.get(PredefinedAtom::Default)?)
    } else {
        None
    };
    let builtin = is_builtin_import_name(ctx, import_name);

    // Built-in modules (e.g. legacy `constants`) expose a designed flat default
    // object as module.exports. Merge missing named exports onto that object,
    // but never attach a synthetic self-referential `default` property — that
    // would fail on frozen defaults and is not how Node surfaces those builtins.
    //
    // User ESM files instead use namespace-style interop: keep `default` and
    // named exports on a fresh exports object so `require("./x.mjs").default`
    // remains available (Babel/TS interop) without mutating a frozen default export.
    if builtin {
        if let Some(default_export) = default_export {
            if let Some(default_object) = default_export.as_object() {
                for prop in props {
                    let (key, value) = prop?;
                    if key == "default" {
                        continue;
                    }
                    if !default_object.contains_key(&key)? {
                        default_object.set(key, value)?;
                    }
                }
                let default_object = default_object.clone().into_value();
                record.set("exports", default_object.clone())?;
                return Ok(default_object);
            }
        }
    }

    // Namespace-style CJS interop for user ESM (and non-object builtin defaults).
    // Match Node: only synthesize `__esModule: true` when the module has a default
    // export and did not export `__esModule` itself. Named-only ESM must not gain it.
    for prop in props {
        let (key, value) = prop?;
        progress.set(key, value)?;
    }
    if has_default && !progress.contains_key("__esModule")? {
        progress.set("__esModule", true)?;
    }

    let value = progress.clone().into_value();
    record.set("exports", value.clone())?;
    Ok(value)
}

fn wait_for_import_promise<'js>(
    ctx: &Ctx<'js>,
    import_promise: rquickjs::Promise<'js>,
) -> Result<()> {
    let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let mut deadline = Instant::now();
    let mut executing_timers = Vec::new();

    loop {
        if let Some(result) = import_promise.result::<Value>() {
            result?;
            break;
        }

        if deadline < Instant::now() {
            poll_timers(rt, &mut executing_timers, None, Some(&mut deadline))?;
        }

        ctx.execute_pending_job();
    }

    Ok(())
}

pub fn load_source_via_import<'js>(
    ctx: Ctx<'js>,
    import_name: Rc<str>,
    filename: &str,
    source: String,
    record: Object<'js>,
) -> Result<Value<'js>> {
    trace!("Require loading inline source for: {}", filename);

    let _current_module_guard = CurrentModuleGuard::push(ctx.clone(), record.clone())?;

    let progress = Object::new(ctx.clone())?;
    let uid = unsafe { qjs::JS_VALUE_GET_PTR(progress.as_object().unwrap().as_raw()) } as usize;
    register_finalization_registry(&ctx, progress.clone().into_value(), uid, None)?;
    invoke_async_hook(&ctx, HookType::Init, ProviderType::TimerWrap, uid)?;

    let declared_source = prepend_cjs_dirname_filename(filename, source.as_bytes())?;
    let module = Module::declare(ctx.clone(), filename, declared_source)?;
    if let Some(binding) = ctx.userdata::<RefCell<ModuleCache>>() {
        binding
            .borrow_mut()
            .esm
            .insert(filename.into(), module.clone());
    }
    let (evaluated_module, import_promise) = module.eval()?;
    wait_for_import_promise(&ctx, import_promise)?;

    let imported_object = evaluated_module.namespace()?;
    collect_imported_exports(&ctx, &import_name, &record, &progress, imported_object)
}

pub fn load_file_via_import<'js>(
    ctx: Ctx<'js>,
    import_name: Rc<str>,
    import_specifier: Rc<str>,
    record: Object<'js>,
) -> Result<Value<'js>> {
    trace!("Require loading: {}", import_specifier);

    let _current_module_guard = CurrentModuleGuard::push(ctx.clone(), record.clone())?;

    let progress = Object::new(ctx.clone())?;
    let uid = unsafe { qjs::JS_VALUE_GET_PTR(progress.as_object().unwrap().as_raw()) } as usize;
    register_finalization_registry(&ctx, progress.clone().into_value(), uid, None)?;
    invoke_async_hook(&ctx, HookType::Init, ProviderType::TimerWrap, uid)?;

    let import_promise = Module::import(&ctx, import_specifier.as_bytes())?;
    let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };

    let mut deadline = Instant::now();
    let mut executing_timers = Vec::new();

    let imported_object = loop {
        if let Some(x) = import_promise.result::<Object>() {
            break x?;
        }

        if deadline < Instant::now() {
            poll_timers(rt, &mut executing_timers, None, Some(&mut deadline))?;
        }

        ctx.execute_pending_job();
    };

    collect_imported_exports(&ctx, &import_name, &record, &progress, imported_object)
}

fn rollback_failed_import<'js>(ctx: &Ctx<'js>, import_name: &str) -> Result<()> {
    let facade = ctx.userdata::<RefCell<ModuleFacadeState>>().or_throw(ctx)?;
    let js_cache = facade.borrow().cache.clone();
    let _ = js_cache.remove(import_name);

    let binding = ctx.userdata::<RefCell<RequireState>>().or_throw(ctx)?;
    let mut state = binding.borrow_mut();
    state.progress.remove(import_name);
    state.cache.remove(import_name);
    Ok(())
}

pub fn load_via_import<'js>(
    ctx: Ctx<'js>,
    import_name: Rc<str>,
    import_specifier: Rc<str>,
    record: Object<'js>,
) -> Result<Value<'js>> {
    let binding = ctx.userdata::<RefCell<RequireState>>().unwrap();
    let obj = Object::new(ctx.clone())?;
    binding
        .borrow_mut()
        .progress
        .insert(import_name.clone(), obj.clone());
    drop(binding);

    let load_result = load_file_via_import(
        ctx.clone(),
        import_name.clone(),
        import_specifier,
        record.clone(),
    );

    match load_result {
        Ok(value) => {
            record.set("loaded", true)?;

            let binding = ctx.userdata::<RefCell<RequireState>>().unwrap();
            let mut state = binding.borrow_mut();
            state.progress.remove(import_name.as_ref());
            state.cache.insert(import_name, value.clone());
            Ok(value)
        },
        Err(err) => {
            let _ = rollback_failed_import(&ctx, import_name.as_ref());
            Err(err)
        },
    }
}
