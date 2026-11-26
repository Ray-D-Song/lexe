// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Ctx, Result, Value,
};

mod cache;
mod context;
mod errors;
mod interrupt;
mod module;
mod script;

pub use context::set_wasm_initializer;

pub struct VmModule;

const EXPORTS: &[&str] = &[
    "Module",
    "Script",
    "SourceTextModule",
    "SyntheticModule",
    "compileFunction",
    "constants",
    "createContext",
    "createScript",
    "isContext",
    "measureMemory",
    "runInContext",
    "runInNewContext",
    "runInThisContext",
];

impl ModuleDef for VmModule {
    fn declare(declare: &Declarations) -> Result<()> {
        for name in EXPORTS {
            declare.declare(*name)?;
        }
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        context::ensure_runtime_state(ctx)?;
        let script_api = script::install_script_api(ctx)?;
        let module_classes = module::install_module_api(ctx)?;
        let constants = context::build_constants(ctx)?;

        export_default(ctx, exports, |default| {
            let script_cls: Value<'js> = script_api.get("Script")?;
            let create_script: Value<'js> = script_api.get("createScript")?;
            let module_cls: Value<'js> = module_classes.get("Module")?;
            let stm_cls: Value<'js> = module_classes.get("SourceTextModule")?;
            let syn_cls: Value<'js> = module_classes.get("SyntheticModule")?;

            default.set("Module", module_cls)?;
            default.set("Script", script_cls)?;
            default.set("SourceTextModule", stm_cls)?;
            default.set("SyntheticModule", syn_cls)?;
            default.set("compileFunction", Func::from(script::compile_function_js))?;
            default.set("constants", constants)?;
            default.set("createContext", Func::from(context::create_context_js))?;
            default.set("createScript", create_script)?;
            default.set("isContext", Func::from(context::is_context_js))?;
            default.set("measureMemory", Func::from(context::measure_memory_js))?;
            default.set("runInContext", Func::from(script::run_in_context_js))?;
            default.set("runInNewContext", Func::from(script::run_in_new_context_js))?;
            default.set(
                "runInThisContext",
                Func::from(script::run_in_this_context_js),
            )?;
            ctx.globals().set(
                "__lexeVmMovePort",
                Func::from(context::move_port_to_context_js),
            )?;
            Ok(())
        })
    }
}

impl From<VmModule> for ModuleInfo<VmModule> {
    fn from(val: VmModule) -> Self {
        ModuleInfo {
            name: "vm",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use lexe_test::{test_async_with, ModuleEvaluator};
    use lexe_utils::event_loop::run_runtime_cleanups;
    use rquickjs::{qjs, Ctx, Error, IntoJs, Object, Result as JsResult, Value};

    use super::*;

    async fn with_vm_cleanup<F>(f: F)
    where
        F: for<'js> FnOnce(
                Ctx<'js>,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'js>>
            + Send
            + 'static,
    {
        test_async_with(|ctx| {
            Box::pin(async move {
                f(ctx.clone()).await;
                unsafe {
                    let rt = qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                    run_runtime_cleanups(rt as *mut std::ffi::c_void);
                }
            })
        })
        .await;
    }

    #[tokio::test]
    async fn test_module_loader_and_basic_exports() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                let vm_mod = ModuleEvaluator::eval_rust::<VmModule>(ctx.clone(), "vm")
                    .await
                    .unwrap();
                let run_fn: rquickjs::Function = vm_mod.get("runInNewContext").unwrap();
                let sandbox = Object::new(ctx.clone()).unwrap();
                sandbox.set("a", 2).unwrap();
                sandbox.set("b", 40).unwrap();
                let sum: i32 = run_fn.call(("a + b", sandbox)).unwrap();
                assert_eq!(sum, 42);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn sync_getter_exception_is_transferred() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                let sandbox = ctx
                    .eval::<Object, _>("({ get boom() { throw new RangeError('getter-boom'); } })")
                    .unwrap();
                let res: JsResult<Value> = script::run_in_new_context_impl(
                    ctx.clone(),
                    "boom".into_js(&ctx).unwrap(),
                    Some(sandbox.into_value()),
                    None,
                );
                assert!(matches!(res, Err(Error::Exception)));
                let exc: Object = ctx.catch().into_object().unwrap();
                let name: String = exc.get("name").unwrap();
                let message: String = exc.get("message").unwrap();
                assert_eq!(name, "RangeError");
                assert_eq!(message, "getter-boom");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn prefers_script_exceptions_over_sync_failures() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                let sandbox = ctx
                    .eval::<Object, _>("({ get y() { throw new Error('sync'); } })")
                    .unwrap();
                let res: JsResult<Value> = script::run_in_new_context_impl(
                    ctx.clone(),
                    "throw new Error('script')".into_js(&ctx).unwrap(),
                    Some(sandbox.into_value()),
                    None,
                );
                assert!(matches!(res, Err(Error::Exception)));
                let exc: Object = ctx.catch().into_object().unwrap();
                let message: String = exc.get("message").unwrap();
                assert_eq!(message, "script");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn returned_child_closure_reads_child_global_bindings() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                let fn_val: rquickjs::Function = script::run_in_new_context_impl(
                    ctx.clone(),
                    "var x = 99; () => x".into_js(&ctx).unwrap(),
                    Some(Object::new(ctx.clone()).unwrap().into_value()),
                    None,
                )
                .unwrap()
                .into_function()
                .unwrap();
                let out: i32 = fn_val.call(()).unwrap();
                assert_eq!(out, 99);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn allows_nul_bytes_in_comments_and_string_literals() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                for code in ["/*\0*/ 1", "\"a\\0b\""] {
                    let result = script::run_in_new_context_impl(
                        ctx.clone(),
                        code.into_js(&ctx).unwrap(),
                        None,
                        None,
                    );
                    assert!(result.is_ok(), "expected code `{code}` to evaluate cleanly");
                }
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_nul_bytes_in_source_with_syntax_error() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                let code = ctx.eval::<String, _>("'1\\u0000'").unwrap();
                let result = script::run_in_new_context_impl(
                    ctx.clone(),
                    code.into_js(&ctx).unwrap(),
                    None,
                    None,
                );
                assert!(matches!(result, Err(Error::Exception)));
                let exc: Object = ctx.catch().into_object().unwrap();
                let name: String = exc.get("name").unwrap();
                assert_eq!(name, "SyntaxError");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_filename_with_embedded_nul_bytes() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                let filename = ctx.eval::<String, _>("'left\\u0000right'").unwrap();
                let options = Object::new(ctx.clone()).unwrap();
                options.set("filename", filename).unwrap();
                let result = script::run_in_new_context_impl(
                    ctx.clone(),
                    "1".into_js(&ctx).unwrap(),
                    None,
                    Some(options.into_value()),
                );
                assert!(result.is_err());
                let _ = ctx.catch();
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_string_shorthand_filename_with_embedded_nul_bytes() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                let filename = ctx.eval::<String, _>("'left\\u0000right'").unwrap();
                let result = script::run_in_new_context_impl(
                    ctx.clone(),
                    "1".into_js(&ctx).unwrap(),
                    None,
                    Some(filename.into_js(&ctx).unwrap()),
                );
                assert!(result.is_err());
                let _ = ctx.catch();
            })
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_null_sandbox_when_global_type_error_is_poisoned() {
        with_vm_cleanup(|ctx| {
            Box::pin(async move {
                ctx.eval::<(), _>("globalThis.TypeError = () => { throw new Error('poisoned'); }")
                    .unwrap();
                let result = script::run_in_new_context_impl(
                    ctx.clone(),
                    "1".into_js(&ctx).unwrap(),
                    Some(Value::new_null(ctx.clone())),
                    None,
                );
                assert!(result.is_err());
                let exc: Object = ctx.catch().into_object().unwrap();
                let msg: String = exc.get("message").unwrap();
                assert_ne!(msg, "poisoned");
            })
        })
        .await;
    }
}
