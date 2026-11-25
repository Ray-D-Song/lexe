// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    cell::{Cell, RefCell},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use rquickjs::{
    class::Trace, prelude::Func, qjs, Class, Ctx, Error, Function, JsLifetime, Object, Result,
    Value,
};

use crate::{
    cache::{decode_cache, encode_cache, extract_cached_data_bytes, to_buffer, CacheKind},
    context::{
        alloc_module_id, drain_child_microtasks_if_needed, ensure_runtime_state,
        is_default_loader_symbol, is_non_function_object, lookup_context_meta,
        register_named_import_hook, register_synthetic_module, set_module_unwrap_fn,
        synthetic_module_init, DynamicImportGuard, DynamicImportSpec,
    },
    errors::{
        into_raw_value, throw_coded, throw_invalid_arg_type, throw_vm_module_already_linked,
        throw_vm_module_cannot_create_cached_data, throw_vm_module_different_context,
        throw_vm_module_status, transfer_exception, validated_cstring, ErrorKind, JsCString,
    },
    interrupt::{validate_break_on_sigint, validate_timeout, InterruptGuard},
    script::{build_cache_meta, serialize_bytecode},
};

#[derive(JsLifetime, Trace)]
#[rquickjs::class]
pub struct VmModuleCore<'js> {
    #[qjs(skip_trace)]
    module_ptr: *mut qjs::JSModuleDef,
    #[qjs(skip_trace)]
    target_ctx: *mut qjs::JSContext,
    #[qjs(skip_trace)]
    ctx_alive: Arc<AtomicBool>,
    #[qjs(skip_trace)]
    microtask_mode_after_eval: bool,
    #[qjs(skip_trace)]
    is_synthetic: bool,
    #[qjs(skip_trace)]
    identifier: String,
    context_obj: Value<'js>,
    #[qjs(skip_trace)]
    linking: Cell<bool>,
    #[qjs(skip_trace)]
    cached_bytecode: Option<Vec<u8>>,
    #[qjs(skip_trace)]
    meta_key: Vec<u8>,
    #[qjs(skip_trace)]
    cached_data_rejected: Option<bool>,
    #[qjs(skip_trace)]
    dep_specifiers: Vec<String>,
    initialize_import_meta: Option<Function<'js>>,
    #[qjs(skip_trace)]
    import_meta_initialized: Cell<bool>,
    #[qjs(skip_trace)]
    import_kind: u8,
    import_fn: Option<Function<'js>>,
    #[qjs(skip_trace)]
    export_names: Vec<String>,
    #[qjs(skip_trace)]
    req_set_count: RefCell<Vec<usize>>,
}

impl<'js> VmModuleCore<'js> {
    fn assert_alive(&self, ctx: &Ctx<'_>) -> Result<()> {
        if !self.ctx_alive.load(Ordering::SeqCst) || self.module_ptr.is_null() {
            return Err(throw_vm_module_status(
                ctx,
                "Module context has been destroyed",
            ));
        }
        Ok(())
    }

    fn status_str(&self) -> &'static str {
        if !self.ctx_alive.load(Ordering::SeqCst) || self.module_ptr.is_null() {
            return "unlinked";
        }
        let code = unsafe { qjs::JS_GetModuleStatus(self.module_ptr) };
        match code {
            0 => {
                if self.linking.get() {
                    "linking"
                } else {
                    "unlinked"
                }
            },
            1 => "linking",
            2 => "linked",
            3 | 4 => "evaluating",
            5 => "evaluated",
            6 => "errored",
            _ => "unlinked",
        }
    }

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
}

fn resolve_module_context<'js>(
    ctx: &Ctx<'js>,
    context_opt: Value<'js>,
    prop_name: &str,
) -> Result<(Value<'js>, *mut qjs::JSContext, Arc<AtomicBool>, bool)> {
    if context_opt.is_undefined() {
        let global_val = ctx.globals().into_value();
        return Ok((
            global_val,
            ctx.as_raw().as_ptr(),
            Arc::new(AtomicBool::new(true)),
            false,
        ));
    }
    let meta = lookup_context_meta(ctx, &context_opt)?.ok_or_else(|| {
        throw_invalid_arg_type(ctx, prop_name, "an instance of vm.Context", &context_opt)
    })?;
    Ok((
        context_opt,
        meta.raw_ctx,
        meta.alive,
        meta.microtask_mode_after_eval,
    ))
}

fn compile_source_text_module<'js>(
    ctx: &Ctx<'js>,
    code: Value<'js>,
    options: Option<Value<'js>>,
) -> Result<VmModuleCore<'js>> {
    ensure_runtime_state(ctx)?;
    if !code.is_string() {
        return Err(throw_invalid_arg_type(ctx, "code", "of type string", &code));
    }
    let code_cstr = JsCString::from_value(ctx, &code)?;

    let id = alloc_module_id(ctx)?;
    let mut identifier = format!("vm:module({id})");
    let mut line_offset: i32 = 0;
    let mut context_opt = Value::new_undefined(ctx.clone());
    let mut cached_data_bytes: Option<Vec<u8>> = None;
    let mut initialize_import_meta: Option<Function<'js>> = None;
    let mut import_kind: u8 = 0;
    let mut import_fn: Option<Function<'js>> = None;

    if let Some(opts_val) = options {
        if !opts_val.is_undefined() {
            if !is_non_function_object(&opts_val) {
                return Err(throw_invalid_arg_type(
                    ctx,
                    "options",
                    "of type object",
                    &opts_val,
                ));
            }
            let opts = opts_val.as_object().unwrap();

            let id_val: Value = opts.get("identifier")?;
            if !id_val.is_undefined() {
                let s = id_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(ctx, "options.identifier", "of type string", &id_val)
                })?;
                identifier = s.to_string()?;
            }

            let line_val: Value = opts.get("lineOffset")?;
            if !line_val.is_undefined() {
                let n = line_val.as_number().ok_or_else(|| {
                    throw_invalid_arg_type(ctx, "options.lineOffset", "of type number", &line_val)
                })?;
                line_offset = n as i32;
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

            context_opt = opts.get("context")?;

            let cd_val: Value = opts.get("cachedData")?;
            if !cd_val.is_undefined() {
                cached_data_bytes = Some(extract_cached_data_bytes(
                    ctx,
                    "options.cachedData",
                    &cd_val,
                )?);
            }

            let meta_fn_val: Value = opts.get("initializeImportMeta")?;
            if !meta_fn_val.is_undefined() {
                let f = meta_fn_val.clone().into_function().ok_or_else(|| {
                    throw_invalid_arg_type(
                        ctx,
                        "options.initializeImportMeta",
                        "of type function",
                        &meta_fn_val,
                    )
                })?;
                initialize_import_meta = Some(f);
            }

            let dyn_val: Value = opts.get("importModuleDynamically")?;
            if !dyn_val.is_undefined() {
                if is_default_loader_symbol(ctx, &dyn_val)? {
                    import_kind = 1;
                } else if let Some(f) = dyn_val.clone().into_function() {
                    import_kind = 2;
                    import_fn = Some(f);
                } else {
                    return Err(throw_invalid_arg_type(
                        ctx,
                        "options.importModuleDynamically",
                        "of type function or vm.constants.USE_MAIN_CONTEXT_DEFAULT_LOADER",
                        &dyn_val,
                    ));
                }
            }
        }
    }

    let id_cstr = validated_cstring(ctx, "options.identifier", &identifier)?;
    let (context_obj, target_ctx, ctx_alive, microtask_mode_after_eval) =
        resolve_module_context(ctx, context_opt, "options.context")?;

    let line_num = 1_i32.saturating_add(line_offset);
    let meta_key = build_cache_meta("module", code_cstr.as_bytes(), &identifier, line_num, "");

    let mut cached_data_rejected = None;
    let mut mod_val_opt: Option<qjs::JSValue> = None;

    if let Some(cd) = &cached_data_bytes {
        if let Some(payload) = decode_cache(CacheKind::Module, &meta_key, cd) {
            let loaded = unsafe {
                qjs::JS_ReadObject2(
                    target_ctx,
                    payload.as_ptr(),
                    payload.len() as qjs::size_t,
                    (qjs::JS_READ_OBJ_BYTECODE as std::ffi::c_int) | qjs::JS_READ_OBJ_NO_RESOLVE,
                    std::ptr::null_mut(),
                )
            };
            if unsafe { qjs::JS_VALUE_GET_NORM_TAG(loaded) == qjs::JS_TAG_EXCEPTION } {
                unsafe {
                    qjs::JS_FreeValue(target_ctx, loaded);
                    let exc = qjs::JS_GetException(target_ctx);
                    qjs::JS_FreeValue(target_ctx, exc);
                }
                cached_data_rejected = Some(true);
            } else if unsafe { qjs::JS_VALUE_GET_NORM_TAG(loaded) == qjs::JS_TAG_MODULE } {
                cached_data_rejected = Some(false);
                mod_val_opt = Some(loaded);
            } else {
                unsafe { qjs::JS_FreeValue(target_ctx, loaded) };
                cached_data_rejected = Some(true);
            }
        } else {
            cached_data_rejected = Some(true);
        }
    }

    let mod_val = match mod_val_opt {
        Some(v) => v,
        None => {
            let (ptr, len) = code_cstr.as_ptr_len();
            let mut eval_opts = qjs::JSEvalOptions {
                version: qjs::JS_EVAL_OPTIONS_VERSION as std::ffi::c_int,
                eval_flags: (qjs::JS_EVAL_TYPE_MODULE as std::ffi::c_int)
                    | (qjs::JS_EVAL_FLAG_COMPILE_ONLY as std::ffi::c_int)
                    | qjs::JS_EVAL_FLAG_NO_RESOLVE,
                filename: id_cstr.as_ptr(),
                line_num: line_num as std::ffi::c_int,
            };
            let v = unsafe { qjs::JS_Eval2(target_ctx, ptr, len as qjs::size_t, &mut eval_opts) };
            if unsafe { qjs::JS_VALUE_GET_NORM_TAG(v) == qjs::JS_TAG_EXCEPTION } {
                unsafe { qjs::JS_FreeValue(target_ctx, v) };
                if target_ctx == ctx.as_raw().as_ptr() {
                    return Err(Error::Exception);
                }
                return Err(unsafe { transfer_exception(target_ctx, ctx) });
            }
            v
        },
    };

    let m_ptr = unsafe { qjs::JS_ValueToModule(mod_val) };
    if m_ptr.is_null() {
        unsafe { qjs::JS_FreeValue(target_ctx, mod_val) };
        return Err(Error::Unknown);
    }
    unsafe { qjs::JS_SetModuleResolved(m_ptr, true) };

    // Serialize bytecode NOW while m->func_obj is still JS_TAG_FUNCTION_BYTECODE.
    let bytecode = unsafe { serialize_bytecode(target_ctx, mod_val) };
    unsafe { qjs::JS_FreeValue(target_ctx, mod_val) };
    let bytecode = bytecode?;

    let req_count = unsafe { qjs::JS_GetModuleReqCount(m_ptr) };
    let mut dep_specifiers = Vec::with_capacity(req_count.max(0) as usize);
    for i in 0..req_count {
        let name_val = unsafe { qjs::JS_GetModuleReqName(target_ctx, m_ptr, i) };
        let name_wrap =
            unsafe { Value::from_raw(Ctx::from_raw(NonNull::new_unchecked(target_ctx)), name_val) };
        let s = name_wrap
            .as_string()
            .and_then(|s| s.to_string().ok())
            .unwrap_or_default();
        dep_specifiers.push(s);
    }

    Ok(VmModuleCore {
        module_ptr: m_ptr,
        target_ctx,
        ctx_alive,
        microtask_mode_after_eval,
        is_synthetic: false,
        identifier,
        context_obj,
        linking: Cell::new(false),
        cached_bytecode: Some(bytecode),
        meta_key,
        cached_data_rejected,
        dep_specifiers,
        initialize_import_meta,
        import_meta_initialized: Cell::new(false),
        import_kind,
        import_fn,
        export_names: Vec::new(),
        req_set_count: RefCell::new(Vec::new()),
    })
}

fn create_synthetic_module<'js>(
    ctx: &Ctx<'js>,
    export_names_val: Value<'js>,
    evaluate_cb_val: Value<'js>,
    options: Option<Value<'js>>,
    mod_js: Value<'js>,
) -> Result<VmModuleCore<'js>> {
    ensure_runtime_state(ctx)?;

    let export_arr = export_names_val.as_array().ok_or_else(|| {
        throw_invalid_arg_type(
            ctx,
            "exportNames",
            "an instance of Array",
            &export_names_val,
        )
    })?;
    let mut export_names = Vec::with_capacity(export_arr.len());
    for item in export_arr.iter::<Value<'js>>() {
        let item = item?;
        let s = item
            .as_string()
            .ok_or_else(|| throw_invalid_arg_type(ctx, "exportNames[]", "of type string", &item))?;
        export_names.push(s.to_string()?);
    }

    let evaluate_cb = evaluate_cb_val.clone().into_function().ok_or_else(|| {
        throw_invalid_arg_type(
            ctx,
            "evaluateCallback",
            "of type function",
            &evaluate_cb_val,
        )
    })?;

    let id = alloc_module_id(ctx)?;
    let mut identifier = format!("vm:module({id})");
    let mut context_opt = Value::new_undefined(ctx.clone());

    if let Some(opts_val) = options {
        if !opts_val.is_undefined() {
            if !is_non_function_object(&opts_val) {
                return Err(throw_invalid_arg_type(
                    ctx,
                    "options",
                    "of type object",
                    &opts_val,
                ));
            }
            let opts = opts_val.as_object().unwrap();
            let id_val: Value = opts.get("identifier")?;
            if !id_val.is_undefined() {
                let s = id_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(ctx, "options.identifier", "of type string", &id_val)
                })?;
                identifier = s.to_string()?;
            }
            context_opt = opts.get("context")?;
        }
    }

    let id_cstr = validated_cstring(ctx, "options.identifier", &identifier)?;
    let (context_obj, target_ctx, ctx_alive, microtask_mode_after_eval) =
        resolve_module_context(ctx, context_opt, "options.context")?;

    let m_ptr =
        unsafe { qjs::JS_NewCModule(target_ctx, id_cstr.as_ptr(), Some(synthetic_module_init)) };
    if m_ptr.is_null() {
        return Err(Error::Allocation);
    }
    unsafe { qjs::JS_SetModuleResolved(m_ptr, true) };

    for exp in &export_names {
        let exp_cstr = validated_cstring(ctx, "exportNames[]", exp)?;
        let rc = unsafe { qjs::JS_AddModuleExport(target_ctx, m_ptr, exp_cstr.as_ptr()) };
        if rc < 0 {
            return Err(Error::Allocation);
        }
    }

    register_synthetic_module(ctx, m_ptr, evaluate_cb, mod_js)?;

    Ok(VmModuleCore {
        module_ptr: m_ptr,
        target_ctx,
        ctx_alive,
        microtask_mode_after_eval,
        is_synthetic: true,
        identifier,
        context_obj,
        linking: Cell::new(false),
        cached_bytecode: None,
        meta_key: Vec::new(),
        cached_data_rejected: None,
        dep_specifiers: Vec::new(),
        initialize_import_meta: None,
        import_meta_initialized: Cell::new(false),
        import_kind: 0,
        import_fn: None,
        export_names,
        req_set_count: RefCell::new(Vec::new()),
    })
}

pub fn install_module_api<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let natives = Object::new(ctx.clone())?;

    natives.set(
        "createSourceTextModule",
        Func::from(
            |ctx: Ctx<'js>, code: Value<'js>, options: Option<Value<'js>>| -> Result<Value<'js>> {
                let core = compile_source_text_module(&ctx, code, options)?;
                let rejected = core.cached_data_rejected;
                let id = core.identifier.clone();
                let deps = core.dep_specifiers.clone();
                let context_obj = core.context_obj.clone();
                let core_inst = Class::instance(ctx.clone(), core)?;
                let res = Object::new(ctx.clone())?;
                res.set("core", core_inst)?;
                res.set("identifier", id)?;
                res.set("depSpecifiers", deps)?;
                res.set("context", context_obj)?;
                if let Some(r) = rejected {
                    res.set("cachedDataRejected", r)?;
                }
                Ok(res.into_value())
            },
        ),
    )?;

    natives.set(
        "createSyntheticModule",
        Func::from(
            |ctx: Ctx<'js>,
             export_names: Value<'js>,
             eval_cb: Value<'js>,
             options: Option<Value<'js>>,
             mod_js: Value<'js>|
             -> Result<Value<'js>> {
                let core = create_synthetic_module(&ctx, export_names, eval_cb, options, mod_js)?;
                let id = core.identifier.clone();
                let context_obj = core.context_obj.clone();
                let core_inst = Class::instance(ctx.clone(), core)?;
                let res = Object::new(ctx.clone())?;
                res.set("core", core_inst)?;
                res.set("identifier", id)?;
                res.set("context", context_obj)?;
                Ok(res.into_value())
            },
        ),
    )?;

    natives.set(
        "getStatus",
        Func::from(
            |core: Class<'js, VmModuleCore<'js>>| -> Result<&'static str> {
                Ok(core.borrow().status_str())
            },
        ),
    )?;

    natives.set(
        "isSynthetic",
        Func::from(|core: Class<'js, VmModuleCore<'js>>| -> Result<bool> {
            Ok(core.borrow().is_synthetic)
        }),
    )?;

    natives.set(
        "getIdentifier",
        Func::from(|core: Class<'js, VmModuleCore<'js>>| -> Result<String> {
            Ok(core.borrow().identifier.clone())
        }),
    )?;

    natives.set(
        "getContext",
        Func::from(
            |core: Class<'js, VmModuleCore<'js>>| -> Result<Value<'js>> {
                Ok(core.borrow().context_obj.clone())
            },
        ),
    )?;

    natives.set(
        "getDepSpecifiers",
        Func::from(
            |core: Class<'js, VmModuleCore<'js>>| -> Result<Vec<String>> {
                Ok(core.borrow().dep_specifiers.clone())
            },
        ),
    )?;

    natives.set(
        "getReqAttributes",
        Func::from(
            |ctx: Ctx<'js>, core: Class<'js, VmModuleCore<'js>>, idx: i32| -> Result<Value<'js>> {
                let b = core.borrow();
                b.assert_alive(&ctx)?;
                let attrs =
                    unsafe { qjs::JS_GetModuleReqAttributes(b.target_ctx, b.module_ptr, idx) };
                if unsafe { qjs::JS_VALUE_GET_NORM_TAG(attrs) == qjs::JS_TAG_UNDEFINED } {
                    let obj_raw = unsafe {
                        qjs::JS_NewObjectProto(
                            ctx.as_raw().as_ptr(),
                            into_raw_value(Value::new_null(ctx.clone())),
                        )
                    };
                    return Ok(unsafe { Value::from_raw(ctx, obj_raw) });
                }
                Ok(unsafe { Value::from_raw(ctx, attrs) })
            },
        ),
    )?;

    natives.set(
        "sameTargetCtx",
        Func::from(
            |a: Class<'js, VmModuleCore<'js>>, b: Class<'js, VmModuleCore<'js>>| -> Result<bool> {
                Ok(a.borrow().target_ctx == b.borrow().target_ctx)
            },
        ),
    )?;

    natives.set(
        "setLinking",
        Func::from(
            |core: Class<'js, VmModuleCore<'js>>, val: bool| -> Result<()> {
                core.borrow().linking.set(val);
                Ok(())
            },
        ),
    )?;

    natives.set(
        "setReqModule",
        Func::from(
            |ctx: Ctx<'js>,
             core: Class<'js, VmModuleCore<'js>>,
             idx: i32,
             dep: Class<'js, VmModuleCore<'js>>|
             -> Result<()> {
                let b = core.borrow();
                let dep_b = dep.borrow();
                b.assert_alive(&ctx)?;
                dep_b.assert_alive(&ctx)?;
                if b.target_ctx != dep_b.target_ctx {
                    return Err(throw_vm_module_different_context(&ctx));
                }
                let rc = unsafe { qjs::JS_SetModuleReqModule(b.module_ptr, idx, dep_b.module_ptr) };
                if rc < 0 {
                    return Err(Error::Unknown);
                }
                b.req_set_count.borrow_mut().push(idx as usize);
                Ok(())
            },
        ),
    )?;

    natives.set(
        "finishLink",
        Func::from(
            |ctx: Ctx<'js>, core: Class<'js, VmModuleCore<'js>>| -> Result<()> {
                let b = core.borrow();
                b.assert_alive(&ctx)?;
                let rc = unsafe { qjs::JS_VmLinkModule(b.target_ctx, b.module_ptr) };
                if rc < 0 {
                    if b.target_ctx == ctx.as_raw().as_ptr() {
                        return Err(Error::Exception);
                    }
                    return Err(unsafe { transfer_exception(b.target_ctx, &ctx) });
                }
                Ok(())
            },
        ),
    )?;

    natives.set(
        "getNamespace",
        Func::from(
            |ctx: Ctx<'js>, core: Class<'js, VmModuleCore<'js>>| -> Result<Value<'js>> {
                let b = core.borrow();
                b.assert_alive(&ctx)?;
                let st = b.status_str();
                if st == "unlinked" || st == "linking" {
                    return Err(throw_vm_module_status(
                        &ctx,
                        "Module status must not be unlinked or linking",
                    ));
                }
                let ns = unsafe { qjs::JS_GetModuleNamespace(b.target_ctx, b.module_ptr) };
                if unsafe { qjs::JS_VALUE_GET_NORM_TAG(ns) == qjs::JS_TAG_EXCEPTION } {
                    unsafe { qjs::JS_FreeValue(b.target_ctx, ns) };
                    if b.target_ctx == ctx.as_raw().as_ptr() {
                        return Err(Error::Exception);
                    }
                    return Err(unsafe { transfer_exception(b.target_ctx, &ctx) });
                }
                Ok(unsafe { Value::from_raw(ctx, ns) })
            },
        ),
    )?;

    natives.set(
        "getError",
        Func::from(
            |ctx: Ctx<'js>, core: Class<'js, VmModuleCore<'js>>| -> Result<Value<'js>> {
                let b = core.borrow();
                b.assert_alive(&ctx)?;
                if b.status_str() != "errored" {
                    return Err(throw_vm_module_status(
                        &ctx,
                        "Module status must be errored",
                    ));
                }
                let exc = unsafe { qjs::JS_GetModuleEvalException(b.target_ctx, b.module_ptr) };
                Ok(unsafe { Value::from_raw(ctx, exc) })
            },
        ),
    )?;

    natives.set(
        "runInitImportMeta",
        Func::from(
            |ctx: Ctx<'js>,
             core: Class<'js, VmModuleCore<'js>>,
             mod_js: Value<'js>|
             -> Result<()> {
                let (init_fn, target_ctx, m_ptr) = {
                    let b = core.borrow();
                    b.assert_alive(&ctx)?;
                    if b.import_meta_initialized.get() {
                        return Ok(());
                    }
                    b.import_meta_initialized.set(true);
                    match &b.initialize_import_meta {
                        Some(f) => (f.clone(), b.target_ctx, b.module_ptr),
                        None => return Ok(()),
                    }
                };
                let meta_raw = unsafe { qjs::JS_GetImportMeta(target_ctx, m_ptr) };
                if unsafe { qjs::JS_VALUE_GET_NORM_TAG(meta_raw) == qjs::JS_TAG_EXCEPTION } {
                    unsafe { qjs::JS_FreeValue(target_ctx, meta_raw) };
                    if target_ctx == ctx.as_raw().as_ptr() {
                        return Err(Error::Exception);
                    }
                    return Err(unsafe { transfer_exception(target_ctx, &ctx) });
                }
                let meta_val = unsafe { Value::from_raw(ctx.clone(), meta_raw) };
                init_fn.call::<_, ()>((meta_val, mod_js))
            },
        ),
    )?;

    natives.set(
        "evaluateModule",
        Func::from(
            |ctx: Ctx<'js>,
             core: Class<'js, VmModuleCore<'js>>,
             referencing: Value<'js>,
             options: Option<Value<'js>>|
             -> Result<Value<'js>> {
                let (timeout_ms, break_on_sigint) = if let Some(opts_val) = &options {
                    if !opts_val.is_undefined() {
                        if !is_non_function_object(opts_val) {
                            return Err(throw_invalid_arg_type(
                                &ctx,
                                "options",
                                "of type object",
                                opts_val,
                            ));
                        }
                        let opts = opts_val.as_object().unwrap();
                        let t_val: Value = opts.get("timeout")?;
                        let b_val: Value = opts.get("breakOnSigint")?;
                        (
                            validate_timeout(&ctx, Some(&t_val))?,
                            validate_break_on_sigint(&ctx, Some(&b_val))?,
                        )
                    } else {
                        (None, false)
                    }
                } else {
                    (None, false)
                };

                let (target_ctx, m_ptr, after_eval, identifier, spec) = {
                    let b = core.borrow();
                    b.assert_alive(&ctx)?;
                    let st = b.status_str();
                    if st != "linked" && st != "evaluated" && st != "errored" {
                        return Err(throw_vm_module_status(
                            &ctx,
                            "Module status must be one of linked, evaluated, or errored",
                        ));
                    }
                    (
                        b.target_ctx,
                        b.module_ptr,
                        b.microtask_mode_after_eval,
                        b.identifier.clone(),
                        b.import_spec(&referencing),
                    )
                };

                register_named_import_hook(&ctx, target_ctx, &identifier, &spec)?;
                let _import_guard = DynamicImportGuard::enter(&ctx, target_ctx, &spec)?;
                let interrupt_guard = InterruptGuard::enter(&ctx, timeout_ms, break_on_sigint);

                let prom_raw = unsafe { qjs::JS_VmEvaluateModule(target_ctx, m_ptr) };
                if let Some(err) = unsafe { interrupt_guard.map_triggered_error(target_ctx, &ctx) }
                {
                    let eval_exc = unsafe { qjs::JS_GetModuleEvalException(target_ctx, m_ptr) };
                    unsafe {
                        qjs::JS_ResetUncatchableError(target_ctx);
                        qjs::JS_FreeValue(target_ctx, eval_exc);
                        qjs::JS_FreeValue(target_ctx, prom_raw);
                    }
                    return Err(err);
                }
                if unsafe { qjs::JS_VALUE_GET_NORM_TAG(prom_raw) == qjs::JS_TAG_EXCEPTION } {
                    unsafe { qjs::JS_FreeValue(target_ctx, prom_raw) };
                    if target_ctx == ctx.as_raw().as_ptr() {
                        return Err(Error::Exception);
                    }
                    return Err(unsafe { transfer_exception(target_ctx, &ctx) });
                }

                if let Err(err) =
                    drain_child_microtasks_if_needed(&ctx, target_ctx, after_eval, &interrupt_guard)
                {
                    unsafe { qjs::JS_FreeValue(target_ctx, prom_raw) };
                    return Err(err);
                }

                Ok(unsafe { Value::from_raw(ctx, prom_raw) })
            },
        ),
    )?;

    natives.set(
        "createCachedData",
        Func::from(
            |ctx: Ctx<'js>, core: Class<'js, VmModuleCore<'js>>| -> Result<Value<'js>> {
                let b = core.borrow();
                b.assert_alive(&ctx)?;
                let st = b.status_str();
                if st == "evaluated" || st == "errored" {
                    return Err(throw_vm_module_cannot_create_cached_data(&ctx));
                }
                let raw = b.cached_bytecode.as_ref().ok_or(Error::Unknown)?;
                let wrapped = encode_cache(CacheKind::Module, &b.meta_key, raw);
                to_buffer(&ctx, wrapped)
            },
        ),
    )?;

    natives.set(
        "setExport",
        Func::from(
            |ctx: Ctx<'js>,
             core: Class<'js, VmModuleCore<'js>>,
             name_val: Value<'js>,
             val: Value<'js>|
             -> Result<()> {
                let b = core.borrow();
                b.assert_alive(&ctx)?;
                let name_str = name_val.as_string().ok_or_else(|| {
                    throw_invalid_arg_type(&ctx, "name", "of type string", &name_val)
                })?;
                let name = name_str.to_string()?;
                let st = b.status_str();
                if st == "unlinked" || st == "linking" {
                    return Err(throw_vm_module_status(&ctx, "Module status must be linked"));
                }
                if !b.export_names.iter().any(|n| n == &name) {
                    return Err(throw_coded(
                        &ctx,
                        ErrorKind::ReferenceError,
                        None,
                        &format!("Export '{name}' is not defined in module"),
                    ));
                }
                let name_cstr = validated_cstring(&ctx, "name", &name)?;
                let val_dup = unsafe { qjs::JS_DupValue(b.target_ctx, val.as_raw()) };
                let rc = unsafe {
                    qjs::JS_SetModuleExport(b.target_ctx, b.module_ptr, name_cstr.as_ptr(), val_dup)
                };
                if rc < 0 {
                    return Err(Error::Unknown);
                }
                Ok(())
            },
        ),
    )?;

    natives.set(
        "throwInvalidLinker",
        Func::from(|ctx: Ctx<'js>, val: Value<'js>| -> Result<()> {
            Err(throw_invalid_arg_type(
                &ctx,
                "linker",
                "of type function",
                &val,
            ))
        }),
    )?;

    natives.set(
        "throwAlreadyLinked",
        Func::from(|ctx: Ctx<'js>| -> Result<()> { Err(throw_vm_module_already_linked(&ctx)) }),
    )?;

    natives.set(
        "throwDifferentContext",
        Func::from(|ctx: Ctx<'js>| -> Result<()> { Err(throw_vm_module_different_context(&ctx)) }),
    )?;

    natives.set(
        "throwNotModule",
        Func::from(|ctx: Ctx<'js>| -> Result<()> {
            Err(throw_coded(
                &ctx,
                ErrorKind::TypeError,
                Some("ERR_VM_MODULE_NOT_MODULE"),
                "Provided module is not an instance of Module",
            ))
        }),
    )?;

    natives.set(
        "throwStatusError",
        Func::from(|ctx: Ctx<'js>, msg: String| -> Result<()> {
            Err(throw_vm_module_status(&ctx, &msg))
        }),
    )?;

    natives.set(
        "registerUnwrap",
        Func::from(|ctx: Ctx<'js>, f: Function<'js>| -> Result<()> {
            set_module_unwrap_fn(&ctx, f)
        }),
    )?;

    let install: Function<'js> = ctx.eval(
        r#"((natives) => {
            const kCore = Symbol("vm_module_core");
            const kChildren = Symbol("vm_module_children");
            const kInit = Symbol("vm_module_init");

            class Module {
                constructor(internalToken) {
                    if (new.target === Module || internalToken !== kInit) {
                        throw new TypeError("Module cannot be constructed directly");
                    }
                    this[kChildren] = [];
                }
                get status() {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    return natives.getStatus(this[kCore]);
                }
                get identifier() {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    return natives.getIdentifier(this[kCore]);
                }
                get context() {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    return natives.getContext(this[kCore]);
                }
                get namespace() {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    return natives.getNamespace(this[kCore]);
                }
                get error() {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    return natives.getError(this[kCore]);
                }
                get dependencySpecifiers() {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    if (natives.isSynthetic(this[kCore])) {
                        return undefined;
                    }
                    return Object.freeze(natives.getDepSpecifiers(this[kCore]));
                }
                async link(linker) {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    if (typeof linker !== "function") {
                        natives.throwInvalidLinker(linker);
                    }
                    if (natives.getStatus(this[kCore]) !== "unlinked") {
                        natives.throwAlreadyLinked();
                    }

                    const visited = new Set();
                    const linkingStarted = [];

                    const resolveDeps = async (mod) => {
                        if (visited.has(mod)) return;
                        visited.add(mod);
                        if (natives.getStatus(mod[kCore]) !== "unlinked") return;

                        natives.setLinking(mod[kCore], true);
                        linkingStarted.push(mod);

                        const specs = natives.getDepSpecifiers(mod[kCore]);
                        const resolvedChildren = [];
                        for (let i = 0; i < specs.length; i++) {
                            const attrs = natives.getReqAttributes(mod[kCore], i);
                            const dep = await linker(specs[i], mod, { attributes: attrs });
                            if (!dep || typeof dep !== "object" || !dep[kCore]) {
                                natives.throwNotModule();
                            }
                            if (!natives.sameTargetCtx(mod[kCore], dep[kCore])) {
                                natives.throwDifferentContext();
                            }
                            natives.setReqModule(mod[kCore], i, dep[kCore]);
                            resolvedChildren.push(dep);
                            await resolveDeps(dep);
                        }
                        mod[kChildren] = resolvedChildren;
                    };

                    try {
                        await resolveDeps(this);
                        for (const m of linkingStarted) {
                            natives.setLinking(m[kCore], false);
                        }
                        natives.finishLink(this[kCore]);
                    } catch (err) {
                        for (const m of linkingStarted) {
                            natives.setLinking(m[kCore], false);
                        }
                        throw err;
                    }
                }
                async evaluate(options) {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    const st = natives.getStatus(this[kCore]);
                    if (st !== "linked" && st !== "evaluated" && st !== "errored") {
                        natives.throwStatusError(
                            "Module status must be one of linked, evaluated, or errored"
                        );
                    }

                    const metaSeen = new Set();
                    const initMetaGraph = (mod) => {
                        if (metaSeen.has(mod)) return;
                        metaSeen.add(mod);
                        const children = mod[kChildren] || [];
                        for (let i = 0; i < children.length; i++) {
                            initMetaGraph(children[i]);
                        }
                        natives.runInitImportMeta(mod[kCore], mod);
                    };
                    initMetaGraph(this);

                    const p = natives.evaluateModule(this[kCore], this, options);
                    return await p;
                }
            }

            class SourceTextModule extends Module {
                constructor(code, options) {
                    super(kInit);
                    const info = natives.createSourceTextModule(code, options);
                    this[kCore] = info.core;
                    if ("cachedDataRejected" in info) {
                        Object.defineProperty(this, "cachedDataRejected", {
                            value: info.cachedDataRejected,
                            writable: true,
                            enumerable: true,
                            configurable: true,
                        });
                    }
                }
                createCachedData() {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    return natives.createCachedData(this[kCore]);
                }
            }

            class SyntheticModule extends Module {
                constructor(exportNames, evaluateCallback, options) {
                    super(kInit);
                    const info = natives.createSyntheticModule(exportNames, evaluateCallback, options, this);
                    this[kCore] = info.core;
                }
                setExport(name, value) {
                    if (!this || !this[kCore]) throw new TypeError("Illegal invocation");
                    natives.setExport(this[kCore], name, value);
                }
            }

            natives.registerUnwrap((val) => {
                if (val && typeof val === "object" && val[kCore]) {
                    return natives.getNamespace(val[kCore]);
                }
                return val;
            });

            return { Module, SourceTextModule, SyntheticModule };
        })"#,
    )?;

    install.call((natives,))
}
