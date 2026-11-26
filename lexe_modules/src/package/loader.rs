// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{cell::RefCell, fs::File, io::Read};

use lexe_json::escape::escape_json_string;
use lexe_utils::result::ResultExt;
use rquickjs::{
    loader::{ImportAttributes, Loader},
    Ctx, Error, Exception, Function, Module, Object, Result, Value,
};
use tracing::trace;

use crate::modules::path::{self, replace_backslash};
use crate::package::resolver::detect_file_format;
use crate::{module::ModuleCache, CJS_IMPORT_PREFIX, CJS_LOADER_PREFIX};

pub(crate) fn prepend_cjs_dirname_filename(filename: &str, source: &[u8]) -> Result<String> {
    let filename = replace_backslash(filename);
    let dirname = replace_backslash(path::dirname(&filename));
    let source = std::str::from_utf8(source).map_err(|_| {
        Error::new_loading_message(filename.as_str(), "invalid UTF-8 in CommonJS module")
    })?;

    let mut result = String::with_capacity(source.len() + filename.len() + dirname.len() + 48);
    result.push_str("var __filename=\"");
    escape_json_string(&mut result, filename.as_bytes());
    result.push_str("\";var __dirname=\"");
    escape_json_string(&mut result, dirname.as_bytes());
    result.push_str("\";\n");
    result.push_str(source);
    Ok(result)
}

#[derive(Debug, Default)]
pub struct PackageLoader;

impl PackageLoader {
    fn load_cjs_module<'js>(name: &str, ctx: Ctx<'js>) -> Result<Module<'js>> {
        let require: Function = ctx.globals().get("require")?;
        // Use the same resolution require() will use so cache keys align.
        let resolve: Function = require.get("resolve")?;
        let resolved: String = resolve.call((name,))?;
        let name = resolved.as_str();

        if let Some(binding) = ctx.userdata::<RefCell<ModuleCache>>() {
            if let Some(module) = binding.borrow().esm.get(name).cloned() {
                return Ok(module);
            }
        }

        // Preload through the normal CJS `_compile` path (CJS-first, ESM-syntax fallback)
        // rather than declaring the source as ESM via `__cjs:`.
        let export_object: Value = require.call((name,))?;

        // If require already instantiated this path as ESM (syntax fallback), reuse it.
        if let Some(binding) = ctx.userdata::<RefCell<ModuleCache>>() {
            if let Some(module) = binding.borrow().esm.get(name).cloned() {
                return Ok(module);
            }
        }

        let mut module = String::with_capacity(name.len() + 512);
        module.push_str("const value = require(\"");

        module.push_str(name);
        module.push_str("\");export default value.default||value;");
        if let Some(obj) = export_object.as_object() {
            let keys: Result<Vec<String>> = obj.keys().collect();
            let keys = keys?;

            if !keys.is_empty() {
                module.push_str("const{");

                for p in keys.iter() {
                    if p == "default" {
                        continue;
                    }
                    module.push_str(p);
                    module.push(',');
                }
                module.truncate(module.len() - 1);
                module.push_str("}=value;");
                module.push_str("export{");
                for p in keys.iter() {
                    if p == "default" {
                        continue;
                    }
                    module.push_str(p);
                    module.push(',');
                }
                module.truncate(module.len() - 1);
                module.push_str("};");
            }
        }
        let module = Module::declare(ctx.clone(), name, module)?;
        if let Some(binding) = ctx.userdata::<RefCell<ModuleCache>>() {
            binding.borrow_mut().esm.insert(name.into(), module.clone());
        }
        Ok(module)
    }

    fn normalize_name(name: &str) -> (bool, bool, &str, &str) {
        if !name.starts_with("__") {
            // If name doesn’t start with "__", return defaults
            return (false, false, name, name);
        }

        if let Some(cjs_path) = name.strip_prefix(CJS_IMPORT_PREFIX) {
            // If it starts with CJS_IMPORT_PREFIX, mark as from_cjs_import
            return (true, false, name, cjs_path);
        }

        if let Some(cjs_path) = name.strip_prefix(CJS_LOADER_PREFIX) {
            // If it starts with CJS_LOADER_PREFIX, mark as is_cjs
            return (false, true, cjs_path, cjs_path);
        }

        // Default return if no prefixes match
        (false, false, name, name)
    }

    fn load_module<'js>(
        ctx: &Ctx<'js>,
        name: &str,
        attr: &Option<ImportAttributes<'js>>,
    ) -> Result<(Module<'js>, Option<String>)> {
        let ctx = ctx.clone();

        let (from_cjs_import, is_cjs, normalized_name, raw_path) = Self::normalize_name(name);

        let url_path = crate::module::resolver::file_url_to_path_and_query(raw_path);
        let fs_path_owned;
        let path: &str = match &url_path {
            Some((fs_path, _)) => {
                fs_path_owned = fs_path.clone();
                fs_path_owned.as_str()
            },
            None => raw_path,
        };

        //json files can never be from CJS imports as they are handled by require
        if !from_cjs_import {
            if path.ends_with(".json") {
                let attr_json = attr
                    .clone()
                    .and_then(|v| v.get_type().ok().flatten())
                    .is_some_and(|v| v == "json");

                if !attr_json {
                    let msg = ["'", path, "' needs an import attribute of 'type: json'"].concat();
                    return Err(Exception::throw_type(&ctx, &msg));
                }

                let mut file = File::open(path)?;
                let mut contents = Vec::new();
                file.read_to_end(&mut contents)?;
                let mut json = String::with_capacity(contents.len() + 40);
                json.push_str("export default JSON.parse(\"");
                escape_json_string(&mut json, &contents);
                json.push_str("\");");
                trace!("+- Loading module as json: {}\n", normalized_name);
                return Ok((Module::declare(ctx, path, json)?, None));
            }
            // .mjs → ESM; .cjs → CJS; .js / extensionless follow package "type"
            // (and `__cjsm:` always forces the CJS facade).
            let format = if is_cjs {
                crate::package::resolver::DetectedFormat::Cjs {
                    allow_esm_detect: false,
                }
            } else {
                detect_file_format(path).map_err(|err| err.throw(&ctx))?
            };
            if format.is_cjs() {
                let url = if url_path.is_some() {
                    raw_path.to_string()
                } else {
                    crate::module::resolver::path_or_url_to_file_url(path)
                };
                trace!("+- Loading cjs module: {}\n", normalized_name);
                return Ok((Self::load_cjs_module(path, ctx)?, Some(url)));
            }
        }

        let resolved_fs = path::resolve_path([path].iter())?;
        let cache_key = if url_path.is_some() {
            raw_path
        } else {
            resolved_fs.as_str()
        };

        let binding = ctx.userdata::<RefCell<ModuleCache>>().or_throw(&ctx)?;

        let module = if let Some(module) = {
            let cache = binding.borrow();
            cache.esm.get(cache_key).cloned()
        } {
            trace!("+- Loading esm module via cache: {}\n", cache_key);
            (module, None)
        } else {
            let bytes = std::fs::read(&resolved_fs)?;
            let module = if from_cjs_import {
                trace!("+- Loading cjs module via import: {}\n", resolved_fs);
                let source = prepend_cjs_dirname_filename(&resolved_fs, &bytes)?;
                Module::declare(ctx.clone(), normalized_name, source)?
            } else {
                let mut source: &[u8] = &bytes;

                if source.starts_with(b"#!") {
                    source = source.splitn(2, |&c| c == b'\n').nth(1).unwrap_or(source);
                }

                trace!("+- Loading esm module: {}\n", resolved_fs);
                Module::declare(ctx.clone(), normalized_name, source)?
            };
            {
                let mut cache = binding.borrow_mut();
                cache.esm.insert(cache_key.into(), module.clone());
            }

            let url = if url_path.is_some() {
                raw_path.to_string()
            } else {
                crate::module::resolver::path_or_url_to_file_url(&resolved_fs)
            };
            (module, Some(url))
        };

        Ok(module)
    }
}

impl Loader for PackageLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        attr: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js>> {
        trace!("Try load '{}'", name);
        let (module, url) = Self::load_module(ctx, name, &attr)?;
        if let Some(url) = url {
            let meta: Object = module.meta()?;
            crate::module::resolver::populate_import_meta(ctx, &meta, &url)?;
        }

        Ok(module)
    }
}

#[cfg(test)]
mod tests {
    use super::prepend_cjs_dirname_filename;

    #[test]
    fn prepend_cjs_dirname_filename_escapes_special_characters() {
        let source = b"module.exports = __filename;";
        let result =
            prepend_cjs_dirname_filename(r#"C:\proj\quote"path\file.cjs"#, source).unwrap();
        assert!(result.starts_with("var __filename=\""));
        assert!(result.contains("quote\\\"path"));
        assert!(result.contains("var __dirname="));
        assert!(!result.contains("const __filename"));
        assert!(!result.contains("const __dirname"));
        assert!(result.ends_with("module.exports = __filename;"));
    }

    #[test]
    fn prepend_cjs_dirname_filename_uses_forward_slashes() {
        let source = b"module.exports = { __filename, __dirname };";
        let result = prepend_cjs_dirname_filename("/a/b/module.cjs", source).unwrap();
        assert!(result.contains("var __filename=\"/a/b/module.cjs\""));
        assert!(result.contains("var __dirname=\"/a/b\""));
    }

    #[test]
    fn prepend_cjs_dirname_filename_uses_var_bindings() {
        let source = b"__dirname = '/mutated';";
        let result = prepend_cjs_dirname_filename("/a/b/module.cjs", source).unwrap();
        assert!(result.starts_with("var __filename="));
        assert!(result.contains(";var __dirname="));
    }
}
