// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::ptr;

use libffi::middle::{Arg, CodePtr, Type};
use rquickjs::{qjs, Ctx, Exception, Result, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfiType {
    Void,
    Bool,
    Char,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Pointer,
    String,
    Usize,
    Isize,
}

impl FfiType {
    pub fn parse(name: &str) -> std::result::Result<Self, String> {
        match name {
            "void" => Ok(Self::Void),
            "bool" => Ok(Self::Bool),
            "char" => Ok(Self::Char),
            "i8" | "int8" | "int8_t" => Ok(Self::I8),
            "u8" | "uint8" | "uint8_t" => Ok(Self::U8),
            "i16" | "int16" | "int16_t" => Ok(Self::I16),
            "u16" | "uint16" | "uint16_t" => Ok(Self::U16),
            "i32" | "int" | "int32" | "int32_t" => Ok(Self::I32),
            "u32" | "uint" | "uint32" | "uint32_t" => Ok(Self::U32),
            "i64" | "int64" | "int64_t" => Ok(Self::I64),
            "u64" | "uint64" | "uint64_t" => Ok(Self::U64),
            "f32" | "float" => Ok(Self::F32),
            "f64" | "double" => Ok(Self::F64),
            "pointer" | "ptr" | "buffer" | "function" | "callback" => Ok(Self::Pointer),
            "string" | "cstring" => Ok(Self::String),
            "usize" => Ok(Self::Usize),
            "isize" => Ok(Self::Isize),
            other => Err(format!("Unsupported node:ffi type: {other}")),
        }
    }

    pub fn to_libffi(self) -> Type {
        match self {
            Self::Void => Type::void(),
            Self::Bool => Type::u8(),
            Self::Char => Type::c_schar(),
            Self::I8 => Type::i8(),
            Self::U8 => Type::u8(),
            Self::I16 => Type::i16(),
            Self::U16 => Type::u16(),
            Self::I32 => Type::i32(),
            Self::U32 => Type::u32(),
            Self::I64 => Type::i64(),
            Self::U64 => Type::u64(),
            Self::F32 => Type::f32(),
            Self::F64 => Type::f64(),
            Self::Pointer | Self::String => Type::pointer(),
            Self::Usize => Type::usize(),
            Self::Isize => Type::isize(),
        }
    }
}

pub struct SyncCif(pub libffi::middle::Cif);

// SAFETY: `ffi_cif` is constructed once at `dlopen`/`registerCallback` time and only read on the JS thread.
unsafe impl Send for SyncCif {}
unsafe impl Sync for SyncCif {}

pub enum FfiArgStorage {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Usize(usize),
    Isize(isize),
    F32(f32),
    F64(f64),
    Ptr(*mut c_void),
}

impl FfiArgStorage {
    pub fn as_arg(&self) -> Arg<'_> {
        match self {
            Self::U8(v) => Arg::new(v),
            Self::I8(v) => Arg::new(v),
            Self::U16(v) => Arg::new(v),
            Self::I16(v) => Arg::new(v),
            Self::U32(v) => Arg::new(v),
            Self::I32(v) => Arg::new(v),
            Self::U64(v) => Arg::new(v),
            Self::I64(v) => Arg::new(v),
            Self::Usize(v) => Arg::new(v),
            Self::Isize(v) => Arg::new(v),
            Self::F32(v) => Arg::new(v),
            Self::F64(v) => Arg::new(v),
            Self::Ptr(p) => Arg::new(p),
        }
    }
}

pub fn throw_qjs_error<'js, T>(ctx: &Ctx<'js>, kind: &str, msg: &str) -> Result<T> {
    Err(match kind {
        "TypeError" => Exception::throw_type(ctx, msg),
        "RangeError" => Exception::throw_range(ctx, msg),
        _ => Exception::throw_message(ctx, msg),
    })
}

/// Extract backing pointer and byte length from `ArrayBuffer`, `TypedArray`/`Buffer`, or `DataView`.
pub unsafe fn extract_buffer_or_view_ptr(
    ctx: *mut qjs::JSContext,
    val: qjs::JSValue,
) -> std::result::Result<(*mut u8, usize), (&'static str, &'static str)> {
    if qjs::JS_IsArrayBuffer(val) {
        let mut len: qjs::size_t = 0;
        let ptr = qjs::JS_GetArrayBuffer(ctx, &mut len, val);
        if ptr.is_null() {
            return Err(("TypeError", "ArrayBuffer is detached"));
        }
        return Ok((ptr, len as usize));
    }

    if qjs::JS_IsDataView(val) {
        let ab = qjs::JS_GetPropertyStr(ctx, val, c"buffer".as_ptr());
        if qjs::JS_IsException(ab) {
            let exc = qjs::JS_GetException(ctx);
            qjs::JS_FreeValue(ctx, exc);
            return Err(("TypeError", "Failed to read DataView buffer"));
        }
        if !qjs::JS_IsArrayBuffer(ab) {
            qjs::JS_FreeValue(ctx, ab);
            return Err((
                "TypeError",
                "node:ffi pointer only supports ArrayBuffer and ArrayBufferView values backed by ArrayBuffer",
            ));
        }
        let mut ab_len: qjs::size_t = 0;
        let base = qjs::JS_GetArrayBuffer(ctx, &mut ab_len, ab);
        let offset_val = qjs::JS_GetPropertyStr(ctx, val, c"byteOffset".as_ptr());
        let len_val = qjs::JS_GetPropertyStr(ctx, val, c"byteLength".as_ptr());
        let mut offset: i64 = 0;
        let mut view_len: i64 = 0;
        let ok_off = qjs::JS_ToInt64(ctx, &mut offset, offset_val) >= 0;
        let ok_len = qjs::JS_ToInt64(ctx, &mut view_len, len_val) >= 0;
        qjs::JS_FreeValue(ctx, offset_val);
        qjs::JS_FreeValue(ctx, len_val);
        qjs::JS_FreeValue(ctx, ab);
        if base.is_null() || !ok_off || !ok_len {
            if !ok_off || !ok_len {
                let exc = qjs::JS_GetException(ctx);
                qjs::JS_FreeValue(ctx, exc);
            }
            return Err(("TypeError", "ArrayBuffer is detached"));
        }
        return Ok((base.add(offset as usize), view_len as usize));
    }

    let mut byte_offset: qjs::size_t = 0;
    let mut byte_length: qjs::size_t = 0;
    let mut bpe: qjs::size_t = 0;
    let ab = qjs::JS_GetTypedArrayBuffer(ctx, val, &mut byte_offset, &mut byte_length, &mut bpe);
    if !qjs::JS_IsException(ab) {
        let mut ab_len: qjs::size_t = 0;
        let base = qjs::JS_GetArrayBuffer(ctx, &mut ab_len, ab);
        qjs::JS_FreeValue(ctx, ab);
        if base.is_null() {
            return Err(("TypeError", "ArrayBuffer is detached"));
        }
        return Ok((base.add(byte_offset as usize), byte_length as usize));
    }

    let exc = qjs::JS_GetException(ctx);
    qjs::JS_FreeValue(ctx, exc);
    Err((
        "TypeError",
        "node:ffi pointer only supports ArrayBuffer and ArrayBufferView values backed by ArrayBuffer",
    ))
}

unsafe fn is_negative_bigint(ctx: *mut qjs::JSContext, val: qjs::JSValue) -> bool {
    let mut len: qjs::size_t = 0;
    let ptr = qjs::JS_ToCStringLen2(ctx, &mut len, val, false);
    if ptr.is_null() {
        let exc = qjs::JS_GetException(ctx);
        qjs::JS_FreeValue(ctx, exc);
        return false;
    }
    let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    let neg = slice.first() == Some(&b'-');
    qjs::JS_FreeCString(ctx, ptr);
    neg
}

pub fn js_to_pointer_u64<'js>(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<u64> {
    let raw = val.as_raw();
    let qctx = ctx.as_raw().as_ptr();
    if unsafe { qjs::JS_IsBigInt(raw) } {
        let mut out: u64 = 0;
        if unsafe { qjs::JS_ToBigUint64(qctx, &mut out, raw) } < 0 {
            return Err(Error::Exception);
        }
        if (out as i64) < 0 && unsafe { is_negative_bigint(qctx, raw) } {
            return throw_qjs_error(ctx, "RangeError", "Pointer address cannot be negative");
        }
        return Ok(out);
    }
    if val.is_number() {
        let n = val
            .as_number()
            .ok_or_else(|| Exception::throw_type(ctx, "Expected a pointer number or BigInt"))?;
        if !n.is_finite() || n.fract() != 0.0 || n < 0.0 || n > (u64::MAX as f64) {
            return throw_qjs_error(ctx, "RangeError", "Invalid pointer address");
        }
        return Ok(n as u64);
    }
    throw_qjs_error(
        ctx,
        "TypeError",
        "Pointer must be a BigInt or integer number",
    )
}

pub fn js_to_i64<'js>(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<i64> {
    let raw = val.as_raw();
    let qctx = ctx.as_raw().as_ptr();
    if unsafe { qjs::JS_IsBigInt(raw) } {
        let mut out: i64 = 0;
        if unsafe { qjs::JS_ToBigInt64(qctx, &mut out, raw) } < 0 {
            return Err(Error::Exception);
        }
        return Ok(out);
    }
    if val.is_number() {
        let n = val.as_number().unwrap_or(0.0);
        return Ok(n as i64);
    }
    if let Some(b) = val.as_bool() {
        return Ok(i64::from(b));
    }
    throw_qjs_error(ctx, "TypeError", "Expected i64 number or BigInt")
}

pub fn js_to_u64<'js>(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<u64> {
    let raw = val.as_raw();
    let qctx = ctx.as_raw().as_ptr();
    if unsafe { qjs::JS_IsBigInt(raw) } {
        let mut out: u64 = 0;
        if unsafe { qjs::JS_ToBigUint64(qctx, &mut out, raw) } < 0 {
            return Err(Error::Exception);
        }
        return Ok(out);
    }
    if val.is_number() {
        let n = val.as_number().unwrap_or(0.0);
        return Ok(if n < 0.0 { (n as i64) as u64 } else { n as u64 });
    }
    if let Some(b) = val.as_bool() {
        return Ok(u64::from(b));
    }
    throw_qjs_error(ctx, "TypeError", "Expected u64 number or BigInt")
}

fn js_to_f64<'js>(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<f64> {
    if val.is_undefined() {
        return Ok(0.0);
    }
    let mut out = 0.0f64;
    if unsafe { qjs::JS_ToFloat64(ctx.as_raw().as_ptr(), &mut out, val.as_raw()) } < 0 {
        return Err(Error::Exception);
    }
    Ok(out)
}

fn js_to_int32<'js>(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<i32> {
    if val.is_undefined() {
        return Ok(0);
    }
    let mut out = 0i32;
    if unsafe { qjs::JS_ToInt32(ctx.as_raw().as_ptr(), &mut out, val.as_raw()) } < 0 {
        return Err(Error::Exception);
    }
    Ok(out)
}

fn js_to_uint32<'js>(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<u32> {
    Ok(js_to_int32(ctx, val)? as u32)
}

use rquickjs::Error;

pub fn marshal_js_arg<'js>(
    ctx: &Ctx<'js>,
    ty: FfiType,
    val: Option<&Value<'js>>,
    c_strings: &mut Vec<CString>,
) -> Result<FfiArgStorage> {
    let undef = Value::new_undefined(ctx.clone());
    let val = val.unwrap_or(&undef);
    let qctx = ctx.as_raw().as_ptr();

    Ok(match ty {
        FfiType::Void => FfiArgStorage::Ptr(ptr::null_mut()),
        FfiType::Bool => {
            let b = if let Some(b) = val.as_bool() {
                u8::from(b)
            } else if val.is_number() {
                u8::from(val.as_number().unwrap_or(0.0) != 0.0)
            } else {
                u8::from(unsafe { qjs::JS_ToBool(qctx, val.as_raw()) } > 0)
            };
            FfiArgStorage::U8(b)
        },
        FfiType::Char | FfiType::I8 => FfiArgStorage::I8(js_to_int32(ctx, val)? as i8),
        FfiType::U8 => FfiArgStorage::U8(js_to_uint32(ctx, val)? as u8),
        FfiType::I16 => FfiArgStorage::I16(js_to_int32(ctx, val)? as i16),
        FfiType::U16 => FfiArgStorage::U16(js_to_uint32(ctx, val)? as u16),
        FfiType::I32 => FfiArgStorage::I32(js_to_int32(ctx, val)?),
        FfiType::U32 => FfiArgStorage::U32(js_to_uint32(ctx, val)?),
        FfiType::I64 => FfiArgStorage::I64(if val.is_undefined() {
            0
        } else {
            js_to_i64(ctx, val)?
        }),
        FfiType::U64 => FfiArgStorage::U64(if val.is_undefined() {
            0
        } else {
            js_to_u64(ctx, val)?
        }),
        FfiType::Usize => FfiArgStorage::Usize(if val.is_undefined() {
            0
        } else {
            js_to_u64(ctx, val)? as usize
        }),
        FfiType::Isize => FfiArgStorage::Isize(if val.is_undefined() {
            0
        } else {
            js_to_i64(ctx, val)? as isize
        }),
        FfiType::F32 => FfiArgStorage::F32(js_to_f64(ctx, val)? as f32),
        FfiType::F64 => FfiArgStorage::F64(js_to_f64(ctx, val)?),
        FfiType::Pointer | FfiType::String => {
            if val.is_null() || val.is_undefined() {
                return Ok(FfiArgStorage::Ptr(ptr::null_mut()));
            }
            if val.is_string() {
                let s = val.as_string().unwrap().to_string()?;
                let cstr = CString::new(s).map_err(|_| {
                    Exception::throw_type(ctx, "FFI string contains interior NUL byte")
                })?;
                let raw_ptr = cstr.as_ptr() as *mut c_void;
                c_strings.push(cstr);
                return Ok(FfiArgStorage::Ptr(raw_ptr));
            }
            if unsafe { qjs::JS_IsBigInt(val.as_raw()) } || val.is_number() {
                let addr = js_to_pointer_u64(ctx, val)? as usize;
                return Ok(FfiArgStorage::Ptr(addr as *mut c_void));
            }
            match unsafe { extract_buffer_or_view_ptr(qctx, val.as_raw()) } {
                Ok((p, len)) => {
                    if len == 0 {
                        FfiArgStorage::Ptr(ptr::null_mut())
                    } else {
                        FfiArgStorage::Ptr(p as *mut c_void)
                    }
                },
                Err((kind, msg)) => return throw_qjs_error(ctx, kind, msg),
            }
        },
    })
}

pub unsafe fn call_cif<'js>(
    ctx: &Ctx<'js>,
    cif: &libffi::middle::Cif,
    code_ptr: CodePtr,
    ret_type: FfiType,
    ffi_args: &[Arg<'_>],
) -> Result<Value<'js>> {
    let qctx = ctx.as_raw().as_ptr();
    Ok(match ret_type {
        FfiType::Void => {
            cif.call::<()>(code_ptr, ffi_args);
            Value::new_undefined(ctx.clone())
        },
        FfiType::Bool => Value::new_bool(ctx.clone(), cif.call::<u8>(code_ptr, ffi_args) != 0),
        FfiType::Char => {
            let v = cif.call::<std::os::raw::c_char>(code_ptr, ffi_args);
            Value::new_int(ctx.clone(), v as i32)
        },
        FfiType::I8 => Value::new_int(ctx.clone(), cif.call::<i8>(code_ptr, ffi_args) as i32),
        FfiType::U8 => Value::new_int(ctx.clone(), cif.call::<u8>(code_ptr, ffi_args) as i32),
        FfiType::I16 => Value::new_int(ctx.clone(), cif.call::<i16>(code_ptr, ffi_args) as i32),
        FfiType::U16 => Value::new_int(ctx.clone(), cif.call::<u16>(code_ptr, ffi_args) as i32),
        FfiType::I32 => Value::new_int(ctx.clone(), cif.call::<i32>(code_ptr, ffi_args)),
        FfiType::U32 => Value::new_float(ctx.clone(), cif.call::<u32>(code_ptr, ffi_args) as f64),
        FfiType::I64 => {
            let v = cif.call::<i64>(code_ptr, ffi_args);
            Value::from_raw(ctx.clone(), qjs::JS_NewBigInt64(qctx, v))
        },
        FfiType::U64 => {
            let v = cif.call::<u64>(code_ptr, ffi_args);
            Value::from_raw(ctx.clone(), qjs::JS_NewBigUint64(qctx, v))
        },
        FfiType::Isize => {
            let v = cif.call::<isize>(code_ptr, ffi_args) as i64;
            Value::from_raw(ctx.clone(), qjs::JS_NewBigInt64(qctx, v))
        },
        FfiType::Usize => {
            let v = cif.call::<usize>(code_ptr, ffi_args) as u64;
            Value::from_raw(ctx.clone(), qjs::JS_NewBigUint64(qctx, v))
        },
        FfiType::F32 => Value::new_float(ctx.clone(), cif.call::<f32>(code_ptr, ffi_args) as f64),
        FfiType::F64 => Value::new_float(ctx.clone(), cif.call::<f64>(code_ptr, ffi_args)),
        FfiType::Pointer => {
            let p = cif.call::<*mut c_void>(code_ptr, ffi_args);
            Value::from_raw(ctx.clone(), qjs::JS_NewBigUint64(qctx, p as usize as u64))
        },
        FfiType::String => {
            let p = cif.call::<*const std::os::raw::c_char>(code_ptr, ffi_args);
            if p.is_null() {
                Value::new_null(ctx.clone())
            } else {
                let s = CStr::from_ptr(p).to_string_lossy();
                rquickjs::String::from_str(ctx.clone(), &s)?.into_value()
            }
        },
    })
}
