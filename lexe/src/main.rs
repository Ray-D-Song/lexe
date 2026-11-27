// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::uninlined_format_args)]

use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
    process::{exit, ExitCode},
    string::String,
    sync::atomic::Ordering,
    time::Instant,
};

mod base;
mod drain;
#[cfg(unix)]
mod fork_child;
mod minimal_tracer;
#[cfg(not(target_family = "wasm"))]
mod repl;
#[cfg(not(target_family = "wasm"))]
mod signal_bridge;
mod worker_isolate;

use constcat::concat;
#[cfg(not(target_family = "wasm"))]
use lexe::{executable, upgrade};
use lexe_core::modules::{
    module_builder::{ModuleBuilder, ModuleBuilderConfig},
    process::{
        argv, set_argv, set_exec_argv, set_source_maps_default, EXIT_CODE, NO_WARNINGS,
        TRACE_WARNINGS,
    },
};
#[cfg(all(feature = "napi", feature = "v8-compat"))]
use lexe_core::vm::{run_final_context_gc, shutdown_context_userdata, shutdown_module_state};
use lexe_core::vm::{Vm, VmOptions};
use minimal_tracer::MinimalTracer;
use tracing::trace;

use crate::base::compiler::compile_file;
use crate::base::{
    bytecode::BYTECODE_EXT,
    libs::utils::{
        fs::DirectoryWalker,
        io::{is_supported_ext, SUPPORTED_EXTENSIONS},
        sysinfo::{ARCH, PLATFORM},
    },
    modules::path::name_extname,
    VERSION,
};

#[cfg(not(any(target_os = "windows", target_family = "wasm", lexe_asan)))]
#[global_allocator]
static ALLOC: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;

// snmalloc 0.7 uses C++ thread_local destructors via __cxa_thread_atexit_impl
// which is unavailable in musl static builds. This fallback uses a pthread TLS
// key to run registered destructors when a thread exits.
#[cfg(target_env = "musl")]
#[no_mangle]
pub unsafe extern "C" fn __cxa_thread_atexit_impl(
    dtor: unsafe extern "C" fn(*mut std::ffi::c_void),
    obj: *mut std::ffi::c_void,
    _dso_symbol: *mut std::ffi::c_void,
) -> std::ffi::c_int {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::Once;

    type Dtor = unsafe extern "C" fn(*mut c_void);
    type DtorList = Vec<(Dtor, *mut c_void)>;

    static INIT: Once = Once::new();
    static mut KEY: libc::pthread_key_t = 0;

    unsafe extern "C" fn run_dtors(ptr: *mut c_void) {
        let mut current = ptr;

        // required by POSIX: destructors may re-register new ones
        while !current.is_null() {
            let list = Box::from_raw(current as *mut DtorList);

            libc::pthread_setspecific(KEY, ptr::null_mut());

            for &(dtor, obj) in list.iter().rev() {
                dtor(obj);
            }

            current = libc::pthread_getspecific(KEY);
        }
    }

    INIT.call_once(|| unsafe {
        let rc = libc::pthread_key_create(&raw mut KEY, Some(run_dtors));
        if rc != 0 {
            std::process::abort();
        }
    });

    let ptr = libc::pthread_getspecific(KEY);

    let mut list: Box<DtorList> = if ptr.is_null() {
        Box::new(Vec::new())
    } else {
        Box::from_raw(ptr as *mut DtorList)
    };

    list.push((dtor, obj));

    let rc = libc::pthread_setspecific(KEY, Box::into_raw(list) as *mut c_void);

    if rc != 0 {
        // if this fails, we must not leak the list but we no longer own the pointer after into_raw. Best recovery is abort (matches libc fatal behavior).
        std::process::abort();
    }

    0
}

#[cfg_attr(not(target_family = "wasm"), tokio::main)]
#[cfg_attr(target_family = "wasm", tokio::main(flavor = "current_thread"))]
async fn main() -> Result<ExitCode, Box<dyn Error + Send + Sync>> {
    #[cfg(feature = "v8-compat")]
    v8_compat::ensure_shim_linked();
    let now = Instant::now();

    MinimalTracer::register().map_err(|err| err.to_string())?;
    trace!("Started runtime");

    worker_isolate::register();

    #[cfg(unix)]
    if env::var("LEXE_FORK_CHILD").ok().as_deref() == Some("1") {
        return fork_child::run()
            .await
            .map(|_| ExitCode::from(0))
            .map_err(|e| e.to_string().into());
    }

    #[cfg(not(target_family = "wasm"))]
    let embedded_bytecode = if let Some(raw_payload) = executable::extract_embedded_bytecode() {
        let payload = lexe_sea::container::decode_embedded_payload(&raw_payload)?;
        if payload.is_sea {
            lexe_sea::init_sea(payload.keys, payload.assets);
        }
        let raw_args: Vec<String> = env::args().collect();
        let app_argv = if payload.sea_argv_mode {
            let mut v = Vec::with_capacity(raw_args.len() + 1);
            let exe = raw_args.first().cloned().unwrap_or_default();
            v.push(exe.clone());
            v.push(exe);
            if raw_args.len() > 1 {
                v.extend_from_slice(&raw_args[1..]);
            }
            v
        } else {
            raw_args
        };
        set_argv(app_argv);
        set_exec_argv(Vec::new());
        Some(payload.bytecode)
    } else {
        None
    };

    #[cfg(target_family = "wasm")]
    let embedded_bytecode: Option<Vec<u8>> = None;

    let sqlite_enabled = if embedded_bytecode.is_none() {
        let cli = parse_node_cli_flags(env::args().collect());
        set_argv(cli.argv);
        set_exec_argv(cli.exec_argv);
        NO_WARNINGS.store(cli.no_warnings, Ordering::Relaxed);
        TRACE_WARNINGS.store(cli.trace_warnings, Ordering::Relaxed);
        set_source_maps_default(cli.source_maps_enabled);
        if cli.permission_enabled {
            lexe_utils::permissions::init(cli.permission_config);
        }
        cli.sqlite_enabled
    } else {
        true
    };

    let vm = Vm::from_options(VmOptions {
        sqlite_enabled,
        module_builder: ModuleBuilder::with_config(ModuleBuilderConfig { sqlite_enabled })
            .with_global(lexe_core::modules::embedded::init)
            .with_global(lexe_core::builtins_inspect::init)
            .with_global(lexe_worker_threads::init)
            .with_module(lexe_core::modules::lexe::hex::LexeHexModule)
            .with_module(lexe_core::modules::lexe::qjs::LexeQjsModule)
            .with_module(lexe_core::modules::lexe::util::LexeUtilModule)
            .with_module(lexe_core::modules::lexe::xml::LexeXmlModule),
        ..VmOptions::default()
    })
    .await?;
    trace!("Initialized VM in {}ms", now.elapsed().as_millis());

    #[cfg(feature = "napi")]
    {
        vm.ctx
            .with(|ctx| {
                lexe_napi::register_async_context(ctx.as_raw());
            })
            .await;
    }

    start_cli(&vm, embedded_bytecode).await;

    #[cfg(not(target_family = "wasm"))]
    signal_bridge::install(&vm).await?;

    drain::run_until_process_exit(&vm).await?;

    // Worker handles keep their channel senders alive process-wide; release
    // them so worker read loops finish and drop the JS values they captured.
    // Then give the loop one more turn to observe the closed channels: a task
    // still parked here would keep its JS roots alive and abort teardown with
    // QuickJS's `gc_obj_list` assertion.
    lexe_worker_threads::shutdown_workers();
    {
        let idle = vm.idle();
        tokio::pin!(idle);
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), idle).await;
    }

    #[cfg(feature = "napi")]
    {
        // Two-phase N-API shutdown with explicit driver wait (no swallowed errors):
        // 1) begin_dispose + return driver Arc
        // 2) idle (run pending JS futures)
        // 3) wait_finished on driver
        // 4) finish_dispose + free Env
        // 5) postcondition: no residual envs/TSFNs
        let driver = vm
            .ctx
            .with(|ctx| lexe_napi::begin_shutdown(&ctx))
            .await
            .map_err(std::io::Error::other)?;

        // Let the driver loop observe sender disconnect and drain TSFN release jobs.
        vm.idle().await?;

        if let Some(ref driver) = driver {
            driver.wait_finished().await;
        }

        // Another idle so any finalizer/GC work scheduled during finish prep runs.
        vm.idle().await?;

        vm.ctx
            .with(|ctx| lexe_napi::finish_shutdown(&ctx))
            .await
            .map_err(std::io::Error::other)?;

        lexe_napi::shutdown_all().map_err(std::io::Error::other)?;
    }

    #[cfg(all(feature = "napi", feature = "v8-compat"))]
    vm.ctx
        .with(|ctx| shutdown_module_state(&ctx).map_err(|err| err.to_string()))
        .await
        .map_err(std::io::Error::other)?;

    #[cfg(all(feature = "napi", feature = "v8-compat"))]
    vm.ctx
        .with(|ctx| shutdown_context_userdata(&ctx).map_err(|err| err.to_string()))
        .await
        .map_err(std::io::Error::other)?;

    #[cfg(all(feature = "napi", feature = "v8-compat"))]
    vm.ctx
        .with(|ctx| unsafe { v8_compat::run_pre_bridge_teardown_gc(ctx.as_raw().as_ptr()) })
        .await
        .map_err(std::io::Error::other)?;

    #[cfg(all(feature = "napi", feature = "v8-compat"))]
    let v8_runtime_addr = vm
        .ctx
        .with(|ctx| lexe_napi::finalize_v8_environment(&ctx))
        .await
        .map_err(std::io::Error::other)?;

    #[cfg(all(feature = "napi", feature = "v8-compat"))]
    vm.ctx
        .with(|ctx| {
            run_final_context_gc(&ctx);
            Ok::<(), String>(())
        })
        .await
        .map_err(std::io::Error::other)?;

    // Release module/context userdata that caches JS roots and drop the
    // context, tear down V8 runtime state while JSRuntime is still live, then
    // free JSRuntime.
    #[cfg(all(feature = "napi", feature = "v8-compat"))]
    vm.shutdown_with(|| unsafe { v8_compat::shutdown_runtime_addr(v8_runtime_addr) })
        .await
        .map_err(std::io::Error::other)?;

    #[cfg(not(all(feature = "napi", feature = "v8-compat")))]
    vm.shutdown().await;

    Ok(ExitCode::from(EXIT_CODE.load(Ordering::Relaxed)))
}

pub const VERSION_STRING: &str = concat!("lexe v", VERSION, " (", PLATFORM, ", ", ARCH, ")");

struct NodeCliFlags {
    argv: Vec<String>,
    exec_argv: Vec<String>,
    sqlite_enabled: bool,
    no_warnings: bool,
    trace_warnings: bool,
    source_maps_enabled: bool,
    permission_enabled: bool,
    permission_config: lexe_utils::permissions::PermissionConfig,
}

fn push_comma_paths(out: &mut Vec<String>, raw: &str) {
    for part in raw.split(',') {
        let p = part.trim();
        if !p.is_empty() {
            out.push(p.to_string());
        }
    }
}

fn parse_node_cli_flags(raw_argv: Vec<String>) -> NodeCliFlags {
    let mut sqlite_enabled = true;
    let mut no_warnings = false;
    let mut trace_warnings = false;
    let mut source_maps_enabled = false;
    let mut permission_enabled = false;
    let mut permission_config = lexe_utils::permissions::PermissionConfig::default();
    let mut argv = Vec::with_capacity(raw_argv.len());
    let mut exec_argv = Vec::new();

    if let Some(first) = raw_argv.first() {
        argv.push(first.clone());
    }

    let mut i = 1usize;
    let mut skip_next_code = false;
    while i < raw_argv.len() {
        let arg = &raw_argv[i];
        if skip_next_code {
            argv.push(arg.clone());
            skip_next_code = false;
            i += 1;
            continue;
        }
        if arg == "-e" || arg == "--eval" {
            argv.push(arg.clone());
            skip_next_code = true;
            i += 1;
            continue;
        }
        match arg.as_str() {
            "--no-experimental-sqlite" => {
                sqlite_enabled = false;
                exec_argv.push(arg.clone());
            },
            "--no-warnings" => {
                no_warnings = true;
                exec_argv.push(arg.clone());
            },
            "--experimental-vm-modules" => {
                exec_argv.push(arg.clone());
            },
            "--trace-warnings" => {
                trace_warnings = true;
                exec_argv.push(arg.clone());
            },
            "--enable-source-maps" => {
                source_maps_enabled = true;
                exec_argv.push(arg.clone());
            },
            "--permission" => {
                permission_enabled = true;
                exec_argv.push(arg.clone());
            },
            "--allow-child-process" => {
                permission_config.allow_child = true;
                exec_argv.push(arg.clone());
            },
            "--allow-worker" => {
                permission_config.allow_worker = true;
                exec_argv.push(arg.clone());
            },
            "--allow-addons" => {
                permission_config.allow_addon = true;
                exec_argv.push(arg.clone());
            },
            "--allow-wasi" => {
                permission_config.allow_wasi = true;
                exec_argv.push(arg.clone());
            },
            "--allow-fs-read" => {
                exec_argv.push(arg.clone());
                if let Some(next) = raw_argv.get(i + 1) {
                    push_comma_paths(&mut permission_config.fs_read, next);
                    exec_argv.push(next.clone());
                    i += 1;
                } else {
                    permission_config.fs_read.push("*".to_string());
                }
            },
            "--allow-fs-write" => {
                exec_argv.push(arg.clone());
                if let Some(next) = raw_argv.get(i + 1) {
                    push_comma_paths(&mut permission_config.fs_write, next);
                    exec_argv.push(next.clone());
                    i += 1;
                } else {
                    permission_config.fs_write.push("*".to_string());
                }
            },
            _ if arg.starts_with("--allow-fs-read=") => {
                let rest = &arg["--allow-fs-read=".len()..];
                push_comma_paths(&mut permission_config.fs_read, rest);
                exec_argv.push(arg.clone());
            },
            _ if arg.starts_with("--allow-fs-write=") => {
                let rest = &arg["--allow-fs-write=".len()..];
                push_comma_paths(&mut permission_config.fs_write, rest);
                exec_argv.push(arg.clone());
            },
            _ => argv.push(arg.clone()),
        }
        i += 1;
    }

    NodeCliFlags {
        argv,
        exec_argv,
        sqlite_enabled,
        no_warnings,
        trace_warnings,
        source_maps_enabled,
        permission_enabled,
        permission_config,
    }
}

fn print_version() {
    println!("{VERSION_STRING}");
}

fn usage() {
    print_version();
    println!(
        r#"

Usage:
  lexe <filename>
  lexe -v | --version
  lexe -h | --help
  lexe -e | --eval <source>
  lexe --import <file> [--import <file> ...] <main.js>
  lexe compile <input.js> [output] [options]
  lexe test <test_args>
  lexe upgrade [options]

Options:
  -v, --version     Print version information
  -h, --help        Print this help message
  -e, --eval        Evaluate the provided source code

  --import <file>   Load and execute the module before running the main script.
                    This can be used to register module hooks or modify global
                    behavior. Multiple --import options may be provided and are
                    executed in order.

  compile           Compile JS to bytecode (.lbt) or a standalone executable:
                      if [output] is omitted in bytecode mode, <input>.lbt is used.
                      lbt file is expected to be executed by the lexe version
                      that created it.
                      --executable           Create a self-contained executable
                      --sea                  Enable node:sea asset/argv mode for executable
                      --sea-config <json>    Load SEA main/output/assets from a JSON config
                      --asset <key>=<path>   Embed an asset (repeatable with --executable)
                      --assets-dir <dir>     Embed all files in directory recursively
                      -o, --output <name>    Output path or base filename
                      -d, --outdir <dir>     Output directory (default: ./dist with -p)
                      -p, --platform <list>  Comma-separated target platforms:
                                             linux-x64,linux-arm64,linux-x64-musl,linux-arm64-musl,
                                             darwin-x64,darwin-arm64,
                                             windows-x64,windows-arm64
                      --runtime-dir <dir>    Directory containing prebuilt lexe binaries
                                             (or set LEXE_BIN_DIR)

  test              Run tests with provided arguments:
                      <test_args> -d <directory> <test-filter>

  upgrade           Upgrade Lexe in place from GitHub Releases:
                      --version <ver>        Install a specific version (e.g. 0.2.0)
                      --dry-run              Check target version without replacing binary
                      -f, --force            Reinstall even if already on target version
                      -h, --help             Show upgrade options

"#
    );
}

async fn start_cli(vm: &Vm, embedded_bytecode: Option<Vec<u8>>) {
    if let Some(module) = embedded_bytecode {
        let exe = std::env::current_exe().ok();
        return vm.run_bytecode(&module, exe.as_deref()).await;
    }

    let args: Vec<String> = argv();

    if args.len() <= 1 {
        #[cfg(not(target_family = "wasm"))]
        {
            repl::run_repl(&vm.ctx).await;
        }
        #[cfg(target_family = "wasm")]
        {
            usage();
        }
        return;
    }

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--version" => {
                print_version();
                return;
            },
            "-h" | "--help" => {
                usage();
                return;
            },
            "-e" | "--eval" => {
                if i + 1 >= args.len() {
                    eprintln!("-e requires an argument");
                    exit(1);
                }
                let source = &args[i + 1];
                vm.run(source.as_bytes(), false, false).await;
                return;
            },
            "--import" => {
                if i + 1 >= args.len() {
                    eprintln!("--import requires a filename");
                    exit(1);
                }
                let file = &args[i + 1];
                vm.run_file(file, true, true).await;
                i += 2;
                continue;
            },
            "test" => {
                if let Err(error) = run_tests(vm, &args[i + 1..]).await {
                    eprintln!("{error}");
                    exit(1);
                }
                return;
            },
            "compile" => {
                #[cfg(not(target_family = "wasm"))]
                {
                    let compile_args = match executable::parse_compile_args(&args[i + 1..]) {
                        Ok(parsed) => parsed,
                        Err(error) => {
                            eprintln!("{error}");
                            exit(1);
                        },
                    };
                    if compile_args.create_executable {
                        if let Err(error) = executable::run_compile_executable(&compile_args).await
                        {
                            eprintln!("{error}");
                            exit(1);
                        }
                    } else {
                        let output_filename =
                            executable::resolve_bytecode_output_path(&compile_args);
                        if let Some(parent) = output_filename
                            .parent()
                            .filter(|p| !p.as_os_str().is_empty())
                        {
                            if let Err(error) = std::fs::create_dir_all(parent) {
                                eprintln!("{error}");
                                exit(1);
                            }
                        }
                        if let Err(error) =
                            compile_file(&compile_args.input, &output_filename).await
                        {
                            eprintln!("{error}");
                            exit(1);
                        }
                    }
                    return;
                }
                #[cfg(target_family = "wasm")]
                {
                    let rest = &args[i + 1..];
                    if rest.iter().any(|a| {
                        a == "--executable"
                            || a == "-p"
                            || a.starts_with("-p=")
                            || a == "--platform"
                            || a.starts_with("--platform=")
                    }) {
                        eprintln!("compile --executable is not supported on WASI");
                        exit(1);
                    }
                    if let Some(filename) = rest.first() {
                        let output_filename = if let Some(out) = rest.get(1) {
                            PathBuf::from(out)
                        } else {
                            let mut buf = PathBuf::from(filename);
                            buf.set_extension("lbt");
                            buf
                        };
                        if let Err(error) =
                            compile_file(Path::new(filename), &output_filename).await
                        {
                            eprintln!("{error}");
                            exit(1);
                        }
                        return;
                    } else {
                        eprintln!("compile: input filename is required.");
                        exit(1);
                    }
                }
            },
            "upgrade" => {
                #[cfg(not(target_family = "wasm"))]
                {
                    let upgrade_args = match upgrade::parse_upgrade_args(&args[i + 1..]) {
                        Ok(parsed) => parsed,
                        Err(error) => {
                            eprintln!("{error}");
                            exit(1);
                        },
                    };
                    if upgrade_args.help {
                        upgrade::print_upgrade_usage();
                        return;
                    }
                    if let Err(error) = upgrade::run_upgrade(&upgrade_args).await {
                        eprintln!("{error}");
                        exit(1);
                    }
                    return;
                }
                #[cfg(target_family = "wasm")]
                {
                    eprintln!("upgrade is not supported on WASI");
                    exit(1);
                }
            },
            _ => break,
        }
    }

    if i >= args.len() {
        eprintln!("No main script provided");
        exit(1);
    }

    let arg = &args[i];
    let (_, ext) = name_extname(arg);

    let filename = Path::new(arg);
    let file_exists = filename.exists();

    let global = ext == ".cjs";

    if is_supported_ext(ext) {
        if file_exists {
            return vm.run_file(arg, true, global).await;
        } else {
            eprintln!("No such file: {}", arg);
            exit(1);
        }
    } else {
        if file_exists {
            return vm.run_file(arg, true, false).await;
        }
        eprintln!("Unknown command: {}", arg);
        usage();
        exit(1);
    }
}

async fn run_tests(vm: &Vm, args: &[std::string::String]) -> Result<(), String> {
    let mut filters: Vec<&str> = Vec::with_capacity(args.len());

    let mut root = ".";

    let mut skip_next = false;

    for (i, arg) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "-d" {
            if let Some(dir) = args.get(i + 1) {
                if !Path::new(dir).exists() {
                    return Err(["\"", dir.as_str(), "\" does not exist"].concat());
                }
                root = dir;
                skip_next = true;
            }
        } else {
            filters.push(arg)
        }
    }

    let now = Instant::now();

    let mut entries: Vec<String> = Vec::with_capacity(100);
    let has_filters = !filters.is_empty();

    if has_filters {
        trace!("Applying filters: {:?}", filters);
    }

    trace!("Scanning directory \"{}\"", root);

    let mut directory_walker = DirectoryWalker::new(PathBuf::from(root), |name| {
        name != "node_modules" && !name.starts_with('.')
    });
    directory_walker.set_recursive(true);

    let test_js_extensions: Vec<String> = SUPPORTED_EXTENSIONS
        .iter()
        .filter(|&ext| *ext != BYTECODE_EXT)
        .flat_map(|ext| [".test", ".test.lexe"].map(|prefix| [prefix, ext].concat()))
        .collect();

    let pwd = env::current_dir().map_err(|e| e.to_string())?;
    while let Some((entry, _)) = directory_walker.walk().await.map_err(|e| e.to_string())? {
        if let Some(name) = entry.file_name() {
            let name = name.to_string_lossy();
            let name = name.as_ref();
            for ext_name in &test_js_extensions {
                if name.ends_with(ext_name)
                    && (!has_filters || filters.iter().any(|&f| name.contains(f)))
                {
                    let path = if entry.is_absolute() {
                        entry.clone()
                    } else {
                        pwd.join(&entry)
                    };
                    entries.push(path.to_string_lossy().into_owned());
                }
            }
        };
    }

    entries.sort_unstable();

    trace!("Found tests in {}ms", now.elapsed().as_millis());

    vm.run_with(|ctx| {
        ctx.globals().set("__testEntries", entries)?;
        Ok(())
    })
    .await;

    vm.run(
        r#"
        import "lexe:test/index"
    "#,
        false,
        false,
    )
    .await;

    Ok(())
}
