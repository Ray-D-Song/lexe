// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{cell::RefCell, collections::HashSet, ffi::CString};

use raster_runtime_path as path;
use raster_runtime_utils::{ctx::CtxExt, result::ResultExt};
use rquickjs::{
    function::{Constructor, Opt},
    prelude::{Func, This},
    qjs, Array, Ctx, Error, Exception, Function, JsLifetime, Object, Result, Value,
};

use crate::modules::path::resolve_path;
use crate::package::resolver::{node_module_paths, require_resolve_with_options};

use super::{
    extensions::{init_extensions_table, module_compile},
    register_hooks, ModuleNames, RequireState,
};

#[derive(JsLifetime)]
pub struct ModuleFacadeState<'js> {
    pub constructor: Constructor<'js>,
    pub cache: Object<'js>,
    pub extensions: Object<'js>,
    pub native_js_handler: Function<'js>,
    pub native_json_handler: Function<'js>,
    pub native_node_handler: Function<'js>,
}

pub fn unknown_builtin_module(ctx: &Ctx<'_>, request: &str) -> Result<()> {
    let message = format!("No such built-in module: {request}");
    let error_ctor: Constructor = ctx.globals().get("Error")?;
    let err: Object = error_ctor.construct((message,))?;
    err.set("code", "ERR_UNKNOWN_BUILTIN_MODULE")?;
    Err(ctx.throw(err.into_value()))
}

pub fn module_not_found(ctx: &Ctx<'_>, request: &str, parent: &str) -> Result<()> {
    let message = format!("Cannot find module '{request}' from '{parent}'");
    let error_ctor: Constructor = ctx.globals().get("Error")?;
    let err: Object = error_ctor.construct((message,))?;
    err.set("code", "MODULE_NOT_FOUND")?;
    Err(ctx.throw(err.into_value()))
}

fn is_builtin_request(ctx: &Ctx<'_>, request: &str) -> bool {
    ctx.userdata::<ModuleNames>()
        .is_some_and(|names| names.is_builtin(request))
}

fn resolve_builtin_request(ctx: &Ctx<'_>, request: &str) -> Option<String> {
    ctx.userdata::<ModuleNames>()?.resolve_builtin(request)
}

fn resolve_builtin_request_for_resolve(ctx: &Ctx<'_>, request: &str) -> Option<String> {
    if request.ends_with('/') {
        return None;
    }
    resolve_builtin_request(ctx, request)
}

fn resolve_require_builtin_request(ctx: &Ctx<'_>, request: &str) -> Option<String> {
    let resolved = resolve_builtin_request_for_resolve(ctx, request)?;
    if request.starts_with("node:") {
        Some(request.to_string())
    } else {
        Some(resolved)
    }
}

pub fn canonical_parent_filename(ctx: &Ctx<'_>, parent: Option<Object<'_>>) -> Result<String> {
    if let Some(parent) = parent {
        if let Ok(filename) = parent.get::<_, String>("filename") {
            return Ok(filename);
        }
        if let Ok(id) = parent.get::<_, String>("id") {
            return Ok(id);
        }
    }

    let name = ctx.get_script_or_module_name()?;
    let name = name.trim_start_matches(super::super::CJS_IMPORT_PREFIX);
    resolve_path([name].iter())
}

fn build_search_paths(ctx: &Ctx<'_>, options: Opt<Object<'_>>) -> Result<Option<Vec<String>>> {
    let options = match options.0 {
        Some(options) => options,
        None => return Ok(None),
    };

    if !options.contains_key("paths")? {
        return Ok(None);
    }

    let paths_value: Value = options.get("paths")?;
    if paths_value.is_null() || paths_value.is_undefined() {
        return Ok(None);
    }

    let paths_array = paths_value.as_array().ok_or_else(|| {
        Exception::throw_type(ctx, "The \"paths\" argument must be of type string[]")
    })?;
    let length = paths_array.len();
    let mut paths = Vec::with_capacity(length);
    // Node 24.3: every entry (including sparse-array holes → undefined) must be a string.
    for index in 0..length {
        let value: Value = paths_array.get(index)?;
        let path = value.as_string().ok_or_else(|| {
            Exception::throw_type(
                ctx,
                &format!("The \"paths[{index}]\" argument must be of type string"),
            )
        })?;
        paths.push(path.to_string()?);
    }

    Ok(Some(paths))
}

pub fn default_resolve_filename(
    ctx: Ctx<'_>,
    request: String,
    parent: Opt<Value<'_>>,
    _is_main: Opt<bool>,
    options: Opt<Object<'_>>,
) -> Result<String> {
    let parent_object = match parent.0 {
        Some(value) if value.is_null() || value.is_undefined() => None,
        Some(value) => value.as_object().cloned(),
        None => None,
    };

    let trimmed = request.trim_end_matches('/');
    if let Some(names) = ctx.userdata::<ModuleNames>() {
        if let Some(builtin_request) = names.disabled_node_builtin_error_request(trimmed) {
            return unknown_builtin_module(&ctx, &builtin_request).map(|_| String::new());
        }
    }

    if let Some(resolved) = resolve_builtin_request_for_resolve(&ctx, &request) {
        return Ok(resolved);
    }

    let parent_filename = canonical_parent_filename(&ctx, parent_object)?;
    let search_paths = build_search_paths(&ctx, options)?;
    let globals = ctx.globals();
    let hooked_fn: Option<Function> = globals.get("__require_hook").ok();

    match require_resolve_with_options(
        &ctx,
        &request,
        &parent_filename,
        hooked_fn,
        false,
        search_paths,
    ) {
        Ok(resolved) => Ok(resolved.into_owned()),
        Err(_) => module_not_found(&ctx, &request, &parent_filename).map(|_| unreachable!()),
    }
}

pub fn call_resolve_filename<'js>(
    ctx: &Ctx<'js>,
    request: &str,
    parent: Option<Object<'js>>,
    options: Option<Object<'js>>,
) -> Result<String> {
    let facade = ctx.userdata::<RefCell<ModuleFacadeState>>().or_throw(ctx)?;
    let constructor = facade.borrow().constructor.clone();
    let resolve_fn: Function = constructor.get("_resolveFilename")?;

    let request = request.to_string();
    let options = match options {
        Some(options) => options.into_value(),
        None => Object::new(ctx.clone())?.into_value(),
    };
    let parent_value = match parent {
        Some(parent) => parent.into_value(),
        None => Value::new_null(ctx.clone()),
    };

    resolve_fn.call((request, parent_value, false, options))
}

fn populate_module_instance<'js>(
    ctx: &Ctx<'js>,
    module: &Object<'js>,
    id: &str,
    filename: &str,
    parent: Option<Object<'js>>,
) -> Result<()> {
    let dir = path::dirname(filename);
    let node_paths = node_module_paths(&dir).unwrap_or_default();
    let paths = Array::new(ctx.clone())?;
    for (index, entry) in node_paths.into_iter().enumerate() {
        paths.set(index, entry)?;
    }
    let exports = Object::new(ctx.clone())?;
    let children = Array::new(ctx.clone())?;

    module.set("id", id)?;
    module.set("filename", filename)?;
    module.set("path", dir)?;
    module.set("paths", paths)?;
    module.set("exports", exports)?;
    module.set("loaded", false)?;
    module.set("children", children)?;
    match parent {
        Some(parent) => module.set("parent", parent)?,
        None => module.set("parent", Value::new_null(ctx.clone()))?,
    }
    Ok(())
}

fn link_child_to_parent<'js>(parent: &Object<'js>, child: &Object<'js>) -> Result<()> {
    let children: Array = parent.get("children")?;
    children.set(children.len(), child.clone())?;
    Ok(())
}

fn module_constructor<'js>(
    ctx: Ctx<'js>,
    id: String,
    parent: Opt<Value<'js>>,
) -> Result<Object<'js>> {
    let parent_object = match parent.0 {
        Some(value) if value.is_null() || value.is_undefined() => None,
        Some(value) => Some(value.into_object().ok_or_else(|| {
            Exception::throw_type(
                &ctx,
                "The \"parent\" argument must be an instance of Module",
            )
        })?),
        None => None,
    };
    let filename = if path::is_absolute(&id) {
        path::replace_backslash(id)
    } else {
        resolve_path([id.as_str()].iter())?
    };
    let module = Object::new(ctx.clone())?;
    populate_module_instance(&ctx, &module, &filename, &filename, parent_object)?;
    Ok(module)
}

/// Clear a pending QuickJS exception left by nested resolve attempts when the
/// overall operation still succeeded. Without this, JS callers observe a
/// phantom `ReferenceError: Error resolving module ...` after a successful
/// `require()` (seen with Next's app-page-turbo load-manifest externals).
fn clear_spurious_resolving_exception(ctx: &Ctx<'_>) {
    if !ctx.has_exception() {
        return;
    }
    let pending = ctx.catch();
    // Intentionally drop. Successful module load is authoritative.
    let _ = pending;
}

fn module_prototype_require<'js>(
    ctx: Ctx<'js>,
    this: This<Object<'js>>,
    request: String,
) -> Result<Value<'js>> {
    let result = super::require::require_from_module(ctx.clone(), this.0.clone(), request, true);
    if result.is_ok() {
        clear_spurious_resolving_exception(&ctx);
    }
    result
}

fn node_module_paths_fn(_ctx: Ctx<'_>, from: String) -> Result<Vec<String>> {
    let from = if from.is_empty() {
        ".".to_string()
    } else {
        from
    };
    node_module_paths(&from)
}

fn require_resolve_fn<'js>(
    ctx: Ctx<'js>,
    request: String,
    options: Opt<Object<'js>>,
    parent: Opt<Value<'js>>,
) -> Result<String> {
    let parent_object = match parent.0 {
        Some(value) if value.is_null() || value.is_undefined() => None,
        Some(value) => value.as_object().cloned(),
        None => None,
    };
    call_resolve_filename(&ctx, &request, parent_object, options.0)
}

fn global_require_impl<'js>(ctx: Ctx<'js>, request: String) -> Result<Value<'js>> {
    let current_module = ctx
        .userdata::<RefCell<RequireState>>()
        .or_throw(&ctx)?
        .borrow()
        .current_module
        .clone();
    let result = if let Some(current) = current_module {
        super::require::require_from_module(ctx.clone(), current, request, true)
    } else {
        let parent_filename = canonical_parent_filename(&ctx, None)?;
        let parent = get_or_create_module_record(&ctx, &parent_filename, None)?;
        super::require::require_from_module(ctx.clone(), parent, request, false)
    };
    if result.is_ok() {
        clear_spurious_resolving_exception(&ctx);
    }
    result
}

fn global_require_resolve_impl<'js>(
    ctx: Ctx<'js>,
    request: String,
    options: Opt<Object<'js>>,
) -> Result<String> {
    if let Some(resolved) = resolve_require_builtin_request(&ctx, &request) {
        return Ok(resolved);
    }
    require_resolve_fn(ctx, request, options, Opt(None))
}

fn require_from_filename<'js>(
    ctx: Ctx<'js>,
    parent_filename: String,
    request: String,
) -> Result<Value<'js>> {
    let parent = get_or_create_module_record(&ctx, &parent_filename, None)?;
    let result = super::require::require_from_module(ctx.clone(), parent, request, true);
    if result.is_ok() {
        clear_spurious_resolving_exception(&ctx);
    }
    result
}

fn resolve_from_filename<'js>(
    ctx: Ctx<'js>,
    parent_filename: String,
    request: String,
    options: Opt<Object<'js>>,
) -> Result<String> {
    if let Some(resolved) = resolve_require_builtin_request(&ctx, &request) {
        return Ok(resolved);
    }
    let parent = get_or_create_module_record(&ctx, &parent_filename, None)?;
    call_resolve_filename(&ctx, &request, Some(parent), options.0)
}

fn install_create_require_factory(ctx: &Ctx<'_>) -> Result<()> {
    ctx.globals()
        .set("__rasterRequireFrom", Func::from(require_from_filename))?;
    ctx.globals()
        .set("__rasterResolveFrom", Func::from(resolve_from_filename))?;
    ctx.eval::<(), _>(
        r#"
        globalThis.__rasterCreateRequire = function(filename) {
            function req(request) {
                return __rasterRequireFrom(filename, request);
            }
            req.resolve = function(request, options) {
                return __rasterResolveFrom(filename, request, options || {});
            };
            Object.defineProperty(req, "cache", {
                get() { return require.cache; },
                configurable: true,
                enumerable: true,
            });
            Object.defineProperty(req, "extensions", {
                get() { return require.extensions; },
                configurable: true,
                enumerable: true,
            });
            return req;
        };
        "#,
    )?;
    Ok(())
}

fn normalize_create_require_filename(filename: String) -> Result<String> {
    let filename = if filename.starts_with("file://") {
        filename.trim_start_matches("file://").to_string()
    } else {
        filename
    };

    if path::is_absolute(&filename) {
        Ok(path::replace_backslash(filename))
    } else {
        resolve_path([filename.as_str()].iter())
    }
}

fn create_require_filename_from_value(ctx: &Ctx<'_>, filename: Value<'_>) -> Result<String> {
    if let Some(url) = filename.as_string() {
        return normalize_create_require_filename(url.to_string()?);
    }

    if let Some(obj) = filename.as_object() {
        if let Ok(href) = obj.get::<_, String>("href") {
            return normalize_create_require_filename(href);
        }
        if let Ok(to_string) = obj.get::<_, Function>("toString") {
            let href = to_string.call::<_, String>(())?;
            return normalize_create_require_filename(href);
        }
    }

    Err(Exception::throw_type(
        ctx,
        "The argument 'filename' must be a file URL object, file URL string, or absolute path string",
    ))
}

fn create_require_impl<'js>(ctx: Ctx<'js>, filename: Value<'js>) -> Result<Value<'js>> {
    let filename = create_require_filename_from_value(&ctx, filename)?;

    let factory: Function = ctx.globals().get("__rasterCreateRequire")?;
    factory.call((filename,))
}

pub fn get_or_create_module_record<'js>(
    ctx: &Ctx<'js>,
    filename: &str,
    parent: Option<Object<'js>>,
) -> Result<Object<'js>> {
    let facade = ctx.userdata::<RefCell<ModuleFacadeState>>().or_throw(ctx)?;
    let constructor = facade.borrow().constructor.clone();
    let cache = facade.borrow().cache.clone();

    if let Ok(record) = cache.get::<_, Object>(filename) {
        return Ok(record);
    }

    let record = module_constructor(
        ctx.clone(),
        filename.to_string(),
        Opt(parent.clone().map(|p| p.into_value())),
    )?;
    let prototype: Object = constructor.get("prototype")?;
    record.set_prototype(Some(&prototype))?;
    cache.set(filename, record.clone())?;

    if let Some(parent) = parent {
        link_child_to_parent(&parent, &record)?;
    }

    Ok(record)
}

/// Names implemented in `module_extras.js` and exported from `node:module`.
pub(super) const MODULE_EXTRA_EXPORTS: [&str; 10] = [
    "SourceMap",
    "findSourceMap",
    "getSourceMapsSupport",
    "setSourceMapsSupport",
    "findPackageJSON",
    "enableCompileCache",
    "flushCompileCache",
    "getCompileCacheDir",
    "constants",
    "syncBuiltinESMExports",
];

const MODULE_EXTRAS_JS: &str = include_str!("module_extras.js");

fn install_module_extras<'js>(ctx: &Ctx<'js>, module_ctor: &Constructor<'js>) -> Result<()> {
    let extras: Object = ctx.eval(MODULE_EXTRAS_JS)?;
    for name in MODULE_EXTRA_EXPORTS {
        if name == "syncBuiltinESMExports" {
            continue;
        }
        module_ctor.set(name, extras.get::<_, Value>(name)?)?;
    }
    module_ctor.set(
        "syncBuiltinESMExports",
        Function::new(ctx.clone(), sync_builtin_esm_exports)?.with_name("syncBuiltinESMExports")?,
    )?;
    Ok(())
}

/// Copy the live properties of each loaded builtin's `module.exports` back into
/// its ES module named bindings, so an override made through `require()` is
/// visible to `import { x } from "node:fs"`.
fn sync_builtin_esm_exports(ctx: Ctx<'_>) -> Result<()> {
    let names: Vec<String> = match ctx.userdata::<ModuleNames>() {
        Some(names) => names
            .ordinary()
            .iter()
            .filter(|name| names.is_builtin(name))
            .cloned()
            .collect(),
        None => Vec::new(),
    };

    extern "C" {
        fn JS_SyncModuleExportsFromDefault(
            ctx: *mut qjs::JSContext,
            module_name: *const std::os::raw::c_char,
        ) -> std::os::raw::c_int;
    }

    for name in names {
        for key in [name.clone(), format!("node:{name}")] {
            let Ok(c_name) = CString::new(key) else {
                continue;
            };
            let written =
                unsafe { JS_SyncModuleExportsFromDefault(ctx.as_raw().as_ptr(), c_name.as_ptr()) };
            if written < 0 {
                return Err(Error::Exception);
            }
        }
    }
    Ok(())
}

pub fn init_module_facade<'js>(
    ctx: &Ctx<'js>,
    module_list: HashSet<String>,
) -> Result<Constructor<'js>> {
    let prototype = Object::new(ctx.clone())?;
    prototype.set("require", Func::from(module_prototype_require))?;
    prototype.set("_compile", Func::from(module_compile))?;

    let module_ctor = Constructor::new_prototype(ctx, prototype, module_constructor)?;

    let builtin_modules = Array::new(ctx.clone())?;
    let mut sorted_modules: Vec<_> = module_list.iter().collect();
    sorted_modules.sort_unstable();
    for (index, name) in sorted_modules.into_iter().enumerate() {
        builtin_modules.set(index, name.as_str())?;
    }

    let cache = Object::new(ctx.clone())?;
    let (extensions, native_js_handler, native_json_handler, native_node_handler) =
        init_extensions_table(ctx)?;
    module_ctor.set("_cache", cache.clone())?;
    module_ctor.set("_extensions", extensions.clone())?;
    module_ctor.set("_nodeModulePaths", Func::from(node_module_paths_fn))?;
    module_ctor.set("_resolveFilename", Func::from(default_resolve_filename))?;
    module_ctor.set("builtinModules", builtin_modules.clone())?;
    module_ctor.set("createRequire", Func::from(create_require_impl))?;
    module_ctor.set(
        "isBuiltin",
        Func::from(|ctx: Ctx<'_>, name: String| -> Result<bool> {
            Ok(is_builtin_request(&ctx, &name))
        }),
    )?;
    module_ctor.set("registerHooks", Func::from(register_hooks))?;
    module_ctor.set("Module", module_ctor.clone())?;
    install_module_extras(ctx, &module_ctor)?;

    install_create_require_factory(ctx)?;

    ctx.store_userdata(RefCell::new(ModuleFacadeState {
        constructor: module_ctor.clone(),
        cache,
        extensions,
        native_js_handler,
        native_json_handler,
        native_node_handler,
    }))?;

    Ok(module_ctor)
}

pub fn init_global_require(ctx: &Ctx<'_>) -> Result<()> {
    let facade = ctx.userdata::<RefCell<ModuleFacadeState>>().or_throw(ctx)?;
    let cache = facade.borrow().cache.clone();
    let extensions = facade.borrow().extensions.clone();

    let require_fn = Function::new(ctx.clone(), global_require_impl)?;
    let resolve_fn = Function::new(ctx.clone(), global_require_resolve_impl)?;
    require_fn.set("resolve", resolve_fn)?;
    require_fn.set("cache", cache.clone())?;
    require_fn.prop("extensions", extensions.clone())?;

    let prototype: Object = facade.borrow().constructor.get("prototype")?;
    let module_require: Function = prototype.get("require")?;
    module_require.set("cache", cache)?;
    module_require.prop("extensions", extensions)?;

    ctx.globals().set("require", require_fn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raster_runtime_test::test_async_with;
    use rquickjs::{Array, CatchResultExt, Object};

    #[tokio::test]
    async fn build_search_paths_valid_strings() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let opts = Object::new(ctx.clone()).unwrap();
                let arr = Array::new(ctx.clone()).unwrap();
                arr.set(0, "/a").unwrap();
                arr.set(1, "/b").unwrap();
                opts.set("paths", arr).unwrap();
                let result = build_search_paths(&ctx, Opt(Some(opts))).unwrap();
                assert_eq!(result, Some(vec!["/a".to_string(), "/b".to_string()]));
            })
        })
        .await;
    }

    #[tokio::test]
    async fn build_search_paths_rejects_non_array() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let opts = Object::new(ctx.clone()).unwrap();
                opts.set("paths", "not-an-array").unwrap();
                let err = build_search_paths(&ctx, Opt(Some(opts)))
                    .catch(&ctx)
                    .unwrap_err();
                let msg = err.to_string();
                assert!(
                    msg.contains("paths") || msg.contains("array") || msg.contains("string"),
                    "unexpected: {msg}"
                );
            })
        })
        .await;
    }

    #[tokio::test]
    async fn build_search_paths_rejects_number_null_undefined() {
        test_async_with(|ctx| {
            Box::pin(async move {
                for (index, bad) in [
                    (0usize, Value::new_int(ctx.clone(), 1)),
                    (0, Value::new_null(ctx.clone())),
                    (0, Value::new_undefined(ctx.clone())),
                ] {
                    let opts = Object::new(ctx.clone()).unwrap();
                    let arr = Array::new(ctx.clone()).unwrap();
                    arr.set(0, bad).unwrap();
                    opts.set("paths", arr).unwrap();
                    let err = build_search_paths(&ctx, Opt(Some(opts)))
                        .catch(&ctx)
                        .unwrap_err();
                    let msg = err.to_string();
                    assert!(
                        msg.contains(&format!("paths[{index}]")),
                        "expected paths[{index}] in: {msg}"
                    );
                }
            })
        })
        .await;
    }

    #[tokio::test]
    async fn build_search_paths_sparse_array_hole_is_type_error() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let opts = Object::new(ctx.clone()).unwrap();
                // Sparse array: length 2, only index 1 set → hole at 0 is undefined.
                let arr: Array = ctx
                    .eval::<Array, _>("(() => { const a = []; a[1] = '/tmp'; return a; })()")
                    .unwrap();
                opts.set("paths", arr).unwrap();
                let err = build_search_paths(&ctx, Opt(Some(opts)))
                    .catch(&ctx)
                    .unwrap_err();
                let msg = err.to_string();
                assert!(
                    msg.contains("paths[0]"),
                    "sparse hole should report paths[0]: {msg}"
                );
            })
        })
        .await;
    }

    #[tokio::test]
    async fn build_search_paths_error_message_exact_index() {
        test_async_with(|ctx| {
            Box::pin(async move {
                let opts = Object::new(ctx.clone()).unwrap();
                let arr = Array::new(ctx.clone()).unwrap();
                arr.set(0, "/ok").unwrap();
                arr.set(1, 42).unwrap();
                opts.set("paths", arr).unwrap();
                let err = build_search_paths(&ctx, Opt(Some(opts)))
                    .catch(&ctx)
                    .unwrap_err();
                let msg = err.to_string();
                assert!(
                    msg.contains("paths[1]"),
                    "error should include exact index: {msg}"
                );
                assert!(
                    msg.contains("must be of type string"),
                    "error should match Node-style message: {msg}"
                );
            })
        })
        .await;
    }
}
