use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::ptr::{self, NonNull};
use std::rc::{Rc, Weak};

use rquickjs::class::Tracer;
use rquickjs::prelude::Rest;
use rquickjs::qjs;
use rquickjs::{class::Trace, function::Function, Ctx, Error, Object, Persistent, Result, Value};

use crate::database::DatabaseInner;
use crate::error::{throw_invalid_arg_type, throw_sqlite_error, throw_type_error};
use crate::ffi::{
    self, sqlite3_context, sqlite3_value, SQLITE_DETERMINISTIC, SQLITE_DIRECTONLY, SQLITE_UTF8,
};
use crate::path::js_type_name;
use crate::value::{get_function_length, js_to_sqlite_result, sqlite_value_to_js};

fn record_callback_error<'js>(
    ctx: &Ctx<'js>,
    db: &DatabaseInner<'js>,
    sqlite_ctx: *mut sqlite3_context,
    error: rquickjs::Error,
) {
    if matches!(error, Error::Exception) {
        db.set_pending_exception(ctx.catch());
    }
    unsafe {
        ffi::sqlite3_result_error(sqlite_ctx, c"JavaScript callback failed".as_ptr(), -1);
    }
}

pub struct CallbackEntry<'js> {
    pub(crate) roots: Vec<Value<'js>>,
}

impl<'js> Trace<'js> for CallbackEntry<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        for root in &self.roots {
            tracer.mark(root);
        }
    }
}

struct ScalarUdf<'js> {
    db: Weak<DatabaseInner<'js>>,
    raw_ctx: NonNull<qjs::JSContext>,
    function: Persistent<Function<'static>>,
    use_bigint_args: bool,
}

impl<'js> ScalarUdf<'js> {
    unsafe extern "C" fn x_func(
        sqlite_ctx: *mut sqlite3_context,
        argc: c_int,
        argv: *mut *mut sqlite3_value,
    ) {
        let self_ptr = ffi::sqlite3_user_data(sqlite_ctx) as *mut ScalarUdf<'js>;
        if self_ptr.is_null() {
            return;
        }
        let self_ref = &*self_ptr;
        let ctx = Ctx::from_raw(self_ref.raw_ctx);
        let db = match self_ref.db.upgrade() {
            Some(db) => db,
            None => {
                ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                return;
            },
        };

        let func = match self_ref.function.clone().restore(&ctx) {
            Ok(f) => f,
            Err(_) => {
                ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                return;
            },
        };

        let mut js_args = Vec::with_capacity(argc as usize);
        for i in 0..argc {
            let value = *argv.add(i as usize);
            match sqlite_value_to_js(&ctx, value, self_ref.use_bigint_args) {
                Ok(v) => js_args.push(v),
                Err(Error::Exception) => {
                    db.set_pending_exception(ctx.catch());
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    return;
                },
                Err(_) => {
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    return;
                },
            }
        }

        match func.call((Rest(js_args),)) {
            Ok(result) => js_to_sqlite_result(&ctx, sqlite_ctx, result),
            Err(e) => record_callback_error(&ctx, &db, sqlite_ctx, e),
        }
    }

    unsafe extern "C" fn x_destroy(ptr: *mut c_void) {
        if !ptr.is_null() {
            drop(Box::from_raw(ptr as *mut ScalarUdf<'js>));
        }
    }
}

#[repr(C)]
struct AggregateCtx {
    id: u64,
    initialized: u8,
    is_window: u8,
}

struct AggregateUdf<'js> {
    db: Weak<DatabaseInner<'js>>,
    raw_ctx: NonNull<qjs::JSContext>,
    start: Persistent<Value<'static>>,
    step: Persistent<Function<'static>>,
    result: Option<Persistent<Function<'static>>>,
    inverse: Option<Persistent<Function<'static>>>,
    use_bigint_args: bool,
}

impl<'js> AggregateUdf<'js> {
    fn agg_state(sqlite_ctx: *mut sqlite3_context) -> *mut AggregateCtx {
        unsafe {
            ffi::sqlite3_aggregate_context(sqlite_ctx, std::mem::size_of::<AggregateCtx>() as c_int)
                as *mut AggregateCtx
        }
    }

    fn ensure_state(
        ctx: &Ctx<'js>,
        db: &DatabaseInner<'js>,
        sqlite_ctx: *mut sqlite3_context,
        start: &Persistent<Value<'static>>,
    ) -> Option<*mut AggregateCtx> {
        let state = Self::agg_state(sqlite_ctx);
        if state.is_null() {
            return None;
        }

        let state_ref = unsafe { &mut *state };
        if state_ref.initialized != 0 {
            return Some(state);
        }

        let mut start_v = match start.clone().restore(ctx) {
            Ok(v) => v,
            Err(_) => return None,
        };

        if let Some(fn_val) = start_v.as_function() {
            match fn_val.call(()) {
                Ok(v) => start_v = v,
                Err(Error::Exception) => {
                    db.set_pending_exception(ctx.catch());
                    return None;
                },
                Err(_) => return None,
            }
        }

        let id = db.alloc_aggregate_id();
        db.aggregate_values.borrow_mut().insert(id, start_v);

        state_ref.id = id;
        state_ref.initialized = 1;
        state_ref.is_window = 0;
        Some(state)
    }

    unsafe extern "C" fn x_step(
        sqlite_ctx: *mut sqlite3_context,
        argc: c_int,
        argv: *mut *mut sqlite3_value,
    ) {
        let self_ptr = ffi::sqlite3_user_data(sqlite_ctx) as *mut AggregateUdf<'js>;
        if self_ptr.is_null() {
            return;
        }
        let self_ref = &*self_ptr;
        let ctx = Ctx::from_raw(self_ref.raw_ctx);
        let db = match self_ref.db.upgrade() {
            Some(db) => db,
            None => {
                ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                return;
            },
        };

        if !Self::ensure_state(&ctx, &db, sqlite_ctx, &self_ref.start).is_some() {
            ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
            return;
        }

        let step_fn = match self_ref.step.clone().restore(&ctx) {
            Ok(f) => f,
            Err(_) => {
                ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                return;
            },
        };

        Self::invoke_step_with_start(
            &ctx,
            &db,
            sqlite_ctx,
            argc,
            argv,
            &step_fn,
            self_ref.use_bigint_args,
            &self_ref.start,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_step_with_start(
        ctx: &Ctx<'js>,
        db: &DatabaseInner<'js>,
        sqlite_ctx: *mut sqlite3_context,
        argc: c_int,
        argv: *mut *mut sqlite3_value,
        step_fn: &Function<'js>,
        use_bigint_args: bool,
        start: &Persistent<Value<'static>>,
    ) {
        if Self::ensure_state(ctx, db, sqlite_ctx, start).is_none() {
            unsafe {
                ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
            }
            return;
        }

        let state = Self::agg_state(sqlite_ctx);
        let id = unsafe { (*state).id };

        let acc_value = match db.aggregate_values.borrow().get(&id) {
            Some(v) => v.clone(),
            None => {
                unsafe {
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                }
                return;
            },
        };

        let mut js_args = vec![acc_value];
        for i in 0..argc {
            let value = unsafe { *argv.add(i as usize) };
            match sqlite_value_to_js(ctx, value, use_bigint_args) {
                Ok(v) => js_args.push(v),
                Err(Error::Exception) => {
                    db.set_pending_exception(ctx.catch());
                    unsafe {
                        ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    }
                    return;
                },
                Err(_) => {
                    unsafe {
                        ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    }
                    return;
                },
            }
        }

        match step_fn.call((Rest(js_args),)) {
            Ok(result) => {
                db.aggregate_values.borrow_mut().insert(id, result);
            },
            Err(e) => {
                record_callback_error(ctx, db, sqlite_ctx, e);
            },
        }
    }

    unsafe extern "C" fn x_inverse(
        sqlite_ctx: *mut sqlite3_context,
        argc: c_int,
        argv: *mut *mut sqlite3_value,
    ) {
        let self_ptr = ffi::sqlite3_user_data(sqlite_ctx) as *mut AggregateUdf<'js>;
        if self_ptr.is_null() {
            return;
        }
        let self_ref = &*self_ptr;
        let ctx = Ctx::from_raw(self_ref.raw_ctx);
        let db = match self_ref.db.upgrade() {
            Some(db) => db,
            None => {
                ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                return;
            },
        };

        let inverse = match self_ref.inverse.as_ref() {
            Some(p) => match p.clone().restore(&ctx) {
                Ok(f) => f,
                Err(_) => {
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    return;
                },
            },
            None => return,
        };

        Self::invoke_step_with_start(
            &ctx,
            &db,
            sqlite_ctx,
            argc,
            argv,
            &inverse,
            self_ref.use_bigint_args,
            &self_ref.start,
        );
    }

    unsafe fn x_value_base(sqlite_ctx: *mut sqlite3_context, is_final: bool) {
        let self_ptr = ffi::sqlite3_user_data(sqlite_ctx) as *mut AggregateUdf<'js>;
        if self_ptr.is_null() {
            return;
        }
        let self_ref = &*self_ptr;
        let ctx = Ctx::from_raw(self_ref.raw_ctx);
        let db = match self_ref.db.upgrade() {
            Some(db) => db,
            None => return,
        };

        if db.pending_exception.borrow().is_some() {
            if is_final {
                Self::destroy_state(&db, sqlite_ctx);
            }
            return;
        }

        let state = Self::agg_state(sqlite_ctx);
        if state.is_null() || unsafe { (*state).initialized } == 0 {
            if is_final {
                Self::destroy_state(&db, sqlite_ctx);
            }
            return;
        }

        let state_ref = unsafe { &mut *state };
        if !is_final {
            state_ref.is_window = 1;
        } else if state_ref.is_window != 0 {
            Self::destroy_state(&db, sqlite_ctx);
            return;
        }

        let id = state_ref.id;
        let result_value = if let Some(result_p) = &self_ref.result {
            let acc = match db.aggregate_values.borrow().get(&id) {
                Some(v) => v.clone(),
                None => {
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    if is_final {
                        Self::destroy_state(&db, sqlite_ctx);
                    }
                    return;
                },
            };
            let result_fn = match result_p.clone().restore(&ctx) {
                Ok(f) => f,
                Err(_) => {
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    if is_final {
                        Self::destroy_state(&db, sqlite_ctx);
                    }
                    return;
                },
            };
            match result_fn.call((acc,)) {
                Ok(v) => v,
                Err(Error::Exception) => {
                    db.set_pending_exception(ctx.catch());
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    if is_final {
                        Self::destroy_state(&db, sqlite_ctx);
                    }
                    return;
                },
                Err(_) => {
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    if is_final {
                        Self::destroy_state(&db, sqlite_ctx);
                    }
                    return;
                },
            }
        } else {
            match db.aggregate_values.borrow().get(&id) {
                Some(v) => v.clone(),
                None => {
                    ffi::sqlite3_result_error(sqlite_ctx, ptr::null(), 0);
                    if is_final {
                        Self::destroy_state(&db, sqlite_ctx);
                    }
                    return;
                },
            }
        };

        js_to_sqlite_result(&ctx, sqlite_ctx, result_value);

        if is_final {
            Self::destroy_state(&db, sqlite_ctx);
        }
    }

    fn destroy_state(db: &DatabaseInner<'js>, sqlite_ctx: *mut sqlite3_context) {
        let state = Self::agg_state(sqlite_ctx);
        if state.is_null() || unsafe { (*state).initialized } == 0 {
            return;
        }
        let id = unsafe { (*state).id };
        db.aggregate_values.borrow_mut().remove(&id);
        unsafe {
            (*state).initialized = 0;
        }
    }

    unsafe extern "C" fn x_value(sqlite_ctx: *mut sqlite3_context) {
        Self::x_value_base(sqlite_ctx, false);
    }

    unsafe extern "C" fn x_final(sqlite_ctx: *mut sqlite3_context) {
        Self::x_value_base(sqlite_ctx, true);
    }

    unsafe extern "C" fn x_destroy(ptr: *mut c_void) {
        if !ptr.is_null() {
            drop(Box::from_raw(ptr as *mut AggregateUdf<'js>));
        }
    }
}

fn read_function_options<'js>(
    ctx: &Ctx<'js>,
    options: &Object<'js>,
) -> Result<(bool, bool, bool, bool)> {
    let mut deterministic = false;
    let mut direct_only = false;
    let mut varargs = false;
    let mut use_bigint_args = false;

    if options.contains_key("deterministic")? {
        let v: Value = options.get("deterministic")?;
        deterministic = v.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(ctx, "options.deterministic", "boolean", &js_type_name(&v))
        })?;
    }
    if options.contains_key("directOnly")? {
        let v: Value = options.get("directOnly")?;
        direct_only = v.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(ctx, "options.directOnly", "boolean", &js_type_name(&v))
        })?;
    }
    if options.contains_key("varargs")? {
        let v: Value = options.get("varargs")?;
        varargs = v.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(ctx, "options.varargs", "boolean", &js_type_name(&v))
        })?;
    }
    if options.contains_key("useBigIntArguments")? {
        let v: Value = options.get("useBigIntArguments")?;
        use_bigint_args = v.as_bool().ok_or_else(|| {
            throw_invalid_arg_type(
                ctx,
                "options.useBigIntArguments",
                "boolean",
                &js_type_name(&v),
            )
        })?;
    }

    Ok((deterministic, direct_only, varargs, use_bigint_args))
}

pub fn register_scalar_function<'js>(
    ctx: &Ctx<'js>,
    db: &Rc<DatabaseInner<'js>>,
    name: &str,
    options: Object<'js>,
    func: Function<'js>,
) -> Result<()> {
    let (deterministic, direct_only, varargs, use_bigint_args) =
        read_function_options(ctx, &options)?;

    let c_name = CString::new(name).map_err(|_| Error::Unknown)?;
    let argc = if varargs {
        -1
    } else {
        get_function_length(ctx, &func)?
    };

    let mut text_rep = SQLITE_UTF8;
    if deterministic {
        text_rep |= SQLITE_DETERMINISTIC;
    }
    if direct_only {
        text_rep |= SQLITE_DIRECTONLY;
    }

    let entry = Rc::new(CallbackEntry {
        roots: vec![func.clone().into_value()],
    });
    db.register_callback(&entry);

    let udf = Box::new(ScalarUdf {
        db: Rc::downgrade(db),
        raw_ctx: ctx.as_raw(),
        function: Persistent::save(ctx, func),
        use_bigint_args,
    });
    let udf_ptr = Box::into_raw(udf);

    let r = unsafe {
        ffi::sqlite3_create_function_v2(
            db.connection(),
            c_name.as_ptr(),
            argc,
            text_rep,
            udf_ptr.cast(),
            Some(ScalarUdf::<'js>::x_func),
            None,
            None,
            Some(ScalarUdf::<'js>::x_destroy),
        )
    };

    if r != ffi::SQLITE_OK {
        return Err(throw_sqlite_error(ctx, db.connection()));
    }

    Ok(())
}

pub fn register_aggregate<'js>(
    ctx: &Ctx<'js>,
    db: &Rc<DatabaseInner<'js>>,
    name: &str,
    options: Object<'js>,
) -> Result<()> {
    let start_v: Value = options.get("start")?;
    if start_v.is_undefined() {
        return Err(throw_type_error(
            ctx,
            "The \"options.start\" argument must be a function or a primitive value.",
        ));
    }

    let step_v: Value = options.get("step")?;
    let step = step_v.as_function().cloned().ok_or_else(|| {
        throw_invalid_arg_type(ctx, "options.step", "function", &js_type_name(&step_v))
    })?;

    let result_v: Value = options.get("result")?;
    let result = if result_v.is_undefined() {
        None
    } else {
        Some(result_v.as_function().cloned().ok_or_else(|| {
            throw_invalid_arg_type(ctx, "options.result", "function", &js_type_name(&result_v))
        })?)
    };

    let inverse_v: Value = options.get("inverse")?;
    let inverse = if inverse_v.is_undefined() {
        None
    } else {
        Some(inverse_v.as_function().cloned().ok_or_else(|| {
            throw_invalid_arg_type(
                ctx,
                "options.inverse",
                "function",
                &js_type_name(&inverse_v),
            )
        })?)
    };

    let (deterministic, direct_only, varargs, use_bigint_args) =
        read_function_options(ctx, &options)?;

    let mut roots = vec![start_v.clone(), step.clone().into_value()];
    if let Some(r) = &result {
        roots.push(r.clone().into_value());
    }
    if let Some(i) = &inverse {
        roots.push(i.clone().into_value());
    }
    let entry = Rc::new(CallbackEntry { roots });
    db.register_callback(&entry);

    let c_name = CString::new(name).map_err(|_| Error::Unknown)?;
    let step_len = get_function_length(ctx, &step)?;
    let argc = if varargs {
        -1
    } else {
        step_len.saturating_sub(1).max(0)
    };

    let mut text_rep = SQLITE_UTF8;
    if deterministic {
        text_rep |= SQLITE_DETERMINISTIC;
    }
    if direct_only {
        text_rep |= SQLITE_DIRECTONLY;
    }

    let has_inverse = inverse.is_some();
    let udf = Box::new(AggregateUdf {
        db: Rc::downgrade(db),
        raw_ctx: ctx.as_raw(),
        start: Persistent::save(ctx, start_v),
        step: Persistent::save(ctx, step),
        result: result.map(|f| Persistent::save(ctx, f)),
        inverse: inverse.map(|f| Persistent::save(ctx, f)),
        use_bigint_args,
    });
    let udf_ptr = Box::into_raw(udf);

    let r = if has_inverse {
        unsafe {
            ffi::sqlite3_create_window_function(
                db.connection(),
                c_name.as_ptr(),
                argc,
                text_rep,
                udf_ptr.cast(),
                Some(AggregateUdf::<'js>::x_step),
                Some(AggregateUdf::<'js>::x_final),
                Some(AggregateUdf::<'js>::x_value),
                Some(AggregateUdf::<'js>::x_inverse),
                Some(AggregateUdf::<'js>::x_destroy),
            )
        }
    } else {
        unsafe {
            ffi::sqlite3_create_function_v2(
                db.connection(),
                c_name.as_ptr(),
                argc,
                text_rep,
                udf_ptr.cast(),
                None,
                Some(AggregateUdf::<'js>::x_step),
                Some(AggregateUdf::<'js>::x_final),
                Some(AggregateUdf::<'js>::x_destroy),
            )
        }
    };

    if r != ffi::SQLITE_OK {
        return Err(throw_sqlite_error(ctx, db.connection()));
    }

    Ok(())
}
