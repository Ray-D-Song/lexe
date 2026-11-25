use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr::{self, NonNull};
use std::rc::{Rc, Weak};

use crate::error::{
    make_sqlite_error, throw_invalid_arg_type, throw_invalid_arg_value, throw_invalid_return_value,
    throw_invalid_state, throw_load_extension_error, throw_out_of_range, throw_sqlite_error,
    throw_type_error,
};
use crate::ffi::{
    self, sqlite3, sqlite3_db_config_enable, SQLITE_DBCONFIG_DEFENSIVE, SQLITE_DBCONFIG_DQS_DDL,
    SQLITE_DBCONFIG_DQS_DML, SQLITE_DBCONFIG_ENABLE_FKEY, SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
    SQLITE_DENY, SQLITE_IGNORE, SQLITE_OK, SQLITE_OPEN_CREATE, SQLITE_OPEN_READONLY,
    SQLITE_OPEN_READWRITE, SQLITE_OPEN_URI,
};
use crate::function::{register_aggregate, register_scalar_function, CallbackEntry};
use crate::path::{js_type_name, parse_path};
use crate::session::{apply_changeset, create_session, SessionInner};
use crate::statement::{StatementInner, StatementSync};
use crate::tag_store::{create_tag_store, SQLTagStore};
use rquickjs::class::{Trace, Tracer};
use rquickjs::function::{Opt, This};
use rquickjs::object::Accessor;
use rquickjs::qjs;
use rquickjs::{Class, Ctx, Error, Function, IntoJs, Object, Persistent, Result, Value};

const LIMIT_DEFS: [(&str, i32); 11] = [
    ("length", ffi::SQLITE_LIMIT_LENGTH),
    ("sqlLength", ffi::SQLITE_LIMIT_SQL_LENGTH),
    ("column", ffi::SQLITE_LIMIT_COLUMN),
    ("exprDepth", ffi::SQLITE_LIMIT_EXPR_DEPTH),
    ("compoundSelect", ffi::SQLITE_LIMIT_COMPOUND_SELECT),
    ("vdbeOp", ffi::SQLITE_LIMIT_VDBE_OP),
    ("functionArg", ffi::SQLITE_LIMIT_FUNCTION_ARG),
    ("attach", ffi::SQLITE_LIMIT_ATTACHED),
    ("likePatternLength", ffi::SQLITE_LIMIT_LIKE_PATTERN_LENGTH),
    ("variableNumber", ffi::SQLITE_LIMIT_VARIABLE_NUMBER),
    ("triggerDepth", ffi::SQLITE_LIMIT_TRIGGER_DEPTH),
];

#[derive(Clone, Debug)]
pub struct OpenConfig {
    pub read_only: bool,
    pub enable_foreign_keys: bool,
    pub enable_dqs: bool,
    pub defensive: bool,
    pub timeout: i32,
    pub allow_extension: bool,
    pub limits: Vec<(i32, i32)>,
}

impl Default for OpenConfig {
    fn default() -> Self {
        Self {
            read_only: false,
            enable_foreign_keys: true,
            enable_dqs: false,
            defensive: true,
            timeout: 0,
            allow_extension: false,
            limits: Vec::new(),
        }
    }
}

pub struct AuthorizerEntry<'js> {
    db: Weak<DatabaseInner<'js>>,
    raw_ctx: NonNull<qjs::JSContext>,
    function: Persistent<Function<'static>>,
}

struct AuthorizingScope<'a, 'js> {
    inner: &'a DatabaseInner<'js>,
    prev: bool,
}

impl<'a, 'js> AuthorizingScope<'a, 'js> {
    fn new(inner: &'a DatabaseInner<'js>) -> Self {
        let prev = inner.authorizing.replace(true);
        Self { inner, prev }
    }
}

impl<'a, 'js> Drop for AuthorizingScope<'a, 'js> {
    fn drop(&mut self) {
        self.inner.authorizing.set(self.prev);
    }
}

impl<'js> AuthorizerEntry<'js> {
    unsafe fn optional_arg<'c>(ctx: &Ctx<'c>, ptr: *const c_char) -> Result<Value<'c>> {
        if ptr.is_null() {
            return Ok(Value::new_null(ctx.clone()));
        }
        let s = std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned();
        s.into_js(ctx)
    }

    unsafe extern "C" fn x_auth(
        user_data: *mut c_void,
        action_code: c_int,
        arg1: *const c_char,
        arg2: *const c_char,
        arg3: *const c_char,
        arg4: *const c_char,
    ) -> c_int {
        if user_data.is_null() {
            return SQLITE_DENY;
        }
        let entry = &*(user_data as *const AuthorizerEntry<'js>);
        let ctx = Ctx::from_raw(entry.raw_ctx);
        let Some(db) = entry.db.upgrade() else {
            return SQLITE_DENY;
        };
        let _scope = AuthorizingScope::new(&db);

        let Ok(func) = entry.function.clone().restore(&ctx) else {
            return SQLITE_DENY;
        };

        let (Ok(a1), Ok(a2), Ok(a3), Ok(a4)) = (
            Self::optional_arg(&ctx, arg1),
            Self::optional_arg(&ctx, arg2),
            Self::optional_arg(&ctx, arg3),
            Self::optional_arg(&ctx, arg4),
        ) else {
            return SQLITE_DENY;
        };

        match func.call::<_, Value<'js>>((action_code, a1, a2, a3, a4)) {
            Ok(ret) => {
                if let Some(code) = ret.as_int() {
                    if matches!(code, SQLITE_OK | SQLITE_DENY | SQLITE_IGNORE) {
                        return code;
                    }
                } else if let Some(n) = ret.as_number() {
                    if n == SQLITE_OK as f64 || n == SQLITE_DENY as f64 || n == SQLITE_IGNORE as f64
                    {
                        return n as c_int;
                    }
                }
                let _ = throw_invalid_return_value(
                    &ctx,
                    "SQLITE_OK, SQLITE_DENY, or SQLITE_IGNORE",
                    "authorizer",
                    &js_type_name(&ret),
                );
                db.set_pending_exception(ctx.catch());
                SQLITE_DENY
            },
            Err(Error::Exception) => {
                db.set_pending_exception(ctx.catch());
                SQLITE_DENY
            },
            Err(_) => SQLITE_DENY,
        }
    }
}

pub struct DatabaseInner<'js> {
    pub connection: Cell<*mut sqlite3>,
    pub config: OpenConfig,
    pub location: Vec<u8>,
    pub open_generation: Cell<u64>,
    pub enable_load_extension: Cell<bool>,
    pub busy_depth: Cell<u32>,
    pub authorizing: Cell<bool>,
    pub statements: RefCell<Vec<Weak<StatementInner<'js>>>>,
    pub sessions: RefCell<Vec<Weak<SessionInner<'js>>>>,
    pub callbacks: RefCell<Vec<Weak<CallbackEntry<'js>>>>,
    pub authorizer: RefCell<Option<Box<AuthorizerEntry<'js>>>>,
    pub aggregate_values: RefCell<HashMap<u64, Value<'js>>>,
    pub pending_exception: RefCell<Option<Value<'js>>>,
    pub next_aggregate_id: Cell<u64>,
}

unsafe impl<'js> rquickjs::JsLifetime<'js> for DatabaseInner<'js> {
    type Changed<'to> = DatabaseInner<'to>;
}

impl<'js> DatabaseInner<'js> {
    pub fn new(location: Vec<u8>, config: OpenConfig) -> Self {
        Self {
            connection: Cell::new(ptr::null_mut()),
            config,
            location,
            open_generation: Cell::new(0),
            enable_load_extension: Cell::new(false),
            busy_depth: Cell::new(0),
            authorizing: Cell::new(false),
            statements: RefCell::new(Vec::new()),
            sessions: RefCell::new(Vec::new()),
            callbacks: RefCell::new(Vec::new()),
            authorizer: RefCell::new(None),
            aggregate_values: RefCell::new(HashMap::new()),
            pending_exception: RefCell::new(None),
            next_aggregate_id: Cell::new(1),
        }
    }

    pub fn connection_ptr(&self) -> Result<*mut sqlite3> {
        let ptr = self.connection.get();
        if ptr.is_null() {
            Err(rquickjs::Error::Unknown)
        } else {
            Ok(ptr)
        }
    }

    pub fn connection(&self) -> *mut sqlite3 {
        self.connection.get()
    }

    pub fn is_open(&self) -> bool {
        !self.connection.get().is_null()
    }

    pub fn open_generation(&self) -> u64 {
        self.open_generation.get()
    }

    pub fn require_open(&self, ctx: &Ctx<'_>) -> Result<()> {
        if self.is_open() {
            Ok(())
        } else {
            Err(throw_invalid_state(ctx, "database is not open"))
        }
    }

    pub fn require_not_authorizing(&self, ctx: &Ctx<'_>) -> Result<()> {
        if self.authorizing.get() {
            Err(throw_invalid_state(
                ctx,
                "cannot call database method from authorizer",
            ))
        } else {
            Ok(())
        }
    }

    pub fn prepare_statement(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        sql: &str,
    ) -> Result<Rc<StatementInner<'js>>> {
        let c_sql = CString::new(sql)
            .map_err(|_| throw_invalid_arg_type(ctx, "sql", "string without NUL", "string"))?;

        let db = self.connection();
        let mut stmt: *mut ffi::sqlite3_stmt = ptr::null_mut();
        let r =
            unsafe { ffi::sqlite3_prepare_v2(db, c_sql.as_ptr(), -1, &mut stmt, ptr::null_mut()) };

        if let Some(pending) = self.take_pending_exception() {
            if !stmt.is_null() {
                unsafe {
                    ffi::sqlite3_finalize(stmt);
                }
            }
            return Err(ctx.throw(pending));
        }

        if r != SQLITE_OK {
            return Err(throw_sqlite_error(ctx, db));
        }

        let stmt_inner = Rc::new(StatementInner::new(self.clone(), stmt));
        self.register_statement(&stmt_inner);
        Ok(stmt_inner)
    }

    pub fn sweep_dead_refs(&self) {
        self.statements
            .borrow_mut()
            .retain(|w| w.strong_count() > 0);
        self.sessions.borrow_mut().retain(|w| w.strong_count() > 0);
        self.callbacks.borrow_mut().retain(|w| w.strong_count() > 0);
    }

    pub fn register_statement(&self, inner: &Rc<StatementInner<'js>>) {
        self.sweep_dead_refs();
        self.statements.borrow_mut().push(Rc::downgrade(inner));
    }

    pub fn register_session(&self, inner: &Rc<SessionInner<'js>>) {
        self.sweep_dead_refs();
        self.sessions.borrow_mut().push(Rc::downgrade(inner));
    }

    pub fn register_callback(&self, entry: &Rc<CallbackEntry<'js>>) {
        self.sweep_dead_refs();
        self.callbacks.borrow_mut().push(Rc::downgrade(entry));
    }

    pub fn take_pending_exception(&self) -> Option<Value<'js>> {
        self.pending_exception.borrow_mut().take()
    }

    pub fn set_pending_exception(&self, value: Value<'js>) {
        *self.pending_exception.borrow_mut() = Some(value);
    }

    pub fn clear_pending_exception(&self) {
        self.pending_exception.borrow_mut().take();
    }

    pub fn alloc_aggregate_id(&self) -> u64 {
        let id = self.next_aggregate_id.get();
        self.next_aggregate_id.set(id.saturating_add(1));
        id
    }

    pub fn open(&self, ctx: &Ctx<'js>) -> Result<()> {
        if self.is_open() {
            return Err(throw_invalid_state(ctx, "database is already open"));
        }

        let flags = if self.config.read_only {
            SQLITE_OPEN_READONLY | SQLITE_OPEN_URI
        } else {
            SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_URI
        };

        let path = CString::new(self.location.as_slice())
            .map_err(|_| throw_invalid_arg_value(ctx, "path cannot contain NUL bytes"))?;

        let mut db: *mut sqlite3 = ptr::null_mut();
        let r = unsafe { ffi::sqlite3_open_v2(path.as_ptr(), &mut db, flags, ptr::null()) };
        if r != SQLITE_OK {
            if !db.is_null() {
                let err = make_sqlite_error(ctx, db)?;
                unsafe {
                    ffi::sqlite3_close_v2(db);
                }
                return Err(ctx.throw(err));
            }
            let err = crate::error::make_sqlite_error_code(ctx, r)?;
            return Err(ctx.throw(err));
        }

        self.connection.set(db);
        self.open_generation
            .set(self.open_generation.get().saturating_add(1));
        self.enable_load_extension.set(self.config.allow_extension);

        let dqs = if self.config.enable_dqs { 1 } else { 0 };
        unsafe {
            sqlite3_db_config_enable(db, SQLITE_DBCONFIG_DQS_DML, dqs);
            sqlite3_db_config_enable(db, SQLITE_DBCONFIG_DQS_DDL, dqs);
        }

        let fkey = if self.config.enable_foreign_keys {
            1
        } else {
            0
        };
        let r = unsafe { sqlite3_db_config_enable(db, SQLITE_DBCONFIG_ENABLE_FKEY, fkey) };
        if r != SQLITE_OK {
            let err = make_sqlite_error(ctx, db)?;
            self.close_internal();
            return Err(ctx.throw(err));
        }

        let defensive = if self.config.defensive { 1 } else { 0 };
        let r = unsafe { sqlite3_db_config_enable(db, SQLITE_DBCONFIG_DEFENSIVE, defensive) };
        if r != SQLITE_OK {
            let err = make_sqlite_error(ctx, db)?;
            self.close_internal();
            return Err(ctx.throw(err));
        }

        if self.config.allow_extension {
            let r =
                unsafe { sqlite3_db_config_enable(db, SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION, 1) };
            if r != SQLITE_OK {
                let err = make_sqlite_error(ctx, db)?;
                self.close_internal();
                return Err(ctx.throw(err));
            }
        }

        if self.config.timeout != 0 {
            let r = unsafe { ffi::sqlite3_busy_timeout(db, self.config.timeout) };
            if r != SQLITE_OK {
                let err = make_sqlite_error(ctx, db)?;
                self.close_internal();
                return Err(ctx.throw(err));
            }
        }

        for &(limit_id, limit_val) in &self.config.limits {
            unsafe {
                ffi::sqlite3_limit(db, limit_id, limit_val);
            }
        }

        Ok(())
    }

    pub fn close_internal(&self) {
        if !self.is_open() {
            return;
        }

        for weak in self.statements.borrow().iter() {
            if let Some(stmt) = weak.upgrade() {
                stmt.finalize();
            }
        }
        self.statements.borrow_mut().clear();

        for weak in self.sessions.borrow().iter() {
            if let Some(session) = weak.upgrade() {
                session.close_handle();
            }
        }
        self.sessions.borrow_mut().clear();

        let db = self.connection.replace(ptr::null_mut());
        if !db.is_null() {
            unsafe {
                ffi::sqlite3_set_authorizer(db, None, ptr::null_mut());
                ffi::sqlite3_close_v2(db);
            }
        }

        self.authorizer.borrow_mut().take();
        self.callbacks.borrow_mut().clear();
        self.aggregate_values.borrow_mut().clear();
        self.pending_exception.borrow_mut().take();
        self.enable_load_extension.set(false);
    }

    pub fn check_sqlite_or_pending(
        &self,
        ctx: &Ctx<'js>,
        db: *mut sqlite3,
        code: i32,
    ) -> Result<()> {
        if let Some(pending) = self.take_pending_exception() {
            return Err(ctx.throw(pending));
        }
        if code != SQLITE_OK {
            return Err(throw_sqlite_error(ctx, db));
        }
        Ok(())
    }
}

pub struct BusyScope<'a, 'js> {
    inner: &'a DatabaseInner<'js>,
}

impl<'a, 'js> BusyScope<'a, 'js> {
    pub fn new(inner: &'a DatabaseInner<'js>) -> Self {
        inner
            .busy_depth
            .set(inner.busy_depth.get().saturating_add(1));
        Self { inner }
    }
}

impl<'a, 'js> Drop for BusyScope<'a, 'js> {
    fn drop(&mut self) {
        self.inner
            .busy_depth
            .set(self.inner.busy_depth.get().saturating_sub(1));
    }
}

#[rquickjs::class(rename = "DatabaseSync")]
#[derive(rquickjs::JsLifetime)]
pub struct DatabaseSync<'js> {
    #[qjs(skip_trace)]
    inner: Rc<DatabaseInner<'js>>,
}

impl<'js> Trace<'js> for DatabaseSync<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        for value in self.inner.aggregate_values.borrow().values() {
            tracer.mark(value);
        }
        if let Some(value) = self.inner.pending_exception.borrow().as_ref() {
            tracer.mark(value);
        }
    }
}

impl<'js> DatabaseSync<'js> {
    pub fn inner(&self) -> &Rc<DatabaseInner<'js>> {
        &self.inner
    }

    fn parse_constructor_options(
        ctx: &Ctx<'js>,
        options: Option<Value<'js>>,
    ) -> Result<(OpenConfig, bool)> {
        let mut config = OpenConfig::default();
        let mut open_immediately = true;

        let Some(options) = options else {
            return Ok((config, open_immediately));
        };

        let Some(obj) = options.as_object() else {
            return Err(throw_invalid_arg_type(
                ctx,
                "options",
                "object",
                &js_type_name(&options),
            ));
        };

        if let Some(v) = read_optional_bool(ctx, obj, "open")? {
            open_immediately = v;
        }
        if let Some(v) = read_optional_bool(ctx, obj, "readOnly")? {
            config.read_only = v;
        }
        if let Some(v) = read_optional_bool(ctx, obj, "enableForeignKeyConstraints")? {
            config.enable_foreign_keys = v;
        }
        if let Some(v) = read_optional_bool(ctx, obj, "enableDoubleQuotedStringLiterals")? {
            config.enable_dqs = v;
        }
        if let Some(v) = read_optional_bool(ctx, obj, "defensive")? {
            config.defensive = v;
        }
        if let Some(v) = read_optional_bool(ctx, obj, "allowExtension")? {
            config.allow_extension = v;
        }
        if obj.contains_key("timeout")? {
            let timeout = read_timeout(ctx, obj)?;
            config.timeout = timeout;
        }
        if obj.contains_key("limits")? {
            let limits_val: Value = obj.get("limits")?;
            if !limits_val.is_undefined() {
                let Some(limits_obj) = limits_val.as_object() else {
                    return Err(throw_invalid_arg_type(
                        ctx,
                        "options.limits",
                        "object",
                        &js_type_name(&limits_val),
                    ));
                };
                for &(name, limit_id) in &LIMIT_DEFS {
                    let val: Value = limits_obj.get(name)?;
                    if !val.is_undefined() {
                        config
                            .limits
                            .push((limit_id, parse_limit_value(ctx, name, &val)?));
                    }
                }
            }
        }

        Ok((config, open_immediately))
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> DatabaseSync<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, path: Value<'js>, options: Opt<Value<'js>>) -> Result<Self> {
        if path.is_undefined() {
            return Err(throw_type_error(
                &ctx,
                "Cannot open database because no path was specified",
            ));
        }

        let location = parse_path(&ctx, path)?;
        let (config, open_immediately) = Self::parse_constructor_options(&ctx, options.0)?;

        let inner = Rc::new(DatabaseInner::new(location, config));
        let this = Self {
            inner: inner.clone(),
        };

        if open_immediately {
            this.inner.open(&ctx)?;
        }

        Ok(this)
    }

    #[qjs(get, enumerable)]
    pub fn is_open(&self) -> bool {
        self.inner.is_open()
    }

    #[qjs(get, enumerable)]
    pub fn is_transaction(&self, ctx: Ctx<'js>) -> Result<bool> {
        self.inner.require_open(&ctx)?;
        let autocommit = unsafe { ffi::sqlite3_get_autocommit(self.inner.connection()) };
        Ok(autocommit == 0)
    }

    pub fn open(&self, ctx: Ctx<'js>) -> Result<()> {
        self.inner.require_not_authorizing(&ctx)?;
        self.inner.open(&ctx)
    }

    pub fn close(&self, ctx: Ctx<'js>) -> Result<()> {
        self.inner.require_not_authorizing(&ctx)?;
        if !self.inner.is_open() {
            return Err(throw_invalid_state(&ctx, "database is not open"));
        }
        self.inner.close_internal();
        Ok(())
    }

    pub fn exec(&self, ctx: Ctx<'js>, sql: Value<'js>) -> Result<()> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;
        let _busy = BusyScope::new(&self.inner);

        let sql = sql
            .as_string()
            .ok_or_else(|| throw_invalid_arg_type(&ctx, "sql", "string", &js_type_name(&sql)))?
            .to_string()?;

        let c_sql = CString::new(sql)
            .map_err(|_| throw_invalid_arg_type(&ctx, "sql", "string without NUL", "string"))?;

        let db = self.inner.connection();
        let r = unsafe {
            ffi::sqlite3_exec(db, c_sql.as_ptr(), None, ptr::null_mut(), ptr::null_mut())
        };
        self.inner.check_sqlite_or_pending(&ctx, db, r)?;
        Ok(())
    }

    pub fn prepare(
        &self,
        ctx: Ctx<'js>,
        sql: Value<'js>,
    ) -> Result<Class<'js, StatementSync<'js>>> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;
        let _busy = BusyScope::new(&self.inner);

        let sql = sql
            .as_string()
            .ok_or_else(|| throw_invalid_arg_type(&ctx, "sql", "string", &js_type_name(&sql)))?
            .to_string()?;

        let stmt_inner = self.inner.prepare_statement(&ctx, &sql)?;
        Class::instance(ctx, StatementSync::from_inner(stmt_inner))
    }

    pub fn create_tag_store(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        max_size: Opt<Value<'js>>,
    ) -> Result<Class<'js, SQLTagStore<'js>>> {
        create_tag_store(this, &ctx, max_size)
    }

    pub fn enable_defensive(&self, ctx: Ctx<'js>, active: Value<'js>) -> Result<()> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;

        let active = active.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(&ctx, "active", "boolean", &js_type_name(&active))
        })?;

        let db = self.inner.connection();
        let r = unsafe {
            sqlite3_db_config_enable(db, SQLITE_DBCONFIG_DEFENSIVE, if active { 1 } else { 0 })
        };
        if r != SQLITE_OK {
            return Err(throw_sqlite_error(&ctx, db));
        }
        Ok(())
    }

    pub fn set_authorizer(&self, ctx: Ctx<'js>, callback: Value<'js>) -> Result<()> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;

        let db = self.inner.connection();
        if callback.is_null() {
            let r = unsafe { ffi::sqlite3_set_authorizer(db, None, ptr::null_mut()) };
            self.inner.authorizer.borrow_mut().take();
            if r != SQLITE_OK {
                return Err(throw_sqlite_error(&ctx, db));
            }
            return Ok(());
        }

        let Some(func) = callback.as_function() else {
            return Err(throw_invalid_arg_type(
                &ctx,
                "callback",
                "function or null",
                &js_type_name(&callback),
            ));
        };

        let mut boxed = Box::new(AuthorizerEntry {
            db: Rc::downgrade(&self.inner),
            raw_ctx: ctx.as_raw(),
            function: Persistent::save(&ctx, func.clone()),
        });
        let ptr = (&mut *boxed as *mut AuthorizerEntry<'js>).cast::<c_void>();
        let r = unsafe { ffi::sqlite3_set_authorizer(db, Some(AuthorizerEntry::x_auth), ptr) };
        if r != SQLITE_OK {
            return Err(throw_sqlite_error(&ctx, db));
        }
        *self.inner.authorizer.borrow_mut() = Some(boxed);
        Ok(())
    }

    #[qjs(get, enumerable)]
    pub fn limits(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;

        let limits_obj = Object::new(ctx.clone())?;
        for &(limit_name, limit_id) in &LIMIT_DEFS {
            let get_inner = Rc::downgrade(&self.inner);
            let set_inner = Rc::downgrade(&self.inner);
            let get_fn = move |ctx: Ctx<'js>| -> Result<i32> {
                let Some(db) = get_inner.upgrade() else {
                    return Err(throw_invalid_state(&ctx, "database is not open"));
                };
                db.require_open(&ctx)?;
                db.require_not_authorizing(&ctx)?;
                Ok(unsafe { ffi::sqlite3_limit(db.connection(), limit_id, -1) })
            };
            let set_fn = move |ctx: Ctx<'js>, val: Value<'js>| -> Result<()> {
                let Some(db) = set_inner.upgrade() else {
                    return Err(throw_invalid_state(&ctx, "database is not open"));
                };
                db.require_open(&ctx)?;
                db.require_not_authorizing(&ctx)?;
                let parsed = parse_limit_value(&ctx, limit_name, &val)?;
                unsafe {
                    ffi::sqlite3_limit(db.connection(), limit_id, parsed);
                }
                Ok(())
            };
            limits_obj.prop(
                limit_name,
                Accessor::new(get_fn, set_fn).enumerable().configurable(),
            )?;
        }

        Ok(limits_obj)
    }

    pub fn location(&self, ctx: Ctx<'js>, db_name: Opt<String>) -> Result<Value<'js>> {
        self.inner.require_open(&ctx)?;
        let name = db_name.0.unwrap_or_else(|| "main".to_string());
        if name.is_empty() {
            return Ok(Value::new_null(ctx));
        }

        let c_name = CString::new(name)
            .map_err(|_| throw_invalid_arg_type(&ctx, "dbName", "string without NUL", "string"))?;

        let filename =
            unsafe { ffi::sqlite3_db_filename(self.inner.connection(), c_name.as_ptr()) };

        if filename.is_null() {
            return Ok(Value::new_null(ctx));
        }

        let s = unsafe { std::ffi::CStr::from_ptr(filename) };
        if s.to_bytes().is_empty() {
            return Ok(Value::new_null(ctx));
        }

        s.to_str()
            .map_err(|_| throw_invalid_arg_type(&ctx, "path", "utf8", "invalid utf8"))?
            .into_js(&ctx)
    }

    pub fn function(
        &self,
        ctx: Ctx<'js>,
        name: String,
        options: Opt<Object<'js>>,
        func: Opt<Function<'js>>,
    ) -> Result<()> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;
        let _busy = BusyScope::new(&self.inner);

        let (options, func) = resolve_function_args(&ctx, options, func)?;
        register_scalar_function(&ctx, &self.inner, &name, options, func)
    }

    pub fn aggregate(&self, ctx: Ctx<'js>, name: String, options: Object<'js>) -> Result<()> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;
        let _busy = BusyScope::new(&self.inner);
        register_aggregate(&ctx, &self.inner, &name, options)
    }

    pub fn create_session(&self, ctx: Ctx<'js>, options: Opt<Value<'js>>) -> Result<Value<'js>> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;
        create_session(&ctx, &self.inner, options)
    }

    pub fn apply_changeset(
        &self,
        ctx: Ctx<'js>,
        changeset: Value<'js>,
        options: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;
        let _busy = BusyScope::new(&self.inner);
        apply_changeset(&ctx, &self.inner, changeset, options)
    }

    pub fn enable_load_extension(&self, ctx: Ctx<'js>, allow: Value<'js>) -> Result<()> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;

        let allow = allow.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(&ctx, "allow", "boolean", &js_type_name(&allow))
        })?;

        if allow && !self.inner.config.allow_extension {
            return Err(throw_invalid_state(
                &ctx,
                "Cannot enable extension loading because it was disabled at database creation.",
            ));
        }

        if allow {
            let r = unsafe {
                sqlite3_db_config_enable(
                    self.inner.connection(),
                    SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
                    1,
                )
            };
            if r != SQLITE_OK {
                return Err(throw_sqlite_error(&ctx, self.inner.connection()));
            }
        } else {
            let r = unsafe {
                sqlite3_db_config_enable(
                    self.inner.connection(),
                    SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
                    0,
                )
            };
            if r != SQLITE_OK {
                return Err(throw_sqlite_error(&ctx, self.inner.connection()));
            }
        }

        self.inner.enable_load_extension.set(allow);
        Ok(())
    }

    pub fn load_extension(
        &self,
        ctx: Ctx<'js>,
        path: Value<'js>,
        entry_point: Opt<String>,
    ) -> Result<()> {
        self.inner.require_open(&ctx)?;
        self.inner.require_not_authorizing(&ctx)?;

        if !self.inner.config.allow_extension || !self.inner.enable_load_extension.get() {
            return Err(throw_invalid_state(
                &ctx,
                "extension loading is not allowed",
            ));
        }

        let path = path
            .as_string()
            .ok_or_else(|| throw_invalid_arg_type(&ctx, "path", "string", &js_type_name(&path)))?
            .to_string()?;

        let c_path = CString::new(path)
            .map_err(|_| throw_invalid_arg_type(&ctx, "path", "string without NUL", "string"))?;

        let entry_c = match entry_point.0 {
            Some(entry) => Some(CString::new(entry).map_err(|_| {
                throw_invalid_arg_type(&ctx, "entryPoint", "string without NUL", "string")
            })?),
            None => None,
        };

        let entry_ptr = entry_c.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());

        let mut errmsg: *mut std::os::raw::c_char = ptr::null_mut();
        let r = unsafe {
            ffi::sqlite3_load_extension(
                self.inner.connection(),
                c_path.as_ptr(),
                entry_ptr,
                &mut errmsg,
            )
        };

        if r != SQLITE_OK {
            let message = if !errmsg.is_null() {
                let msg = unsafe { std::ffi::CStr::from_ptr(errmsg) }
                    .to_string_lossy()
                    .into_owned();
                unsafe {
                    ffi::sqlite3_free(errmsg.cast());
                }
                msg
            } else {
                unsafe {
                    std::ffi::CStr::from_ptr(ffi::sqlite3_errstr(r))
                        .to_string_lossy()
                        .into_owned()
                }
            };
            return Err(throw_load_extension_error(&ctx, message));
        }

        Ok(())
    }

    /// `DatabaseSync.prototype[Symbol.dispose]` — swallow errors.
    pub fn dispose(&self, ctx: Ctx<'js>) -> Result<()> {
        if self.inner.is_open() {
            let _ = self.close(ctx);
        }
        Ok(())
    }
}

pub fn install_database_sync_extras<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let dispose_sym: rquickjs::Symbol = ctx.eval("Symbol.dispose")?;
    let sqlite_type_sym: rquickjs::Symbol = ctx.eval("Symbol.for('sqlite-type')")?;
    if let Some(proto) = Class::<DatabaseSync>::prototype(ctx)? {
        let func = Function::new(
            ctx.clone(),
            |this: rquickjs::function::This<Class<'js, DatabaseSync<'js>>>, ctx: Ctx<'js>| {
                this.0.borrow().dispose(ctx)
            },
        )?;
        proto.set(dispose_sym, func)?;
        proto.set(sqlite_type_sym, "node:sqlite")?;
    }
    Ok(())
}

fn read_optional_bool<'js>(ctx: &Ctx<'js>, obj: &Object<'js>, key: &str) -> Result<Option<bool>> {
    if !obj.contains_key(key)? {
        return Ok(None);
    }
    let value: Value = obj.get(key)?;
    value
        .as_bool()
        .ok_or_else(|| throw_invalid_arg_type(ctx, key, "boolean", &js_type_name(&value)))
        .map(Some)
}

fn parse_limit_value<'js>(ctx: &Ctx<'js>, name: &str, value: &Value<'js>) -> Result<i32> {
    let Some(n) = value.as_number() else {
        return Err(throw_invalid_arg_type(
            ctx,
            name,
            "number",
            &js_type_name(value),
        ));
    };
    if n == f64::INFINITY {
        return Ok(i32::MAX);
    }
    if n == f64::NEG_INFINITY || n < 0.0 || n > i32::MAX as f64 {
        return Err(throw_out_of_range(
            ctx,
            format!("The value of \"{name}\" is out of range"),
        ));
    }
    if n.is_nan() || n.trunc() != n {
        return Err(throw_invalid_arg_type(ctx, name, "integer", "number"));
    }
    Ok(n as i32)
}

fn read_timeout<'js>(ctx: &Ctx<'js>, obj: &Object<'js>) -> Result<i32> {
    let value: Value = obj.get("timeout")?;
    if value.is_int() {
        let v = value.as_int().unwrap();
        if v < 0 {
            return Err(throw_out_of_range(
                ctx,
                "timeout must be a non-negative integer",
            ));
        }
        return Ok(v);
    }
    if let Some(n) = value.as_number() {
        if n.is_finite() && n.trunc() == n && n >= 0.0 && n <= i32::MAX as f64 {
            return Ok(n as i32);
        }
    }
    Err(throw_invalid_arg_type(
        ctx,
        "options.timeout",
        "integer",
        &js_type_name(&value),
    ))
}

fn resolve_function_args<'js>(
    ctx: &Ctx<'js>,
    options: Opt<Object<'js>>,
    func: Opt<Function<'js>>,
) -> Result<(Object<'js>, Function<'js>)> {
    match (options.0, func.0) {
        (Some(opts), Some(f)) => Ok((opts, f)),
        (Some(val), None) => {
            if let Some(f) = val.as_function().cloned() {
                let empty = Object::new(ctx.clone())?;
                Ok((empty, f))
            } else {
                Err(throw_invalid_arg_type(ctx, "fn", "function", "object"))
            }
        },
        (None, Some(f)) => {
            let empty = Object::new(ctx.clone())?;
            Ok((empty, f))
        },
        (None, None) => Err(throw_type_error(ctx, "fn argument is required")),
    }
}

impl<'js> Drop for DatabaseSync<'js> {
    fn drop(&mut self) {
        self.inner.close_internal();
    }
}
