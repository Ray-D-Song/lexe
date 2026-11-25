// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::ffi::c_void;

use rquickjs::{
    class::Trace, function::Opt, object::Property, qjs, Ctx, Error, Function, JsLifetime, Object,
    Result, Value,
};

use crate::{
    cache::{decode_cache, encode_cache, extract_cached_data_bytes, to_buffer, CacheKind},
    context::{
        create_context_with_prefix, drain_child_microtasks_if_needed, ensure_runtime_state,
        is_default_loader_symbol, is_non_function_object, lookup_context_meta,
        parse_dynamic_import_option, register_named_import_hook, DynamicImportGuard,
        DynamicImportSpec,
    },
    errors::{
        into_raw_value, throw_invalid_arg_type, transfer_exception, validated_cstring, JsCString,
    },
    interrupt::{validate_break_on_sigint, validate_timeout, InterruptGuard},
};

const DEFAULT_FILENAME: &str = "evalmachine.<anonymous>";

pub fn parse_source_directives(source: &str) -> (String, String) {
    let mut source_url = String::new();
    let mut source_map_url = String::new();

    for line in source.lines() {
        let trimmed = line.trim_start();
        let rest = if let Some(r) = trimmed.strip_prefix("//#") {
            r
        } else if let Some(r) = trimmed.strip_prefix("//@") {
            r
        } else {
            continue;
        };
        let rest = rest.trim_start_matches([' ', '\t']);
        if let Some(after) = rest.strip_prefix("sourceURL=") {
            if let Some(val) = after.split_whitespace().next() {
                source_url = val.to_string();
            }
        } else if let Some(after) = rest.strip_prefix("sourceMappingURL=") {
            if let Some(val) = after.split_whitespace().next() {
                source_map_url = val.to_string();
            }
        }
    }
    (source_url, source_map_url)
}

pub fn build_cache_meta(
    kind: &str,
    code_bytes: &[u8],
    filename: &str,
    line_offset: i32,
    extra: &str,
) -> Vec<u8> {
    let mut meta =
        Vec::with_capacity(kind.len() + filename.len() + extra.len() + code_bytes.len() + 16);
    meta.extend_from_slice(kind.as_bytes());
    meta.push(0);
    meta.extend_from_slice(filename.as_bytes());
    meta.push(0);
    meta.extend_from_slice(&line_offset.to_le_bytes());
    meta.extend_from_slice(extra.as_bytes());
    meta.push(0);
    meta.extend_from_slice(code_bytes);
    meta
}

/// Serialize a QuickJS bytecode function or module via `JS_WriteObject`.
///
/// # Safety
/// `ctx_ptr` must be valid and `val` must be a compiled function bytecode or unlinked module.
pub unsafe fn serialize_bytecode(
    ctx_ptr: *mut qjs::JSContext,
    val: qjs::JSValue,
) -> Result<Vec<u8>> {
    let mut size: qjs::size_t = 0;
    let ptr = qjs::JS_WriteObject(
        ctx_ptr,
        &mut size,
        val,
        qjs::JS_WRITE_OBJ_BYTECODE as std::ffi::c_int,
    );
    if ptr.is_null() {
        return Err(Error::Exception);
    }
    let slice = std::slice::from_raw_parts(ptr, size as usize);
    let vec = slice.to_vec();
    qjs::js_free(ctx_ptr, ptr as *mut c_void);
    Ok(vec)
}

fn compile_global_bytecode(
    compile_ctx: &Ctx<'_>,
    code_bytes: &[u8],
    code_ptr: *const std::ffi::c_char,
    filename: &str,
    line_num: i32,
) -> Result<Vec<u8>> {
    let filename_cstr = validated_cstring(compile_ctx, "options.filename", filename)?;
    let ctx_raw = compile_ctx.as_raw().as_ptr();
    let mut eval_opts = qjs::JSEvalOptions {
        version: qjs::JS_EVAL_OPTIONS_VERSION as std::ffi::c_int,
        eval_flags: (qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as std::ffi::c_int,
        filename: filename_cstr.as_ptr(),
        line_num: line_num as std::ffi::c_int,
    };
    let compiled = unsafe {
        qjs::JS_Eval2(
            ctx_raw,
            code_ptr,
            code_bytes.len() as qjs::size_t,
            &mut eval_opts,
        )
    };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(compiled) == qjs::JS_TAG_EXCEPTION } {
        unsafe { qjs::JS_FreeValue(ctx_raw, compiled) };
        return Err(Error::Exception);
    }
    let bytecode = unsafe { serialize_bytecode(ctx_raw, compiled) };
    unsafe { qjs::JS_FreeValue(ctx_raw, compiled) };
    bytecode
}

fn validate_compiled_bytecode(ctx: &Ctx<'_>, raw_bytes: &[u8]) -> bool {
    let ctx_raw = ctx.as_raw().as_ptr();
    let obj = unsafe {
        qjs::JS_ReadObject(
            ctx_raw,
            raw_bytes.as_ptr(),
            raw_bytes.len() as qjs::size_t,
            qjs::JS_READ_OBJ_BYTECODE as std::ffi::c_int,
        )
    };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(obj) == qjs::JS_TAG_EXCEPTION } {
        unsafe {
            qjs::JS_FreeValue(ctx_raw, obj);
            let exc = qjs::JS_GetException(ctx_raw);
            qjs::JS_FreeValue(ctx_raw, exc);
        }
        return false;
    }
    unsafe { qjs::JS_FreeValue(ctx_raw, obj) };
    true
}

#[derive(JsLifetime, Trace)]
#[rquickjs::class]
pub struct VmScriptNative<'js> {
    #[qjs(skip_trace)]
    bytecode: Vec<u8>,
    #[qjs(skip_trace)]
    meta_key: Vec<u8>,
    #[qjs(skip_trace)]
    source_url: String,
    #[qjs(skip_trace)]
    source_map_url: String,
    #[qjs(skip_trace)]
    filename: String,
    #[qjs(skip_trace)]
    import_kind: u8, // 0=none, 1=defaultLoader, 2=custom
    import_fn: Option<Function<'js>>,
    #[qjs(skip_trace)]
    cached_data_rejected: Option<bool>,
    #[qjs(skip_trace)]
    produce_cached_data: bool,
}

#[rquickjs::methods]
impl<'js> VmScriptNative<'js> {
    #[qjs(rename = "createCachedData")]
    pub fn create_cached_data(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let wrapped = encode_cache(CacheKind::Script, &self.meta_key, &self.bytecode);
        to_buffer(&ctx, wrapped)
    }

    #[qjs(rename = "sourceURL", get)]
    pub fn source_url(&self) -> String {
        self.source_url.clone()
    }

    #[qjs(rename = "sourceMapURL", get)]
    pub fn source_map_url(&self) -> String {
        self.source_map_url.clone()
    }

    #[qjs(rename = "cachedDataProduced", get)]
    pub fn cached_data_produced(&self) -> Option<bool> {
        if self.produce_cached_data {
            Some(true)
        } else {
            None
        }
    }

    #[qjs(rename = "cachedDataRejected", get)]
    pub fn cached_data_rejected(&self) -> Option<bool> {
        self.cached_data_rejected
    }
}

impl<'js> VmScriptNative<'js> {
    fn import_spec(&self, referencing: &Value<'js>) -> DynamicImportSpec<'js> {
        match self.import_kind {
            1 => DynamicImportSpec::MainContextDefaultLoader,
            2 => {
                if let Some(f) = &self.import_fn {
                    DynamicImportSpec::Custom {
                        callback: f.clone(),
                        referencing: referencing.clone(),
                    }
                } else {
                    DynamicImportSpec::Missing
                }
            },
            _ => DynamicImportSpec::Missing,
        }
    }

    pub fn eval_in_target(
        &self,
        caller_ctx: &Ctx<'js>,
        target_ctx: *mut qjs::JSContext,
        after_eval: bool,
        referencing: &Value<'js>,
        run_options: Option<Value<'js>>,
    ) -> Result<Value<'js>> {
        let (timeout_ms, break_on_sigint) = parse_run_options(caller_ctx, run_options.as_ref())?;
        let spec = self.import_spec(referencing);
        register_named_import_hook(caller_ctx, target_ctx, &self.filename, &spec)?;

        let _import_guard = DynamicImportGuard::enter(caller_ctx, target_ctx, &spec)?;
        let interrupt_guard = InterruptGuard::enter(caller_ctx, timeout_ms, break_on_sigint);

        let fun_obj = unsafe {
            qjs::JS_ReadObject(
                target_ctx,
                self.bytecode.as_ptr(),
                self.bytecode.len() as qjs::size_t,
                qjs::JS_READ_OBJ_BYTECODE as std::ffi::c_int,
            )
        };
        if unsafe { qjs::JS_VALUE_GET_NORM_TAG(fun_obj) == qjs::JS_TAG_EXCEPTION } {
            unsafe { qjs::JS_FreeValue(target_ctx, fun_obj) };
            return Err(unsafe { transfer_exception(target_ctx, caller_ctx) });
        }

        let res = unsafe { qjs::JS_EvalFunction(target_ctx, fun_obj) };
        if unsafe { qjs::JS_VALUE_GET_NORM_TAG(res) == qjs::JS_TAG_EXCEPTION } {
            unsafe { qjs::JS_FreeValue(target_ctx, res) };
            if let Some(err) =
                unsafe { interrupt_guard.map_triggered_error(target_ctx, caller_ctx) }
            {
                return Err(err);
            }
            if target_ctx == caller_ctx.as_raw().as_ptr() {
                return Err(Error::Exception);
            }
            return Err(unsafe { transfer_exception(target_ctx, caller_ctx) });
        }

        if let Err(err) =
            drain_child_microtasks_if_needed(caller_ctx, target_ctx, after_eval, &interrupt_guard)
        {
            unsafe { qjs::JS_FreeValue(target_ctx, res) };
            return Err(err);
        }

        Ok(unsafe { Value::from_raw(caller_ctx.clone(), res) })
    }
}

fn parse_run_options(ctx: &Ctx<'_>, opts_val: Option<&Value<'_>>) -> Result<(Option<u64>, bool)> {
    let Some(val) = opts_val else {
        return Ok((None, false));
    };
    if val.is_undefined() {
        return Ok((None, false));
    }
    if !is_non_function_object(val) {
        return Err(throw_invalid_arg_type(
            ctx,
            "options",
            "of type object",
            val,
        ));
    }
    let obj = val.as_object().unwrap();
    let timeout_val: Value = obj.get("timeout")?;
    let timeout_ms = validate_timeout(ctx, Some(&timeout_val))?;
    let sigint_val: Value = obj.get("breakOnSigint")?;
    let break_on_sigint = validate_break_on_sigint(ctx, Some(&sigint_val))?;
    let display_val: Value = obj.get("displayErrors")?;
    if !display_val.is_undefined() && display_val.as_bool().is_none() {
        return Err(throw_invalid_arg_type(
            ctx,
            "options.displayErrors",
            "of type boolean",
            &display_val,
        ));
    }
    Ok((timeout_ms, break_on_sigint))
}

struct ParsedScriptOptions<'js> {
    filename: String,
    line_offset: i32,
    produce_cached_data: bool,
    cached_data: Option<Vec<u8>>,
    import_kind: u8,
    import_fn: Option<Function<'js>>,
}

fn parse_script_options<'js>(
    ctx: &Ctx<'js>,
    options: Option<Value<'js>>,
) -> Result<ParsedScriptOptions<'js>> {
    let mut parsed = ParsedScriptOptions {
        filename: DEFAULT_FILENAME.to_string(),
        line_offset: 0,
        produce_cached_data: false,
        cached_data: None,
        import_kind: 0,
        import_fn: None,
    };

    let Some(opts_val) = options else {
        return Ok(parsed);
    };
    if opts_val.is_undefined() {
        return Ok(parsed);
    }
    if let Some(s) = opts_val.as_string() {
        parsed.filename = s.to_string()?;
        return Ok(parsed);
    }
    if !is_non_function_object(&opts_val) {
        return Err(throw_invalid_arg_type(
            ctx,
            "options",
            "a string or an object",
            &opts_val,
        ));
    }
    let opts = opts_val.as_object().unwrap();

    let fn_val: Value = opts.get("filename")?;
    if !fn_val.is_undefined() {
        let s = fn_val.as_string().ok_or_else(|| {
            throw_invalid_arg_type(ctx, "options.filename", "of type string", &fn_val)
        })?;
        parsed.filename = s.to_string()?;
    }

    let line_val: Value = opts.get("lineOffset")?;
    if !line_val.is_undefined() {
        let n = line_val.as_number().ok_or_else(|| {
            throw_invalid_arg_type(ctx, "options.lineOffset", "of type number", &line_val)
        })?;
        parsed.line_offset = n as i32;
    }

    let col_val: Value = opts.get("columnOffset")?;
    if !col_val.is_undefined() && col_val.as_number().is_none() {
        return Err(throw_invalid_arg_type(
            ctx,
            "options.columnOffset",
            "of type number",
            &col_val,
        ));
    }

    let prod_val: Value = opts.get("produceCachedData")?;
    if !prod_val.is_undefined() {
        parsed.produce_cached_data = prod_val.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(
                ctx,
                "options.produceCachedData",
                "of type boolean",
                &prod_val,
            )
        })?;
    }

    let cd_val: Value = opts.get("cachedData")?;
    if !cd_val.is_undefined() {
        parsed.cached_data = Some(extract_cached_data_bytes(
            ctx,
            "options.cachedData",
            &cd_val,
        )?);
    }

    let dyn_val: Value = opts.get("importModuleDynamically")?;
    if !dyn_val.is_undefined() {
        if is_default_loader_symbol(ctx, &dyn_val)? {
            parsed.import_kind = 1;
        } else if let Some(f) = dyn_val.clone().into_function() {
            parsed.import_kind = 2;
            parsed.import_fn = Some(f);
        } else {
            return Err(throw_invalid_arg_type(
                ctx,
                "options.importModuleDynamically",
                "of type function or vm.constants.USE_MAIN_CONTEXT_DEFAULT_LOADER",
                &dyn_val,
            ));
        }
    }

    Ok(parsed)
}

fn compile_script_native<'js>(
    ctx: &Ctx<'js>,
    code: Value<'js>,
    options: Option<Value<'js>>,
) -> Result<(VmScriptNative<'js>, Option<Value<'js>>)> {
    ensure_runtime_state(ctx)?;
    if !code.is_string() {
        return Err(throw_invalid_arg_type(ctx, "code", "of type string", &code));
    }
    let code_cstr = JsCString::from_value(ctx, &code)?;
    let opts = parse_script_options(ctx, options)?;
    let _ = validated_cstring(ctx, "options.filename", &opts.filename)?;

    let lossy = String::from_utf8_lossy(code_cstr.as_bytes());
    let (source_url, source_map_url) = parse_source_directives(&lossy);

    let line_num = 1_i32.saturating_add(opts.line_offset);
    let meta_key = build_cache_meta("script", code_cstr.as_bytes(), &opts.filename, line_num, "");

    let mut cached_data_rejected = None;
    let mut bytecode_opt = None;

    if let Some(cd) = &opts.cached_data {
        if let Some(payload) = decode_cache(CacheKind::Script, &meta_key, cd) {
            if validate_compiled_bytecode(ctx, payload) {
                cached_data_rejected = Some(false);
                bytecode_opt = Some(payload.to_vec());
            } else {
                cached_data_rejected = Some(true);
            }
        } else {
            cached_data_rejected = Some(true);
        }
    }

    let bytecode = match bytecode_opt {
        Some(b) => b,
        None => {
            let (ptr, _len) = code_cstr.as_ptr_len();
            compile_global_bytecode(ctx, code_cstr.as_bytes(), ptr, &opts.filename, line_num)?
        },
    };

    let cached_data_val = if opts.produce_cached_data {
        let encoded = encode_cache(CacheKind::Script, &meta_key, &bytecode);
        Some(to_buffer(ctx, encoded)?)
    } else {
        None
    };

    Ok((
        VmScriptNative {
            bytecode,
            meta_key,
            source_url,
            source_map_url,
            filename: opts.filename,
            import_kind: opts.import_kind,
            import_fn: opts.import_fn,
            cached_data_rejected,
            produce_cached_data: opts.produce_cached_data,
        },
        cached_data_val,
    ))
}

pub fn install_script_api<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let natives = Object::new(ctx.clone())?;
    natives.set(
        "createScriptNative",
        rquickjs::prelude::Func::from(
            |ctx: Ctx<'js>,
             code: Opt<Value<'js>>,
             options: Opt<Value<'js>>|
             -> Result<Value<'js>> {
                let code_val = code.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                let (native, cd_buf) = compile_script_native(&ctx, code_val, options.0)?;
                let produce = native.produce_cached_data;
                let rejected = native.cached_data_rejected;
                let src_url = native.source_url.clone();
                let map_url = native.source_map_url.clone();
                let inst = rquickjs::Class::instance(ctx.clone(), native)?;
                let ret = Object::new(ctx.clone())?;
                ret.set("native", inst)?;
                ret.set("sourceURL", src_url)?;
                ret.set("sourceMapURL", map_url)?;
                if let Some(buf) = cd_buf {
                    ret.set("cachedData", buf)?;
                }
                if produce {
                    ret.set("cachedDataProduced", true)?;
                }
                if let Some(r) = rejected {
                    ret.set("cachedDataRejected", r)?;
                }
                Ok(ret.into_value())
            },
        ),
    )?;
    natives.set(
        "runInThisContextNative",
        rquickjs::prelude::Func::from(
            |ctx: Ctx<'js>,
             native: rquickjs::Class<'js, VmScriptNative<'js>>,
             referencing: Value<'js>,
             options: Option<Value<'js>>|
             -> Result<Value<'js>> {
                native.borrow().eval_in_target(
                    &ctx,
                    ctx.as_raw().as_ptr(),
                    false,
                    &referencing,
                    options,
                )
            },
        ),
    )?;
    natives.set(
        "runInContextNative",
        rquickjs::prelude::Func::from(
            |ctx: Ctx<'js>,
             native: rquickjs::Class<'js, VmScriptNative<'js>>,
             referencing: Value<'js>,
             sandbox: Value<'js>,
             options: Option<Value<'js>>|
             -> Result<Value<'js>> {
                let meta = lookup_context_meta(&ctx, &sandbox)?.ok_or_else(|| {
                    throw_invalid_arg_type(
                        &ctx,
                        "contextifiedObject",
                        "an instance of vm.Context",
                        &sandbox,
                    )
                })?;
                native.borrow().eval_in_target(
                    &ctx,
                    meta.raw_ctx,
                    meta.microtask_mode_after_eval,
                    &referencing,
                    options,
                )
            },
        ),
    )?;
    natives.set(
        "runInNewContextNative",
        rquickjs::prelude::Func::from(
            |ctx: Ctx<'js>,
             native: rquickjs::Class<'js, VmScriptNative<'js>>,
             referencing: Value<'js>,
             context_obj: Option<Value<'js>>,
             options: Option<Value<'js>>|
             -> Result<Value<'js>> {
                let sb = create_context_with_prefix(&ctx, context_obj, options.clone(), "context")?;
                let meta = lookup_context_meta(&ctx, &sb)?.ok_or(Error::Unknown)?;
                native.borrow().eval_in_target(
                    &ctx,
                    meta.raw_ctx,
                    meta.microtask_mode_after_eval,
                    &referencing,
                    options,
                )
            },
        ),
    )?;

    let install: Function<'js> = ctx.eval(
        r#"((natives) => {
            const kNative = Symbol("vm_script_native");
            function Script(code, options) {
                if (!(this instanceof Script)) {
                    return new Script(code, options);
                }
                const info = natives.createScriptNative(code, options);
                this[kNative] = info.native;
                this.sourceURL = info.sourceURL;
                this.sourceMapURL = info.sourceMapURL;
                this.cachedData = info.cachedData;
                this.cachedDataProduced = info.cachedDataProduced;
                this.cachedDataRejected = info.cachedDataRejected;
            }
            Script.prototype.runInThisContext = function runInThisContext(options) {
                if (!this || !this[kNative]) {
                    throw new TypeError("Illegal invocation");
                }
                return natives.runInThisContextNative(this[kNative], this, options);
            };
            Script.prototype.runInContext = function runInContext(contextifiedObject, options) {
                if (!this || !this[kNative]) {
                    throw new TypeError("Illegal invocation");
                }
                return natives.runInContextNative(this[kNative], this, contextifiedObject, options);
            };
            Script.prototype.runInNewContext = function runInNewContext(contextObject, options) {
                if (!this || !this[kNative]) {
                    throw new TypeError("Illegal invocation");
                }
                return natives.runInNewContextNative(this[kNative], this, contextObject, options);
            };
            Script.prototype.createCachedData = function createCachedData() {
                if (!this || !this[kNative]) {
                    throw new TypeError("Illegal invocation");
                }
                return this[kNative].createCachedData();
            };
            function createScript(code, options) {
                return new Script(code, options);
            }
            return { Script, createScript };
        })"#,
    )?;

    install.call((natives,))
}

pub fn run_in_this_context_js<'js>(
    ctx: Ctx<'js>,
    code: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let code_val = code.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    run_in_this_context_impl(ctx, code_val, options.0)
}

pub fn run_in_this_context_impl<'js>(
    ctx: Ctx<'js>,
    code: Value<'js>,
    options: Option<Value<'js>>,
) -> Result<Value<'js>> {
    let (native, _) = compile_script_native(&ctx, code, options.clone())?;
    let ref_obj = Object::new(ctx.clone())?.into_value();
    native.eval_in_target(&ctx, ctx.as_raw().as_ptr(), false, &ref_obj, options)
}

pub fn run_in_context_js<'js>(
    ctx: Ctx<'js>,
    code: Opt<Value<'js>>,
    contextified_object: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let code_val = code.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    let ctx_val = contextified_object
        .0
        .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    run_in_context_impl(ctx, code_val, ctx_val, options.0)
}

pub fn run_in_context_impl<'js>(
    ctx: Ctx<'js>,
    code: Value<'js>,
    contextified_object: Value<'js>,
    options: Option<Value<'js>>,
) -> Result<Value<'js>> {
    if !code.is_string() {
        return Err(throw_invalid_arg_type(
            &ctx,
            "code",
            "of type string",
            &code,
        ));
    }
    let meta = lookup_context_meta(&ctx, &contextified_object)?.ok_or_else(|| {
        throw_invalid_arg_type(
            &ctx,
            "contextifiedObject",
            "an instance of vm.Context",
            &contextified_object,
        )
    })?;
    let (native, _) = compile_script_native(&ctx, code, options.clone())?;
    let ref_obj = Object::new(ctx.clone())?.into_value();
    native.eval_in_target(
        &ctx,
        meta.raw_ctx,
        meta.microtask_mode_after_eval,
        &ref_obj,
        options,
    )
}

pub fn run_in_new_context_js<'js>(
    ctx: Ctx<'js>,
    code: Opt<Value<'js>>,
    context_object: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let code_val = code.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    run_in_new_context_impl(ctx, code_val, context_object.0, options.0)
}

pub fn run_in_new_context_impl<'js>(
    ctx: Ctx<'js>,
    code: Value<'js>,
    context_object: Option<Value<'js>>,
    options: Option<Value<'js>>,
) -> Result<Value<'js>> {
    let script_opts = parse_script_options(&ctx, options.clone())?;
    let _ = validated_cstring(&ctx, "options.filename", &script_opts.filename)?;
    // Validate `code` and `filename` cleanly before creating context?
    if !code.is_string() {
        return Err(throw_invalid_arg_type(
            &ctx,
            "code",
            "of type string",
            &code,
        ));
    }
    let sb = create_context_with_prefix(&ctx, context_object, options.clone(), "context")?;
    let meta = lookup_context_meta(&ctx, &sb)?.ok_or(Error::Unknown)?;
    let (native, _) = compile_script_native(&ctx, code, options.clone())?;
    let ref_obj = Object::new(ctx.clone())?.into_value();
    native.eval_in_target(
        &ctx,
        meta.raw_ctx,
        meta.microtask_mode_after_eval,
        &ref_obj,
        options,
    )
}

pub fn compile_function_js<'js>(
    ctx: Ctx<'js>,
    code: Opt<Value<'js>>,
    params: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let code_val = code.0.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    compile_function_impl(ctx, code_val, params.0, options.0)
}

pub fn compile_function_impl<'js>(
    ctx: Ctx<'js>,
    code: Value<'js>,
    params: Option<Value<'js>>,
    options: Option<Value<'js>>,
) -> Result<Value<'js>> {
    ensure_runtime_state(&ctx)?;
    if !code.is_string() {
        return Err(throw_invalid_arg_type(
            &ctx,
            "code",
            "of type string",
            &code,
        ));
    }
    let code_cstr = JsCString::from_value(&ctx, &code)?;

    let mut param_list: Vec<String> = Vec::new();
    if let Some(p_val) = params {
        if !p_val.is_undefined() {
            let arr = p_val.as_array().ok_or_else(|| {
                throw_invalid_arg_type(&ctx, "params", "an instance of Array", &p_val)
            })?;
            for item in arr.iter::<Value<'js>>() {
                let item = item?;
                let s = item.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(&ctx, "params[]", "of type string", &item)
                })?;
                param_list.push(s.to_string()?);
            }
        }
    }

    let mut filename = String::new();
    let mut line_offset: i32 = 0;
    let mut produce_cached_data = false;
    let mut cached_data_bytes: Option<Vec<u8>> = None;
    let mut target_ctx_raw = ctx.as_raw().as_ptr();
    let mut context_extensions: Vec<Value<'js>> = Vec::new();
    let mut import_option: Value<'js> = Value::new_undefined(ctx.clone());

    if let Some(opts_val) = options {
        if !opts_val.is_undefined() {
            if !is_non_function_object(&opts_val) {
                return Err(throw_invalid_arg_type(
                    &ctx,
                    "options",
                    "of type object",
                    &opts_val,
                ));
            }
            let opts = opts_val.as_object().unwrap();

            let fn_val: Value = opts.get("filename")?;
            if !fn_val.is_undefined() {
                let s = fn_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(&ctx, "options.filename", "of type string", &fn_val)
                })?;
                filename = s.to_string()?;
            }

            let line_val: Value = opts.get("lineOffset")?;
            if !line_val.is_undefined() {
                let n = line_val.as_number().ok_or_else(|| {
                    throw_invalid_arg_type(&ctx, "options.lineOffset", "of type number", &line_val)
                })?;
                line_offset = n as i32;
            }

            let col_val: Value = opts.get("columnOffset")?;
            if !col_val.is_undefined() && col_val.as_number().is_none() {
                return Err(throw_invalid_arg_type(
                    &ctx,
                    "options.columnOffset",
                    "of type number",
                    &col_val,
                ));
            }

            let prod_val: Value = opts.get("produceCachedData")?;
            if !prod_val.is_undefined() {
                produce_cached_data = prod_val.as_bool().ok_or_else(|| {
                    throw_invalid_arg_type(
                        &ctx,
                        "options.produceCachedData",
                        "of type boolean",
                        &prod_val,
                    )
                })?;
            }

            let cd_val: Value = opts.get("cachedData")?;
            if !cd_val.is_undefined() {
                cached_data_bytes = Some(extract_cached_data_bytes(
                    &ctx,
                    "options.cachedData",
                    &cd_val,
                )?);
            }

            let pc_val: Value = opts.get("parsingContext")?;
            if !pc_val.is_undefined() {
                let meta = lookup_context_meta(&ctx, &pc_val)?.ok_or_else(|| {
                    throw_invalid_arg_type(
                        &ctx,
                        "options.parsingContext",
                        "an instance of vm.Context",
                        &pc_val,
                    )
                })?;
                target_ctx_raw = meta.raw_ctx;
            }

            let ext_val: Value = opts.get("contextExtensions")?;
            if !ext_val.is_undefined() {
                let arr = ext_val.as_array().ok_or_else(|| {
                    throw_invalid_arg_type(
                        &ctx,
                        "options.contextExtensions",
                        "an instance of Array",
                        &ext_val,
                    )
                })?;
                for item in arr.iter::<Value<'js>>() {
                    let item = item?;
                    if !is_non_function_object(&item) {
                        return Err(throw_invalid_arg_type(
                            &ctx,
                            "options.contextExtensions[...]",
                            "of type object",
                            &item,
                        ));
                    }
                    context_extensions.push(item);
                }
            }

            import_option = opts.get("importModuleDynamically")?;
        }
    }

    let eff_filename = if filename.is_empty() {
        DEFAULT_FILENAME.to_string()
    } else {
        filename
    };
    let filename_cstr = validated_cstring(&ctx, "options.filename", &eff_filename)?;
    let target_ctx = unsafe { Ctx::from_raw(std::ptr::NonNull::new_unchecked(target_ctx_raw)) };

    // Validate parameter syntax independently so malformed params cannot escape the wrapper.
    let params_joined = param_list.join(", ");
    let pcheck_src = format!("(function({params_joined}) {{}})");
    let pcheck_bytes = pcheck_src.into_bytes();
    let mut pcheck_with_nul = pcheck_bytes.clone();
    pcheck_with_nul.push(0);
    let mut pcheck_opts = qjs::JSEvalOptions {
        version: qjs::JS_EVAL_OPTIONS_VERSION as std::ffi::c_int,
        eval_flags: (qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as std::ffi::c_int,
        filename: filename_cstr.as_ptr(),
        line_num: 1,
    };
    let pcheck_res = unsafe {
        qjs::JS_Eval2(
            target_ctx_raw,
            pcheck_with_nul.as_ptr() as *const std::ffi::c_char,
            pcheck_bytes.len() as qjs::size_t,
            &mut pcheck_opts,
        )
    };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(pcheck_res) == qjs::JS_TAG_EXCEPTION } {
        unsafe { qjs::JS_FreeValue(target_ctx_raw, pcheck_res) };
        if target_ctx_raw == ctx.as_raw().as_ptr() {
            return Err(Error::Exception);
        }
        return Err(unsafe { transfer_exception(target_ctx_raw, &ctx) });
    }
    unsafe { qjs::JS_FreeValue(target_ctx_raw, pcheck_res) };

    // Build wrapper source preserving embedded NULs inside code bytes.
    let mut wrapper_bytes: Vec<u8> = Vec::new();
    if context_extensions.is_empty() {
        wrapper_bytes.extend_from_slice(b"(function(");
        wrapper_bytes.extend_from_slice(params_joined.as_bytes());
        wrapper_bytes.extend_from_slice(b") {");
        wrapper_bytes.extend_from_slice(code_cstr.as_bytes());
        wrapper_bytes.extend_from_slice(b"\n})");
    } else {
        wrapper_bytes.extend_from_slice(b"(function(__lexe_vm_exts) { ");
        for idx in 0..context_extensions.len() {
            wrapper_bytes.extend_from_slice(format!("with (__lexe_vm_exts[{idx}]) {{ ").as_bytes());
        }
        wrapper_bytes.extend_from_slice(b"return (function(");
        wrapper_bytes.extend_from_slice(params_joined.as_bytes());
        wrapper_bytes.extend_from_slice(b") {");
        wrapper_bytes.extend_from_slice(code_cstr.as_bytes());
        wrapper_bytes.extend_from_slice(b"\n}); ");
        for _ in 0..context_extensions.len() {
            wrapper_bytes.extend_from_slice(b"} ");
        }
        wrapper_bytes.extend_from_slice(b"})");
    }

    let line_num = 1_i32.saturating_add(line_offset);
    let extra_meta = format!("{params_joined}\0{}", context_extensions.len());
    let meta_key = build_cache_meta(
        "func",
        code_cstr.as_bytes(),
        &eff_filename,
        line_num,
        &extra_meta,
    );

    let mut cached_data_rejected = None;
    let mut bytecode_opt = None;
    if let Some(cd) = &cached_data_bytes {
        if let Some(payload) = decode_cache(CacheKind::Function, &meta_key, cd) {
            if validate_compiled_bytecode(&target_ctx, payload) {
                cached_data_rejected = Some(false);
                bytecode_opt = Some(payload.to_vec());
            } else {
                cached_data_rejected = Some(true);
            }
        } else {
            cached_data_rejected = Some(true);
        }
    }

    let bytecode = match bytecode_opt {
        Some(b) => b,
        None => {
            let mut src_with_nul = wrapper_bytes.clone();
            src_with_nul.push(0);
            let mut eval_opts = qjs::JSEvalOptions {
                version: qjs::JS_EVAL_OPTIONS_VERSION as std::ffi::c_int,
                eval_flags: (qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY)
                    as std::ffi::c_int,
                filename: filename_cstr.as_ptr(),
                line_num: line_num as std::ffi::c_int,
            };
            let compiled = unsafe {
                qjs::JS_Eval2(
                    target_ctx_raw,
                    src_with_nul.as_ptr() as *const std::ffi::c_char,
                    wrapper_bytes.len() as qjs::size_t,
                    &mut eval_opts,
                )
            };
            if unsafe { qjs::JS_VALUE_GET_NORM_TAG(compiled) == qjs::JS_TAG_EXCEPTION } {
                unsafe { qjs::JS_FreeValue(target_ctx_raw, compiled) };
                if target_ctx_raw == ctx.as_raw().as_ptr() {
                    return Err(Error::Exception);
                }
                return Err(unsafe { transfer_exception(target_ctx_raw, &ctx) });
            }
            let bytes_res = unsafe { serialize_bytecode(target_ctx_raw, compiled) };
            unsafe { qjs::JS_FreeValue(target_ctx_raw, compiled) };
            bytes_res?
        },
    };

    let compiled_val = unsafe {
        qjs::JS_ReadObject(
            target_ctx_raw,
            bytecode.as_ptr(),
            bytecode.len() as qjs::size_t,
            qjs::JS_READ_OBJ_BYTECODE as std::ffi::c_int,
        )
    };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(compiled_val) == qjs::JS_TAG_EXCEPTION } {
        unsafe { qjs::JS_FreeValue(target_ctx_raw, compiled_val) };
        if target_ctx_raw == ctx.as_raw().as_ptr() {
            return Err(Error::Exception);
        }
        return Err(unsafe { transfer_exception(target_ctx_raw, &ctx) });
    }

    let eval_res = unsafe { qjs::JS_EvalFunction(target_ctx_raw, compiled_val) };
    if unsafe { qjs::JS_VALUE_GET_NORM_TAG(eval_res) == qjs::JS_TAG_EXCEPTION } {
        unsafe { qjs::JS_FreeValue(target_ctx_raw, eval_res) };
        if target_ctx_raw == ctx.as_raw().as_ptr() {
            return Err(Error::Exception);
        }
        return Err(unsafe { transfer_exception(target_ctx_raw, &ctx) });
    }

    let func_raw = if context_extensions.is_empty() {
        eval_res
    } else {
        let ext_arr = rquickjs::Array::new(target_ctx.clone())?;
        for (i, item) in context_extensions.into_iter().enumerate() {
            let item_dup = unsafe {
                Value::from_raw(
                    target_ctx.clone(),
                    qjs::JS_DupValue(target_ctx_raw, item.as_raw()),
                )
            };
            ext_arr.set(i, item_dup)?;
        }
        let mut arg_raw = into_raw_value(ext_arr.into_value());
        let called =
            unsafe { qjs::JS_Call(target_ctx_raw, eval_res, qjs::JS_UNDEFINED, 1, &mut arg_raw) };
        unsafe {
            qjs::JS_FreeValue(target_ctx_raw, arg_raw);
            qjs::JS_FreeValue(target_ctx_raw, eval_res);
        }
        if unsafe { qjs::JS_VALUE_GET_NORM_TAG(called) == qjs::JS_TAG_EXCEPTION } {
            unsafe { qjs::JS_FreeValue(target_ctx_raw, called) };
            if target_ctx_raw == ctx.as_raw().as_ptr() {
                return Err(Error::Exception);
            }
            return Err(unsafe { transfer_exception(target_ctx_raw, &ctx) });
        }
        called
    };

    let func_val = unsafe { Value::from_raw(ctx.clone(), func_raw) };

    if !import_option.is_undefined() {
        let spec = parse_dynamic_import_option(
            &ctx,
            &import_option,
            &func_val,
            "options.importModuleDynamically",
        )?;
        register_named_import_hook(&ctx, target_ctx_raw, &eff_filename, &spec)?;
    }

    if let Some(func_obj) = func_val.as_object() {
        if produce_cached_data {
            let encoded = encode_cache(CacheKind::Function, &meta_key, &bytecode);
            let buf = to_buffer(&ctx, encoded)?;
            func_obj.prop("cachedData", Property::from(buf).configurable())?;
            func_obj.prop("cachedDataProduced", Property::from(true).configurable())?;
        }
        if let Some(rej) = cached_data_rejected {
            func_obj.prop("cachedDataRejected", Property::from(rej).configurable())?;
        }
    }

    Ok(func_val)
}
