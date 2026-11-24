// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{cell::RefCell, fs, rc::Rc};

use lexe_json::parse::json_parse;
use lexe_path as path;
use lexe_utils::{io::SUPPORTED_EXTENSIONS, result::ResultExt};
use rquickjs::{
    context::EvalOptions, function::Args, function::Opt, function::This, Ctx, Exception, Function,
    Object, Result, Value,
};

use super::current_module::CurrentModuleGuard;
use super::facade::ModuleFacadeState;
use super::import_load::load_source_via_import;
use super::RequireState;
use crate::package::resolver::detect_file_format;

/// Build a Node-style `require` closed over this Module instance so free
/// `require` always uses the correct parent (including lazy webpack external
/// factories after CurrentModuleGuard is dropped).
fn make_module_require<'js>(
    ctx: &Ctx<'js>,
    module: &Object<'js>,
    _filename: &str,
) -> Result<Function<'js>> {
    // module.require.bind(module) — same shape as Node's makeRequireFunction.
    ctx.eval::<Function, _>(
        r#"(function (mod) {
            function req(request) { return mod.require(request); }
            req.resolve = function (request, options) {
                return require.resolve(request, options);
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
        })"#,
    )?
    .call((module.clone(),))
}

pub fn extensions_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let facade = ctx.userdata::<RefCell<ModuleFacadeState>>().or_throw(ctx)?;
    let extensions = facade.borrow().extensions.clone();
    Ok(extensions)
}

pub fn static_extension_candidates() -> Vec<String> {
    SUPPORTED_EXTENSIONS
        .iter()
        .map(|ext| (*ext).to_string())
        .collect()
}

pub fn resolve_extension_candidates<'js>(ctx: &Ctx<'js>) -> Result<Vec<String>> {
    let extensions = extensions_object(ctx)?;
    let mut candidates = Vec::new();

    for entry in extensions.props::<String, Value>() {
        let (ext, value) = entry?;
        if value.is_function() {
            candidates.push(ext);
        }
    }

    for ext in SUPPORTED_EXTENSIONS {
        if !candidates.iter().any(|candidate| candidate == ext) {
            candidates.push((*ext).to_string());
        }
    }

    Ok(candidates)
}

pub fn find_longest_registered_extension<'js>(ctx: &Ctx<'js>, filename: &str) -> Result<String> {
    let extensions = extensions_object(ctx)?;
    let basename = path::basename(filename.to_string(), Opt(None));
    let bytes = basename.as_bytes();

    let mut start = 0usize;

    while start < bytes.len() {
        let Some(relative) = bytes[start..].iter().position(|&b| b == b'.') else {
            break;
        };
        let index = start + relative;
        if index == 0 {
            start = index + 1;
            continue;
        }

        let ext = std::str::from_utf8(&bytes[index..]).or_throw(ctx)?;
        if extensions
            .get::<_, Value>(ext)
            .ok()
            .is_some_and(|v| v.is_function())
        {
            return Ok(ext.to_string());
        }
        start = index + 1;
    }

    Ok(".js".to_string())
}

fn strip_shebang(code: &str) -> &str {
    if code.starts_with("#!") {
        code.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
    } else {
        code
    }
}

pub fn module_compile<'js>(
    ctx: Ctx<'js>,
    this: This<Object<'js>>,
    code: String,
    filename: String,
) -> Result<()> {
    let module = this.0;
    let dirname = path::dirname(&filename);
    let exports: Object = module.get("exports")?;
    // Node gives each CJS module its own `require` closed over that Module so
    // free `require` always uses the correct parent — including lazy webpack
    // external factories that run after CurrentModuleGuard is dropped.
    let require = make_module_require(&ctx, &module, &filename)?;

    let _guard = CurrentModuleGuard::push(ctx.clone(), module.clone())?;

    // Match Node: shebang is not part of the CJS wrapper body.
    let code = strip_shebang(&code);

    // After a CJS wrapper parse failure:
    // - explicit ESM (`.mjs` / `type: "module"`) → hand off to the ESM loader
    // - ambiguous typeless/no-scope → allow detect-module ESM fallback
    // - explicit CommonJS (`.cjs` / `type: "commonjs"`) → keep the CJS error
    let allow_esm_loader_fallback = match detect_file_format(&filename) {
        Ok(format) => format.allow_esm_loader_fallback(),
        Err(err) => return Err(err.throw(&ctx)),
    };

    let wrapped =
        format!("(function(exports, require, module, __filename, __dirname) {{\n{code}\n}})");

    // Pass the real filename so relative `import()` and error stacks resolve against
    // this CommonJS file instead of the default `eval_script` script name.
    let mut options = EvalOptions::default();
    options.filename = Some(filename.clone());

    match ctx.eval_with_options::<Function, _>(wrapped.as_str(), options) {
        Ok(compiled) => {
            compiled.call::<_, ()>((
                exports.clone(),
                require,
                module.clone(),
                filename,
                dirname,
            ))?;
            Ok(())
        },
        Err(cjs_err) => {
            if !allow_esm_loader_fallback {
                return Err(cjs_err);
            }
            match load_esm_source_via_import(&ctx, module, &filename, code.to_string()) {
                Ok(()) => Ok(()),
                Err(_) => Err(cjs_err),
            }
        },
    }
}

fn load_esm_source_via_import<'js>(
    ctx: &Ctx<'js>,
    module: Object<'js>,
    filename: &str,
    source: String,
) -> Result<()> {
    let import_name: Rc<str> = filename.into();
    let exports =
        load_source_via_import(ctx.clone(), import_name, filename, source, module.clone())?;
    module.set("exports", exports)?;
    Ok(())
}

pub fn default_js_handler<'js>(ctx: Ctx<'js>, module: Object<'js>, filename: String) -> Result<()> {
    let source = fs::read_to_string(&filename)?;
    let compile: Function = module.get("_compile")?;
    let mut args = Args::new(ctx.clone(), 2);
    args.this(module)?;
    args.push_arg(source)?;
    args.push_arg(filename)?;
    compile.call_arg(args)
}

fn default_json_handler<'js>(ctx: Ctx<'js>, module: Object<'js>, filename: String) -> Result<()> {
    let json = fs::read_to_string(&filename)?;
    let parsed = json_parse(&ctx, json)?;
    module.set("exports", parsed)?;
    Ok(())
}

#[cfg(feature = "napi")]
fn default_node_handler<'js>(ctx: Ctx<'js>, module: Object<'js>, filename: String) -> Result<()> {
    let ctx_ptr = ctx.as_raw();
    let exports: Object = module.get("exports")?;
    let exports_val = exports.clone().into_value().as_raw();

    let new_exports = lexe_napi::dlopen::process_dlopen(ctx_ptr, exports_val, &filename, 0)
        .map_err(|e| Exception::throw_message(&ctx, &e))?;
    unsafe {
        let module_val = module.clone().into_value().as_raw();
        rquickjs::qjs::JS_SetPropertyStr(
            ctx_ptr.as_ptr(),
            module_val,
            c"exports".as_ptr(),
            new_exports,
        );
    }

    Ok(())
}

#[cfg(not(feature = "napi"))]
fn default_node_handler_unsupported<'js>(
    ctx: Ctx<'js>,
    _module: Object<'js>,
    filename: String,
) -> Result<()> {
    Err(Exception::throw_message(
        &ctx,
        &format!(
            "Native addon loading is not supported (build lexe with --features napi): {}",
            filename
        ),
    ))
}

pub fn init_extensions_table<'js>(
    ctx: &Ctx<'js>,
) -> Result<(Object<'js>, Function<'js>, Function<'js>, Function<'js>)> {
    let extensions: Object = ctx.eval("Object.create(null)")?;
    let js_handler = Function::new(ctx.clone(), default_js_handler)?;
    let json_handler = Function::new(ctx.clone(), default_json_handler)?;
    extensions.set(".js", js_handler.clone())?;
    extensions.set(".json", json_handler.clone())?;
    #[cfg(feature = "napi")]
    {
        let node_handler = Function::new(ctx.clone(), default_node_handler)?;
        extensions.set(".node", node_handler.clone())?;
        return Ok((extensions, js_handler, json_handler, node_handler));
    }
    #[cfg(not(feature = "napi"))]
    {
        let node_handler = Function::new(ctx.clone(), default_node_handler_unsupported)?;
        Ok((extensions, js_handler, json_handler, node_handler))
    }
}

fn resolve_extension_handler<'js>(ctx: &Ctx<'js>, extension: &str) -> Result<Function<'js>> {
    let facade = ctx.userdata::<RefCell<ModuleFacadeState>>().or_throw(ctx)?;
    let facade = facade.borrow();
    let extensions = facade.extensions.clone();

    if let Ok(value) = extensions.get::<_, Value>(extension) {
        if let Some(handler) = value.as_function() {
            return Ok(handler.clone());
        }
    }

    match extension {
        // Lexe keeps native handlers when an extension entry is deleted so bare
        // `.js`/`.json` resolution still works; Node leaves the slot empty instead.
        ".js" => Ok(facade.native_js_handler.clone()),
        ".json" => Ok(facade.native_json_handler.clone()),
        ".node" => Ok(facade.native_node_handler.clone()),
        _ => Err(Exception::throw_type(
            ctx,
            "require.extensions handler must be a function",
        )),
    }
}

pub fn call_extension_handler<'js>(
    ctx: &Ctx<'js>,
    module: &Object<'js>,
    filename: &str,
) -> Result<()> {
    let extension = find_longest_registered_extension(ctx, filename)?;
    let handler = resolve_extension_handler(ctx, &extension)?;
    handler.call::<_, ()>((module.clone(), filename.to_string()))?;
    Ok(())
}

fn rollback_failed_load<'js>(ctx: &Ctx<'js>, import_name: &str) -> Result<()> {
    let facade = ctx.userdata::<RefCell<ModuleFacadeState>>().or_throw(ctx)?;
    let js_cache = facade.borrow().cache.clone();
    let _ = js_cache.remove(import_name);

    let binding = ctx.userdata::<RefCell<RequireState>>().or_throw(ctx)?;
    let mut state = binding.borrow_mut();
    state.progress.remove(import_name);
    state.cache.remove(import_name);
    Ok(())
}

pub fn load_via_extensions<'js>(
    ctx: Ctx<'js>,
    import_name: Rc<str>,
    _import_specifier: Rc<str>,
    record: Object<'js>,
) -> Result<Value<'js>> {
    let facade = ctx
        .userdata::<RefCell<ModuleFacadeState>>()
        .or_throw(&ctx)?;
    let js_cache = facade.borrow().cache.clone();

    if record.get::<_, bool>("loaded").unwrap_or(false) {
        return record.get("exports");
    }

    let binding = ctx.userdata::<RefCell<RequireState>>().or_throw(&ctx)?;
    let mut state = binding.borrow_mut();

    if let Some(cached_value) = state.cache.get(import_name.as_ref()) {
        if js_cache.get::<_, Object>(import_name.as_ref()).is_err() {
            state.cache.remove(import_name.as_ref());
        } else {
            return Ok(cached_value.clone());
        }
    }

    if state.progress.contains_key(import_name.as_ref()) {
        return record.get("exports");
    }

    record.set("loaded", false)?;
    state.progress.insert(import_name.clone(), record.clone());
    drop(state);

    if let Err(err) = call_extension_handler(&ctx, &record, import_name.as_ref()) {
        let _ = rollback_failed_load(&ctx, import_name.as_ref());
        return Err(err);
    }

    let exports: Value = record.get("exports")?;
    record.set("loaded", true)?;

    let binding = ctx.userdata::<RefCell<RequireState>>().or_throw(&ctx)?;
    let mut state = binding.borrow_mut();
    state.progress.remove(import_name.as_ref());
    state.cache.insert(import_name.clone(), exports.clone());

    Ok(exports)
}
