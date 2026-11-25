mod backup;
mod database;
mod error;
mod ffi;
mod function;
mod native;
mod path;
mod session;
mod statement;
mod tag_store;
mod value;

use lexe_utils::module::ModuleInfo;
use rquickjs::{
    function::Func,
    module::{Declarations, Exports, ModuleDef},
    object::Property,
    Class, Ctx, Function, Object, Result,
};

use crate::backup::backup;
use crate::database::{install_database_sync_extras, DatabaseSync};
use crate::ffi::{
    SQLITE_ALTER_TABLE, SQLITE_ANALYZE, SQLITE_ATTACH, SQLITE_CHANGESET_ABORT,
    SQLITE_CHANGESET_CONFLICT, SQLITE_CHANGESET_CONSTRAINT, SQLITE_CHANGESET_DATA,
    SQLITE_CHANGESET_FOREIGN_KEY, SQLITE_CHANGESET_NOTFOUND, SQLITE_CHANGESET_OMIT,
    SQLITE_CHANGESET_REPLACE, SQLITE_COPY, SQLITE_CREATE_INDEX, SQLITE_CREATE_TABLE,
    SQLITE_CREATE_TEMP_INDEX, SQLITE_CREATE_TEMP_TABLE, SQLITE_CREATE_TEMP_TRIGGER,
    SQLITE_CREATE_TEMP_VIEW, SQLITE_CREATE_TRIGGER, SQLITE_CREATE_VIEW, SQLITE_CREATE_VTABLE,
    SQLITE_DELETE, SQLITE_DENY, SQLITE_DETACH, SQLITE_DROP_INDEX, SQLITE_DROP_TABLE,
    SQLITE_DROP_TEMP_INDEX, SQLITE_DROP_TEMP_TABLE, SQLITE_DROP_TEMP_TRIGGER,
    SQLITE_DROP_TEMP_VIEW, SQLITE_DROP_TRIGGER, SQLITE_DROP_VIEW, SQLITE_DROP_VTABLE,
    SQLITE_FUNCTION, SQLITE_IGNORE, SQLITE_INSERT, SQLITE_OK, SQLITE_PRAGMA, SQLITE_READ,
    SQLITE_RECURSIVE, SQLITE_REINDEX, SQLITE_SAVEPOINT, SQLITE_SELECT, SQLITE_TRANSACTION,
    SQLITE_UPDATE,
};
use crate::session::Session;
use crate::statement::{install_statement_iterator_prototype, StatementSync};
use crate::tag_store::SQLTagStore;

pub const NODE_SQLITE_MODULE_NAME: &str = "node:sqlite";
pub const SQLITE_VERSION: &str = "3.50.1";

pub fn sqlite_version() -> &'static str {
    SQLITE_VERSION
}

fn emit_experimental_warning(ctx: &Ctx<'_>) -> Result<()> {
    if lexe_process::no_warnings() {
        return Ok(());
    }

    let globals = ctx.globals();
    let process: Object = globals.get("process")?;
    let emit_warning: Function = process.get("emitWarning")?;
    let () = emit_warning.call((
        "SQLite is an experimental feature and might change at any time",
        "ExperimentalWarning",
    ))?;
    Ok(())
}

fn define_constants(constants: &Object<'_>) -> Result<()> {
    const CONSTANTS: [(&str, i32); 45] = [
        ("SQLITE_CHANGESET_OMIT", SQLITE_CHANGESET_OMIT),
        ("SQLITE_CHANGESET_REPLACE", SQLITE_CHANGESET_REPLACE),
        ("SQLITE_CHANGESET_ABORT", SQLITE_CHANGESET_ABORT),
        ("SQLITE_CHANGESET_DATA", SQLITE_CHANGESET_DATA),
        ("SQLITE_CHANGESET_NOTFOUND", SQLITE_CHANGESET_NOTFOUND),
        ("SQLITE_CHANGESET_CONFLICT", SQLITE_CHANGESET_CONFLICT),
        ("SQLITE_CHANGESET_CONSTRAINT", SQLITE_CHANGESET_CONSTRAINT),
        ("SQLITE_CHANGESET_FOREIGN_KEY", SQLITE_CHANGESET_FOREIGN_KEY),
        ("SQLITE_OK", SQLITE_OK),
        ("SQLITE_DENY", SQLITE_DENY),
        ("SQLITE_IGNORE", SQLITE_IGNORE),
        ("SQLITE_CREATE_INDEX", SQLITE_CREATE_INDEX),
        ("SQLITE_CREATE_TABLE", SQLITE_CREATE_TABLE),
        ("SQLITE_CREATE_TEMP_INDEX", SQLITE_CREATE_TEMP_INDEX),
        ("SQLITE_CREATE_TEMP_TABLE", SQLITE_CREATE_TEMP_TABLE),
        ("SQLITE_CREATE_TEMP_TRIGGER", SQLITE_CREATE_TEMP_TRIGGER),
        ("SQLITE_CREATE_TEMP_VIEW", SQLITE_CREATE_TEMP_VIEW),
        ("SQLITE_CREATE_TRIGGER", SQLITE_CREATE_TRIGGER),
        ("SQLITE_CREATE_VIEW", SQLITE_CREATE_VIEW),
        ("SQLITE_DELETE", SQLITE_DELETE),
        ("SQLITE_DROP_INDEX", SQLITE_DROP_INDEX),
        ("SQLITE_DROP_TABLE", SQLITE_DROP_TABLE),
        ("SQLITE_DROP_TEMP_INDEX", SQLITE_DROP_TEMP_INDEX),
        ("SQLITE_DROP_TEMP_TABLE", SQLITE_DROP_TEMP_TABLE),
        ("SQLITE_DROP_TEMP_TRIGGER", SQLITE_DROP_TEMP_TRIGGER),
        ("SQLITE_DROP_TEMP_VIEW", SQLITE_DROP_TEMP_VIEW),
        ("SQLITE_DROP_TRIGGER", SQLITE_DROP_TRIGGER),
        ("SQLITE_DROP_VIEW", SQLITE_DROP_VIEW),
        ("SQLITE_INSERT", SQLITE_INSERT),
        ("SQLITE_PRAGMA", SQLITE_PRAGMA),
        ("SQLITE_READ", SQLITE_READ),
        ("SQLITE_SELECT", SQLITE_SELECT),
        ("SQLITE_TRANSACTION", SQLITE_TRANSACTION),
        ("SQLITE_UPDATE", SQLITE_UPDATE),
        ("SQLITE_ATTACH", SQLITE_ATTACH),
        ("SQLITE_DETACH", SQLITE_DETACH),
        ("SQLITE_ALTER_TABLE", SQLITE_ALTER_TABLE),
        ("SQLITE_REINDEX", SQLITE_REINDEX),
        ("SQLITE_ANALYZE", SQLITE_ANALYZE),
        ("SQLITE_CREATE_VTABLE", SQLITE_CREATE_VTABLE),
        ("SQLITE_DROP_VTABLE", SQLITE_DROP_VTABLE),
        ("SQLITE_FUNCTION", SQLITE_FUNCTION),
        ("SQLITE_SAVEPOINT", SQLITE_SAVEPOINT),
        ("SQLITE_COPY", SQLITE_COPY),
        ("SQLITE_RECURSIVE", SQLITE_RECURSIVE),
    ];

    for (name, value) in CONSTANTS {
        constants.prop(name, Property::from(value).enumerable())?;
    }
    Ok(())
}

pub struct SqliteModule;

impl ModuleDef for SqliteModule {
    fn declare(declare: &Declarations<'_>) -> Result<()> {
        declare.declare("DatabaseSync")?;
        declare.declare("StatementSync")?;
        declare.declare("backup")?;
        declare.declare("constants")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        emit_experimental_warning(ctx)?;

        let _ = Class::<Session>::create_constructor(ctx)?;
        let _ = Class::<SQLTagStore>::create_constructor(ctx)?;
        install_statement_iterator_prototype(ctx)?;

        let default = Object::new(ctx.clone())?;
        Class::<DatabaseSync>::define(&default)?;
        install_database_sync_extras(ctx)?;
        Class::<StatementSync>::define(&default)?;

        let constants = Object::new(ctx.clone())?;
        define_constants(&constants)?;
        default.set("constants", constants.clone())?;
        default.set("backup", Func::from(backup))?;

        exports.export(
            "DatabaseSync",
            default.get::<_, rquickjs::Value>("DatabaseSync")?,
        )?;
        exports.export(
            "StatementSync",
            default.get::<_, rquickjs::Value>("StatementSync")?,
        )?;
        exports.export("constants", constants)?;
        exports.export("backup", default.get::<_, rquickjs::Value>("backup")?)?;
        exports.export("default", default)?;

        Ok(())
    }
}

impl From<SqliteModule> for ModuleInfo<SqliteModule> {
    fn from(val: SqliteModule) -> Self {
        ModuleInfo {
            name: NODE_SQLITE_MODULE_NAME,
            module: val,
        }
    }
}
