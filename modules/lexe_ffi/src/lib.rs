// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `node:ffi` native FFI module (QuickJS-backed).

#![allow(clippy::missing_safety_doc)]

mod registry;
mod types;

use std::ffi::CString;
use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use lexe_utils::module::{export_default, ModuleInfo};
use libffi::low::ffi_abi_FFI_DEFAULT_ABI;
use libffi::middle::{Arg, Cif, CodePtr};
use libloading::Library;
use rquickjs::{
    function::{Opt, Rest},
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    qjs, Array, Ctx, Exception, Function, Object, Result, Value,
};

pub use registry::shutdown_context;
use registry::{close_lib_handle, new_lib_handle, register_lib_callback, unregister_lib_callback};
use types::{
    call_cif, extract_buffer_or_view_ptr, js_to_pointer_u64, marshal_js_arg, throw_qjs_error,
    FfiArgStorage, FfiType, SyncCif,
};

pub const NODE_FFI_MODULE_NAME: &str = "node:ffi";

#[cfg(target_os = "macos")]
const FFI_SUFFIX: &str = "dylib";
#[cfg(windows)]
const FFI_SUFFIX: &str = "dll";
#[cfg(all(unix, not(target_os = "macos")))]
const FFI_SUFFIX: &str = "so";

fn get_raw_pointer<'js>(ctx: Ctx<'js>, source: Value<'js>) -> Result<Value<'js>> {
    let qctx = ctx.as_raw().as_ptr();
    match unsafe { extract_buffer_or_view_ptr(qctx, source.as_raw()) } {
        Ok((ptr, _len)) => {
            let bi = unsafe { qjs::JS_NewBigUint64(qctx, ptr as usize as u64) };
            Ok(unsafe { Value::from_raw(ctx, bi) })
        },
        Err((kind, msg)) => throw_qjs_error(&ctx, kind, msg),
    }
}

fn to_array_buffer<'js>(
    ctx: Ctx<'js>,
    pointer: Value<'js>,
    length: Value<'js>,
    copy: Opt<bool>,
) -> Result<Value<'js>> {
    let addr = js_to_pointer_u64(&ctx, &pointer)?;
    let len_f = length
        .as_number()
        .ok_or_else(|| Exception::throw_type(&ctx, "toArrayBuffer length must be a number"))?;
    if !len_f.is_finite() || len_f < 0.0 || len_f.fract() != 0.0 || len_f > (i32::MAX as f64) {
        return throw_qjs_error(&ctx, "RangeError", "Invalid toArrayBuffer length");
    }
    let len = len_f as usize;
    let qctx = ctx.as_raw().as_ptr();

    if len == 0 {
        let ab = unsafe { qjs::JS_NewArrayBufferCopy(qctx, ptr::null(), 0) };
        if unsafe { qjs::JS_IsException(ab) } {
            return Err(rquickjs::Error::Exception);
        }
        return Ok(unsafe { Value::from_raw(ctx, ab) });
    }

    if addr == 0 {
        return throw_qjs_error(
            &ctx,
            "TypeError",
            "Cannot create non-empty ArrayBuffer from null pointer",
        );
    }

    let copy_bytes = copy.0.unwrap_or(true);
    let ab = unsafe {
        if copy_bytes {
            qjs::JS_NewArrayBufferCopy(qctx, addr as usize as *const u8, len as qjs::size_t)
        } else {
            qjs::JS_NewArrayBuffer(
                qctx,
                addr as usize as *mut u8,
                len as qjs::size_t,
                None,
                ptr::null_mut(),
                false,
            )
        }
    };
    if unsafe { qjs::JS_IsException(ab) } {
        return Err(rquickjs::Error::Exception);
    }
    Ok(unsafe { Value::from_raw(ctx, ab) })
}

fn get_int32<'js>(ctx: Ctx<'js>, pointer: Value<'js>, offset: Opt<i64>) -> Result<i32> {
    let addr = js_to_pointer_u64(&ctx, &pointer)? as usize;
    if addr == 0 {
        return throw_qjs_error(&ctx, "TypeError", "Cannot read from null pointer");
    }
    let off = isize::try_from(offset.0.unwrap_or(0))
        .map_err(|_| Exception::throw_range(&ctx, "Pointer offset out of range"))?;
    let target = addr
        .checked_add_signed(off)
        .ok_or_else(|| Exception::throw_range(&ctx, "Pointer offset overflow"))?;
    if target == 0 {
        return throw_qjs_error(&ctx, "TypeError", "Cannot read from null pointer");
    }
    Ok(unsafe { ptr::read_unaligned(target as *const i32) })
}

fn parse_type_list<'js>(ctx: &Ctx<'js>, def: &Object<'js>) -> Result<Vec<FfiType>> {
    let raw_list: Value<'js> = if def.contains_key("arguments")? {
        def.get("arguments")?
    } else if def.contains_key("parameters")? {
        def.get("parameters")?
    } else {
        return Ok(Vec::new());
    };
    if raw_list.is_undefined() || raw_list.is_null() {
        return Ok(Vec::new());
    }
    let arr = Array::from_value(raw_list)
        .map_err(|_| Exception::throw_type(ctx, "FFI arguments/parameters must be an array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr.iter::<String>() {
        let name = entry.map_err(|_| Exception::throw_type(ctx, "FFI type must be a string"))?;
        let parsed = FfiType::parse(&name).map_err(|e| Exception::throw_type(ctx, &e))?;
        out.push(parsed);
    }
    Ok(out)
}

fn parse_return_type<'js>(ctx: &Ctx<'js>, def: &Object<'js>) -> Result<FfiType> {
    let name: Option<String> = if def.contains_key("return")? {
        def.get("return")?
    } else if def.contains_key("result")? {
        def.get("result")?
    } else {
        None
    };
    match name {
        Some(s) => FfiType::parse(&s).map_err(|e| Exception::throw_type(ctx, &e)),
        None => Ok(FfiType::Void),
    }
}

fn open_library(path: Option<&str>) -> std::result::Result<Library, String> {
    match path {
        Some(p) => unsafe { Library::new(p) }
            .map_err(|e| format!("Failed to open dynamic library '{p}': {e}")),
        None => {
            #[cfg(unix)]
            {
                Ok(libloading::os::unix::Library::this().into())
            }
            #[cfg(windows)]
            {
                libloading::os::windows::Library::this()
                    .map(Into::into)
                    .map_err(|e| format!("Failed to open current process library: {e}"))
            }
        },
    }
}

fn dlopen<'js>(
    ctx: Ctx<'js>,
    path_val: Value<'js>,
    symbols_val: Value<'js>,
) -> Result<Object<'js>> {
    let path_opt: Option<String> = if path_val.is_null() || path_val.is_undefined() {
        None
    } else if let Some(s) = path_val.as_string() {
        Some(s.to_string()?)
    } else {
        return throw_qjs_error(&ctx, "TypeError", "dlopen path must be a string or null");
    };

    let symbols_obj = Object::from_value(symbols_val)
        .map_err(|_| Exception::throw_type(&ctx, "dlopen symbols must be an object"))?;

    let library =
        open_library(path_opt.as_deref()).map_err(|e| Exception::throw_message(&ctx, &e))?;
    let lib_handle = new_lib_handle(ctx.as_raw().as_ptr(), library);

    let functions_obj = Object::new(ctx.clone())?;
    for key_res in symbols_obj.keys::<String>() {
        let js_name = key_res?;
        let def_val: Value<'js> = symbols_obj.get(&js_name)?;
        let def_obj = Object::from_value(def_val).map_err(|_| {
            Exception::throw_type(
                &ctx,
                &format!("Symbol definition for '{js_name}' must be an object"),
            )
        })?;

        let native_name: Option<String> = def_obj.get("symbol")?;
        let sym_name = native_name.unwrap_or_else(|| js_name.clone());
        let c_sym = CString::new(sym_name.as_str())
            .map_err(|_| Exception::throw_type(&ctx, "Symbol name contains interior NUL byte"))?;

        let param_types = parse_type_list(&ctx, &def_obj)?;
        let ret_type = parse_return_type(&ctx, &def_obj)?;

        let code_addr = {
            let guard = lib_handle.state.lock().unwrap();
            let lib = guard
                .library
                .as_ref()
                .ok_or_else(|| Exception::throw_message(&ctx, "FFI library is closed"))?;
            let sym: libloading::Symbol<'_, *const c_void> =
                unsafe { lib.get(c_sym.as_bytes_with_nul()) }.map_err(|e| {
                    Exception::throw_message(
                        &ctx,
                        &format!("Failed to resolve FFI symbol '{sym_name}': {e}"),
                    )
                })?;
            (*sym) as usize
        };

        let ffi_params: Vec<libffi::middle::Type> =
            param_types.iter().map(|t| t.to_libffi()).collect();
        let cif = Arc::new(SyncCif(Cif::new_with_abi(
            ffi_params,
            ret_type.to_libffi(),
            ffi_abi_FFI_DEFAULT_ABI,
        )));

        let handle_for_fn = Arc::clone(&lib_handle);
        let bound_fn = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> Result<Value<'js>> {
                if handle_for_fn.is_closed.load(Ordering::SeqCst) {
                    return Err(Exception::throw_message(&ctx, "FFI library is closed"));
                }
                let mut storages: Vec<FfiArgStorage> = Vec::with_capacity(param_types.len());
                let mut c_strings: Vec<CString> = Vec::new();
                for (i, ty) in param_types.iter().copied().enumerate() {
                    storages.push(marshal_js_arg(&ctx, ty, args.0.get(i), &mut c_strings)?);
                }
                let ffi_args: Vec<Arg<'_>> = storages.iter().map(FfiArgStorage::as_arg).collect();
                unsafe {
                    call_cif(
                        &ctx,
                        &cif.0,
                        CodePtr(code_addr as *mut c_void),
                        ret_type,
                        &ffi_args,
                    )
                }
            },
        )?;
        functions_obj.set(&js_name, bound_fn)?;
    }

    let lib_obj = Object::new(ctx.clone())?;
    {
        let h = Arc::clone(&lib_handle);
        lib_obj.set(
            "close",
            Func::from(move |ctx: Ctx<'js>| {
                close_lib_handle(&h, ctx.as_raw().as_ptr());
            }),
        )?;
    }
    {
        let h = Arc::clone(&lib_handle);
        lib_obj.set(
            "registerCallback",
            Func::from(
                move |ctx: Ctx<'js>,
                      def_val: Value<'js>,
                      js_fn: Value<'js>|
                      -> Result<Value<'js>> {
                    let def_obj = Object::from_value(def_val).map_err(|_| {
                        Exception::throw_type(&ctx, "Callback definition must be an object")
                    })?;
                    let arg_types = parse_type_list(&ctx, &def_obj)?;
                    let ret_type = parse_return_type(&ctx, &def_obj)?;
                    let addr =
                        register_lib_callback(&ctx, &h, arg_types, ret_type, js_fn.as_raw())?;
                    let qctx = ctx.as_raw().as_ptr();
                    Ok(unsafe { Value::from_raw(ctx, qjs::JS_NewBigUint64(qctx, addr)) })
                },
            ),
        )?;
    }
    {
        let h = Arc::clone(&lib_handle);
        lib_obj.set(
            "unregisterCallback",
            Func::from(move |ctx: Ctx<'js>, ptr_val: Value<'js>| -> Result<()> {
                let addr = js_to_pointer_u64(&ctx, &ptr_val)?;
                unregister_lib_callback(&h, ctx.as_raw().as_ptr(), addr);
                Ok(())
            }),
        )?;
    }

    let out = Object::new(ctx)?;
    out.set("lib", lib_obj)?;
    out.set("functions", functions_obj)?;
    Ok(out)
}

pub struct FfiModule;

impl ModuleDef for FfiModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("suffix")?;
        declare.declare("dlopen")?;
        declare.declare("getRawPointer")?;
        declare.declare("toArrayBuffer")?;
        declare.declare("getInt32")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            default.set("suffix", FFI_SUFFIX)?;
            default.set("dlopen", Func::from(dlopen))?;
            default.set("getRawPointer", Func::from(get_raw_pointer))?;
            default.set("toArrayBuffer", Func::from(to_array_buffer))?;
            default.set("getInt32", Func::from(get_int32))?;
            Ok(())
        })
    }
}

impl From<FfiModule> for ModuleInfo<FfiModule> {
    fn from(val: FfiModule) -> Self {
        ModuleInfo {
            name: NODE_FFI_MODULE_NAME,
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    use super::*;

    #[tokio::test]
    async fn ffi_pointers_and_to_array_buffer() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<FfiModule>(ctx.clone(), "node:ffi")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test_ptrs",
                    r#"
                    import ffi, { suffix, getRawPointer, toArrayBuffer, getInt32 } from "node:ffi";

                    export async function test() {
                        if (suffix !== ffi.suffix || !["dylib", "dll", "so"].includes(suffix)) {
                            throw new Error("bad suffix: " + suffix);
                        }
                        const arr = new Int32Array([40, 10, 30, 20]);
                        const ptr = getRawPointer(arr);
                        if (typeof ptr !== "bigint" || ptr <= 0n) {
                            throw new Error("bad ptr");
                        }
                        if (getInt32(ptr, 4) !== 10) {
                            throw new Error("bad getInt32");
                        }
                        const copyBuf = toArrayBuffer(ptr, 16, true);
                        const zeroBuf = toArrayBuffer(ptr, 16, false);
                        const emptyBuf = toArrayBuffer(0n, 0, false);
                        if (emptyBuf.byteLength !== 0) {
                            throw new Error("bad zero length buffer");
                        }
                        arr[0] = 99;
                        return JSON.stringify({
                            zero0: new Int32Array(zeroBuf)[0],
                            copy0: new Int32Array(copyBuf)[0],
                        });
                    }
                    "#,
                )
                .await
                .unwrap();

                let out: String = call_test(&ctx, &module, ()).await;
                assert_eq!(out, r#"{"zero0":99,"copy0":40}"#);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn ffi_dlopen_without_callbacks() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<FfiModule>(ctx.clone(), "node:ffi")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test_basic_dlopen",
                    r#"
                    import { dlopen } from "node:ffi";

                    export async function test() {
                        const { lib, functions } = dlopen(null, {
                            abs: { arguments: ["i32"], return: "i32" },
                            strlen: { arguments: ["string"], return: "u64" },
                        });
                        const a = functions.abs(-42);
                        const s = functions.strlen("hello");
                        lib.close();
                        return `${a}:${s}`;
                    }
                    "#,
                )
                .await
                .unwrap();

                let out: String = call_test(&ctx, &module, ()).await;
                shutdown_context(&ctx).unwrap();
                assert_eq!(out, "42:5");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn ffi_dlopen_qsort_and_shutdown_cleanup() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<FfiModule>(ctx.clone(), "node:ffi")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test_qsort",
                    r#"
                    import { dlopen, getInt32 } from "node:ffi";

                    export async function test() {
                        const arr = new Int32Array([40, 10, 30, 20]);
                        const { lib, functions } = dlopen(null, {
                            abs: { arguments: ["i32"], return: "i32" },
                            strlen: { arguments: ["string"], return: "u64" },
                            qsort: { arguments: ["pointer", "u64", "u64", "pointer"], return: "void" },
                        });
                        if (functions.abs(-42) !== 42) {
                            throw new Error("abs failed");
                        }
                        if (functions.strlen("hello") !== 5n) {
                            throw new Error("strlen failed");
                        }

                        const cmp = lib.registerCallback(
                            { arguments: ["pointer", "pointer"], return: "i32" },
                            (a, b) => getInt32(a) - getInt32(b)
                        );
                        functions.qsort(arr, 4n, 4n, cmp);
                        lib.unregisterCallback(cmp);

                        const sorted = Array.from(arr);
                        lib.close();

                        // Register another callback without unregistering to verify context teardown frees it cleanly.
                        const dl2 = dlopen(null, {});
                        dl2.lib.registerCallback({ arguments: ["i32"], return: "i32" }, (x) => x + 1);

                        return JSON.stringify(sorted);
                    }
                    "#,
                )
                .await
                .unwrap();

                let out: String = call_test(&ctx, &module, ()).await;
                shutdown_context(&ctx).unwrap();
                assert_eq!(out, "[10,20,30,40]");
            })
        })
        .await;
    }
}
