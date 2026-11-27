// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use lexe_json::escape::escape_json_string;
use lexe_utils::{any_of::AnyOf2, bytes::ObjectBytes, object::ObjectExt, result::ResultExt};
use rquickjs::{
    loader::{ImportAttributes, Loader},
    module::ModuleDef,
    prelude::{Func, Opt},
    Ctx, Error, Exception, Module, Object, Result, Value,
};
use tracing::trace;

use super::{
    compile_cache::declare_esm_with_cache,
    facade::get_or_create_module_record,
    resolver::file_url_to_path_and_query,
    ts_strip::{strip_ts, StripOptions},
    Hook, ModuleHookState,
};
use crate::package::resolver::detect_file_format;

type LoadFn = for<'js> fn(Ctx<'js>, Vec<u8>) -> Result<Module<'js>>;
type Source<'js> = AnyOf2<String, ObjectBytes<'js>>;

#[derive(Debug, Default)]
pub struct ModuleLoader {
    modules: HashMap<String, LoadFn>,
}

impl ModuleLoader {
    fn load_func<'js, D: ModuleDef>(ctx: Ctx<'js>, name: Vec<u8>) -> Result<Module<'js>> {
        Module::declare_def::<D, _>(ctx, name)
    }

    pub fn add_module<N: Into<String>, M: ModuleDef>(&mut self, name: N, _module: M) -> &mut Self {
        self.modules.insert(name.into(), Self::load_func::<M>);
        self
    }

    #[must_use]
    pub fn with_module<N: Into<String>, M: ModuleDef>(mut self, name: N, module: M) -> Self {
        self.add_module(name, module);
        self
    }
}

impl Loader for ModuleLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        attr: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js>> {
        trace!("Try load '{}'", name);
        let (short_circuit, next_load, format, source) =
            module_hook_load(ctx, name, attr.as_ref())?;

        if short_circuit {
            trace!("+- Loading module via ShortCircuit: {}\n", name);
            let raw_bytes: Vec<u8> = match &source {
                AnyOf2::A(s) => s.as_bytes().to_vec(),
                AnyOf2::B(b) => b.as_bytes(ctx)?.to_vec(),
            };
            let module = match format.as_deref() {
                Some("json") => {
                    let mut json = String::with_capacity(raw_bytes.len() + 40);
                    json.push_str("export default JSON.parse(\"");
                    escape_json_string(&mut json, &raw_bytes);
                    json.push_str("\");");
                    Module::declare(ctx.clone(), name, json)?
                },
                Some("typescript" | "module-typescript") => {
                    let s = std::str::from_utf8(&raw_bytes).map_err(|_| {
                        Exception::throw_type(ctx, "Invalid UTF-8 in TypeScript hook source")
                    })?;
                    let stripped =
                        strip_ts(s, &StripOptions::default()).map_err(|e| e.throw(ctx))?;
                    declare_esm_with_cache(ctx, name, stripped.as_bytes())?
                },
                Some("commonjs" | "commonjs-typescript") => {
                    let s = std::str::from_utf8(&raw_bytes).map_err(|_| {
                        Exception::throw_type(ctx, "Invalid UTF-8 in CommonJS hook source")
                    })?;
                    let code = if format.as_deref() == Some("commonjs-typescript") {
                        strip_ts(s, &StripOptions::default()).map_err(|e| e.throw(ctx))?
                    } else {
                        s.to_string()
                    };
                    let fs_path = file_url_to_path_and_query(name)
                        .map(|(p, _)| p)
                        .unwrap_or_else(|| name.to_string());
                    let record = get_or_create_module_record(ctx, &fs_path, None)?;
                    super::extensions::module_compile(
                        ctx.clone(),
                        rquickjs::function::This(record.clone()),
                        code,
                        fs_path.clone(),
                    )?;
                    record.set("loaded", true)?;
                    let exports: Value = record.get("exports")?;
                    let mut wrapper = String::with_capacity(fs_path.len() + 256);
                    wrapper.push_str("const value = require(");
                    wrapper.push('"');
                    escape_json_string(&mut wrapper, fs_path.as_bytes());
                    wrapper.push_str("\");export default value?.default ?? value;");
                    if let Some(obj) = exports.as_object() {
                        let keys: Vec<String> = obj.keys().collect::<Result<Vec<_>>>()?;
                        let named: Vec<_> = keys.into_iter().filter(|k| k != "default").collect();
                        if !named.is_empty() {
                            wrapper.push_str("const{");
                            wrapper.push_str(&named.join(","));
                            wrapper.push_str("}=value;export{");
                            wrapper.push_str(&named.join(","));
                            wrapper.push_str("};");
                        }
                    }
                    Module::declare(ctx.clone(), name, wrapper)?
                },
                _ => declare_esm_with_cache(ctx, name, &raw_bytes)?,
            };
            let meta: Object = module.meta()?;
            super::resolver::populate_import_meta(ctx, &meta, name)?;
            return Ok(module);
        };

        let load = self
            .modules
            .remove(name)
            .ok_or_else(|| Error::new_loading(name))?;

        if next_load {
            trace!("|  Determined as `nextResolve`: {}", name);
        } else {
            trace!("|  Determined as `NormalCircuit`: {}", name);
        }

        trace!("+- Loading module: {}\n", name);
        (load)(ctx.clone(), Vec::from(name))
    }
}

pub fn module_hook_load<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    attr: Option<&ImportAttributes<'js>>,
) -> Result<(bool, bool, Option<String>, Source<'js>)> {
    let bind_state = ctx.userdata::<RefCell<ModuleHookState>>().or_throw(ctx)?;
    let hooks = Rc::new(bind_state.borrow().hooks.clone());

    if hooks.is_empty() {
        return Ok((false, false, None, AnyOf2::A("".into())));
    }

    let result = call_load_hooks_with_attrs(ctx, &hooks, name, attr)?;

    let short_circuit = result
        .get_optional::<_, bool>("shortCircuit")?
        .unwrap_or(false);

    let next_load = result
        .get_optional::<_, bool>("__nextLoad")?
        .unwrap_or(false);

    let format = result.get_optional::<_, String>("format")?;

    let source = result
        .get_optional::<_, Source>("source")?
        .unwrap_or(AnyOf2::A("".into()));

    Ok((short_circuit, next_load, format, source))
}

fn decode_data_url_source(url: &str) -> Option<String> {
    let rest = url.strip_prefix("data:")?;
    let (meta, body) = rest.split_once(',')?;
    let body = body.split('#').next().unwrap_or(body);
    let is_b64 = meta
        .split(';')
        .any(|p| p.trim().eq_ignore_ascii_case("base64"));
    let bytes = if is_b64 {
        lexe_encoding_b64_decode(body)?
    } else {
        percent_decode_bytes(body.as_bytes())
    };
    String::from_utf8(bytes).ok()
}

fn percent_decode_bytes(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            if let (Some(hi), Some(lo)) = (hex(input[i + 1]), hex(input[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

fn lexe_encoding_b64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |b: u8| match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let clean: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut i = 0;
    while i + 1 < clean.len() {
        let a = val(clean[i])?;
        let b = val(clean[i + 1])?;
        out.push((a << 2) | (b >> 4));
        if i + 2 < clean.len() {
            let c = val(clean[i + 2])?;
            out.push((b << 4) | (c >> 2));
            if i + 3 < clean.len() {
                let d = val(clean[i + 3])?;
                out.push((c << 6) | d);
            }
        }
        i += 4;
    }
    Some(out)
}

#[cfg(test)]
fn call_load_hooks<'js>(
    ctx: &Ctx<'js>,
    hooks: &Rc<Vec<Hook<'js>>>,
    url: &str,
) -> Result<Object<'js>> {
    call_load_hooks_from(ctx, hooks, 0, url, None)
}

fn call_load_hooks_with_attrs<'js>(
    ctx: &Ctx<'js>,
    hooks: &Rc<Vec<Hook<'js>>>,
    url: &str,
    attr: Option<&ImportAttributes<'js>>,
) -> Result<Object<'js>> {
    call_load_hooks_from(ctx, hooks, 0, url, attr)
}

fn default_next_load<'js>(ctx: &Ctx<'js>, url: &str) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("url", url)?;
    obj.set("__nextLoad", false)?;

    if url.starts_with("data:") {
        if let Some(src) = decode_data_url_source(url) {
            obj.set("format", "module")?;
            obj.set("source", src)?;
            obj.set("shortCircuit", true)?;
            return Ok(obj);
        }
    }

    if let Some((fs_path, _)) = file_url_to_path_and_query(url) {
        if let Ok(source) = std::fs::read_to_string(&fs_path) {
            let format = if fs_path.ends_with(".json") {
                "json"
            } else if fs_path.ends_with(".mts") {
                "module-typescript"
            } else if fs_path.ends_with(".cts") {
                "commonjs-typescript"
            } else if fs_path.ends_with(".ts") {
                let is_cjs = detect_file_format(&fs_path)
                    .map(|f| f.is_cjs())
                    .unwrap_or(false);
                if is_cjs {
                    "commonjs-typescript"
                } else {
                    "module-typescript"
                }
            } else {
                let is_cjs = detect_file_format(&fs_path)
                    .map(|f| f.is_cjs())
                    .unwrap_or(false);
                if is_cjs {
                    "commonjs"
                } else {
                    "module"
                }
            };
            obj.set("format", format)?;
            obj.set("source", source)?;
            obj.set("shortCircuit", true)?;
            return Ok(obj);
        }
    }

    obj.set("format", "builtin")?;
    obj.set("shortCircuit", false)?;
    Ok(obj)
}

fn call_load_hooks_from<'js>(
    ctx: &Ctx<'js>,
    hooks: &Rc<Vec<Hook<'js>>>,
    start_index: usize,
    url: &str,
    attr: Option<&ImportAttributes<'js>>,
) -> Result<Object<'js>> {
    for index in start_index..hooks.len() {
        let Some(load_fn) = &hooks[index].load else {
            continue;
        };

        let called_next = Rc::new(Cell::new(false));
        let called_next_ref = Rc::clone(&called_next);

        let context = Object::new(ctx.clone())?;
        let attrs_obj = Object::new(ctx.clone())?;
        if let Some(attr) = attr {
            if let Ok(Some(t)) = attr.get_type() {
                attrs_obj.set("type", t)?;
            }
        }
        context.set("importAttributes", attrs_obj)?;

        let hooks_clone = Rc::clone(hooks);

        let next_func = Func::new(
            move |ctx: Ctx<'js>,
                  new_url: String,
                  _opt_ctx: Opt<Value<'js>>|
                  -> Result<Object<'js>> {
                called_next_ref.set(true);
                call_load_hooks_from(&ctx, &hooks_clone, index + 1, &new_url, None)
            },
        );

        let result = load_fn.call::<_, Object>((url, context, next_func))?;
        result.set("__nextLoad", called_next.get())?;

        return Ok(result);
    }

    default_next_load(ctx, url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_async_with;
    use rquickjs::Function;

    #[tokio::test]
    async fn test_hook_override_import() {
        use lexe_test::ModuleEvaluator;

        test_async_with(|ctx| {
            Box::pin(async move {
                let _ = ctx.store_userdata(RefCell::new(ModuleHookState::default()));

                let hook_code = r#"
                    globalThis.hookCalled = false;
                    globalThis.nextLoadCalled = false;
                    
                    export function load(url, context, nextLoad) {
                        globalThis.hookCalled = true;
                        if (url === "math") {
                            return {
                                format: "module",
                                shortCircuit: true,
                                source: "export function add(a, b) { return a + b + 1; }"
                            };
                        }
                        globalThis.nextLoadCalled = true;
                        return nextLoad(url, context);
                    }
                "#;

                let hook_module = ModuleEvaluator::eval_js(ctx.clone(), "hook", hook_code)
                    .await
                    .unwrap();

                let load_fn: Function = hook_module.get("load").unwrap();
                let hook = Hook {
                    resolve: None,
                    load: Some(load_fn),
                };

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                binding.borrow_mut().hooks.push(hook);

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                let hooks = Rc::new(binding.borrow().hooks.clone());

                let result = call_load_hooks(&ctx, &hooks, "math").unwrap();

                let globals = ctx.globals();
                assert!(globals.get::<_, bool>("hookCalled").unwrap());
                assert!(result.get::<_, bool>("shortCircuit").unwrap());
                assert_eq!(
                    result.get::<_, String>("source").unwrap(),
                    "export function add(a, b) { return a + b + 1; }"
                );

                let result2 = call_load_hooks(&ctx, &hooks, "other").unwrap();
                assert!(globals.get::<_, bool>("nextLoadCalled").unwrap());
                assert!(!result2.get::<_, bool>("shortCircuit").unwrap());
                assert_eq!(result2.get::<_, String>("url").unwrap(), "other");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_multiple_hooks_chain() {
        use lexe_test::ModuleEvaluator;

        test_async_with(|ctx| {
            Box::pin(async move {
                let _ = ctx.store_userdata(RefCell::new(ModuleHookState::default()));

                let hook1_code = r#"
                    export function load(url, context, nextLoad) {
                        globalThis.hook1Called = true;
                        if (url === "skip") {
                            return nextLoad(url, context);
                        }
                        return nextLoad("modified-" + url, context);
                    }
                "#;

                let hook2_code = r#"
                    export function load(url, context, nextLoad) {
                        globalThis.hook2Called = true;
                        globalThis.finalUrl = url;
                        return {
                            shortCircuit: true,
                            source: "export const value = 42;"
                        };
                    }
                "#;

                let hook1 = ModuleEvaluator::eval_js(ctx.clone(), "hook1", hook1_code)
                    .await
                    .unwrap();
                let hook2 = ModuleEvaluator::eval_js(ctx.clone(), "hook2", hook2_code)
                    .await
                    .unwrap();

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook1.get("load").unwrap()),
                });
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook2.get("load").unwrap()),
                });

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                let hooks = Rc::new(binding.borrow().hooks.clone());

                let result = call_load_hooks(&ctx, &hooks, "test").unwrap();

                let globals = ctx.globals();
                assert!(globals.get::<_, bool>("hook1Called").unwrap());
                assert!(globals.get::<_, bool>("hook2Called").unwrap());
                assert_eq!(
                    globals.get::<_, String>("finalUrl").unwrap(),
                    "modified-test"
                );
                assert!(result.get::<_, bool>("shortCircuit").unwrap());
                assert_eq!(
                    result.get::<_, String>("source").unwrap(),
                    "export const value = 42;"
                );
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_hook_without_nextload() {
        use lexe_test::ModuleEvaluator;

        test_async_with(|ctx| {
            Box::pin(async move {
                let _ = ctx.store_userdata(RefCell::new(ModuleHookState::default()));

                let hook_code = r#"
                    export function load(url, context, nextLoad) {
                        return {
                            shortCircuit: true,
                            source: "export const intercepted = true;"
                        };
                    }
                "#;

                let hook_module = ModuleEvaluator::eval_js(ctx.clone(), "hook", hook_code)
                    .await
                    .unwrap();

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook_module.get("load").unwrap()),
                });

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                let hooks = Rc::new(binding.borrow().hooks.clone());

                let result = call_load_hooks(&ctx, &hooks, "any").unwrap();

                assert!(result.get::<_, bool>("shortCircuit").unwrap());
                assert_eq!(
                    result.get::<_, String>("source").unwrap(),
                    "export const intercepted = true;"
                );
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_hook_passthrough_all() {
        use lexe_test::ModuleEvaluator;

        test_async_with(|ctx| {
            Box::pin(async move {
                let _ = ctx.store_userdata(RefCell::new(ModuleHookState::default()));

                let hook_code = r#"
                    export function load(url, context, nextLoad) {
                        globalThis.passedThrough = url;
                        return nextLoad(url, context);
                    }
                "#;

                let hook_module = ModuleEvaluator::eval_js(ctx.clone(), "hook", hook_code)
                    .await
                    .unwrap();

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook_module.get("load").unwrap()),
                });

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                let hooks = Rc::new(binding.borrow().hooks.clone());

                let result = call_load_hooks(&ctx, &hooks, "passthrough").unwrap();

                let globals = ctx.globals();
                assert_eq!(
                    globals.get::<_, String>("passedThrough").unwrap(),
                    "passthrough"
                );
                assert!(!result.get::<_, bool>("shortCircuit").unwrap());
                assert_eq!(result.get::<_, String>("url").unwrap(), "passthrough");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_hook_conditional_intercept() {
        use lexe_test::ModuleEvaluator;

        test_async_with(|ctx| {
            Box::pin(async move {
                let _ = ctx.store_userdata(RefCell::new(ModuleHookState::default()));

                let hook_code = r#"
                    export function load(url, context, nextLoad) {
                        if (url.startsWith("internal:")) {
                            return {
                                shortCircuit: true,
                                source: "export const internal = true;"
                            };
                        }
                        return nextLoad(url, context);
                    }
                "#;

                let hook_module = ModuleEvaluator::eval_js(ctx.clone(), "hook", hook_code)
                    .await
                    .unwrap();

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook_module.get("load").unwrap()),
                });

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                let hooks = Rc::new(binding.borrow().hooks.clone());

                let result1 = call_load_hooks(&ctx, &hooks, "internal:test").unwrap();
                assert!(result1.get::<_, bool>("shortCircuit").unwrap());
                assert_eq!(
                    result1.get::<_, String>("source").unwrap(),
                    "export const internal = true;"
                );

                let result2 = call_load_hooks(&ctx, &hooks, "external:test").unwrap();
                assert!(!result2.get::<_, bool>("shortCircuit").unwrap());
                assert_eq!(result2.get::<_, String>("url").unwrap(), "external:test");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_three_hooks_selective_intercept() {
        use lexe_test::ModuleEvaluator;

        test_async_with(|ctx| {
            Box::pin(async move {
                let _ = ctx.store_userdata(RefCell::new(ModuleHookState::default()));

                let hook1_code = r#"
                    export function load(url, context, nextLoad) {
                        globalThis.order = ["hook1"];
                        return nextLoad(url, context);
                    }
                "#;

                let hook2_code = r#"
                    export function load(url, context, nextLoad) {
                        globalThis.order.push("hook2");
                        if (url === "intercept-here") {
                            return {
                                shortCircuit: true,
                                source: "export const from = 'hook2';"
                            };
                        }
                        return nextLoad(url, context);
                    }
                "#;

                let hook3_code = r#"
                    export function load(url, context, nextLoad) {
                        globalThis.order.push("hook3");
                        return {
                            shortCircuit: true,
                            source: "export const from = 'hook3';"
                        };
                    }
                "#;

                let hook1 = ModuleEvaluator::eval_js(ctx.clone(), "hook1", hook1_code)
                    .await
                    .unwrap();
                let hook2 = ModuleEvaluator::eval_js(ctx.clone(), "hook2", hook2_code)
                    .await
                    .unwrap();
                let hook3 = ModuleEvaluator::eval_js(ctx.clone(), "hook3", hook3_code)
                    .await
                    .unwrap();

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook1.get("load").unwrap()),
                });
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook2.get("load").unwrap()),
                });
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook3.get("load").unwrap()),
                });

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                let hooks = Rc::new(binding.borrow().hooks.clone());

                let result1 = call_load_hooks(&ctx, &hooks, "intercept-here").unwrap();
                let globals = ctx.globals();
                let order: Vec<String> = globals.get("order").unwrap();
                assert_eq!(order, vec!["hook1", "hook2"]);
                assert_eq!(
                    result1.get::<_, String>("source").unwrap(),
                    "export const from = 'hook2';"
                );
                assert!(result1.get::<_, bool>("shortCircuit").unwrap());

                let result2 = call_load_hooks(&ctx, &hooks, "other").unwrap();
                let order2: Vec<String> = globals.get("order").unwrap();
                assert_eq!(order2, vec!["hook1", "hook2", "hook3"]);
                assert_eq!(
                    result2.get::<_, String>("source").unwrap(),
                    "export const from = 'hook3';"
                );
                assert!(result2.get::<_, bool>("shortCircuit").unwrap());
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_hook_url_transformation_chain() {
        use lexe_test::ModuleEvaluator;

        test_async_with(|ctx| {
            Box::pin(async move {
                let _ = ctx.store_userdata(RefCell::new(ModuleHookState::default()));

                let hook1_code = r#"
                    export function load(url, context, nextLoad) {
                        return nextLoad(url.replace("@", "node_modules/"), context);
                    }
                "#;

                let hook2_code = r#"
                    export function load(url, context, nextLoad) {
                        globalThis.transformedUrl = url;
                        return nextLoad(url + "/index.js", context);
                    }
                "#;

                let hook3_code = r#"
                    export function load(url, context, nextLoad) {
                        globalThis.finalUrl = url;
                        return {
                            shortCircuit: true,
                            source: "export default {};"
                        };
                    }
                "#;

                let hook1 = ModuleEvaluator::eval_js(ctx.clone(), "hook1", hook1_code)
                    .await
                    .unwrap();
                let hook2 = ModuleEvaluator::eval_js(ctx.clone(), "hook2", hook2_code)
                    .await
                    .unwrap();
                let hook3 = ModuleEvaluator::eval_js(ctx.clone(), "hook3", hook3_code)
                    .await
                    .unwrap();

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook1.get("load").unwrap()),
                });
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook2.get("load").unwrap()),
                });
                binding.borrow_mut().hooks.push(Hook {
                    resolve: None,
                    load: Some(hook3.get("load").unwrap()),
                });

                let binding = ctx.userdata::<RefCell<ModuleHookState>>().unwrap();
                let hooks = Rc::new(binding.borrow().hooks.clone());

                let result = call_load_hooks(&ctx, &hooks, "@pkg/module").unwrap();

                let globals = ctx.globals();
                assert_eq!(
                    globals.get::<_, String>("transformedUrl").unwrap(),
                    "node_modules/pkg/module"
                );
                assert_eq!(
                    globals.get::<_, String>("finalUrl").unwrap(),
                    "node_modules/pkg/module/index.js"
                );
                assert!(result.get::<_, bool>("shortCircuit").unwrap());
                assert_eq!(
                    result.get::<_, String>("source").unwrap(),
                    "export default {};"
                );
            })
        })
        .await;
    }
}
