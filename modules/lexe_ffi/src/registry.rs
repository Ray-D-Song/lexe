// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::ThreadId;

use libffi::low::{ffi_abi_FFI_DEFAULT_ABI, ffi_cif};
use libffi::middle::{Cif, Closure};
use libloading::Library;
use rquickjs::{qjs, Ctx, Exception, Result};

use crate::types::{extract_buffer_or_view_ptr, FfiType};

struct CallbackUserdata {
    ctx: *mut qjs::JSContext,
    rt: *mut qjs::JSRuntime,
    js_thread_id: ThreadId,
    js_fn: qjs::JSValue,
    arg_types: Vec<FfiType>,
    ret_type: FfiType,
    active: AtomicBool,
}

pub struct RegisteredCallback {
    closure: Option<Closure<'static>>,
    userdata: *mut CallbackUserdata,
}

// SAFETY: Access to `closure` and `userdata` is synchronized via `Mutex<LibState>` and JS execution only occurs on `js_thread_id`.
unsafe impl Send for RegisteredCallback {}

impl RegisteredCallback {
    pub fn destroy(&mut self, ctx: *mut qjs::JSContext) {
        // Drop the libffi executable closure before freeing `userdata`.
        self.closure.take();
        if !self.userdata.is_null() {
            unsafe {
                let mut boxed = Box::from_raw(self.userdata);
                boxed.active.store(false, Ordering::SeqCst);
                if !qjs::JS_IsUndefined(boxed.js_fn) && std::ptr::eq(boxed.ctx, ctx) {
                    qjs::JS_FreeValue(ctx, boxed.js_fn);
                    boxed.js_fn = qjs::JS_UNDEFINED;
                }
            }
            self.userdata = ptr::null_mut();
        }
    }
}

impl Drop for RegisteredCallback {
    fn drop(&mut self) {
        self.closure.take();
        if !self.userdata.is_null() {
            unsafe {
                let mut boxed = Box::from_raw(self.userdata);
                boxed.active.store(false, Ordering::SeqCst);
                if !qjs::JS_IsUndefined(boxed.js_fn)
                    && std::thread::current().id() == boxed.js_thread_id
                {
                    qjs::JS_FreeValueRT(boxed.rt, boxed.js_fn);
                }
                boxed.js_fn = qjs::JS_UNDEFINED;
            }
            self.userdata = ptr::null_mut();
        }
    }
}

pub struct LibState {
    pub closed: bool,
    pub library: Option<Library>,
    callbacks: HashMap<u64, RegisteredCallback>,
}

pub struct LibHandle {
    id: u64,
    ctx_ptr: usize,
    pub is_closed: AtomicBool,
    pub state: Mutex<LibState>,
}

static NEXT_LIB_ID: AtomicU64 = AtomicU64::new(1);
static CTX_LIBS: LazyLock<Mutex<HashMap<usize, HashMap<u64, Arc<LibHandle>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn new_lib_handle(ctx: *mut qjs::JSContext, library: Library) -> Arc<LibHandle> {
    let ctx_ptr = ctx as usize;
    let id = NEXT_LIB_ID.fetch_add(1, Ordering::Relaxed);
    let handle = Arc::new(LibHandle {
        id,
        ctx_ptr,
        is_closed: AtomicBool::new(false),
        state: Mutex::new(LibState {
            closed: false,
            library: Some(library),
            callbacks: HashMap::new(),
        }),
    });
    CTX_LIBS
        .lock()
        .unwrap()
        .entry(ctx_ptr)
        .or_default()
        .insert(id, Arc::clone(&handle));
    handle
}

pub fn close_lib_handle(handle: &LibHandle, ctx: *mut qjs::JSContext) {
    handle.is_closed.store(true, Ordering::SeqCst);
    {
        let mut ctx_libs = CTX_LIBS.lock().unwrap();
        if let Some(map) = ctx_libs.get_mut(&handle.ctx_ptr) {
            map.remove(&handle.id);
            if map.is_empty() {
                ctx_libs.remove(&handle.ctx_ptr);
            }
        }
    }
    let (mut callbacks, library) = {
        let mut guard = handle.state.lock().unwrap();
        if guard.closed {
            return;
        }
        guard.closed = true;
        let callbacks = std::mem::take(&mut guard.callbacks);
        let library = guard.library.take();
        (callbacks, library)
    };
    for (_, mut cb) in callbacks.drain() {
        cb.destroy(ctx);
    }
    drop(library);
}

pub fn unregister_lib_callback(handle: &LibHandle, ctx: *mut qjs::JSContext, ptr: u64) {
    let removed = {
        let mut guard = handle.state.lock().unwrap();
        if guard.closed {
            None
        } else {
            guard.callbacks.remove(&ptr)
        }
    };
    if let Some(mut cb) = removed {
        cb.destroy(ctx);
    }
}

pub fn register_lib_callback(
    ctx: &Ctx<'_>,
    handle: &LibHandle,
    arg_types: Vec<FfiType>,
    ret_type: FfiType,
    js_fn_raw: qjs::JSValue,
) -> Result<u64> {
    let qctx = ctx.as_raw().as_ptr();
    if !unsafe { qjs::JS_IsFunction(qctx, js_fn_raw) } {
        return Err(Exception::throw_type(
            ctx,
            "registerCallback requires a function",
        ));
    }

    if handle.is_closed.load(Ordering::SeqCst) {
        return Err(Exception::throw_message(ctx, "FFI library is closed"));
    }

    let ffi_args: Vec<libffi::middle::Type> = arg_types.iter().map(|t| t.to_libffi()).collect();
    let ffi_ret = ret_type.to_libffi();
    let cif = Cif::new_with_abi(ffi_args, ffi_ret, ffi_abi_FFI_DEFAULT_ABI);

    let duped_fn = unsafe { qjs::JS_DupValue(qctx, js_fn_raw) };
    let rt = unsafe { qjs::JS_GetRuntime(qctx) };
    let userdata_ptr = Box::into_raw(Box::new(CallbackUserdata {
        ctx: qctx,
        rt,
        js_thread_id: std::thread::current().id(),
        js_fn: duped_fn,
        arg_types,
        ret_type,
        active: AtomicBool::new(true),
    }));

    // SAFETY: `userdata_ptr` outlives the `Closure` because `RegisteredCallback::destroy` / `Drop` drops `closure` before `userdata_ptr`.
    let userdata_ref: &'static CallbackUserdata = unsafe { &*userdata_ptr };
    let closure = match Closure::try_new(cif, ffi_closure_trampoline, userdata_ref) {
        Ok(c) => c,
        Err(err) => {
            unsafe {
                let mut boxed = Box::from_raw(userdata_ptr);
                qjs::JS_FreeValue(qctx, boxed.js_fn);
                boxed.js_fn = qjs::JS_UNDEFINED;
            }
            return Err(Exception::throw_message(
                ctx,
                &format!("Failed to allocate FFI callback closure: {err:?}"),
            ));
        },
    };

    let code_addr = (*closure.code_ptr() as *const () as usize) as u64;
    let reg = RegisteredCallback {
        closure: Some(closure),
        userdata: userdata_ptr,
    };

    {
        let mut guard = handle.state.lock().unwrap();
        if guard.closed {
            drop(guard);
            let mut reg = reg;
            reg.destroy(qctx);
            return Err(Exception::throw_message(ctx, "FFI library is closed"));
        }
        guard.callbacks.insert(code_addr, reg);
    }

    Ok(code_addr)
}

pub fn shutdown_context<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let qctx = ctx.as_raw().as_ptr();
    let key = qctx as usize;
    let handles = CTX_LIBS.lock().unwrap().remove(&key).unwrap_or_default();
    for handle in handles.into_values() {
        close_lib_handle(&handle, qctx);
    }
    Ok(())
}

unsafe fn write_zero_result(result: &mut usize, ret_type: FfiType) {
    let out_ptr = result as *mut usize as *mut c_void;
    match ret_type {
        FfiType::Void => {},
        FfiType::Char | FfiType::I8 | FfiType::I16 | FfiType::I32 => {
            *(out_ptr as *mut isize) = 0;
        },
        FfiType::U8 | FfiType::Bool | FfiType::U16 | FfiType::U32 => {
            *(out_ptr as *mut usize) = 0;
        },
        FfiType::I64 => *(out_ptr as *mut i64) = 0,
        FfiType::U64 => *(out_ptr as *mut u64) = 0,
        FfiType::Isize => *(out_ptr as *mut isize) = 0,
        FfiType::Usize => *(out_ptr as *mut usize) = 0,
        FfiType::F32 => *(out_ptr as *mut f32) = 0.0,
        FfiType::F64 => *(out_ptr as *mut f64) = 0.0,
        FfiType::Pointer | FfiType::String => *(out_ptr as *mut *mut c_void) = ptr::null_mut(),
    }
}

#[inline]
fn new_int32(val: i32) -> qjs::JSValue {
    if (-(1 << 30)..=(1 << 30)).contains(&val) {
        qjs::JS_MKVAL(qjs::JS_TAG_INT, val)
    } else {
        qjs::JS_NewFloat64(val as f64)
    }
}

unsafe fn c_arg_to_js(
    ctx: *mut qjs::JSContext,
    arg_ptr: *const c_void,
    ty: FfiType,
) -> qjs::JSValue {
    match ty {
        FfiType::Void => qjs::JS_UNDEFINED,
        FfiType::Bool => {
            if *(arg_ptr as *const u8) != 0 {
                qjs::JS_TRUE
            } else {
                qjs::JS_FALSE
            }
        },
        FfiType::Char => new_int32(*(arg_ptr as *const c_char) as i32),
        FfiType::I8 => new_int32(*(arg_ptr as *const i8) as i32),
        FfiType::U8 => new_int32(*(arg_ptr as *const u8) as i32),
        FfiType::I16 => new_int32(*(arg_ptr as *const i16) as i32),
        FfiType::U16 => new_int32(*(arg_ptr as *const u16) as i32),
        FfiType::I32 => new_int32(*(arg_ptr as *const i32)),
        FfiType::U32 => qjs::JS_NewFloat64(*(arg_ptr as *const u32) as f64),
        FfiType::I64 => qjs::JS_NewBigInt64(ctx, *(arg_ptr as *const i64)),
        FfiType::U64 => qjs::JS_NewBigUint64(ctx, *(arg_ptr as *const u64)),
        FfiType::Isize => qjs::JS_NewBigInt64(ctx, *(arg_ptr as *const isize) as i64),
        FfiType::Usize => qjs::JS_NewBigUint64(ctx, *(arg_ptr as *const usize) as u64),
        FfiType::F32 => qjs::JS_NewFloat64(*(arg_ptr as *const f32) as f64),
        FfiType::F64 => qjs::JS_NewFloat64(*(arg_ptr as *const f64)),
        FfiType::Pointer => {
            let p = *(arg_ptr as *const *mut c_void);
            qjs::JS_NewBigUint64(ctx, p as usize as u64)
        },
        FfiType::String => {
            let p = *(arg_ptr as *const *const c_char);
            if p.is_null() {
                qjs::JS_NULL
            } else {
                let s = CStr::from_ptr(p).to_string_lossy();
                qjs::JS_NewStringLen(ctx, s.as_ptr().cast(), s.len() as qjs::size_t)
            }
        },
    }
}

unsafe fn js_return_to_c(
    ctx: *mut qjs::JSContext,
    val: qjs::JSValue,
    ret_type: FfiType,
    result: &mut usize,
) {
    let out_ptr = result as *mut usize as *mut c_void;
    match ret_type {
        FfiType::Void => {},
        FfiType::Bool => {
            let b = qjs::JS_ToBool(ctx, val) > 0;
            *(out_ptr as *mut usize) = usize::from(b);
        },
        FfiType::Char => {
            let mut v = 0i32;
            let _ = qjs::JS_ToInt32(ctx, &mut v, val);
            *(out_ptr as *mut isize) = (v as c_char) as isize;
        },
        FfiType::I8 => {
            let mut v = 0i32;
            let _ = qjs::JS_ToInt32(ctx, &mut v, val);
            *(out_ptr as *mut isize) = (v as i8) as isize;
        },
        FfiType::U8 => {
            let mut v = 0i32;
            let _ = qjs::JS_ToInt32(ctx, &mut v, val);
            *(out_ptr as *mut usize) = ((v as u32) as u8) as usize;
        },
        FfiType::I16 => {
            let mut v = 0i32;
            let _ = qjs::JS_ToInt32(ctx, &mut v, val);
            *(out_ptr as *mut isize) = (v as i16) as isize;
        },
        FfiType::U16 => {
            let mut v = 0i32;
            let _ = qjs::JS_ToInt32(ctx, &mut v, val);
            *(out_ptr as *mut usize) = ((v as u32) as u16) as usize;
        },
        FfiType::I32 => {
            let mut v = 0i32;
            let _ = qjs::JS_ToInt32(ctx, &mut v, val);
            *(out_ptr as *mut isize) = v as isize;
        },
        FfiType::U32 => {
            let mut v = 0i32;
            let _ = qjs::JS_ToInt32(ctx, &mut v, val);
            *(out_ptr as *mut usize) = (v as u32) as usize;
        },
        FfiType::I64 | FfiType::Isize => {
            let v = if qjs::JS_IsBigInt(val) {
                let mut out = 0i64;
                let _ = qjs::JS_ToBigInt64(ctx, &mut out, val);
                out
            } else {
                let mut out = 0i64;
                let _ = qjs::JS_ToInt64(ctx, &mut out, val);
                out
            };
            if matches!(ret_type, FfiType::Isize) {
                *(out_ptr as *mut isize) = v as isize;
            } else {
                *(out_ptr as *mut i64) = v;
            }
        },
        FfiType::U64 | FfiType::Usize => {
            let v = if qjs::JS_IsBigInt(val) {
                let mut out = 0u64;
                let _ = qjs::JS_ToBigUint64(ctx, &mut out, val);
                out
            } else {
                let mut out = 0.0f64;
                let _ = qjs::JS_ToFloat64(ctx, &mut out, val);
                out as u64
            };
            if matches!(ret_type, FfiType::Usize) {
                *(out_ptr as *mut usize) = v as usize;
            } else {
                *(out_ptr as *mut u64) = v;
            }
        },
        FfiType::F32 => {
            let mut v = 0.0f64;
            let _ = qjs::JS_ToFloat64(ctx, &mut v, val);
            *(out_ptr as *mut f32) = v as f32;
        },
        FfiType::F64 => {
            let mut v = 0.0f64;
            let _ = qjs::JS_ToFloat64(ctx, &mut v, val);
            *(out_ptr as *mut f64) = v;
        },
        FfiType::Pointer | FfiType::String => {
            let ptr_val = if qjs::JS_IsNull(val) || qjs::JS_IsUndefined(val) {
                ptr::null_mut()
            } else if qjs::JS_IsBigInt(val) {
                let mut out = 0u64;
                let _ = qjs::JS_ToBigUint64(ctx, &mut out, val);
                out as usize as *mut c_void
            } else if qjs::JS_IsNumber(val) {
                let mut out = 0.0f64;
                let _ = qjs::JS_ToFloat64(ctx, &mut out, val);
                (out as u64 as usize) as *mut c_void
            } else if let Ok((p, len)) = extract_buffer_or_view_ptr(ctx, val) {
                if len == 0 {
                    ptr::null_mut()
                } else {
                    p as *mut c_void
                }
            } else {
                ptr::null_mut()
            };
            *(out_ptr as *mut *mut c_void) = ptr_val;
        },
    }
    if qjs::JS_HasException(ctx) {
        let exc = qjs::JS_GetException(ctx);
        qjs::JS_FreeValue(ctx, exc);
    }
}

unsafe extern "C" fn ffi_closure_trampoline(
    _cif: &ffi_cif,
    result: &mut usize,
    args: *const *const c_void,
    userdata: &CallbackUserdata,
) {
    if !userdata.active.load(Ordering::SeqCst)
        || std::thread::current().id() != userdata.js_thread_id
        || qjs::JS_IsUndefined(userdata.js_fn)
    {
        write_zero_result(result, userdata.ret_type);
        return;
    }

    let ctx = userdata.ctx;
    let mut js_args = Vec::with_capacity(userdata.arg_types.len());
    for (idx, ty) in userdata.arg_types.iter().copied().enumerate() {
        let arg_ptr = *args.add(idx);
        js_args.push(c_arg_to_js(ctx, arg_ptr, ty));
    }

    let ret_val = qjs::JS_Call(
        ctx,
        userdata.js_fn,
        qjs::JS_UNDEFINED,
        js_args.len() as i32,
        js_args.as_mut_ptr(),
    );
    for v in js_args {
        qjs::JS_FreeValue(ctx, v);
    }

    if qjs::JS_IsException(ret_val) {
        let exc = qjs::JS_GetException(ctx);
        qjs::JS_FreeValue(ctx, exc);
        write_zero_result(result, userdata.ret_type);
        return;
    }

    js_return_to_c(ctx, ret_val, userdata.ret_type, result);
    qjs::JS_FreeValue(ctx, ret_val);
}
