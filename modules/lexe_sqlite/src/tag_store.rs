use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use rquickjs::class::{Trace, Tracer};
use rquickjs::function::{Opt, This};
use rquickjs::prelude::Rest;
use rquickjs::{Class, Coerced, Ctx, FromJs, JsLifetime, Object, Result, Value};

use crate::database::{BusyScope, DatabaseInner, DatabaseSync};
use crate::error::{
    throw_illegal_constructor, throw_invalid_arg_type, throw_invalid_arg_value, throw_out_of_range,
};
use crate::path::js_type_name;
use crate::statement::{StatementInner, StatementIterator};

pub const DEFAULT_TAG_STORE_MAX_SIZE: usize = 1000;

struct CachedStatement<'js> {
    sql: String,
    stmt: Rc<StatementInner<'js>>,
}

#[rquickjs::class(rename = "SQLTagStore")]
pub struct SQLTagStore<'js> {
    db_class: Class<'js, DatabaseSync<'js>>,
    db: Rc<DatabaseInner<'js>>,
    capacity: usize,
    cache: RefCell<VecDeque<CachedStatement<'js>>>,
}

unsafe impl<'js> JsLifetime<'js> for SQLTagStore<'js> {
    type Changed<'to> = SQLTagStore<'to>;
}

impl<'js> Trace<'js> for SQLTagStore<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        tracer.mark(&self.db_class);
    }
}

impl<'js> Drop for SQLTagStore<'js> {
    fn drop(&mut self) {
        for entry in self.cache.borrow_mut().drain(..) {
            if !entry.stmt.is_iterator_active() {
                entry.stmt.finalize();
            }
        }
    }
}

impl<'js> SQLTagStore<'js> {
    pub fn create(
        ctx: &Ctx<'js>,
        db_class: Class<'js, DatabaseSync<'js>>,
        db: Rc<DatabaseInner<'js>>,
        max_size: Opt<Value<'js>>,
    ) -> Result<Class<'js, Self>> {
        let capacity = parse_max_size(ctx, max_size.0)?;
        Class::instance(
            ctx.clone(),
            Self {
                db_class,
                db,
                capacity,
                cache: RefCell::new(VecDeque::new()),
            },
        )
    }

    fn prune_finalized(&self) {
        self.cache
            .borrow_mut()
            .retain(|entry| !entry.stmt.is_finalized());
    }

    fn get_or_prepare(&self, ctx: &Ctx<'js>, sql: &str) -> Result<Rc<StatementInner<'js>>> {
        self.prune_finalized();

        {
            let mut cache = self.cache.borrow_mut();
            if let Some(pos) = cache
                .iter()
                .position(|entry| entry.sql == sql && !entry.stmt.is_iterator_active())
            {
                let entry = cache.remove(pos).expect("valid cache index");
                let stmt = entry.stmt.clone();
                cache.push_back(entry);
                return Ok(stmt);
            }
        }

        let stmt = self.db.prepare_statement(ctx, sql)?;

        {
            let mut cache = self.cache.borrow_mut();
            if cache.len() >= self.capacity {
                if let Some(evicted) = cache.pop_front() {
                    if !evicted.stmt.is_iterator_active() {
                        evicted.stmt.finalize();
                    }
                }
            }
            cache.push_back(CachedStatement {
                sql: sql.to_string(),
                stmt: stmt.clone(),
            });
        }

        Ok(stmt)
    }

    fn prepare_tagged(
        &self,
        ctx: &Ctx<'js>,
        strings: Value<'js>,
        values: &[Value<'js>],
    ) -> Result<Rc<StatementInner<'js>>> {
        self.db.require_open(ctx)?;
        self.db.require_not_authorizing(ctx)?;

        let sql = parse_tagged_sql(ctx, strings)?;
        let stmt = self.get_or_prepare(ctx, &sql)?;
        stmt.bind_positional_params(ctx, values)?;
        Ok(stmt)
    }
}

#[rquickjs::methods]
impl<'js> SQLTagStore<'js> {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'js>) -> Result<Self> {
        Err(throw_illegal_constructor(&ctx))
    }

    #[qjs(get, enumerable)]
    pub fn size(&self) -> usize {
        self.prune_finalized();
        self.cache.borrow().len()
    }

    #[qjs(get, enumerable)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[qjs(get, enumerable)]
    pub fn db(&self) -> Class<'js, DatabaseSync<'js>> {
        self.db_class.clone()
    }

    pub fn clear(&self) {
        for entry in self.cache.borrow_mut().drain(..) {
            if !entry.stmt.is_iterator_active() {
                entry.stmt.finalize();
            }
        }
    }

    pub fn get(
        &self,
        ctx: Ctx<'js>,
        strings: Value<'js>,
        values: Rest<Value<'js>>,
    ) -> Result<Value<'js>> {
        let _busy = BusyScope::new(&self.db);
        let stmt = self.prepare_tagged(&ctx, strings, &values)?;
        stmt.execute_get(&ctx)
    }

    pub fn all(
        &self,
        ctx: Ctx<'js>,
        strings: Value<'js>,
        values: Rest<Value<'js>>,
    ) -> Result<Value<'js>> {
        let _busy = BusyScope::new(&self.db);
        let stmt = self.prepare_tagged(&ctx, strings, &values)?;
        stmt.execute_all(&ctx)
    }

    pub fn run(
        &self,
        ctx: Ctx<'js>,
        strings: Value<'js>,
        values: Rest<Value<'js>>,
    ) -> Result<Object<'js>> {
        let _busy = BusyScope::new(&self.db);
        let stmt = self.prepare_tagged(&ctx, strings, &values)?;
        stmt.execute_run(&ctx)
    }

    pub fn iterate(
        &self,
        ctx: Ctx<'js>,
        strings: Value<'js>,
        values: Rest<Value<'js>>,
    ) -> Result<Class<'js, StatementIterator<'js>>> {
        let _busy = BusyScope::new(&self.db);
        let stmt = self.prepare_tagged(&ctx, strings, &values)?;
        stmt.execute_iterate(ctx)
    }
}

pub(crate) fn create_tag_store<'js>(
    this: This<Class<'js, DatabaseSync<'js>>>,
    ctx: &Ctx<'js>,
    max_size: Opt<Value<'js>>,
) -> Result<Class<'js, SQLTagStore<'js>>> {
    let db = this.borrow().inner().clone();
    db.require_open(ctx)?;
    db.require_not_authorizing(ctx)?;
    SQLTagStore::create(ctx, this.0, db, max_size)
}

fn parse_max_size(ctx: &Ctx<'_>, value: Option<Value<'_>>) -> Result<usize> {
    let Some(value) = value else {
        return Ok(DEFAULT_TAG_STORE_MAX_SIZE);
    };
    if value.is_undefined() {
        return Ok(DEFAULT_TAG_STORE_MAX_SIZE);
    }
    let Some(n) = value.as_number() else {
        return Err(throw_invalid_arg_type(
            ctx,
            "maxSize",
            "number",
            &js_type_name(&value),
        ));
    };
    if !n.is_finite() || n.trunc() != n {
        return Err(throw_out_of_range(
            ctx,
            "maxSize must be an integer between 1 and 4294967295",
        ));
    }
    if n < 1.0 || n > u32::MAX as f64 {
        return Err(throw_out_of_range(
            ctx,
            "The value of \"maxSize\" is out of range",
        ));
    }
    Ok(n as usize)
}

fn parse_tagged_sql<'js>(ctx: &Ctx<'js>, strings: Value<'js>) -> Result<String> {
    let Some(arr) = strings.as_array() else {
        return Err(throw_invalid_arg_type(
            ctx,
            "strings",
            "Array",
            &js_type_name(&strings),
        ));
    };
    let len = arr.len();
    if len == 0 {
        return Err(throw_invalid_arg_value(
            ctx,
            "Template literal strings array cannot be empty",
        ));
    }
    let mut parts = Vec::with_capacity(len);
    for i in 0..len {
        let item: Value<'js> = arr.get(i)?;
        let s: String = Coerced::<String>::from_js(ctx, item)?.0;
        parts.push(s);
    }
    Ok(parts.join("?"))
}
