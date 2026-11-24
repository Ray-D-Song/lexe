mod backup;
mod database;
mod error;
mod ffi;
mod function;
mod native;
mod path;
mod session;
mod statement;
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
    SQLITE_CHANGESET_ABORT, SQLITE_CHANGESET_CONFLICT, SQLITE_CHANGESET_CONSTRAINT,
    SQLITE_CHANGESET_DATA, SQLITE_CHANGESET_FOREIGN_KEY, SQLITE_CHANGESET_NOTFOUND,
    SQLITE_CHANGESET_OMIT, SQLITE_CHANGESET_REPLACE,
};
use crate::session::Session;
use crate::statement::{install_statement_iterator_prototype, StatementSync};

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

fn define_changeset_constants(_ctx: &Ctx<'_>, constants: &Object<'_>) -> Result<()> {
    constants.prop(
        "SQLITE_CHANGESET_OMIT",
        Property::from(SQLITE_CHANGESET_OMIT).enumerable(),
    )?;
    constants.prop(
        "SQLITE_CHANGESET_REPLACE",
        Property::from(SQLITE_CHANGESET_REPLACE).enumerable(),
    )?;
    constants.prop(
        "SQLITE_CHANGESET_ABORT",
        Property::from(SQLITE_CHANGESET_ABORT).enumerable(),
    )?;
    constants.prop(
        "SQLITE_CHANGESET_DATA",
        Property::from(SQLITE_CHANGESET_DATA).enumerable(),
    )?;
    constants.prop(
        "SQLITE_CHANGESET_NOTFOUND",
        Property::from(SQLITE_CHANGESET_NOTFOUND).enumerable(),
    )?;
    constants.prop(
        "SQLITE_CHANGESET_CONFLICT",
        Property::from(SQLITE_CHANGESET_CONFLICT).enumerable(),
    )?;
    constants.prop(
        "SQLITE_CHANGESET_CONSTRAINT",
        Property::from(SQLITE_CHANGESET_CONSTRAINT).enumerable(),
    )?;
    constants.prop(
        "SQLITE_CHANGESET_FOREIGN_KEY",
        Property::from(SQLITE_CHANGESET_FOREIGN_KEY).enumerable(),
    )?;
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
        install_statement_iterator_prototype(ctx)?;

        let default = Object::new(ctx.clone())?;
        Class::<DatabaseSync>::define(&default)?;
        install_database_sync_extras(ctx)?;
        Class::<StatementSync>::define(&default)?;

        let constants = Object::new(ctx.clone())?;
        define_changeset_constants(ctx, &constants)?;
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
