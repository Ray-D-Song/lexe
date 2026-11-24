use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, CString};
use std::ptr;
use std::rc::Rc;

use rquickjs::atom::PredefinedAtom;
use rquickjs::class::{Trace, Tracer};
use rquickjs::function::Opt;
use rquickjs::prelude::Rest;
use rquickjs::{Array, Class, Ctx, IntoJs, JsLifetime, Object, Result, Value};

use crate::database::{BusyScope, DatabaseInner};
use crate::error::{
    throw_illegal_constructor, throw_invalid_arg_type, throw_invalid_state, throw_sqlite_error,
};
use crate::ffi::{self, sqlite3_stmt, SQLITE_DONE, SQLITE_ROW};
use crate::path::{js_type_name, null_prototype_object};
use crate::value::{bind_js_value, is_plain_named_params, row_array, row_object};

pub struct StatementInner<'js> {
    pub database: Rc<DatabaseInner<'js>>,
    pub statement: Cell<*mut sqlite3_stmt>,
    pub origin_generation: u64,
    pub read_bigints: Cell<bool>,
    pub return_arrays: Cell<bool>,
    pub allow_bare_named: Cell<bool>,
    pub allow_unknown_named: Cell<bool>,
    bare_named_cache: RefCell<Option<HashMap<String, String>>>,
    iterator_active: Cell<bool>,
}

unsafe impl<'js> JsLifetime<'js> for StatementInner<'js> {
    type Changed<'to> = StatementInner<'to>;
}

impl<'js> StatementInner<'js> {
    pub fn new(database: Rc<DatabaseInner<'js>>, statement: *mut sqlite3_stmt) -> Self {
        let origin_generation = database.open_generation();
        Self {
            database,
            statement: Cell::new(statement),
            origin_generation,
            read_bigints: Cell::new(false),
            return_arrays: Cell::new(false),
            allow_bare_named: Cell::new(true),
            allow_unknown_named: Cell::new(false),
            bare_named_cache: RefCell::new(None),
            iterator_active: Cell::new(false),
        }
    }

    pub fn as_ptr(&self) -> *mut sqlite3_stmt {
        self.statement.get()
    }

    pub fn is_finalized(&self) -> bool {
        self.statement.get().is_null()
            || !self.database.is_open()
            || self.database.open_generation() != self.origin_generation
    }

    pub fn require_live(&self, ctx: &Ctx<'_>) -> Result<()> {
        if self.is_finalized() {
            Err(throw_invalid_state(ctx, "statement has been finalized"))
        } else {
            Ok(())
        }
    }

    pub fn finalize(&self) {
        let stmt = self.statement.replace(ptr::null_mut());
        if !stmt.is_null() {
            unsafe {
                ffi::sqlite3_finalize(stmt);
            }
        }
    }

    fn prepare_execution(&self, ctx: &Ctx<'js>) -> Result<()> {
        let stmt = self.as_ptr();
        let r = unsafe { ffi::sqlite3_reset(stmt) };
        self.database
            .check_sqlite_or_pending(ctx, self.database.connection(), r)?;
        let r = unsafe { ffi::sqlite3_clear_bindings(stmt) };
        self.database
            .check_sqlite_or_pending(ctx, self.database.connection(), r)?;
        Ok(())
    }

    fn reset_after_execution(&self, ctx: &Ctx<'js>) -> Result<()> {
        let stmt = self.as_ptr();
        let r = unsafe { ffi::sqlite3_reset(stmt) };
        self.database
            .check_sqlite_or_pending(ctx, self.database.connection(), r)?;
        Ok(())
    }

    fn bind_params(&self, ctx: &Ctx<'js>, args: &[Value<'js>]) -> Result<()> {
        self.prepare_execution(ctx)?;
        *self.bare_named_cache.borrow_mut() = None;

        let mut anon_start = 0usize;

        if let Some(arg0) = args.first() {
            if is_plain_named_params(arg0) {
                self.bind_named_params(ctx, arg0.as_object().unwrap())?;
                anon_start = 1;
            }
        }

        let stmt = self.as_ptr();
        let mut anon_idx = 1i32;
        for value in args.iter().skip(anon_start) {
            while {
                let name = unsafe { ffi::sqlite3_bind_parameter_name(stmt, anon_idx) };
                !(name.is_null() || unsafe { *name } == b'?' as c_char)
            } {
                anon_idx += 1;
            }
            bind_js_value(ctx, stmt, anon_idx, value.clone())?;
            anon_idx += 1;
        }

        Ok(())
    }

    fn bind_named_params(&self, ctx: &Ctx<'js>, named: &Object<'js>) -> Result<()> {
        let stmt = self.as_ptr();
        let param_count = unsafe { ffi::sqlite3_bind_parameter_count(stmt) };

        if self.allow_bare_named.get() && self.bare_named_cache.borrow().is_none() {
            let mut bare = HashMap::new();
            for i in 1..=param_count {
                let full_ptr = unsafe { ffi::sqlite3_bind_parameter_name(stmt, i) };
                if full_ptr.is_null() {
                    continue;
                }
                let full = unsafe { std::ffi::CStr::from_ptr(full_ptr) }
                    .to_string_lossy()
                    .into_owned();
                if full.is_empty() {
                    continue;
                }
                let bare_name = full[1..].to_string();
                if let Some(existing) = bare.get(&bare_name) {
                    return Err(throw_invalid_state(
                        ctx,
                        format!(
                            "Cannot create bare named parameter '{bare_name}' because of conflicting names '{existing}' and '{full}'."
                        ),
                    ));
                }
                bare.insert(bare_name, full);
            }
            *self.bare_named_cache.borrow_mut() = Some(bare);
        }

        for key in named.keys::<String>() {
            let key = key?;
            let index = self.resolve_named_index(ctx, stmt, &key)?;
            if index == 0 {
                if self.allow_unknown_named.get() {
                    continue;
                }
                return Err(throw_invalid_state(
                    ctx,
                    format!("Unknown named parameter '{key}'"),
                ));
            }

            if self.is_finalized() {
                return Err(throw_sqlite_error(ctx, ptr::null_mut()));
            }

            let value: Value = named.get(key.as_str())?;
            bind_js_value(ctx, stmt, index, value)?;
        }

        Ok(())
    }

    fn resolve_named_index(
        &self,
        ctx: &Ctx<'js>,
        stmt: *mut sqlite3_stmt,
        key: &str,
    ) -> Result<i32> {
        let key_c = CString::new(key)
            .map_err(|_| throw_invalid_arg_type(ctx, "key", "string without NUL", "string"))?;
        let mut index = unsafe { ffi::sqlite3_bind_parameter_index(stmt, key_c.as_ptr()) };
        if index == 0 && self.allow_bare_named.get() {
            if let Some(map) = self.bare_named_cache.borrow().as_ref() {
                if let Some(full) = map.get(key) {
                    let full_c = CString::new(full.as_str()).unwrap();
                    index = unsafe { ffi::sqlite3_bind_parameter_index(stmt, full_c.as_ptr()) };
                }
            }
        }
        Ok(index)
    }

    fn row_value(&self, ctx: &Ctx<'js>, stmt: *mut sqlite3_stmt) -> Result<Value<'js>> {
        if self.return_arrays.get() {
            return row_array(ctx, stmt, self.read_bigints.get()).map(|a| a.into_value());
        }
        row_object(ctx, stmt, self.read_bigints.get()).map(|o| o.into_value())
    }
}

#[rquickjs::class(rename = "StatementSync")]
#[derive(JsLifetime)]
pub struct StatementSync<'js> {
    #[qjs(skip_trace)]
    inner: Rc<StatementInner<'js>>,
}

impl<'js> StatementSync<'js> {
    pub fn from_inner(inner: Rc<StatementInner<'js>>) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &Rc<StatementInner<'js>> {
        &self.inner
    }
}

impl<'js> Trace<'js> for StatementSync<'js> {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> StatementSync<'js> {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'js>) -> Result<Self> {
        Err(throw_illegal_constructor(&ctx))
    }

    pub fn get(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Value<'js>> {
        self.inner.require_live(&ctx)?;
        let _busy = BusyScope::new(&self.inner.database);
        self.inner.bind_params(&ctx, &args)?;

        let stmt = self.inner.as_ptr();
        let r = unsafe { ffi::sqlite3_step(stmt) };

        if let Some(pending) = self.inner.database.take_pending_exception() {
            let _ = self.inner.reset_after_execution(&ctx);
            return Err(ctx.throw(pending));
        }

        if r == SQLITE_DONE {
            let _ = self.inner.reset_after_execution(&ctx);
            return Ok(Value::new_undefined(ctx));
        }

        if r != SQLITE_ROW {
            let _ = self.inner.reset_after_execution(&ctx);
            return Err(throw_sqlite_error(&ctx, self.inner.database.connection()));
        }

        let value = self.inner.row_value(&ctx, stmt)?;
        let _ = self.inner.reset_after_execution(&ctx);
        Ok(value)
    }

    pub fn all(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Value<'js>> {
        self.inner.require_live(&ctx)?;
        let _busy = BusyScope::new(&self.inner.database);
        self.inner.bind_params(&ctx, &args)?;

        let stmt = self.inner.as_ptr();
        let arr = Array::new(ctx.clone())?;
        let mut index = 0u32;

        loop {
            let r = unsafe { ffi::sqlite3_step(stmt) };

            if let Some(pending) = self.inner.database.take_pending_exception() {
                let _ = self.inner.reset_after_execution(&ctx);
                return Err(ctx.throw(pending));
            }

            if r == SQLITE_DONE {
                break;
            }
            if r != SQLITE_ROW {
                let _ = self.inner.reset_after_execution(&ctx);
                return Err(throw_sqlite_error(&ctx, self.inner.database.connection()));
            }

            let row = self.inner.row_value(&ctx, stmt)?;
            arr.set(index as usize, row)?;
            index += 1;
        }

        let _ = self.inner.reset_after_execution(&ctx);
        Ok(arr.into_value())
    }

    pub fn run(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
        self.inner.require_live(&ctx)?;
        let _busy = BusyScope::new(&self.inner.database);
        self.inner.bind_params(&ctx, &args)?;

        let stmt = self.inner.as_ptr();
        let r = unsafe { ffi::sqlite3_step(stmt) };

        if let Some(pending) = self.inner.database.take_pending_exception() {
            let _ = self.inner.reset_after_execution(&ctx);
            return Err(ctx.throw(pending));
        }

        if r != SQLITE_DONE && r != SQLITE_ROW {
            let _ = self.inner.reset_after_execution(&ctx);
            return Err(throw_sqlite_error(&ctx, self.inner.database.connection()));
        }

        let db = self.inner.database.connection();
        let changes = unsafe { ffi::sqlite3_changes64(db) };
        let last_id = unsafe { ffi::sqlite3_last_insert_rowid(db) };

        let _ = self.inner.reset_after_execution(&ctx);

        let result = null_prototype_object(&ctx)?;
        if self.inner.read_bigints.get() {
            result.set("changes", changes)?;
            result.set("lastInsertRowid", last_id)?;
        } else {
            result.set("changes", changes as f64)?;
            result.set("lastInsertRowid", last_id as f64)?;
        }
        Ok(result)
    }

    pub fn iterate(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> Result<Class<'js, StatementIterator<'js>>> {
        self.inner.require_live(&ctx)?;
        self.inner.bind_params(&ctx, &args)?;
        self.inner.iterator_active.set(true);

        Class::instance(
            ctx,
            StatementIterator {
                inner: self.inner.clone(),
                done: Cell::new(false),
            },
        )
    }

    pub fn columns(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        self.inner.require_live(&ctx)?;
        let stmt = self.inner.as_ptr();
        let count = unsafe { ffi::sqlite3_column_count(stmt) };
        let arr = Array::new(ctx.clone())?;

        for i in 0..count {
            let meta = null_prototype_object(&ctx)?;
            let column_ptr = unsafe { ffi::sqlite3_column_origin_name(stmt, i) };
            meta.set("column", optional_sqlite_str(&ctx, column_ptr)?)?;

            let db_ptr = unsafe { ffi::sqlite3_column_database_name(stmt, i) };
            meta.set("database", optional_sqlite_str(&ctx, db_ptr)?)?;

            let name_ptr = unsafe { ffi::sqlite3_column_name(stmt, i) };
            let name = if name_ptr.is_null() {
                String::new()
            } else {
                unsafe { std::ffi::CStr::from_ptr(name_ptr) }
                    .to_string_lossy()
                    .into_owned()
            };
            meta.set("name", name.as_str())?;

            let table_ptr = unsafe { ffi::sqlite3_column_table_name(stmt, i) };
            meta.set("table", optional_sqlite_str(&ctx, table_ptr)?)?;

            let type_ptr = unsafe { ffi::sqlite3_column_decltype(stmt, i) };
            meta.set("type", optional_sqlite_str(&ctx, type_ptr)?)?;

            arr.set(i as usize, meta)?;
        }

        Ok(arr)
    }

    pub fn set_read_big_ints(&self, ctx: Ctx<'js>, value: Value<'js>) -> Result<()> {
        self.inner.require_live(&ctx)?;
        let value = value.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(&ctx, "value", "boolean", &js_type_name(&value))
        })?;
        self.inner.read_bigints.set(value);
        Ok(())
    }

    pub fn set_return_arrays(&self, ctx: Ctx<'js>, value: Value<'js>) -> Result<()> {
        self.inner.require_live(&ctx)?;
        let value = value.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(&ctx, "value", "boolean", &js_type_name(&value))
        })?;
        self.inner.return_arrays.set(value);
        Ok(())
    }

    pub fn set_allow_bare_named_parameters(&self, ctx: Ctx<'js>, value: Value<'js>) -> Result<()> {
        self.inner.require_live(&ctx)?;
        let value = value.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(&ctx, "value", "boolean", &js_type_name(&value))
        })?;
        self.inner.allow_bare_named.set(value);
        *self.inner.bare_named_cache.borrow_mut() = None;
        Ok(())
    }

    pub fn set_allow_unknown_named_parameters(
        &self,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        self.inner.require_live(&ctx)?;
        let value = value.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(&ctx, "value", "boolean", &js_type_name(&value))
        })?;
        self.inner.allow_unknown_named.set(value);
        Ok(())
    }

    #[qjs(get, rename = "sourceSQL", enumerable)]
    pub fn source_sql(&self, ctx: Ctx<'js>) -> Result<String> {
        self.inner.require_live(&ctx)?;
        let ptr = unsafe { ffi::sqlite3_sql(self.inner.as_ptr()) };
        if ptr.is_null() {
            return Ok(String::new());
        }
        Ok(unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned())
    }

    #[qjs(get, rename = "expandedSQL", enumerable)]
    pub fn expanded_sql(&self, ctx: Ctx<'js>) -> Result<String> {
        self.inner.require_live(&ctx)?;
        let ptr = unsafe { ffi::sqlite3_expanded_sql(self.inner.as_ptr()) };
        if ptr.is_null() {
            return Err(throw_sqlite_error(&ctx, self.inner.database.connection()));
        }
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe {
            ffi::sqlite3_free(ptr.cast());
        }
        Ok(s)
    }
}

#[rquickjs::class(rename = "StatementSyncIterator")]
#[derive(JsLifetime)]
pub struct StatementIterator<'js> {
    inner: Rc<StatementInner<'js>>,
    done: Cell<bool>,
}

impl<'js> Trace<'js> for StatementIterator<'js> {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

#[rquickjs::methods]
impl<'js> StatementIterator<'js> {
    #[qjs(rename = PredefinedAtom::SymbolIterator)]
    pub fn symbol_iterator(this: rquickjs::function::This<Class<'js, Self>>) -> Class<'js, Self> {
        this.0
    }

    pub fn next(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        if self.inner.is_finalized() {
            return Err(throw_invalid_state(&ctx, "statement has been finalized"));
        }

        if self.done.get() {
            return iterator_result(&ctx, true);
        }

        let stmt = self.inner.as_ptr();
        let r = unsafe { ffi::sqlite3_step(stmt) };

        if let Some(pending) = self.inner.database.take_pending_exception() {
            self.finish(&ctx);
            return Err(ctx.throw(pending));
        }

        if r == SQLITE_DONE {
            self.finish(&ctx);
            return iterator_result(&ctx, true);
        }

        if r != SQLITE_ROW {
            self.finish(&ctx);
            return Err(throw_sqlite_error(&ctx, self.inner.database.connection()));
        }

        let value = self.inner.row_value(&ctx, stmt)?;
        iterator_result_with_value(&ctx, false, value)
    }

    #[qjs(rename = "return")]
    pub fn return_method(&self, ctx: Ctx<'js>, _value: Opt<Value<'js>>) -> Result<Object<'js>> {
        self.finish(&ctx);
        iterator_result(&ctx, true)
    }
}

impl<'js> StatementIterator<'js> {
    fn finish(&self, ctx: &Ctx<'js>) {
        self.done.set(true);
        self.inner.iterator_active.set(false);
        let _ = self.inner.reset_after_execution(ctx);
    }
}

impl<'js> Drop for StatementIterator<'js> {
    fn drop(&mut self) {
        if !self.done.get() {
            self.done.set(true);
            self.inner.iterator_active.set(false);
            let stmt = self.inner.as_ptr();
            if !stmt.is_null() {
                unsafe {
                    ffi::sqlite3_reset(stmt);
                }
            }
        }
    }
}

impl<'js> Drop for StatementSync<'js> {
    fn drop(&mut self) {
        self.inner.finalize();
    }
}

pub fn install_statement_iterator_prototype<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let iterator_ctor: Object = ctx.globals().get("Iterator")?;
    let iterator_proto: Object = iterator_ctor.get(PredefinedAtom::Prototype)?;

    if let Some(proto) = Class::<StatementIterator>::prototype(ctx)? {
        proto.set_prototype(Some(&iterator_proto))?;
    }

    Ok(())
}

fn iterator_result<'js>(ctx: &Ctx<'js>, done: bool) -> Result<Object<'js>> {
    let obj = null_prototype_object(ctx)?;
    obj.set("done", done)?;
    obj.set("value", Value::new_null(ctx.clone()))?;
    Ok(obj)
}

fn iterator_result_with_value<'js>(
    ctx: &Ctx<'js>,
    done: bool,
    value: Value<'js>,
) -> Result<Object<'js>> {
    let obj = null_prototype_object(ctx)?;
    obj.set("done", done)?;
    obj.set("value", value)?;
    Ok(obj)
}

fn optional_sqlite_str<'js>(ctx: &Ctx<'js>, ptr: *const c_char) -> Result<Value<'js>> {
    if ptr.is_null() {
        return Ok(Value::new_null(ctx.clone()));
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(ptr) };
    if cstr.to_bytes().is_empty() {
        return Ok(Value::new_null(ctx.clone()));
    }
    cstr.to_str()
        .map_err(|_| throw_invalid_arg_type(ctx, "value", "utf8", "invalid utf8"))?
        .into_js(ctx)
}
