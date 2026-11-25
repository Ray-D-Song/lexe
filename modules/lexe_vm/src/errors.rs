// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{ffi::CString, mem::MaybeUninit};

use rquickjs::{object::Property, qjs, Ctx, Error, Exception, Function, JsLifetime, Result, Value};

pub fn into_raw_value(val: Value<'_>) -> qjs::JSValue {
    let raw = unsafe { qjs::JS_DupValue(val.ctx().as_raw().as_ptr(), val.as_raw()) };
    drop(val);
    raw
}

#[derive(JsLifetime)]
pub struct VmPrimordials<'js> {
    pub eval_error_ctor: Option<Function<'js>>,
}

impl<'js> VmPrimordials<'js> {
    pub fn init_if_needed(ctx: &Ctx<'js>) -> Result<()> {
        if ctx.userdata::<Self>().is_some() {
            return Ok(());
        }
        let globals = ctx.globals();
        let eval_error_ctor = globals
            .get::<_, Value>("EvalError")
            .ok()
            .and_then(|v| v.into_function());
        let _ = ctx.store_userdata(Self { eval_error_ctor });
        Ok(())
    }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum ErrorKind {
    Error,
    TypeError,
    RangeError,
    EvalError,
    ReferenceError,
}

/// Owned C string returned by QuickJS `JS_ToCStringLen`, preserving embedded NULs.
pub struct JsCString {
    ctx: *mut qjs::JSContext,
    ptr: *const std::ffi::c_char,
    len: usize,
}

impl JsCString {
    pub fn from_value(ctx: &Ctx<'_>, val: &Value<'_>) -> Result<Self> {
        let mut len = MaybeUninit::uninit();
        let ctx_ptr = ctx.as_raw().as_ptr();
        let ptr = unsafe { qjs::JS_ToCStringLen(ctx_ptr, len.as_mut_ptr(), val.as_raw()) };
        if ptr.is_null() {
            return Err(Error::Exception);
        }
        let len = unsafe { len.assume_init() };
        Ok(Self {
            ctx: ctx_ptr,
            ptr,
            len,
        })
    }

    pub fn as_ptr_len(&self) -> (*const std::ffi::c_char, usize) {
        (self.ptr, self.len)
    }

    pub fn as_bytes(&self) -> &[u8] {
        if self.ptr.is_null() || self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }

    pub fn as_str(&self) -> Result<&str> {
        std::str::from_utf8(self.as_bytes()).map_err(|_| Error::Unknown)
    }
}

impl Drop for JsCString {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { qjs::JS_FreeCString(self.ctx, self.ptr) };
        }
    }
}

/// Validates that a string has no embedded NUL bytes and converts it to a `CString`.
pub fn validated_cstring(ctx: &Ctx<'_>, prop_name: &str, value: &str) -> Result<CString> {
    if value.as_bytes().contains(&0) {
        let msg = format!("The \"{prop_name}\" property must not contain null bytes");
        return Err(throw_coded(
            ctx,
            ErrorKind::TypeError,
            Some("ERR_INVALID_ARG_VALUE"),
            &msg,
        ));
    }
    CString::new(value).map_err(|_| Error::Unknown)
}

/// Describes a JS value cleanly for `ERR_INVALID_ARG_TYPE` without calling user getters.
pub fn describe_received(val: &Value<'_>) -> String {
    if val.is_null() {
        "null".to_string()
    } else if val.is_undefined() {
        "undefined".to_string()
    } else if let Some(b) = val.as_bool() {
        format!("type boolean ({b})")
    } else if let Some(n) = val.as_int() {
        format!("type number ({n})")
    } else if let Some(f) = val.as_float() {
        format!("type number ({f})")
    } else if let Some(s) = val.as_string() {
        match s.to_string() {
            Ok(text) => format!("type string ({text:?})"),
            Err(_) => "type string".to_string(),
        }
    } else if val.is_function() {
        "type function".to_string()
    } else if val.is_symbol() {
        "type symbol".to_string()
    } else if val.is_big_int() {
        "type bigint".to_string()
    } else if val.is_array() {
        "an instance of Array".to_string()
    } else if val.is_object() {
        "an instance of Object".to_string()
    } else {
        format!("type {}", val.type_name())
    }
}

/// Constructs an Error instance using QuickJS native error internals so even poisoned
/// `globalThis.TypeError` / `Error` cannot hijack error construction.
fn make_native_error<'js>(ctx: &Ctx<'js>, kind: ErrorKind, msg: &str) -> Value<'js> {
    let err_marker = match kind {
        ErrorKind::Error | ErrorKind::EvalError => Exception::throw_message(ctx, msg),
        ErrorKind::TypeError => Exception::throw_type(ctx, msg),
        ErrorKind::RangeError => Exception::throw_range(ctx, msg),
        ErrorKind::ReferenceError => Exception::throw_reference(ctx, msg),
    };
    let _ = err_marker;
    let val = ctx.catch();
    if let Some(obj) = val.as_object() {
        if matches!(kind, ErrorKind::EvalError) {
            if let Some(p) = ctx.userdata::<VmPrimordials<'js>>() {
                if let Some(eval_ctor) = p.eval_error_ctor.as_ref() {
                    if let Ok(eval_err) = eval_ctor.call::<_, Value>((msg,)) {
                        if eval_err.is_object() {
                            return eval_err;
                        }
                    }
                }
            }
            let _ = obj.set("name", "EvalError");
        }
    }
    val
}

pub fn make_coded_error<'js>(
    ctx: &Ctx<'js>,
    kind: ErrorKind,
    code: Option<&str>,
    msg: &str,
) -> Value<'js> {
    let err_val = make_native_error(ctx, kind, msg);
    if let (Some(code), Some(obj)) = (code, err_val.as_object()) {
        let _ = obj.prop("code", Property::from(code).writable().configurable());
    }
    err_val
}

pub fn throw_coded(ctx: &Ctx<'_>, kind: ErrorKind, code: Option<&str>, msg: &str) -> Error {
    let err_val = make_coded_error(ctx, kind, code, msg);
    ctx.throw(err_val)
}

pub fn throw_invalid_arg_type(
    ctx: &Ctx<'_>,
    name: &str,
    expected: &str,
    actual: &Value<'_>,
) -> Error {
    let msg = format!(
        "The \"{name}\" argument must be {expected}. Received {}",
        describe_received(actual)
    );
    throw_coded(
        ctx,
        ErrorKind::TypeError,
        Some("ERR_INVALID_ARG_TYPE"),
        &msg,
    )
}

pub fn throw_invalid_arg_value(ctx: &Ctx<'_>, name: &str, value_repr: &str, reason: &str) -> Error {
    let msg = format!("The argument '{name}' {reason}. Received {value_repr}");
    throw_coded(
        ctx,
        ErrorKind::TypeError,
        Some("ERR_INVALID_ARG_VALUE"),
        &msg,
    )
}

pub fn throw_out_of_range(ctx: &Ctx<'_>, name: &str, range: &str, actual: &str) -> Error {
    let msg =
        format!("The value of \"{name}\" is out of range. It must be {range}. Received {actual}");
    throw_coded(ctx, ErrorKind::RangeError, Some("ERR_OUT_OF_RANGE"), &msg)
}

pub fn throw_script_timeout(ctx: &Ctx<'_>, timeout_ms: u64) -> Error {
    let msg = format!("Script execution timed out after {timeout_ms}ms");
    throw_coded(
        ctx,
        ErrorKind::Error,
        Some("ERR_SCRIPT_EXECUTION_TIMEOUT"),
        &msg,
    )
}

pub fn throw_script_interrupted(ctx: &Ctx<'_>) -> Error {
    throw_coded(
        ctx,
        ErrorKind::Error,
        Some("ERR_SCRIPT_EXECUTION_INTERRUPTED"),
        "Script execution was interrupted by `SIGINT`",
    )
}

pub fn make_dynamic_import_missing_error<'js>(ctx: &Ctx<'js>) -> Value<'js> {
    make_coded_error(
        ctx,
        ErrorKind::TypeError,
        Some("ERR_VM_DYNAMIC_IMPORT_CALLBACK_MISSING"),
        "A dynamic import callback was not specified.",
    )
}

pub fn throw_vm_module_status(ctx: &Ctx<'_>, msg: &str) -> Error {
    throw_coded(ctx, ErrorKind::Error, Some("ERR_VM_MODULE_STATUS"), msg)
}

pub fn throw_vm_module_already_linked(ctx: &Ctx<'_>) -> Error {
    throw_coded(
        ctx,
        ErrorKind::Error,
        Some("ERR_VM_MODULE_ALREADY_LINKED"),
        "Module has already been linked",
    )
}

pub fn throw_vm_module_different_context(ctx: &Ctx<'_>) -> Error {
    throw_coded(
        ctx,
        ErrorKind::Error,
        Some("ERR_VM_MODULE_DIFFERENT_CONTEXT"),
        "Linked modules must use the same context",
    )
}

pub fn throw_vm_module_cannot_create_cached_data(ctx: &Ctx<'_>) -> Error {
    throw_coded(
        ctx,
        ErrorKind::Error,
        Some("ERR_VM_MODULE_CANNOT_CREATE_CACHED_DATA"),
        "Cached data cannot be created for a module which has been evaluated",
    )
}

/// Transfers any pending exception on `child_raw` to `parent_ctx` within the same JSRuntime.
///
/// # Safety
/// `child_raw` and `parent_ctx` must belong to the same alive `JSRuntime`.
pub unsafe fn transfer_exception(child_raw: *mut qjs::JSContext, parent_ctx: &Ctx<'_>) -> Error {
    let exc = qjs::JS_GetException(child_raw);
    if qjs::JS_VALUE_GET_NORM_TAG(exc) == qjs::JS_TAG_NULL
        || qjs::JS_VALUE_GET_NORM_TAG(exc) == qjs::JS_TAG_UNDEFINED
    {
        qjs::JS_FreeValue(child_raw, exc);
        return Error::Unknown;
    }
    qjs::JS_ResetUncatchableError(child_raw);
    qjs::JS_Throw(parent_ctx.as_raw().as_ptr(), exc);
    Error::Exception
}
