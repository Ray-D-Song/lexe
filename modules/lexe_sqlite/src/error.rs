use lexe_utils::primordials::{BasePrimordials, Primordial};
use rquickjs::{Ctx, Error, Object, Result, Value};

use crate::ffi::{self, sqlite3};

pub fn throw_type_error(ctx: &Ctx<'_>, message: impl AsRef<str>) -> Error {
    let primordials = BasePrimordials::get(ctx).expect("primordials");
    let err: Object = primordials
        .constructor_type_error
        .construct((message.as_ref().to_string(),))
        .expect("type error");
    ctx.throw(err.into_value())
}

pub fn throw_type_error_code(ctx: &Ctx<'_>, code: &str, message: impl AsRef<str>) -> Error {
    let primordials = BasePrimordials::get(ctx).expect("primordials");
    let err: Object = primordials
        .constructor_type_error
        .construct((message.as_ref().to_string(),))
        .expect("type error");
    err.set("code", code).expect("code");
    ctx.throw(err.into_value())
}

pub fn throw_invalid_arg_type(ctx: &Ctx<'_>, name: &str, expected: &str, actual: &str) -> Error {
    throw_type_error_code(
        ctx,
        "ERR_INVALID_ARG_TYPE",
        format!("The \"{name}\" argument must be of type {expected}. Received type {actual}"),
    )
}

pub fn throw_invalid_return_value(
    ctx: &Ctx<'_>,
    expected: &str,
    name: &str,
    actual: &str,
) -> Error {
    throw_type_error_code(
        ctx,
        "ERR_INVALID_RETURN_VALUE",
        format!("Expected {expected} to be returned from the \"{name}\", got {actual}"),
    )
}

pub fn throw_invalid_arg_value(ctx: &Ctx<'_>, message: impl AsRef<str>) -> Error {
    throw_type_error_code(ctx, "ERR_INVALID_ARG_VALUE", message)
}

pub fn throw_invalid_state(ctx: &Ctx<'_>, message: impl AsRef<str>) -> Error {
    let primordials = BasePrimordials::get(ctx).expect("primordials");
    let err: Object = primordials
        .constructor_error
        .construct((message.as_ref().to_string(),))
        .expect("error");
    err.set("code", "ERR_INVALID_STATE").expect("code");
    ctx.throw(err.into_value())
}

pub fn throw_out_of_range(ctx: &Ctx<'_>, message: impl AsRef<str>) -> Error {
    let primordials = BasePrimordials::get(ctx).expect("primordials");
    let err: Object = primordials
        .constructor_range_error
        .construct((message.as_ref().to_string(),))
        .expect("range error");
    err.set("code", "ERR_OUT_OF_RANGE").expect("code");
    ctx.throw(err.into_value())
}

pub fn throw_illegal_constructor(ctx: &Ctx<'_>) -> Error {
    throw_type_error_code(ctx, "ERR_ILLEGAL_CONSTRUCTOR", "Illegal constructor")
}

pub fn make_sqlite_error<'js>(ctx: &Ctx<'js>, db: *mut sqlite3) -> Result<Value<'js>> {
    unsafe {
        let errmsg = std::ffi::CStr::from_ptr(ffi::sqlite3_errmsg(db))
            .to_string_lossy()
            .into_owned();
        let errcode = ffi::sqlite3_extended_errcode(db);
        let errstr = std::ffi::CStr::from_ptr(ffi::sqlite3_errstr(errcode))
            .to_string_lossy()
            .into_owned();
        make_sqlite_error_parts(ctx, &errmsg, errcode, &errstr)
    }
}

pub fn make_sqlite_error_code<'js>(ctx: &Ctx<'js>, errcode: i32) -> Result<Value<'js>> {
    unsafe {
        let errstr = std::ffi::CStr::from_ptr(ffi::sqlite3_errstr(errcode))
            .to_string_lossy()
            .into_owned();
        make_sqlite_error_parts(ctx, &errstr, errcode, &errstr)
    }
}

fn make_sqlite_error_parts<'js>(
    ctx: &Ctx<'js>,
    message: &str,
    errcode: i32,
    errstr: &str,
) -> Result<Value<'js>> {
    let primordials = BasePrimordials::get(ctx)?;
    let err: Object = primordials
        .constructor_error
        .construct((message.to_string(),))?;
    err.set("code", "ERR_SQLITE_ERROR")?;
    err.set("errcode", errcode)?;
    err.set("errstr", errstr)?;
    Ok(err.into_value())
}

pub fn throw_sqlite_error(ctx: &Ctx<'_>, db: *mut sqlite3) -> Error {
    match make_sqlite_error(ctx, db) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

pub fn throw_load_extension_error(ctx: &Ctx<'_>, message: impl AsRef<str>) -> Error {
    let primordials = BasePrimordials::get(ctx).expect("primordials");
    let err: Object = primordials
        .constructor_error
        .construct((message.as_ref().to_string(),))
        .expect("error");
    err.set("code", "ERR_LOAD_SQLITE_EXTENSION").expect("code");
    ctx.throw(err.into_value())
}
