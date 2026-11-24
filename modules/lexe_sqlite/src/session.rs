use std::cell::Cell;
use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::ptr::{self, NonNull};
use std::rc::{Rc, Weak};

use rquickjs::class::Trace;
use rquickjs::function::Opt;
use rquickjs::qjs;
use rquickjs::{
    Class, Ctx, Error, Function, IntoJs, JsLifetime, Object, Persistent, Result, TypedArray, Value,
};

use crate::database::DatabaseInner;
use crate::error::{
    throw_illegal_constructor, throw_invalid_arg_type, throw_invalid_state, throw_sqlite_error,
    throw_type_error_code,
};
use crate::ffi::{self, sqlite3_session, SQLITE_ABORT, SQLITE_CHANGESET_ABORT, SQLITE_OK};
use crate::path::js_type_name;
use crate::value::{checked_sqlite_len, sqlite_bytes};

pub struct SessionInner<'js> {
    pub database: Weak<DatabaseInner<'js>>,
    pub session: Cell<*mut sqlite3_session>,
}

impl<'js> SessionInner<'js> {
    pub fn close_handle(&self) {
        let handle = self.session.replace(ptr::null_mut());
        if !handle.is_null() {
            unsafe {
                ffi::sqlite3session_delete(handle);
            }
        }
    }
}

unsafe impl<'js> JsLifetime<'js> for SessionInner<'js> {
    type Changed<'to> = SessionInner<'to>;
}

#[rquickjs::class(rename = "Session")]
#[derive(JsLifetime)]
pub struct Session<'js> {
    #[qjs(skip_trace)]
    inner: Rc<SessionInner<'js>>,
}

impl<'js> Trace<'js> for Session<'js> {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

fn has_own_property(ctx: &Ctx<'_>, object: &Object<'_>, key: &str) -> Result<bool> {
    let key_c = CString::new(key).map_err(|_| Error::Unknown)?;
    let atom = unsafe { qjs::JS_NewAtom(ctx.as_raw().as_ptr(), key_c.as_ptr()) };
    if atom == qjs::JS_ATOM_NULL {
        return Err(Error::Unknown);
    }

    let mut desc = qjs::JSPropertyDescriptor {
        flags: 0,
        value: qjs::JS_UNDEFINED,
        getter: qjs::JS_UNDEFINED,
        setter: qjs::JS_UNDEFINED,
    };
    let rc =
        unsafe { qjs::JS_GetOwnProperty(ctx.as_raw().as_ptr(), &mut desc, object.as_raw(), atom) };
    unsafe {
        qjs::JS_FreeAtom(ctx.as_raw().as_ptr(), atom);
    }
    if rc < 0 {
        return Err(Error::Exception);
    }
    if rc > 0 {
        unsafe {
            qjs::JS_FreeValue(ctx.as_raw().as_ptr(), desc.value);
            if !qjs::JS_IsUndefined(desc.getter) {
                qjs::JS_FreeValue(ctx.as_raw().as_ptr(), desc.getter);
            }
            if !qjs::JS_IsUndefined(desc.setter) {
                qjs::JS_FreeValue(ctx.as_raw().as_ptr(), desc.setter);
            }
        }
    }
    Ok(rc > 0)
}

fn js_to_bool(ctx: &Ctx<'_>, value: &Value<'_>) -> bool {
    unsafe { qjs::JS_ToBool(ctx.as_raw().as_ptr(), value.as_raw()) != 0 }
}

fn require_string<'js>(ctx: &Ctx<'js>, value: Value<'js>, name: &str) -> Result<String> {
    if value.is_null() || value.is_undefined() {
        return Err(throw_invalid_arg_type(
            ctx,
            name,
            "string",
            if value.is_null() { "null" } else { "undefined" },
        ));
    }
    value
        .as_string()
        .ok_or_else(|| throw_invalid_arg_type(ctx, name, "string", &js_type_name(&value)))
        .and_then(|s| s.to_string().map_err(|_| Error::Unknown))
}

pub fn create_session<'js>(
    ctx: &Ctx<'js>,
    db: &Rc<DatabaseInner<'js>>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let mut db_name = "main".to_string();
    let mut table: Option<String> = None;

    if let Some(options) = options.0 {
        let object = options.as_object().ok_or_else(|| {
            throw_type_error_code(
                ctx,
                "ERR_INVALID_ARG_TYPE",
                "The \"options\" argument must be an object.",
            )
        })?;

        if has_own_property(ctx, object, "db")? {
            let value: Value = object.get("db")?;
            db_name = require_string(ctx, value, "options.db")?;
        }

        if has_own_property(ctx, object, "table")? {
            let value: Value = object.get("table")?;
            table = Some(require_string(ctx, value, "options.table")?);
        }
    }

    let c_db = CString::new(db_name).map_err(|_| rquickjs::Error::Unknown)?;
    let mut session_ptr: *mut sqlite3_session = ptr::null_mut();
    let r = unsafe { ffi::sqlite3session_create(db.connection(), c_db.as_ptr(), &mut session_ptr) };
    if r != SQLITE_OK {
        return Err(throw_sqlite_error(ctx, db.connection()));
    }

    let table_c = if let Some(table) = &table {
        Some(CString::new(table.as_str()).map_err(|_| rquickjs::Error::Unknown)?)
    } else {
        None
    };

    let attach_r = unsafe {
        ffi::sqlite3session_attach(
            session_ptr,
            table_c.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
        )
    };
    if attach_r != SQLITE_OK {
        unsafe {
            ffi::sqlite3session_delete(session_ptr);
        }
        return Err(throw_sqlite_error(ctx, db.connection()));
    }

    let inner = Rc::new(SessionInner {
        database: Rc::downgrade(db),
        session: Cell::new(session_ptr),
    });
    db.register_session(&inner);

    let instance = Class::instance(ctx.clone(), Session { inner })?;
    Ok(instance.into_value())
}

fn changeset_bytes<'js>(
    ctx: &Ctx<'js>,
    session_ptr: *mut sqlite3_session,
    db: &DatabaseInner<'js>,
    patchset: bool,
) -> Result<Value<'js>> {
    let mut nbytes = 0;
    let mut buf: *mut c_void = ptr::null_mut();
    let r = unsafe {
        if patchset {
            ffi::sqlite3session_patchset(session_ptr, &mut nbytes, &mut buf)
        } else {
            ffi::sqlite3session_changeset(session_ptr, &mut nbytes, &mut buf)
        }
    };
    if r != SQLITE_OK {
        return Err(throw_sqlite_error(ctx, db.connection()));
    }

    let len = nbytes as usize;
    let owned = sqlite_bytes(buf as *const u8, len)
        .map_err(|_| throw_sqlite_error(ctx, db.connection()))?
        .to_vec();
    if !buf.is_null() {
        unsafe {
            ffi::sqlite3_free(buf);
        }
    }
    let view = TypedArray::<u8>::new(ctx.clone(), owned)?;
    Ok(view.into_value())
}

fn value_to_i32(value: &Value<'_>) -> Option<i32> {
    value.as_int()
}

struct ChangesetCallbackContext<'js> {
    raw_ctx: NonNull<qjs::JSContext>,
    db: Weak<DatabaseInner<'js>>,
    filter: Option<Persistent<Function<'static>>>,
    on_conflict: Option<Persistent<Function<'static>>>,
}

impl<'js> ChangesetCallbackContext<'js> {
    unsafe extern "C" fn x_filter(
        ctx_ptr: *mut c_void,
        table_name: *const std::os::raw::c_char,
    ) -> c_int {
        let callback_ctx = &*(ctx_ptr as *const ChangesetCallbackContext<'js>);
        let ctx = Ctx::from_raw(callback_ctx.raw_ctx);
        let db = match callback_ctx.db.upgrade() {
            Some(db) => db,
            None => return 0,
        };

        if db.pending_exception.borrow().is_some() {
            return 0;
        }

        let filter = match callback_ctx.filter.as_ref() {
            Some(p) => match p.clone().restore(&ctx) {
                Ok(f) => f,
                Err(_) => return 0,
            },
            None => return 1,
        };

        let table = if table_name.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(table_name)
                .to_string_lossy()
                .into_owned()
        };

        match filter.call::<(String,), Value>((table,)) {
            Ok(v) => i32::from(js_to_bool(&ctx, &v)),
            Err(Error::Exception) => {
                db.set_pending_exception(ctx.catch());
                0
            },
            Err(_) => 0,
        }
    }

    unsafe extern "C" fn x_conflict(
        ctx_ptr: *mut c_void,
        conflict_type: c_int,
        _iterator: *mut ffi::sqlite3_changeset_iter,
    ) -> c_int {
        let callback_ctx = &*(ctx_ptr as *const ChangesetCallbackContext<'js>);
        let ctx = Ctx::from_raw(callback_ctx.raw_ctx);
        let db = match callback_ctx.db.upgrade() {
            Some(db) => db,
            None => return SQLITE_CHANGESET_ABORT,
        };

        if db.pending_exception.borrow().is_some() {
            return SQLITE_CHANGESET_ABORT;
        }

        let on_conflict = match callback_ctx.on_conflict.as_ref() {
            Some(p) => match p.clone().restore(&ctx) {
                Ok(f) => f,
                Err(_) => return SQLITE_CHANGESET_ABORT,
            },
            None => return SQLITE_CHANGESET_ABORT,
        };

        match on_conflict.call::<(i32,), Value>((conflict_type,)) {
            Ok(v) => value_to_i32(&v).unwrap_or(-1),
            Err(Error::Exception) => {
                db.set_pending_exception(ctx.catch());
                SQLITE_CHANGESET_ABORT
            },
            Err(_) => -1,
        }
    }

    unsafe extern "C" fn x_conflict_default(
        _ctx_ptr: *mut c_void,
        _conflict_type: c_int,
        _iterator: *mut ffi::sqlite3_changeset_iter,
    ) -> c_int {
        SQLITE_CHANGESET_ABORT
    }
}

pub fn apply_changeset<'js>(
    ctx: &Ctx<'js>,
    db: &Rc<DatabaseInner<'js>>,
    changeset: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let typed = TypedArray::<u8>::from_value(changeset)
        .map_err(|_| throw_invalid_arg_type(ctx, "changeset", "Uint8Array", "invalid"))?;
    let data = typed
        .as_bytes()
        .ok_or_else(|| throw_invalid_arg_type(ctx, "changeset", "Uint8Array", "detached"))?;

    let mut filter_fn: Option<Function<'js>> = None;
    let mut on_conflict_fn: Option<Function<'js>> = None;

    if let Some(options) = options.0 {
        if options.is_null() {
            return Err(throw_type_error_code(
                ctx,
                "ERR_INVALID_ARG_TYPE",
                "The \"options\" argument must be an object.",
            ));
        }
        let object = options.as_object().ok_or_else(|| {
            throw_type_error_code(
                ctx,
                "ERR_INVALID_ARG_TYPE",
                "The \"options\" argument must be an object.",
            )
        })?;

        if has_own_property(ctx, object, "filter")? {
            let value: Value = object.get("filter")?;
            if value.is_null() || value.is_undefined() {
                return Err(throw_invalid_arg_type(
                    ctx,
                    "options.filter",
                    "function",
                    if value.is_null() { "null" } else { "undefined" },
                ));
            }
            filter_fn = Some(value.as_function().cloned().ok_or_else(|| {
                throw_invalid_arg_type(ctx, "options.filter", "function", &js_type_name(&value))
            })?);
        }

        let on_conflict_value: Value = object.get("onConflict")?;
        if !on_conflict_value.is_undefined() {
            on_conflict_fn = Some(on_conflict_value.as_function().cloned().ok_or_else(|| {
                throw_invalid_arg_type(
                    ctx,
                    "options.onConflict",
                    "function",
                    &js_type_name(&on_conflict_value),
                )
            })?);
        }
    }

    let mut callback_ctx = ChangesetCallbackContext {
        raw_ctx: ctx.as_raw(),
        db: Rc::downgrade(db),
        filter: filter_fn.map(|f| Persistent::save(ctx, f)),
        on_conflict: on_conflict_fn.map(|f| Persistent::save(ctx, f)),
    };

    let x_filter = if callback_ctx.filter.is_some() {
        Some(
            ChangesetCallbackContext::<'js>::x_filter
                as unsafe extern "C" fn(*mut c_void, *const std::os::raw::c_char) -> c_int,
        )
    } else {
        None
    };

    let x_conflict = if callback_ctx.on_conflict.is_some() {
        Some(
            ChangesetCallbackContext::<'js>::x_conflict
                as unsafe extern "C" fn(
                    *mut c_void,
                    c_int,
                    *mut ffi::sqlite3_changeset_iter,
                ) -> c_int,
        )
    } else {
        Some(
            ChangesetCallbackContext::<'js>::x_conflict_default
                as unsafe extern "C" fn(
                    *mut c_void,
                    c_int,
                    *mut ffi::sqlite3_changeset_iter,
                ) -> c_int,
        )
    };

    let changeset_len = checked_sqlite_len(ctx, data.len())?;

    let r = unsafe {
        ffi::sqlite3changeset_apply(
            db.connection(),
            changeset_len,
            data.as_ptr().cast::<c_void>().cast_mut(),
            x_filter,
            x_conflict,
            &mut callback_ctx as *mut ChangesetCallbackContext<'js> as *mut c_void,
        )
    };

    if let Some(exception) = db.take_pending_exception() {
        return Err(ctx.throw(exception));
    }

    match r {
        SQLITE_OK => true.into_js(ctx),
        SQLITE_ABORT => false.into_js(ctx),
        code => Err(ctx.throw(crate::error::make_sqlite_error_code(ctx, code)?)),
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Session<'js> {
    #[qjs(constructor)]
    pub fn illegal_constructor(ctx: Ctx<'js>) -> Result<Self> {
        Err(throw_illegal_constructor(&ctx))
    }

    pub fn changeset(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let db = self
            .inner
            .database
            .upgrade()
            .ok_or_else(|| throw_invalid_state(&ctx, "database is not open"))?;
        if !db.is_open() {
            return Err(throw_invalid_state(&ctx, "database is not open"));
        }
        if self.inner.session.get().is_null() {
            return Err(throw_invalid_state(&ctx, "session is not open"));
        }
        changeset_bytes(&ctx, self.inner.session.get(), &db, false)
    }

    pub fn patchset(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let db = self
            .inner
            .database
            .upgrade()
            .ok_or_else(|| throw_invalid_state(&ctx, "database is not open"))?;
        if !db.is_open() {
            return Err(throw_invalid_state(&ctx, "database is not open"));
        }
        if self.inner.session.get().is_null() {
            return Err(throw_invalid_state(&ctx, "session is not open"));
        }
        changeset_bytes(&ctx, self.inner.session.get(), &db, true)
    }

    pub fn close(&self, ctx: Ctx<'js>) -> Result<()> {
        let db = self
            .inner
            .database
            .upgrade()
            .ok_or_else(|| throw_invalid_state(&ctx, "database is not open"))?;
        if !db.is_open() {
            return Err(throw_invalid_state(&ctx, "database is not open"));
        }
        if self.inner.session.get().is_null() {
            return Err(throw_invalid_state(&ctx, "session is not open"));
        }
        self.inner.close_handle();
        Ok(())
    }
}

impl<'js> Drop for Session<'js> {
    fn drop(&mut self) {
        self.inner.close_handle();
    }
}
