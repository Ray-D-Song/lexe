//! Minimal SQLite C ABI bindings used by `node:sqlite`.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::{c_char, c_int, c_void};

pub const SQLITE_OK: c_int = 0;
pub const SQLITE_ERROR: c_int = 1;
pub const SQLITE_BUSY: c_int = 5;
pub const SQLITE_LOCKED: c_int = 6;
pub const SQLITE_MISUSE: c_int = 21;
pub const SQLITE_RANGE: c_int = 25;
pub const SQLITE_DONE: c_int = 101;
pub const SQLITE_ROW: c_int = 100;
pub const SQLITE_ABORT: c_int = 4;
pub const SQLITE_INTERRUPT: c_int = 9;

pub const SQLITE_OPEN_READONLY: c_int = 0x0000_0001;
pub const SQLITE_OPEN_READWRITE: c_int = 0x0000_0002;
pub const SQLITE_OPEN_CREATE: c_int = 0x0000_0004;
pub const SQLITE_OPEN_URI: c_int = 0x0000_0040;

pub const SQLITE_UTF8: c_int = 1;
pub const SQLITE_INTEGER: c_int = 1;
pub const SQLITE_FLOAT: c_int = 2;
pub const SQLITE_TEXT: c_int = 3;
pub const SQLITE_BLOB: c_int = 4;
pub const SQLITE_NULL: c_int = 5;

pub const SQLITE_DETERMINISTIC: c_int = 0x0000_0800;
pub const SQLITE_DIRECTONLY: c_int = 0x0008_0000;

pub const SQLITE_CHANGESET_OMIT: c_int = 0;
pub const SQLITE_CHANGESET_REPLACE: c_int = 1;
pub const SQLITE_CHANGESET_ABORT: c_int = 2;

pub const SQLITE_CHANGESET_DATA: c_int = 1;
pub const SQLITE_CHANGESET_NOTFOUND: c_int = 2;
pub const SQLITE_CHANGESET_CONFLICT: c_int = 3;
pub const SQLITE_CHANGESET_CONSTRAINT: c_int = 4;
pub const SQLITE_CHANGESET_FOREIGN_KEY: c_int = 5;

pub const SQLITE_DBCONFIG_ENABLE_FKEY: c_int = 1002;
pub const SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION: c_int = 1015;
pub const SQLITE_DBCONFIG_DQS_DDL: c_int = 1013;
pub const SQLITE_DBCONFIG_DQS_DML: c_int = 1014;

pub type sqlite3 = c_void;
pub type sqlite3_stmt = c_void;
pub type sqlite3_context = c_void;
pub type sqlite3_value = c_void;
pub type sqlite3_session = c_void;
pub type sqlite3_backup = c_void;
pub type sqlite3_int64 = i64;

pub type xFunc = Option<unsafe extern "C" fn(*mut sqlite3_context, c_int, *mut *mut sqlite3_value)>;
pub type xStep = Option<unsafe extern "C" fn(*mut sqlite3_context, c_int, *mut *mut sqlite3_value)>;
pub type xFinal = Option<unsafe extern "C" fn(*mut sqlite3_context)>;
pub type xDestroy = Option<unsafe extern "C" fn(*mut c_void)>;

#[repr(C)]
pub struct sqlite3_api_routines {
    _private: [u8; 0],
}

extern "C" {
    pub fn sqlite3_libversion() -> *const c_char;
    pub fn sqlite3_compileoption_get(n: c_int) -> *const c_char;
    pub fn sqlite3_compileoption_used(z: *const c_char) -> c_int;
    pub fn sqlite3_threadsafe() -> c_int;

    pub fn sqlite3_open_v2(
        filename: *const c_char,
        pp_db: *mut *mut sqlite3,
        flags: c_int,
        z_vfs: *const c_char,
    ) -> c_int;

    pub fn sqlite3_close_v2(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_errmsg(db: *mut sqlite3) -> *const c_char;
    pub fn sqlite3_errcode(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_extended_errcode(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_errstr(code: c_int) -> *const c_char;

    pub fn sqlite3_exec(
        db: *mut sqlite3,
        sql: *const c_char,
        callback: Option<
            unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int,
        >,
        arg: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;

    pub fn sqlite3_prepare_v2(
        db: *mut sqlite3,
        z_sql: *const c_char,
        n_byte: c_int,
        pp_stmt: *mut *mut sqlite3_stmt,
        pz_tail: *mut *const c_char,
    ) -> c_int;

    pub fn sqlite3_finalize(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_reset(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_clear_bindings(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_step(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_bind_parameter_count(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_bind_parameter_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_bind_parameter_index(stmt: *mut sqlite3_stmt, name: *const c_char) -> c_int;
    pub fn sqlite3_bind_null(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    pub fn sqlite3_bind_double(stmt: *mut sqlite3_stmt, i: c_int, val: f64) -> c_int;
    pub fn sqlite3_bind_int64(stmt: *mut sqlite3_stmt, i: c_int, val: sqlite3_int64) -> c_int;
    pub fn sqlite3_bind_text(
        stmt: *mut sqlite3_stmt,
        i: c_int,
        val: *const c_char,
        n: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    pub fn sqlite3_bind_blob(
        stmt: *mut sqlite3_stmt,
        i: c_int,
        val: *const c_void,
        n: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;

    pub fn sqlite3_column_count(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_column_type(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    pub fn sqlite3_column_int64(stmt: *mut sqlite3_stmt, i: c_int) -> sqlite3_int64;
    pub fn sqlite3_column_double(stmt: *mut sqlite3_stmt, i: c_int) -> f64;
    pub fn sqlite3_column_text(stmt: *mut sqlite3_stmt, i: c_int) -> *const u8;
    pub fn sqlite3_column_bytes(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    pub fn sqlite3_column_blob(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_void;
    pub fn sqlite3_column_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_column_origin_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_column_database_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_column_table_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_column_decltype(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;

    pub fn sqlite3_sql(stmt: *mut sqlite3_stmt) -> *const c_char;
    pub fn sqlite3_expanded_sql(stmt: *mut sqlite3_stmt) -> *mut c_char;

    pub fn sqlite3_changes(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_changes64(db: *mut sqlite3) -> sqlite3_int64;
    pub fn sqlite3_last_insert_rowid(db: *mut sqlite3) -> sqlite3_int64;
    pub fn sqlite3_get_autocommit(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_db_filename(db: *mut sqlite3, z_db_name: *const c_char) -> *const c_char;
    pub fn sqlite3_busy_timeout(db: *mut sqlite3, ms: c_int) -> c_int;

    pub fn sqlite3_create_function_v2(
        db: *mut sqlite3,
        z_function_name: *const c_char,
        n_arg: c_int,
        e_text_rep: c_int,
        p_app: *mut c_void,
        x_func: xFunc,
        x_step: xStep,
        x_final: xFinal,
        x_destroy: xDestroy,
    ) -> c_int;

    pub fn sqlite3_create_window_function(
        db: *mut sqlite3,
        z_function_name: *const c_char,
        n_arg: c_int,
        e_text_rep: c_int,
        p_app: *mut c_void,
        x_step: xStep,
        x_final: xFinal,
        x_value: xFinal,
        x_inverse: xStep,
        x_destroy: xDestroy,
    ) -> c_int;

    pub fn sqlite3_backup_init(
        dest: *mut sqlite3,
        dest_name: *const c_char,
        source: *mut sqlite3,
        source_name: *const c_char,
    ) -> *mut sqlite3_backup;

    pub fn sqlite3_backup_step(p: *mut sqlite3_backup, n_page: c_int) -> c_int;
    pub fn sqlite3_backup_finish(p: *mut sqlite3_backup) -> c_int;
    pub fn sqlite3_backup_pagecount(p: *mut sqlite3_backup) -> c_int;
    pub fn sqlite3_backup_remaining(p: *mut sqlite3_backup) -> c_int;

    pub fn sqlite3_user_data(ctx: *mut sqlite3_context) -> *mut c_void;
    pub fn sqlite3_aggregate_context(ctx: *mut sqlite3_context, n_bytes: c_int) -> *mut c_void;
    pub fn sqlite3_result_null(ctx: *mut sqlite3_context);
    pub fn sqlite3_result_double(ctx: *mut sqlite3_context, val: f64);
    pub fn sqlite3_result_int64(ctx: *mut sqlite3_context, val: sqlite3_int64);
    pub fn sqlite3_result_text(
        ctx: *mut sqlite3_context,
        val: *const c_char,
        n: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    );
    pub fn sqlite3_result_blob(
        ctx: *mut sqlite3_context,
        val: *const c_void,
        n: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    );
    pub fn sqlite3_result_error(ctx: *mut sqlite3_context, msg: *const c_char, n: c_int);

    pub fn sqlite3_value_type(value: *mut sqlite3_value) -> c_int;
    pub fn sqlite3_value_int64(value: *mut sqlite3_value) -> sqlite3_int64;
    pub fn sqlite3_value_double(value: *mut sqlite3_value) -> f64;
    pub fn sqlite3_value_text(value: *mut sqlite3_value) -> *const u8;
    pub fn sqlite3_value_bytes(value: *mut sqlite3_value) -> c_int;
    pub fn sqlite3_value_blob(value: *mut sqlite3_value) -> *const c_void;

    pub fn sqlite3_load_extension(
        db: *mut sqlite3,
        z_file: *const c_char,
        z_proc: *const c_char,
        pz_err_msg: *mut *mut c_char,
    ) -> c_int;

    pub fn sqlite3_free(ptr: *mut c_void);

    pub fn sqlite3session_create(
        db: *mut sqlite3,
        z_db: *const c_char,
        pp_session: *mut *mut sqlite3_session,
    ) -> c_int;
    pub fn sqlite3session_delete(p_session: *mut sqlite3_session);
    pub fn sqlite3session_enable(p_session: *mut sqlite3_session, b_enable: c_int) -> c_int;
    pub fn sqlite3session_attach(p_session: *mut sqlite3_session, z_tab: *const c_char) -> c_int;
    pub fn sqlite3session_changeset(
        p_session: *mut sqlite3_session,
        pn_changeset: *mut c_int,
        pp_changeset: *mut *mut c_void,
    ) -> c_int;
    pub fn sqlite3session_patchset(
        p_session: *mut sqlite3_session,
        pn_patchset: *mut c_int,
        pp_patchset: *mut *mut c_void,
    ) -> c_int;

    pub fn sqlite3changeset_apply(
        db: *mut sqlite3,
        n_changeset: c_int,
        p_changeset: *mut c_void,
        x_filter: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int>,
        x_conflict: Option<
            unsafe extern "C" fn(*mut c_void, c_int, *mut sqlite3_changeset_iter) -> c_int,
        >,
        p_ctx: *mut c_void,
    ) -> c_int;

    pub fn lexe_sqlite3_bind_text_transient(
        stmt: *mut sqlite3_stmt,
        index: c_int,
        data: *const c_char,
        length: c_int,
    ) -> c_int;
    pub fn lexe_sqlite3_bind_blob_transient(
        stmt: *mut sqlite3_stmt,
        index: c_int,
        data: *const c_void,
        length: c_int,
    ) -> c_int;
    pub fn lexe_sqlite3_result_text_transient(
        context: *mut sqlite3_context,
        data: *const c_char,
        length: c_int,
    );
    pub fn lexe_sqlite3_result_blob_transient(
        context: *mut sqlite3_context,
        data: *const c_void,
        length: c_int,
    );
}

pub type sqlite3_changeset_iter = c_void;

/// Type-safe shim for variadic `sqlite3_db_config`.
pub unsafe fn sqlite3_db_config_enable(db: *mut sqlite3, op: c_int, value: c_int) -> c_int {
    lexe_sqlite3_db_config_enable(db, op, value)
}

extern "C" {
    fn lexe_sqlite3_db_config_enable(db: *mut sqlite3, op: c_int, value: c_int) -> c_int;
}

pub const SQLITE_VERSION: &str = "3.50.1";

pub fn version_string() -> &'static str {
    SQLITE_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;

    #[test]
    fn sqlite_version_matches() {
        unsafe {
            let ver = std::ffi::CStr::from_ptr(sqlite3_libversion());
            assert_eq!(ver.to_str().unwrap(), SQLITE_VERSION);
        }
    }

    #[test]
    fn sqlite_is_threadsafe() {
        unsafe {
            assert_eq!(sqlite3_threadsafe(), 1);
        }
    }

    #[test]
    fn transient_text_bind_preserves_embedded_nul() {
        unsafe {
            let mut db: *mut sqlite3 = ptr::null_mut();
            assert_eq!(
                sqlite3_open_v2(
                    c":memory:".as_ptr(),
                    &mut db,
                    SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE,
                    ptr::null(),
                ),
                SQLITE_OK
            );

            let sql = CString::new("SELECT ?").unwrap();
            let mut stmt: *mut sqlite3_stmt = ptr::null_mut();
            assert_eq!(
                sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut stmt, ptr::null_mut()),
                SQLITE_OK
            );

            let text = "a\0b";
            assert_eq!(
                lexe_sqlite3_bind_text_transient(stmt, 1, text.as_ptr().cast(), 3),
                SQLITE_OK
            );
            assert_eq!(sqlite3_step(stmt), SQLITE_ROW);
            let ptr = sqlite3_column_text(stmt, 0);
            let len = sqlite3_column_bytes(stmt, 0) as usize;
            assert_eq!(std::slice::from_raw_parts(ptr, len), b"a\0b");

            sqlite3_finalize(stmt);
            sqlite3_close_v2(db);
        }
    }

    #[test]
    fn transient_blob_bind_zero_length() {
        unsafe {
            let mut db: *mut sqlite3 = ptr::null_mut();
            assert_eq!(
                sqlite3_open_v2(
                    c":memory:".as_ptr(),
                    &mut db,
                    SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE,
                    ptr::null(),
                ),
                SQLITE_OK
            );

            let sql = CString::new("SELECT ?").unwrap();
            let mut stmt: *mut sqlite3_stmt = ptr::null_mut();
            assert_eq!(
                sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut stmt, ptr::null_mut()),
                SQLITE_OK
            );

            assert_eq!(
                lexe_sqlite3_bind_blob_transient(stmt, 1, ptr::null(), 0),
                SQLITE_OK
            );
            assert_eq!(sqlite3_step(stmt), SQLITE_ROW);
            assert_eq!(sqlite3_column_bytes(stmt, 0), 0);

            sqlite3_finalize(stmt);
            sqlite3_close_v2(db);
        }
    }
}
