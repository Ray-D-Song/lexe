use lexe_url::url_class::URL;
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{Class, Ctx, Object, Result, TypedArray, Value};

use crate::error::{throw_invalid_arg_type, throw_invalid_arg_value};

pub fn parse_path<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Vec<u8>> {
    if let Some(s) = value.as_string() {
        let s = s.to_string()?;
        if s.contains('\0') {
            return Err(throw_invalid_arg_value(
                ctx,
                "path cannot contain NUL bytes",
            ));
        }
        return Ok(s.into_bytes());
    }

    if let Some(obj) = value.as_object() {
        if let Some(url_inst) = Class::<URL>::from_object(obj) {
            let url = url_inst.borrow();
            let protocol = url.protocol();
            if protocol != "file:" {
                return Err(throw_invalid_arg_value(
                    ctx,
                    "Only file: URLs are supported",
                ));
            }
            let path = url.pathname();
            if path.contains('\0') {
                return Err(throw_invalid_arg_value(
                    ctx,
                    "path cannot contain NUL bytes",
                ));
            }
            return Ok(path.into_bytes());
        }

        if let Some(bytes) = ObjectBytes::from_array_buffer_view(obj)? {
            let slice = bytes.as_bytes(ctx)?;
            if slice.contains(&0) {
                return Err(throw_invalid_arg_value(
                    ctx,
                    "path cannot contain NUL bytes",
                ));
            }
            return Ok(slice.to_vec());
        }
    }

    Err(throw_invalid_arg_type(
        ctx,
        "path",
        "string, Uint8Array, or file: URL",
        &js_type_name(&value),
    ))
}

pub fn js_type_name(value: &Value<'_>) -> String {
    if value.is_undefined() {
        "undefined".into()
    } else if value.is_null() {
        "null".into()
    } else if value.is_bool() {
        "boolean".into()
    } else if value.is_number() {
        "number".into()
    } else if value.is_string() {
        "string".into()
    } else if value.is_function() {
        "function".into()
    } else if value.is_object() {
        "object".into()
    } else if value.is_symbol() {
        "symbol".into()
    } else if value.is_big_int() {
        "bigint".into()
    } else {
        "unknown".into()
    }
}

pub fn null_prototype_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set_prototype(None)?;
    Ok(obj)
}

pub fn copy_buffer_to_uint8array<'js>(ctx: &Ctx<'js>, data: &[u8]) -> Result<TypedArray<'js, u8>> {
    TypedArray::new_copy(ctx.clone(), data)
}
