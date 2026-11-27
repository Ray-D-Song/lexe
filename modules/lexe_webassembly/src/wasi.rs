// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node.js `node:wasi` implementation (`WASI` class) backed by `wasmz::WasiHost`.

use std::rc::Rc;

use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    atom::PredefinedAtom,
    class::Trace,
    module::{Declarations, Exports, ModuleDef},
    object::Property,
    prelude::Opt,
    Array, Class, Coerced, Ctx, Exception, FromJs, JsLifetime, Object, Result, Value,
};

use crate::instance::WasmInstance;

pub const WASI_HOST_ID_KEY: &str = "\0lexe:wasi_host_id";

const WASI_PREVIEW1_FUNCS: &[&str] = &[
    "args_sizes_get",
    "args_get",
    "environ_sizes_get",
    "environ_get",
    "clock_res_get",
    "clock_time_get",
    "fd_advise",
    "fd_allocate",
    "fd_close",
    "fd_datasync",
    "fd_fdstat_get",
    "fd_fdstat_set_flags",
    "fd_fdstat_set_rights",
    "fd_filestat_get",
    "fd_filestat_set_size",
    "fd_filestat_set_times",
    "fd_pread",
    "fd_prestat_get",
    "fd_prestat_dir_name",
    "fd_pwrite",
    "fd_read",
    "fd_readdir",
    "fd_renumber",
    "fd_seek",
    "fd_sync",
    "fd_tell",
    "fd_write",
    "path_create_directory",
    "path_filestat_get",
    "path_filestat_set_times",
    "path_link",
    "path_open",
    "path_readlink",
    "path_remove_directory",
    "path_rename",
    "path_symlink",
    "path_unlink_file",
    "poll_oneoff",
    "proc_exit",
    "proc_raise",
    "random_get",
    "sched_yield",
    "sock_accept",
    "sock_recv",
    "sock_send",
    "sock_shutdown",
];

fn throw_error(ctx: &Ctx<'_>, msg: &str) -> rquickjs::Error {
    match Exception::from_message(ctx.clone(), msg) {
        Ok(exc) => ctx.throw(exc.into_value()),
        Err(e) => e,
    }
}

pub struct WasiState {
    pub host: wasmz::WasiHost,
    pub return_on_exit: bool,
    pub started: bool,
}

#[derive(JsLifetime)]
#[rquickjs::class(rename = "WASI")]
pub struct WasiClass<'js> {
    realm_id: u64,
    wasi_id: u64,
    wasi_import: Object<'js>,
}

impl<'js> Trace<'js> for WasiClass<'js> {
    fn trace<'a>(&self, tracer: rquickjs::class::Tracer<'a, 'js>) {
        tracer.mark(self.wasi_import.as_value());
    }
}

fn parse_string_array<'js>(ctx: &Ctx<'js>, val: Value<'js>, label: &str) -> Result<Vec<String>> {
    if !val.is_array() {
        return Err(Exception::throw_type(
            ctx,
            &format!("The \"options.{label}\" property must be an instance of Array"),
        ));
    }
    let arr = Array::from_value(val)?;
    let mut out = Vec::with_capacity(arr.len());
    for i in 0..arr.len() {
        let item: Value = arr.get(i)?;
        if !item.is_string() {
            return Err(Exception::throw_type(
                ctx,
                &format!("The \"options.{label}[{i}]\" property must be of type string"),
            ));
        }
        out.push(Coerced::<String>::from_js(ctx, item)?.0);
    }
    Ok(out)
}

fn parse_string_map<'js>(
    ctx: &Ctx<'js>,
    val: Value<'js>,
    label: &str,
) -> Result<Vec<(String, String)>> {
    let Some(obj) = val.as_object() else {
        return Err(Exception::throw_type(
            ctx,
            &format!("The \"options.{label}\" property must be of type object"),
        ));
    };
    let mut out = Vec::new();
    for prop in obj.props::<String, Value<'js>>() {
        let (k, v) = prop?;
        if !v.is_undefined() {
            let s = Coerced::<String>::from_js(ctx, v)?.0;
            out.push((k, s));
        }
    }
    Ok(out)
}

fn build_wasi_import_object<'js>(
    ctx: &Ctx<'js>,
    host: &crate::host_state::HostState,
    wasi_id: u64,
) -> Result<Object<'js>> {
    let import_obj = Object::new(ctx.clone())?;
    import_obj.prop(WASI_HOST_ID_KEY, Property::from(wasi_id as f64))?;
    for &name in WASI_PREVIEW1_FUNCS {
        let fn_val = rquickjs::Function::new(ctx.clone(), move |c: Ctx<'js>| -> Result<()> {
            Err(Exception::throw_type(
                &c,
                &format!(
                    "WASI import '{name}' cannot be called directly outside a WASI-bound WebAssembly instance"
                ),
            ))
        })?;
        let _ = fn_val.set_name(name);
        host.register_wasi_stub(&fn_val.clone().into_value());
        import_obj.set(name, fn_val)?;
    }
    Ok(import_obj)
}

#[rquickjs::methods]
impl<'js> WasiClass<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, options: Opt<Value<'js>>) -> Result<Self> {
        let realm = crate::realm::realm(&ctx)?;
        let Some(opts_val) = options.0.filter(|v| !v.is_undefined()) else {
            return Err(Exception::throw_type(
                &ctx,
                "The \"options.version\" property must be of type string. Received undefined",
            ));
        };
        let Some(opts) = opts_val.as_object() else {
            return Err(Exception::throw_type(
                &ctx,
                "The \"options\" argument must be of type object",
            ));
        };

        let version_val: Value = opts.get("version")?;
        if !version_val.is_string() {
            return Err(Exception::throw_type(
                &ctx,
                "The \"options.version\" property must be of type string",
            ));
        }
        let version: String = Coerced::<String>::from_js(&ctx, version_val)?.0;
        if version != "preview1" {
            return Err(Exception::throw_type(
                &ctx,
                &format!("Unsupported WASI version: {version}"),
            ));
        }

        let mut wasi_host = wasmz::WasiHost::new()
            .map_err(|err| Exception::throw_internal(&ctx, &err.to_string()))?;

        let return_on_exit_val: Value = opts.get("returnOnExit")?;
        let return_on_exit = if return_on_exit_val.is_undefined() {
            true
        } else {
            if !return_on_exit_val.is_bool() {
                return Err(Exception::throw_type(
                    &ctx,
                    "The \"options.returnOnExit\" property must be of type boolean",
                ));
            }
            Coerced::<bool>::from_js(&ctx, return_on_exit_val)?.0
        };
        wasi_host.set_return_on_exit(true);

        let args_val: Value = opts.get("args")?;
        if !args_val.is_undefined() {
            let args = parse_string_array(&ctx, args_val, "args")?;
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            wasi_host
                .set_args(&refs)
                .map_err(|err| Exception::throw_internal(&ctx, &err.to_string()))?;
        }

        let env_val: Value = opts.get("env")?;
        if !env_val.is_undefined() {
            let pairs = parse_string_map(&ctx, env_val, "env")?;
            let refs: Vec<(&str, &str)> = pairs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            wasi_host
                .set_env(&refs)
                .map_err(|err| Exception::throw_internal(&ctx, &err.to_string()))?;
        }

        let preopens_val: Value = opts.get("preopens")?;
        if !preopens_val.is_undefined() {
            let preopens = parse_string_map(&ctx, preopens_val, "preopens")?;
            for (guest, host_path) in preopens {
                wasi_host
                    .add_preopen(&guest, &host_path)
                    .map_err(|err| throw_error(&ctx, &err.to_string()))?;
            }
        }

        let stdin_val: Value = opts.get("stdin")?;
        let stdout_val: Value = opts.get("stdout")?;
        let stderr_val: Value = opts.get("stderr")?;
        let stdin_fd = if stdin_val.is_undefined() {
            -1
        } else {
            Coerced::<i32>::from_js(&ctx, stdin_val)?.0
        };
        let stdout_fd = if stdout_val.is_undefined() {
            -1
        } else {
            Coerced::<i32>::from_js(&ctx, stdout_val)?.0
        };
        let stderr_fd = if stderr_val.is_undefined() {
            -1
        } else {
            Coerced::<i32>::from_js(&ctx, stderr_val)?.0
        };
        if stdin_fd >= 0 || stdout_fd >= 0 || stderr_fd >= 0 {
            wasi_host.set_stdio_fds(stdin_fd, stdout_fd, stderr_fd);
        }

        let wasi_id = realm.state.register_wasi(WasiState {
            host: wasi_host,
            return_on_exit,
            started: false,
        });
        let wasi_import = build_wasi_import_object(&ctx, &realm.state, wasi_id)?;

        Ok(Self {
            realm_id: realm.state.realm_id,
            wasi_id,
            wasi_import,
        })
    }

    #[qjs(get, rename = "wasiImport")]
    pub fn wasi_import(&self) -> Object<'js> {
        self.wasi_import.clone()
    }

    #[qjs(rename = "getImportObject")]
    pub fn get_import_object(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let out = Object::new(ctx)?;
        out.set("wasi_snapshot_preview1", self.wasi_import.clone())?;
        Ok(out)
    }

    pub fn start(&self, ctx: Ctx<'js>, instance_val: Value<'js>) -> Result<u32> {
        let realm = crate::realm::realm(&ctx)?;
        let host = realm.state.clone();
        if realm.state.realm_id != self.realm_id {
            return Err(host.throw_link_error(&ctx, "WASI belongs to a different realm"));
        }

        let inst_class = Class::<WasmInstance<'js>>::from_value(&instance_val)
            .map_err(|_| Exception::throw_type(&ctx, "instance must be a WebAssembly.Instance"))?;
        let inst_ref = inst_class.borrow();
        if inst_ref.realm_id != self.realm_id {
            return Err(host.throw_link_error(&ctx, "Instance belongs to a different realm"));
        }
        let exports = inst_ref.exports();
        let start_val: Value = exports.get("_start")?;
        if !start_val.is_function() {
            return Err(Exception::throw_type(
                &ctx,
                "WebAssembly.Instance must export a '_start' function",
            ));
        }
        let inst_rc: Rc<wasmz::Instance> = inst_ref.handle.clone();
        drop(inst_ref);

        let wasi_rc = host
            .wasi_state(self.wasi_id)
            .ok_or_else(|| Exception::throw_internal(&ctx, "missing WASI host state"))?;
        {
            let mut state = wasi_rc.borrow_mut();
            if state.started {
                return Err(throw_error(&ctx, "WASI instance has already started"));
            }
            state.started = true;
        }

        crate::memory::sync_all_js_to_wasm(&ctx, &host)?;
        let call_res = inst_rc.call_start();
        let sync_res = crate::memory::sync_all_wasm_to_js(&ctx, &host);

        let state = wasi_rc.borrow();
        match call_res {
            Ok(()) => {
                sync_res?;
                if !state.return_on_exit {
                    std::process::exit(0);
                }
                Ok(0)
            },
            Err(err) => {
                if let Some(code) = state.host.exit_code() {
                    if state.return_on_exit {
                        sync_res?;
                        return Ok(code);
                    }
                    std::process::exit(code as i32);
                }
                if sync_res.is_err() {
                    let _ = ctx.catch();
                }
                Err(crate::errors::throw_wasmz_error(&ctx, &host, err))
            },
        }
    }

    pub fn initialize(&self, ctx: Ctx<'js>, instance_val: Value<'js>) -> Result<()> {
        let realm = crate::realm::realm(&ctx)?;
        let host = realm.state.clone();
        if realm.state.realm_id != self.realm_id {
            return Err(host.throw_link_error(&ctx, "WASI belongs to a different realm"));
        }

        let inst_class = Class::<WasmInstance<'js>>::from_value(&instance_val)
            .map_err(|_| Exception::throw_type(&ctx, "instance must be a WebAssembly.Instance"))?;
        let inst_ref = inst_class.borrow();
        if inst_ref.realm_id != self.realm_id {
            return Err(host.throw_link_error(&ctx, "Instance belongs to a different realm"));
        }
        let exports = inst_ref.exports();
        let start_val: Value = exports.get("_start")?;
        if !start_val.is_undefined() {
            return Err(Exception::throw_type(
                &ctx,
                "WebAssembly.Instance must not export '_start' when calling WASI.initialize",
            ));
        }
        let inst_rc = inst_ref.handle.clone();
        drop(inst_ref);

        let wasi_rc = host
            .wasi_state(self.wasi_id)
            .ok_or_else(|| Exception::throw_internal(&ctx, "missing WASI host state"))?;
        {
            let mut state = wasi_rc.borrow_mut();
            if state.started {
                return Err(throw_error(&ctx, "WASI instance has already started"));
            }
            state.started = true;
        }

        crate::memory::sync_all_js_to_wasm(&ctx, &host)?;
        let call_res = inst_rc.initialize();
        let sync_res = crate::memory::sync_all_wasm_to_js(&ctx, &host);

        match call_res {
            Ok(()) => {
                sync_res?;
                Ok(())
            },
            Err(err) => {
                let state = wasi_rc.borrow();
                if let Some(code) = state.host.exit_code() {
                    if state.return_on_exit {
                        sync_res?;
                        return Ok(());
                    }
                    std::process::exit(code as i32);
                }
                if sync_res.is_err() {
                    let _ = ctx.catch();
                }
                Err(crate::errors::throw_wasmz_error(&ctx, &host, err))
            },
        }
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "WASI"
    }
}

pub fn extract_wasi_host_id(namespace: &Object<'_>) -> Result<Option<u64>> {
    let val: Value = namespace.get(WASI_HOST_ID_KEY)?;
    if let Some(n) = val.as_number() {
        if n > 0.0 {
            return Ok(Some(n as u64));
        }
    }
    Ok(None)
}

pub struct WasiModule;

impl ModuleDef for WasiModule {
    fn declare(declare: &Declarations<'_>) -> Result<()> {
        declare.declare("WASI")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        // Ensure WebAssembly realm exists even if only `node:wasi` was imported before touching `WebAssembly`.
        if crate::realm::realm(ctx).is_err() {
            crate::init(ctx)?;
        }
        let ctor = Class::<WasiClass>::create_constructor(ctx)?.expect("WASI constructor");
        export_default(ctx, exports, |default| {
            default.set("WASI", ctor)?;
            Ok(())
        })
    }
}

impl From<WasiModule> for ModuleInfo<WasiModule> {
    fn from(val: WasiModule) -> Self {
        ModuleInfo {
            name: "wasi",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    #[tokio::test]
    async fn wasi_module_runs_start_with_return_on_exit_and_initialize() {
        let cmd_wasm = wat::parse_str(
            r#"
            (module
                (import "wasi_snapshot_preview1" "args_sizes_get" (func $args (param i32 i32) (result i32)))
                (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
                (memory (export "memory") 1)
                (func (export "_start")
                    i32.const 0
                    i32.const 4
                    call $args
                    drop
                    i32.const 42
                    call $exit)
            )
            "#,
        )
        .unwrap();

        let reactor_wasm = wat::parse_str(
            r#"
            (module
                (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
                (memory (export "memory") 1)
                (global $count (mut i32) (i32.const 0))
                (func (export "_initialize")
                    i32.const 10
                    global.set $count)
                (func (export "inc") (result i32)
                    global.get $count
                    i32.const 1
                    i32.add
                    global.set $count
                    global.get $count)
            )
            "#,
        )
        .unwrap();

        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                let cmd_buf = rquickjs::ArrayBuffer::new_copy(ctx.clone(), &cmd_wasm).unwrap();
                let reactor_buf =
                    rquickjs::ArrayBuffer::new_copy(ctx.clone(), &reactor_wasm).unwrap();
                ctx.globals().set("__cmdWasm", cmd_buf).unwrap();
                ctx.globals().set("__reactorWasm", reactor_buf).unwrap();

                let module = ModuleEvaluator::eval_rust::<WasiModule>(ctx.clone(), "wasi")
                    .await
                    .unwrap();
                let ctor: Value = module.get("WASI").unwrap();
                ctx.globals().set("__WASI", ctor).unwrap();

                let runner = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test_wasi",
                    r#"
                    export async function test() {
                        let threw = false;
                        try { new globalThis.__WASI(); } catch (e) { threw = e instanceof TypeError; }
                        if (!threw) return "no-version";

                        const wasi = new globalThis.__WASI({
                            version: "preview1",
                            args: ["node", "app.wasm", "--ok"],
                            env: { FOO: "BAR" },
                            returnOnExit: true,
                        });
                        if (Object.keys(wasi.wasiImport).length !== 46) return "bad-import-count";
                        const imp = wasi.getImportObject();
                        if (imp.wasi_snapshot_preview1 !== wasi.wasiImport) return "bad-getImportObject";

                        const mod = new WebAssembly.Module(globalThis.__cmdWasm);
                        const inst = new WebAssembly.Instance(mod, imp);
                        const rc = wasi.start(inst);
                        if (rc !== 42) return "bad-rc:" + rc;
                        const view = new DataView(inst.exports.memory.buffer);
                        if (view.getUint32(0, true) !== 3) return "bad-argc:" + view.getUint32(0, true);

                        const rWasi = new globalThis.__WASI({ version: "preview1" });
                        const rInst = new WebAssembly.Instance(
                            new WebAssembly.Module(globalThis.__reactorWasm),
                            rWasi.getImportObject()
                        );
                        rWasi.initialize(rInst);
                        if (rInst.exports.inc() !== 11) return "bad-reactor";
                        return "ok";
                    }
                    "#,
                )
                .await
                .unwrap();
                let res: String = call_test(&ctx, &runner, ()).await;
                assert_eq!(res, "ok");
            })
        })
        .await;
    }
}
