// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
};

use lexe_utils::{object::ObjectExt, result::ResultExt};
use rquickjs::{
    function::Constructor,
    loader::{ImportAttributes, Resolver},
    prelude::{Func, Opt},
    Array, Ctx, Error, Function, Object, Result, Value,
};
use tracing::trace;
use url::Url;

use crate::{modules::path, package::resolver::require_resolve_with_options, CJS_IMPORT_PREFIX};

use super::{facade::unknown_builtin_module, Hook, ModuleHookState, ModuleNames};

#[derive(Debug, Default)]
pub struct ModuleResolver {
    modules: HashSet<String>,
}

impl ModuleResolver {
    #[must_use]
    pub fn add_name<P: Into<String>>(mut self, path: P) -> Self {
        self.modules.insert(path.into());
        self
    }
}

impl Resolver for ModuleResolver {
    fn resolve(
        &mut self,
        ctx: &Ctx<'_>,
        base: &str,
        name: &str,
        _attr: Option<ImportAttributes<'_>>,
    ) -> Result<String> {
        let name = name.trim_start_matches(CJS_IMPORT_PREFIX);
        let name = if name.len() >= 5 && name[..5].eq_ignore_ascii_case("data:") {
            name
        } else {
            name.trim_end_matches('/')
        };

        let base = base.trim_start_matches(CJS_IMPORT_PREFIX);

        trace!("Try resolve '{}' from '{}'", name, base);

        let has_hooks = ctx
            .userdata::<RefCell<ModuleHookState>>()
            .is_some_and(|b| !b.borrow().hooks.is_empty());

        if !has_hooks {
            if let Some(resolved) = resolve_native_module_name(name, &self.modules) {
                trace!("+- Resolved by `NativeModule`: {}", resolved);
                return Ok(resolved);
            }
            return Err(Error::new_resolving(base, name));
        }

        let parent_url = path_or_url_to_file_url(base);
        let (short_circuit, next_resolve, x) = module_hook_resolve(ctx, name, &parent_url)?;

        if next_resolve {
            trace!("|  Determined as `nextResolve`: {}", x);
        } else {
            trace!("|  Determined as `NormalCircuit`: {}", x);
        }

        if let Some(resolved) = resolve_native_module_name(&x, &self.modules) {
            trace!("+- Resolved by `NativeModule`: {}", resolved);
            return Ok(resolved);
        }

        if short_circuit || x.starts_with("file://") || x.starts_with("node:") || x.contains(':') {
            trace!("+- Resolved by hook/URL: {}", x);
            return Ok(x);
        }

        Err(Error::new_resolving(base, x))
    }
}

pub fn path_or_url_to_file_url(input: &str) -> String {
    if input.contains("://")
        || input.starts_with("node:")
        || input.starts_with("data:")
        || input.starts_with("opencode:")
    {
        return input.to_string();
    }
    let candidate = if input.is_empty() || input == "eval_script" {
        if let Ok(cwd) = std::env::current_dir() {
            cwd.join(if input.is_empty() { "index.js" } else { input })
        } else {
            PathBuf::from(input)
        }
    } else if path::is_absolute(input) {
        PathBuf::from(input)
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(input)
    } else {
        PathBuf::from(input)
    };
    Url::from_file_path(&candidate)
        .map(String::from)
        .unwrap_or_else(|()| {
            #[cfg(windows)]
            {
                format!("file:///{}", candidate.to_string_lossy().replace('\\', "/"))
            }
            #[cfg(not(windows))]
            {
                format!("file://{}", candidate.to_string_lossy())
            }
        })
}

pub fn file_url_to_path_and_query(input: &str) -> Option<(String, Option<String>)> {
    if !input.starts_with("file://") {
        return None;
    }
    let parsed = Url::parse(input).ok()?;
    let query = parsed.query().map(|q| format!("?{q}"));
    let fs_path = parsed.to_file_path().ok()?;
    Some((fs_path.to_string_lossy().replace('\\', "/"), query))
}

fn url_or_path_to_fs_path(input: &str) -> String {
    if let Some((fs_path, _)) = file_url_to_path_and_query(input) {
        return fs_path;
    }
    if input.is_empty() || input == "eval_script" {
        if let Ok(cwd) = std::env::current_dir() {
            return cwd
                .join(if input.is_empty() { "index.js" } else { input })
                .to_string_lossy()
                .replace('\\', "/");
        }
    }
    input.to_string()
}

fn module_not_found_esm(ctx: &Ctx<'_>, spec: &str, parent: &str) -> Result<String> {
    let message = format!("Cannot find module '{spec}' imported from {parent}");
    let err_ctor: Constructor = ctx.globals().get("Error")?;
    let err: Object = err_ctor.construct((message,))?;
    err.set("code", "ERR_MODULE_NOT_FOUND")?;
    Err(ctx.throw(err.into_value()))
}

pub fn default_esm_resolve(ctx: &Ctx<'_>, spec: &str, parent_url_or_path: &str) -> Result<String> {
    let trimmed = spec.trim_end_matches('/');
    if trimmed.is_empty() {
        return module_not_found_esm(ctx, spec, parent_url_or_path);
    }

    if let Some(names) = ctx.userdata::<ModuleNames>() {
        if let Some(builtin_request) = names.disabled_node_builtin_error_request(trimmed) {
            unknown_builtin_module(ctx, &builtin_request)?;
            unreachable!();
        }
        if names.resolve_builtin(trimmed).is_some() {
            let bare = trimmed.strip_prefix("node:").unwrap_or(trimmed);
            return Ok(format!("node:{bare}"));
        }
    }

    if let Ok(hooked_fn) = ctx.globals().get::<_, Function>("__require_hook") {
        let embedded_spec = trimmed.strip_prefix("node:").unwrap_or(trimmed);
        if hooked_fn
            .call::<_, String>((embedded_spec, parent_url_or_path))
            .is_ok()
        {
            return Ok(format!("node:{embedded_spec}"));
        }
    }

    if spec.contains("://") && !spec.starts_with("file://") {
        return Ok(spec.to_string());
    }

    let parent_fs = url_or_path_to_fs_path(parent_url_or_path);

    if let Some((fs_path, query)) = file_url_to_path_and_query(spec) {
        if Path::new(&fs_path).is_file() {
            let mut url = path_or_url_to_file_url(&fs_path);
            if let Some(q) = query {
                url.push_str(&q);
            }
            return Ok(url);
        }
        return module_not_found_esm(ctx, spec, parent_url_or_path);
    }

    match require_resolve_with_options(ctx, spec, &parent_fs, None, true, None) {
        Ok(resolved) => Ok(path_or_url_to_file_url(&resolved)),
        Err(_) => {
            if ctx.has_exception() {
                let _ = ctx.catch();
            }
            module_not_found_esm(ctx, spec, parent_url_or_path)
        },
    }
}

pub fn resolve_import_meta(ctx: &Ctx<'_>, parent_url: &str, spec: &str) -> Result<String> {
    let has_hooks = ctx
        .userdata::<RefCell<ModuleHookState>>()
        .is_some_and(|b| !b.borrow().hooks.is_empty());
    if has_hooks {
        let (_, _, resolved) = module_hook_resolve(ctx, spec, parent_url)?;
        if resolved.contains("://")
            || resolved.starts_with("node:")
            || resolved.starts_with("data:")
        {
            Ok(resolved)
        } else {
            default_esm_resolve(ctx, &resolved, parent_url)
        }
    } else {
        default_esm_resolve(ctx, spec, parent_url)
    }
}

pub fn populate_import_meta<'js>(_ctx: &Ctx<'js>, meta: &Object<'js>, url: &str) -> Result<()> {
    meta.set("url", url)?;
    if let Some((fs_path, _)) = file_url_to_path_and_query(url) {
        let dirname = path::dirname(&fs_path);
        meta.set("dirname", dirname)?;
        meta.set("filename", fs_path)?;
    } else if !url.contains("://") && !url.starts_with("node:") {
        if let Some(parent) = Path::new(url).parent() {
            meta.set("dirname", parent.to_string_lossy().as_ref())?;
        }
        meta.set("filename", url)?;
    }

    let parent_url = path_or_url_to_file_url(url);
    meta.set(
        "resolve",
        Func::from(move |ctx: Ctx<'_>, spec: String| -> Result<String> {
            resolve_import_meta(&ctx, &parent_url, &spec)
        }),
    )?;
    Ok(())
}

fn resolve_native_module_name(name: &str, modules: &HashSet<String>) -> Option<String> {
    if modules.contains(name) {
        return Some(name.to_string());
    }

    if let Some(bare) = name.strip_prefix("node:") {
        if modules.contains(bare) {
            return Some(bare.to_string());
        }
    }

    None
}

pub fn module_hook_resolve<'js>(ctx: &Ctx<'js>, x: &str, y: &str) -> Result<(bool, bool, String)> {
    trace!("|  module_hook_resolve(x, y):({}, {})", x, y);

    let bind_state = ctx.userdata::<RefCell<ModuleHookState>>().or_throw(ctx)?;
    let hooks = Rc::new(bind_state.borrow().hooks.clone());

    if hooks.is_empty() {
        return Ok((false, false, x.into()));
    }

    let parent_url = path_or_url_to_file_url(y);
    let result = call_resolve_hooks(ctx, &hooks, x, &parent_url)?;

    let short_circuit = result
        .get_optional::<_, bool>("shortCircuit")?
        .unwrap_or(false);

    let next_resolve = result
        .get_optional::<_, bool>("__nextResolve")?
        .unwrap_or(false);

    let url = result.get::<_, String>("url")?;

    Ok((short_circuit, next_resolve, url))
}

fn call_resolve_hooks<'js>(
    ctx: &Ctx<'js>,
    hooks: &Rc<Vec<Hook<'js>>>,
    spec: &str,
    parent_url: &str,
) -> Result<Object<'js>> {
    call_resolve_hooks_from(ctx, hooks, 0, spec, parent_url)
}

fn call_resolve_hooks_from<'js>(
    ctx: &Ctx<'js>,
    hooks: &Rc<Vec<Hook<'js>>>,
    start_index: usize,
    spec: &str,
    parent_url: &str,
) -> Result<Object<'js>> {
    for index in start_index..hooks.len() {
        let Some(resolve_fn) = &hooks[index].resolve else {
            continue;
        };

        let context = Object::new(ctx.clone())?;
        context.set("parentURL", parent_url)?;
        let conditions = Array::new(ctx.clone())?;
        conditions.set(0, "node")?;
        conditions.set(1, "import")?;
        context.set("conditions", conditions)?;

        let called_next = Rc::new(Cell::new(false));
        let called_next_ref = Rc::clone(&called_next);

        let parent_clone = parent_url.to_string();
        let hooks_clone = Rc::clone(hooks);

        let next_func = Func::new(
            move |ctx: Ctx<'js>,
                  new_spec: String,
                  opt_ctx: Opt<Value<'js>>|
                  -> Result<Object<'js>> {
                let new_parent = if let Some(val) = opt_ctx.0 {
                    if let Some(ctx_obj) = val.as_object() {
                        ctx_obj
                            .get::<_, String>("parentURL")
                            .unwrap_or_else(|_| parent_clone.clone())
                    } else {
                        parent_clone.clone()
                    }
                } else {
                    parent_clone.clone()
                };
                called_next_ref.set(true);
                call_resolve_hooks_from(&ctx, &hooks_clone, index + 1, &new_spec, &new_parent)
            },
        );

        let result = resolve_fn.call::<_, Object>((spec, context, next_func))?;
        result.set("__nextResolve", called_next.get())?;

        return Ok(result);
    }

    let resolved = default_esm_resolve(ctx, spec, parent_url)?;
    let obj = Object::new(ctx.clone())?;
    obj.set("url", resolved)?;
    obj.set("shortCircuit", true)?;
    obj.set("__nextResolve", false)?;
    Ok(obj)
}
