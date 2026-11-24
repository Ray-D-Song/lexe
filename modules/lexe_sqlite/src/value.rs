use lexe_utils::bytes::ObjectBytes;
use rquickjs::{Array, Ctx, Function, IntoJs, Object, Result, Value};

use crate::error::{throw_invalid_arg_type, throw_out_of_range, throw_sqlite_error};
use crate::ffi::{
    self, sqlite3_context, sqlite3_stmt, sqlite3_value, SQLITE_BLOB, SQLITE_FLOAT, SQLITE_INTEGER,
    SQLITE_NULL, SQLITE_TEXT,
};
use crate::path::{copy_buffer_to_uint8array, null_prototype_object};

pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
pub const MIN_SAFE_INTEGER: i64 = -9_007_199_254_740_991;

pub fn checked_sqlite_len(ctx: &Ctx<'_>, len: usize) -> Result<i32> {
    i32::try_from(len)
        .map_err(|_| throw_out_of_range(ctx, "SQLite value exceeds the maximum length"))
}

pub fn sqlite_bytes<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8]> {
    if len == 0 {
        Ok(&[])
    } else if ptr.is_null() {
        Err(rquickjs::Error::Unknown)
    } else {
        debug_assert!(!ptr.is_null());
        Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
    }
}

pub fn is_safe_integer(val: i64) -> bool {
    (MIN_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&val)
}

pub fn sqlite_value_to_js<'js>(
    ctx: &Ctx<'js>,
    value: *mut sqlite3_value,
    read_bigints: bool,
) -> Result<Value<'js>> {
    unsafe {
        match ffi::sqlite3_value_type(value) {
            SQLITE_NULL => Ok(Value::new_null(ctx.clone())),
            SQLITE_INTEGER => {
                let val = ffi::sqlite3_value_int64(value);
                if read_bigints {
                    return val.into_js(ctx);
                }
                if is_safe_integer(val) {
                    return Ok(Value::new_float(ctx.clone(), val as f64));
                }
                Err(throw_out_of_range(
                    ctx,
                    format!("Value is too large to be represented as a JavaScript number: {val}"),
                ))
            },
            SQLITE_FLOAT => Ok(Value::new_float(
                ctx.clone(),
                ffi::sqlite3_value_double(value),
            )),
            SQLITE_TEXT => {
                let ptr = ffi::sqlite3_value_text(value);
                let len = ffi::sqlite3_value_bytes(value) as usize;
                let bytes = sqlite_bytes(ptr, len)
                    .map_err(|_| throw_invalid_arg_type(ctx, "value", "string", "invalid utf8"))?;
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                let s = std::str::from_utf8(&bytes[..end])
                    .map_err(|_| throw_invalid_arg_type(ctx, "value", "string", "invalid utf8"))?;
                s.into_js(ctx)
            },
            SQLITE_BLOB => {
                let len = ffi::sqlite3_value_bytes(value) as usize;
                let ptr = ffi::sqlite3_value_blob(value) as *const u8;
                let data = sqlite_bytes(ptr, len)
                    .map_err(|_| throw_invalid_arg_type(ctx, "value", "blob", "invalid"))?;
                copy_buffer_to_uint8array(ctx, data).map(|v| v.into_value())
            },
            _ => Err(throw_invalid_arg_type(
                ctx,
                "value",
                "sqlite value",
                "unknown",
            )),
        }
    }
}

pub fn js_to_sqlite_result<'js>(
    ctx: &Ctx<'js>,
    sqlite_ctx: *mut sqlite3_context,
    value: Value<'js>,
) {
    unsafe {
        if value.is_null() || value.is_undefined() {
            ffi::sqlite3_result_null(sqlite_ctx);
            return;
        }
        if value.is_number() {
            ffi::sqlite3_result_double(sqlite_ctx, value.as_number().unwrap());
            return;
        }
        if let Some(s) = value.as_string() {
            if let Ok(text) = s.to_string() {
                let bytes = text.as_bytes();
                if let Ok(len) = checked_sqlite_len(ctx, bytes.len()) {
                    ffi::lexe_sqlite3_result_text_transient(sqlite_ctx, bytes.as_ptr().cast(), len);
                    return;
                }
            }
            ffi::sqlite3_result_error(sqlite_ctx, c"invalid string".as_ptr(), -1);
            return;
        }
        if value.is_big_int() {
            if let Ok(v) = value.as_big_int().unwrap().clone().to_i64() {
                ffi::sqlite3_result_int64(sqlite_ctx, v);
                return;
            }
            ffi::sqlite3_result_error(
                sqlite_ctx,
                c"BigInt value is too large for SQLite".as_ptr(),
                -1,
            );
            return;
        }
        if let Some(obj) = value.as_object() {
            if let Ok(Some(bytes)) = ObjectBytes::from_array_buffer_view(obj) {
                if let Ok(data) = bytes.as_bytes(ctx) {
                    if let Ok(len) = checked_sqlite_len(ctx, data.len()) {
                        ffi::lexe_sqlite3_result_blob_transient(
                            sqlite_ctx,
                            data.as_ptr().cast(),
                            len,
                        );
                        return;
                    }
                }
            }
        }
        if value.is_promise() {
            ffi::sqlite3_result_error(
                sqlite_ctx,
                c"Asynchronous user-defined functions are not supported".as_ptr(),
                -1,
            );
            return;
        }
        ffi::sqlite3_result_error(
            sqlite_ctx,
            c"Returned JavaScript value cannot be converted to a SQLite value".as_ptr(),
            -1,
        );
    }
}

pub fn column_to_js<'js>(
    ctx: &Ctx<'js>,
    stmt: *mut sqlite3_stmt,
    column: i32,
    read_bigints: bool,
) -> Result<Value<'js>> {
    unsafe {
        match ffi::sqlite3_column_type(stmt, column) {
            SQLITE_NULL => Ok(Value::new_null(ctx.clone())),
            SQLITE_INTEGER => {
                let val = ffi::sqlite3_column_int64(stmt, column);
                if read_bigints {
                    return val.into_js(ctx);
                }
                if is_safe_integer(val) {
                    return Ok(Value::new_float(ctx.clone(), val as f64));
                }
                Err(throw_out_of_range(
                    ctx,
                    format!("Value is too large to be represented as a JavaScript number: {val}"),
                ))
            },
            SQLITE_FLOAT => Ok(Value::new_float(
                ctx.clone(),
                ffi::sqlite3_column_double(stmt, column),
            )),
            SQLITE_TEXT => {
                let ptr = ffi::sqlite3_column_text(stmt, column);
                let len = ffi::sqlite3_column_bytes(stmt, column) as usize;
                let bytes = sqlite_bytes(ptr, len)
                    .map_err(|_| throw_invalid_arg_type(ctx, "value", "string", "invalid utf8"))?;
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                let s = std::str::from_utf8(&bytes[..end])
                    .map_err(|_| throw_invalid_arg_type(ctx, "value", "string", "invalid utf8"))?;
                s.into_js(ctx)
            },
            SQLITE_BLOB => {
                let len = ffi::sqlite3_column_bytes(stmt, column) as usize;
                let ptr = ffi::sqlite3_column_blob(stmt, column) as *const u8;
                let data = sqlite_bytes(ptr, len)
                    .map_err(|_| throw_invalid_arg_type(ctx, "value", "blob", "invalid"))?;
                copy_buffer_to_uint8array(ctx, data).map(|v| v.into_value())
            },
            _ => Err(throw_invalid_arg_type(
                ctx,
                "value",
                "sqlite value",
                "unknown",
            )),
        }
    }
}

pub fn row_object<'js>(
    ctx: &Ctx<'js>,
    stmt: *mut sqlite3_stmt,
    read_bigints: bool,
) -> Result<Object<'js>> {
    let obj = null_prototype_object(ctx)?;
    let count = unsafe { ffi::sqlite3_column_count(stmt) };
    for i in 0..count {
        let name_ptr = unsafe { ffi::sqlite3_column_name(stmt, i) };
        let name = unsafe {
            std::ffi::CStr::from_ptr(name_ptr)
                .to_string_lossy()
                .into_owned()
        };
        let value = column_to_js(ctx, stmt, i, read_bigints)?;
        obj.set(name.as_str(), value)?;
    }
    Ok(obj)
}

pub fn row_array<'js>(
    ctx: &Ctx<'js>,
    stmt: *mut sqlite3_stmt,
    read_bigints: bool,
) -> Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    let count = unsafe { ffi::sqlite3_column_count(stmt) };
    for i in 0..count {
        let value = column_to_js(ctx, stmt, i, read_bigints)?;
        arr.set(i as usize, value)?;
    }
    Ok(arr)
}

pub fn bind_js_value<'js>(
    ctx: &Ctx<'js>,
    stmt: *mut sqlite3_stmt,
    index: i32,
    value: Value<'js>,
) -> Result<()> {
    unsafe {
        if value.is_null() || value.is_undefined() {
            let r = ffi::sqlite3_bind_null(stmt, index);
            if r != ffi::SQLITE_OK {
                return Err(throw_sqlite_error(ctx, ptr::null_mut()));
            }
            return Ok(());
        }
        if value.is_number() {
            let r = ffi::sqlite3_bind_double(stmt, index, value.as_number().unwrap());
            if r != ffi::SQLITE_OK {
                return Err(throw_sqlite_error(ctx, ptr::null_mut()));
            }
            return Ok(());
        }
        if let Some(s) = value.as_string() {
            let text = s.to_string()?;
            let bytes = text.as_bytes();
            let len = checked_sqlite_len(ctx, bytes.len())?;
            let r = ffi::lexe_sqlite3_bind_text_transient(stmt, index, bytes.as_ptr().cast(), len);
            if r != ffi::SQLITE_OK {
                return Err(throw_sqlite_error(ctx, ptr::null_mut()));
            }
            return Ok(());
        }
        if value.is_big_int() {
            let v = value
                .as_big_int()
                .unwrap()
                .clone()
                .to_i64()
                .map_err(|_| throw_out_of_range(ctx, "BigInt value is too large for SQLite"))?;
            let r = ffi::sqlite3_bind_int64(stmt, index, v);
            if r != ffi::SQLITE_OK {
                return Err(throw_sqlite_error(ctx, ptr::null_mut()));
            }
            return Ok(());
        }
        if let Some(obj) = value.as_object() {
            if let Ok(Some(bytes)) = ObjectBytes::from_array_buffer_view(obj) {
                let data = bytes.as_bytes(ctx)?;
                let len = checked_sqlite_len(ctx, data.len())?;
                let r =
                    ffi::lexe_sqlite3_bind_blob_transient(stmt, index, data.as_ptr().cast(), len);
                if r != ffi::SQLITE_OK {
                    return Err(throw_sqlite_error(ctx, ptr::null_mut()));
                }
                return Ok(());
            }
        }
        Err(throw_invalid_arg_type(
            ctx,
            "value",
            "number, string, bigint, null, undefined, or ArrayBufferView",
            &super::path::js_type_name(&value),
        ))
    }
}

use std::ptr;

pub fn is_plain_named_params<'js>(value: &Value<'js>) -> bool {
    if let Some(obj) = value.as_object() {
        if obj.is_array() {
            return false;
        }
        if ObjectBytes::from_array_buffer_view(obj)
            .ok()
            .flatten()
            .is_some()
        {
            return false;
        }
        return true;
    }
    false
}

pub fn get_function_length<'js>(_ctx: &Ctx<'js>, func: &Function<'js>) -> Result<i32> {
    let len: i32 = func.get("length")?;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_bytes_zero_length_null_pointer() {
        assert_eq!(sqlite_bytes(std::ptr::null(), 0).unwrap(), &[] as &[u8]);
    }

    #[test]
    fn sqlite_bytes_nonzero_null_pointer_errors() {
        assert!(sqlite_bytes(std::ptr::null(), 1).is_err());
    }

    #[test]
    fn is_safe_integer_bounds() {
        assert!(is_safe_integer(0));
        assert!(is_safe_integer(MAX_SAFE_INTEGER));
        assert!(is_safe_integer(MIN_SAFE_INTEGER));
        assert!(!is_safe_integer(MAX_SAFE_INTEGER + 1));
        assert!(!is_safe_integer(MIN_SAFE_INTEGER - 1));
        assert!(!is_safe_integer(i64::MIN));
    }

    #[test]
    fn checked_sqlite_len_accepts_valid_length() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let ctx = rquickjs::Context::full(&runtime).unwrap();
        ctx.with(|ctx| {
            assert_eq!(checked_sqlite_len(&ctx, 4).unwrap(), 4);
        });
    }
}
