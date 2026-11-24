use std::ffi::CString;
use std::os::raw::c_int;
use std::ptr;
use std::rc::Rc;

use rquickjs::{
    class::Class, function::Opt, Ctx, Error, Function, Object, Persistent, Promise, Result, Value,
};

use crate::database::{DatabaseInner, DatabaseSync};
use crate::error::{
    make_sqlite_error, make_sqlite_error_code, throw_invalid_arg_type, throw_invalid_state,
};
use crate::ffi::{self, sqlite3, SQLITE_OPEN_CREATE, SQLITE_OPEN_READWRITE, SQLITE_OPEN_URI};
use crate::native::{Backup, Connection};
use crate::path::{js_type_name, parse_path};

struct BackupState<'js> {
    #[allow(dead_code)]
    inner: Rc<DatabaseInner<'js>>,
    dest: Connection,
    backup: Backup,
    pages: c_int,
    progress: Option<Persistent<Function<'static>>>,
}

impl<'js> BackupState<'js> {
    fn cleanup(&mut self) {
        self.backup = Backup::new(ptr::null_mut());
    }
}

pub fn backup<'js>(
    ctx: Ctx<'js>,
    source_db: Object<'js>,
    path: Value<'js>,
    options: Opt<Object<'js>>,
) -> Result<Promise<'js>> {
    let db_class = Class::<DatabaseSync>::from_object(&source_db)
        .ok_or_else(|| throw_invalid_arg_type(&ctx, "sourceDb", "DatabaseSync", "object"))?;
    let db = db_class.borrow();
    let inner = db.inner();

    if !inner.is_open() {
        return Err(throw_invalid_state(&ctx, "database is not open"));
    }

    let dest_path = parse_path(&ctx, path)?;
    let mut rate = 100;
    let mut source_name = "main".to_string();
    let mut dest_name = "main".to_string();
    let mut progress_fn: Option<Function<'js>> = None;

    if let Some(opts) = options.0 {
        let v: Value = opts.get("rate")?;
        if !v.is_undefined() {
            if let Some(n) = v.as_int() {
                rate = n;
            } else {
                return Err(throw_invalid_arg_type(
                    &ctx,
                    "options.rate",
                    "integer",
                    &js_type_name(&v),
                ));
            }
        }

        let v: Value = opts.get("source")?;
        if !v.is_undefined() {
            if let Some(s) = v.as_string() {
                source_name = s.to_string()?;
            } else {
                return Err(throw_invalid_arg_type(
                    &ctx,
                    "options.source",
                    "string",
                    &js_type_name(&v),
                ));
            }
        }

        let v: Value = opts.get("target")?;
        if !v.is_undefined() {
            if let Some(s) = v.as_string() {
                dest_name = s.to_string()?;
            } else {
                return Err(throw_invalid_arg_type(
                    &ctx,
                    "options.target",
                    "string",
                    &js_type_name(&v),
                ));
            }
        }

        let v: Value = opts.get("progress")?;
        if !v.is_undefined() {
            if let Some(f) = v.as_function() {
                progress_fn = Some(f.clone());
            } else {
                return Err(throw_invalid_arg_type(
                    &ctx,
                    "options.progress",
                    "function",
                    &js_type_name(&v),
                ));
            }
        }
    }

    let (promise, resolve, reject) = Promise::new(&ctx)?;
    let resolve_p = Persistent::save(&ctx, resolve);
    let reject_p = Persistent::save(&ctx, reject);

    // The backup runs as a spawned task while the loop is otherwise idle, so it
    // must hold a loop reference: otherwise a natural drain could decide the
    // process is finished while SQLite is still stepping pages.
    let loop_ref = {
        let rt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt as *mut std::ffi::c_void);
        lexe_utils::event_loop::EventLoopRef::new_referenced(rt_key)
    };

    let source_ptr = inner.connection_ptr()?;
    let dest_c = CString::new(dest_path).map_err(|_| Error::Unknown)?;
    let mut dest_ptr: *mut sqlite3 = ptr::null_mut();

    let open_r = unsafe {
        ffi::sqlite3_open_v2(
            dest_c.as_ptr(),
            &mut dest_ptr,
            SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_URI,
            ptr::null(),
        )
    };

    if open_r != ffi::SQLITE_OK {
        let err = if dest_ptr.is_null() {
            make_sqlite_error_code(&ctx, open_r)?
        } else {
            make_sqlite_error(&ctx, dest_ptr)?
        };
        if !dest_ptr.is_null() {
            unsafe {
                ffi::sqlite3_close_v2(dest_ptr);
            }
        }
        let () = reject_p.clone().restore(&ctx)?.call((err,))?;
        return Ok(promise);
    }

    let source_c = CString::new(source_name).map_err(|_| Error::Unknown)?;
    let dest_db_c = CString::new(dest_name).map_err(|_| Error::Unknown)?;

    let backup_ptr = unsafe {
        ffi::sqlite3_backup_init(dest_ptr, dest_db_c.as_ptr(), source_ptr, source_c.as_ptr())
    };

    if backup_ptr.is_null() {
        let err = make_sqlite_error(&ctx, dest_ptr)?;
        unsafe {
            ffi::sqlite3_close_v2(dest_ptr);
        }
        let () = reject_p.clone().restore(&ctx)?.call((err,))?;
        return Ok(promise);
    }

    let state = BackupState {
        inner: inner.clone(),
        dest: Connection::new(dest_ptr),
        backup: Backup::new(backup_ptr),
        pages: rate,
        progress: progress_fn.map(|f| Persistent::save(&ctx, f)),
    };

    ctx.clone().spawn(async move {
        let result = run_backup_steps(state, &ctx).await;
        // Release the loop reference only after the promise settles.
        drop(loop_ref);
        match result {
            Ok(total_pages) => {
                if let Ok(resolve) = resolve_p.clone().restore(&ctx) {
                    let _ = resolve.call::<(i32,), ()>((total_pages,));
                }
            },
            Err(err) => {
                if let Ok(reject) = reject_p.clone().restore(&ctx) {
                    let reject_value = match err {
                        BackupRunError::Value(v) => v,
                        BackupRunError::Internal(_e) => {
                            if let Ok(v) = make_sqlite_error_code(&ctx, ffi::SQLITE_ERROR) {
                                v
                            } else {
                                return;
                            }
                        },
                    };
                    let _ = reject.call::<(Value<'_>,), ()>((reject_value,));
                }
            },
        }
    });

    Ok(promise)
}

enum BackupRunError<'js> {
    Value(Value<'js>),
    Internal(Error),
}

async fn run_backup_steps<'js>(
    mut state: BackupState<'js>,
    ctx: &Ctx<'js>,
) -> std::result::Result<i32, BackupRunError<'js>> {
    let backup_ptr = state.backup.as_ptr();
    let backup_handle = backup_ptr as usize;
    let pages = state.pages;

    loop {
        let status = match tokio::task::spawn_blocking(move || {
            let ptr = backup_handle as *mut ffi::sqlite3_backup;
            unsafe { ffi::sqlite3_backup_step(ptr, pages) }
        })
        .await
        {
            Ok(status) => status,
            Err(_) => {
                state.cleanup();
                return Err(BackupRunError::Internal(Error::Unknown));
            },
        };

        if status != ffi::SQLITE_OK
            && status != ffi::SQLITE_DONE
            && status != ffi::SQLITE_BUSY
            && status != ffi::SQLITE_LOCKED
        {
            state.cleanup();
            return Err(BackupRunError::Value(
                make_sqlite_error_code(ctx, status).map_err(BackupRunError::Internal)?,
            ));
        }

        let total_pages = unsafe { ffi::sqlite3_backup_pagecount(backup_ptr) };
        let remaining = unsafe { ffi::sqlite3_backup_remaining(backup_ptr) };

        if remaining != 0 {
            if let Some(progress_p) = &state.progress {
                let progress_fn = progress_p
                    .clone()
                    .restore(ctx)
                    .map_err(BackupRunError::Internal)?;
                let info = Object::new(ctx.clone()).map_err(BackupRunError::Internal)?;
                info.set("totalPages", total_pages)
                    .map_err(BackupRunError::Internal)?;
                info.set("remainingPages", remaining)
                    .map_err(BackupRunError::Internal)?;
                match progress_fn.call::<(Object<'_>,), ()>((info,)) {
                    Ok(_) => {},
                    Err(Error::Exception) => {
                        let err = ctx.catch();
                        state.cleanup();
                        return Err(BackupRunError::Value(err));
                    },
                    Err(e) => {
                        state.cleanup();
                        return Err(BackupRunError::Internal(e));
                    },
                }
            }
            continue;
        }

        if status != ffi::SQLITE_DONE {
            state.cleanup();
            return Err(BackupRunError::Value(
                make_sqlite_error(ctx, state.dest.as_ptr()).map_err(BackupRunError::Internal)?,
            ));
        }

        let total = total_pages;
        state.cleanup();
        return Ok(total);
    }
}
