// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    rc::Rc,
};

use raster_runtime_utils::{ctx::CtxExt, module::ModuleInfo, result::ResultExt};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    object::Accessor,
    prelude::Func,
    qjs, Ctx, Error, Exception, Function, JsLifetime, Module, Object, Result, Value,
};

mod current_module;
pub mod extensions;
pub mod facade;
pub mod import_load;
pub mod loader;
mod require;
pub mod resolver;

use crate::CJS_IMPORT_PREFIX;

use facade::init_global_require;
use facade::init_module_facade;

#[derive(JsLifetime)]
pub struct ModuleNames<'js> {
    ordinary: HashSet<String>,
    enabled_node_only: HashSet<String>,
    disabled_node_only: HashSet<String>,
    _marker: PhantomData<&'js ()>,
}

impl ModuleNames<'_> {
    pub fn new(
        ordinary: HashSet<String>,
        enabled_node_only: HashSet<String>,
        disabled_node_only: HashSet<String>,
    ) -> Self {
        Self {
            ordinary,
            enabled_node_only,
            disabled_node_only,
            _marker: PhantomData,
        }
    }

    pub fn ordinary(&self) -> &HashSet<String> {
        &self.ordinary
    }

    pub fn enabled_node_only(&self) -> &HashSet<String> {
        &self.enabled_node_only
    }

    pub fn disabled_node_only(&self) -> &HashSet<String> {
        &self.disabled_node_only
    }

    pub fn get_list(&self) -> HashSet<String> {
        let mut list = self.ordinary.clone();
        for name in &self.enabled_node_only {
            list.insert(format!("node:{name}"));
        }
        for name in &self.disabled_node_only {
            list.insert(format!("node:{name}"));
        }
        list
    }

    pub fn is_builtin(&self, request: &str) -> bool {
        let request = request.trim_end_matches('/');
        if let Some(bare) = request.strip_prefix("node:") {
            if self.disabled_node_only.contains(bare) {
                return false;
            }
            if self.enabled_node_only.contains(bare) {
                return true;
            }
            return self.ordinary.contains(bare);
        }
        self.ordinary.contains(request) && !self.enabled_node_only.contains(request)
    }

    pub fn is_known_disabled(&self, request: &str) -> bool {
        let request = request.trim_end_matches('/');
        if let Some(bare) = request.strip_prefix("node:") {
            return self.disabled_node_only.contains(bare);
        }
        self.disabled_node_only.contains(request)
    }

    pub fn disabled_node_builtin_error_request(&self, request: &str) -> Option<String> {
        let request = request.trim_end_matches('/');
        let bare = request.strip_prefix("node:")?;
        if self.disabled_node_only.contains(bare) {
            Some(request.to_string())
        } else {
            None
        }
    }

    pub fn resolve_builtin(&self, request: &str) -> Option<String> {
        let request = request.trim_end_matches('/');
        if let Some(bare) = request.strip_prefix("node:") {
            if self.disabled_node_only.contains(bare) {
                return None;
            }
            if self.enabled_node_only.contains(bare) {
                return Some(format!("node:{bare}"));
            }
            if self.ordinary.contains(bare) {
                return Some(bare.to_string());
            }
            return None;
        }
        if self.disabled_node_only.contains(request) {
            return None;
        }
        if self.ordinary.contains(request) && !self.enabled_node_only.contains(request) {
            return Some(request.to_string());
        }
        None
    }
}

#[derive(Default)]
pub struct RequireState<'js> {
    pub cache: HashMap<Rc<str>, Value<'js>>,
    pub exports: HashMap<Rc<str>, Value<'js>>,
    pub progress: HashMap<Rc<str>, Object<'js>>,
    pub current_module: Option<Object<'js>>,
}

unsafe impl<'js> JsLifetime<'js> for RequireState<'js> {
    type Changed<'to> = RequireState<'to>;
}

#[derive(Clone, JsLifetime)]
struct Hook<'js> {
    resolve: Option<Function<'js>>,
    load: Option<Function<'js>>,
}

#[derive(JsLifetime)]
pub struct ModuleHookState<'js> {
    hooks: Vec<Hook<'js>>,
}

impl Default for ModuleHookState<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleHookState<'_> {
    fn new() -> Self {
        Self { hooks: Vec::new() }
    }
}

#[derive(Default)]
pub(crate) struct ModuleCache<'js> {
    pub(crate) esm: HashMap<Rc<str>, Module<'js>>,
}

unsafe impl<'js> JsLifetime<'js> for ModuleCache<'js> {
    type Changed<'to> = ModuleCache<'to>;
}

pub struct ModuleModule;

fn is_builtin(ctx: Ctx<'_>, name: String) -> Result<bool> {
    let module_names = ctx
        .userdata::<ModuleNames>()
        .ok_or_else(|| Exception::throw_reference(&ctx, "is_builtin is not supported"))?;

    Ok(module_names.is_builtin(name.trim_end_matches('/')))
}

pub fn register_hooks<'js>(ctx: Ctx<'js>, hooks_obj: Object<'js>) -> Result<()> {
    let resolve = hooks_obj.get::<_, Function>("resolve").ok();
    let load = hooks_obj.get::<_, Function>("load").ok();

    let hook = Hook { resolve, load };

    let binding = ctx.userdata::<RefCell<ModuleHookState>>().or_throw(&ctx)?;
    let mut state = binding.borrow_mut();
    state.hooks.push(hook);

    Ok(())
}

impl ModuleDef for ModuleModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("Module")?;
        declare.declare("builtinModules")?;
        declare.declare("createRequire")?;
        declare.declare("isBuiltin")?;
        declare.declare("registerHooks")?;
        declare.declare("_nodeModulePaths")?;
        declare.declare("_resolveFilename")?;
        declare.declare("_cache")?;
        declare.declare("_extensions")?;
        for name in facade::MODULE_EXTRA_EXPORTS {
            declare.declare(name)?;
        }
        declare.declare("default")?;

        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let module_list = ctx
            .userdata::<ModuleNames>()
            .map_or_else(HashSet::new, |v| v.get_list());

        let module_ctor = if let Some(facade) = ctx.userdata::<RefCell<facade::ModuleFacadeState>>()
        {
            facade.borrow().constructor.clone()
        } else {
            init_module_facade(ctx, module_list.clone())?
        };

        exports.export("default", module_ctor.clone())?;
        // Named `Module` is the same constructor as `default` / `require("module").Module`.
        exports.export("Module", module_ctor.clone())?;
        exports.export(
            "builtinModules",
            module_ctor.get::<_, Value>("builtinModules")?,
        )?;
        exports.export(
            "createRequire",
            module_ctor.get::<_, Value>("createRequire")?,
        )?;
        exports.export("isBuiltin", Func::from(is_builtin))?;
        exports.export("registerHooks", Func::from(register_hooks))?;
        exports.export(
            "_nodeModulePaths",
            module_ctor.get::<_, Value>("_nodeModulePaths")?,
        )?;
        exports.export(
            "_resolveFilename",
            module_ctor.get::<_, Value>("_resolveFilename")?,
        )?;
        exports.export("_cache", module_ctor.get::<_, Value>("_cache")?)?;
        exports.export("_extensions", module_ctor.get::<_, Value>("_extensions")?)?;
        for name in facade::MODULE_EXTRA_EXPORTS {
            exports.export(name, module_ctor.get::<_, Value>(name)?)?;
        }

        Ok(())
    }
}

impl From<ModuleModule> for ModuleInfo<ModuleModule> {
    fn from(val: ModuleModule) -> Self {
        ModuleInfo {
            name: "module",
            module: val,
        }
    }
}

/// Tear down module-related context userdata before V8/context shutdown.
///
/// Clears JS `require.cache` and removes Rust userdata holding `Value`/`Object`/`Module`
/// references so QuickJS GC does not retain `JS_CONTEXT` roots after drop.
pub fn shutdown_context_state<'js>(ctx: &Ctx<'js>) -> Result<()> {
    // Drop ESM module environments (and their locals like `db`) before tearing down
    // the CJS require graph that native addons may still reference.
    unsafe {
        extern "C" {
            fn JS_FreeAllModules(ctx: *mut qjs::JSContext);
        }
        JS_FreeAllModules(ctx.as_raw().as_ptr());
    }
    {
        let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        for _ in 0..4 {
            ctx.run_gc();
            unsafe {
                qjs::JS_RunGC(rt);
            }
        }
    }

    let _: usize = ctx.eval(
        r#"
        (() => {
            try {
                const req = globalThis.require;
                if (req?.cache) {
                    for (const key of Object.keys(req.cache)) {
                        const entry = req.cache[key];
                        if (entry?.exports) {
                            for (const exportKey of Object.keys(entry.exports)) {
                                try {
                                    entry.exports[exportKey] = undefined;
                                } catch (_err) {
                                    // ignore non-configurable export properties
                                }
                            }
                            entry.exports = null;
                        }
                        if (entry) {
                            try {
                                entry.children = [];
                            } catch (_err) {
                                // ignore read-only children
                            }
                            try {
                                entry.parent = null;
                            } catch (_err) {
                                // ignore read-only parent
                            }
                        }
                        delete req.cache[key];
                    }
                }
                if (req?.extensions) {
                    for (const key of Object.keys(req.extensions)) {
                        delete req.extensions[key];
                    }
                }
                globalThis.require = undefined;
                globalThis.module = undefined;
                globalThis.exports = undefined;
            } catch (_err) {
                // Best-effort: require may already be torn down during N-API shutdown.
            }
            return 0;
        })()
        "#,
    )?;

    if let Some(binding) = ctx.userdata::<RefCell<RequireState>>() {
        let mut state = binding.borrow_mut();
        state.cache.clear();
        state.exports.clear();
        state.progress.clear();
        state.current_module = None;
    }

    if let Some(binding) = ctx.userdata::<RefCell<ModuleCache>>() {
        binding.borrow_mut().esm.clear();
    }

    if let Some(binding) = ctx.userdata::<RefCell<ModuleHookState>>() {
        binding.borrow_mut().hooks.clear();
    }

    unsafe {
        extern "C" {
            fn JS_ReleaseContextClassProtos(ctx: *mut qjs::JSContext);
            fn JS_FreeAllModules(ctx: *mut qjs::JSContext);
        }
        JS_FreeAllModules(ctx.as_raw().as_ptr());
        JS_ReleaseContextClassProtos(ctx.as_raw().as_ptr());
    }
    {
        let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        for _ in 0..8 {
            ctx.run_gc();
            unsafe {
                qjs::JS_RunGC(rt);
            }
        }
    }

    if ctx.userdata::<RefCell<RequireState>>().is_some() {
        ctx.remove_userdata::<RefCell<RequireState>>()
            .map_err(|_| Error::Unknown)?;
        if ctx.userdata::<RefCell<RequireState>>().is_some() {
            return Err(Error::Unknown);
        }
    }
    if ctx.userdata::<RefCell<ModuleCache>>().is_some() {
        ctx.remove_userdata::<RefCell<ModuleCache>>()
            .map_err(|_| Error::Unknown)?;
        if ctx.userdata::<RefCell<ModuleCache>>().is_some() {
            return Err(Error::Unknown);
        }
    }
    if ctx.userdata::<RefCell<ModuleHookState>>().is_some() {
        ctx.remove_userdata::<RefCell<ModuleHookState>>()
            .map_err(|_| Error::Unknown)?;
        if ctx.userdata::<RefCell<ModuleHookState>>().is_some() {
            return Err(Error::Unknown);
        }
    }
    if ctx
        .userdata::<RefCell<facade::ModuleFacadeState>>()
        .is_some()
    {
        ctx.remove_userdata::<RefCell<facade::ModuleFacadeState>>()
            .map_err(|_| Error::Unknown)?;
        if ctx
            .userdata::<RefCell<facade::ModuleFacadeState>>()
            .is_some()
        {
            return Err(Error::Unknown);
        }
    }

    Ok(())
}

pub fn init(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();

    ctx.store_userdata(RefCell::new(RequireState::default()))?;
    ctx.store_userdata(RefCell::new(ModuleHookState::default()))?;
    ctx.store_userdata(RefCell::new(ModuleCache::default()))?;

    let module_list = ctx
        .userdata::<ModuleNames>()
        .map_or_else(HashSet::new, |v| v.get_list());
    init_module_facade(ctx, module_list)?;

    let exports_accessor = Accessor::new(
        |ctx| {
            struct Args<'js>(Ctx<'js>);
            let Args(ctx) = Args(ctx);
            let name = ctx.get_script_or_module_name()?;
            let name = name.trim_start_matches(CJS_IMPORT_PREFIX);

            let binding = ctx.userdata::<RefCell<RequireState>>().unwrap();
            let mut state = binding.borrow_mut();

            if let Some(value) = state.exports.get(name) {
                Ok::<_, Error>(value.clone())
            } else {
                let obj = Object::new(ctx.clone())?.into_value();
                state.exports.insert(name.into(), obj.clone());
                Ok::<_, Error>(obj)
            }
        },
        |ctx, exports| {
            struct Args<'js>(Ctx<'js>, Value<'js>);
            let Args(ctx, exports) = Args(ctx, exports);
            let name = ctx.get_script_or_module_name()?;
            let name = name.trim_start_matches(CJS_IMPORT_PREFIX);
            let binding = ctx.userdata::<RefCell<RequireState>>().unwrap();
            let mut state = binding.borrow_mut();
            state.exports.insert(name.into(), exports);
            Ok::<_, Error>(())
        },
    )
    .configurable()
    .enumerable();

    init_global_require(ctx)?;

    let module_instance = facade::get_or_create_module_record(
        ctx,
        &facade::canonical_parent_filename(ctx, None)?,
        None,
    )?;
    module_instance.prop("exports", exports_accessor)?;
    globals.prop("module", module_instance)?;
    globals.prop("exports", exports_accessor)?;

    Ok(())
}
