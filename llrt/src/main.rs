// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
    process::exit,
    time::Instant,
};

mod build;
mod core;
mod minimal_tracer;
#[cfg(not(feature = "lambda"))]
mod repl;

use build::{extract_code_binary, has_magic_number, LexeBuild, Platform};
use constcat::concat;
use minimal_tracer::MinimalTracer;
use tracing::trace;

#[cfg(not(feature = "lambda"))]
use crate::core::compiler::compile_file;
use crate::core::{
    bytecode::BYTECODE_EXT,
    libs::utils::{
        fs::DirectoryWalker,
        sysinfo::{ARCH, PLATFORM},
    },
    modules::path::name_extname,
    runtime_client,
    utils::io::{is_supported_ext, SUPPORTED_EXTENSIONS},
    vm::Vm,
    VERSION,
};

// rquickjs components
use crate::core::{async_with, CatchResultExt};

#[cfg(not(target_os = "windows"))]
#[global_allocator]
static ALLOC: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let now = Instant::now();

    MinimalTracer::register()?;
    trace!("Started runtime");

    let vm = Vm::new().await?;
    trace!("Initialized VM in {}ms", now.elapsed().as_millis());

    if env::var("AWS_LAMBDA_RUNTIME_API").is_ok() && env::var("_HANDLER").is_ok() {
        start_runtime(&vm).await
    } else {
        start_cli(&vm).await;
    }

    vm.idle().await?;

    Ok(())
}

pub const VERSION_STRING: &str = concat!("LLRT v", VERSION, " (", PLATFORM, ", ", ARCH, ")");

fn print_version() {
    println!("{VERSION_STRING}");
}

fn usage() {
    print_version();
    println!(
        r#"

Usage:
  llrt <filename>
  llrt -v | --version
  llrt -h | --help
  llrt -e | --eval <source>
  llrt compile input.js [output.lrt]
  llrt test <test_args>
  llrt build <build_args>

Options:
  -v, --version     Print version information
  -h, --help        Print this help message
  -e, --eval        Evaluate the provided source code
  compile           Compile JS to bytecode and compress it with zstd:
                      if [output.lrt] is omitted, <input>.lrt is used.
                      lrt file is expected to be executed by the llrt version
                      that created it
  test              Run tests with provided arguments:
                      <test_args> -d <directory> <test-filter>
  build             This command will create a standalone executable from the source code.
                    build_args:
                      -i      input file(required)
                      -o      output file(optional, default: <input file name>-<platform>)
                      -d      output directory(optional, default: ./dist)
                      -p      target platform, use "," to separate multiple platforms
                                options: linux-x64,linux-arm64,darwin-x64,darwin-arm64,windows-x64
                                (optional, default: current platform)
                    
                    like: llrt build -i=input.js -o=binary -d=dist -p=linux-x64,windows-x64
"#
    );
}

async fn start_runtime(vm: &Vm) {
    async_with!(vm.ctx => |ctx|{
        if let Err(err) = runtime_client::start(&ctx).await.catch(&ctx) {
            Vm::print_error_and_exit(&ctx, err)
        }
    })
    .await;
}

async fn start_cli(vm: &Vm) {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        for (i, arg) in args.iter().enumerate() {
            let arg = arg.as_str();
            if i == 1 {
                match arg {
                    "-v" | "--version" => {
                        print_version();
                        return;
                    },
                    "-h" | "--help" => {
                        usage();
                        return;
                    },
                    "-e" | "--eval" => {
                        if let Some(source) = args.get(i + 1) {
                            vm.run(source.as_bytes(), false, false).await;
                        }
                        return;
                    },
                    "test" => {
                        if let Err(error) = run_tests(vm, &args[i + 1..]).await {
                            eprintln!("{error}");
                            exit(1);
                        }
                        return;
                    },
                    "compile" => {
                        #[cfg(not(feature = "lambda"))]
                        {
                            if let Some(filename) = args.get(i + 1) {
                                let output_filename = if let Some(arg) = args.get(i + 2) {
                                    arg.to_string()
                                } else {
                                    let mut buf = PathBuf::from(filename);
                                    buf.set_extension("lrt");
                                    buf.to_string_lossy().to_string()
                                };

                                let filename = Path::new(filename);
                                let output_filename = Path::new(&output_filename);
                                if let Err(error) = compile_file(filename, output_filename).await {
                                    eprintln!("{error}");
                                    exit(1);
                                }
                                return;
                            } else {
                                eprintln!("compile: input filename is required.");
                                exit(1);
                            }
                        }
                        #[cfg(feature = "lambda")]
                        {
                            eprintln!("Not supported in \"lambda\" version.");
                            exit(1);
                        }
                    },
                    "build" => {
                        #[cfg(not(feature = "lambda"))]
                        {
                            let build_args = args[i + 1..].to_vec();
                            match LexeBuild::validate_args(&build_args) {
                                Ok(args) => {
                                    if let Err(err) = args.run_build().await {
                                        eprintln!("{}", err);
                                        exit(1);
                                    }
                                },
                                Err(err) => {
                                    eprintln!("{}", err);
                                    exit(1);
                                },
                            }
                            exit(1);
                        }
                        #[cfg(feature = "lambda")]
                        {
                            eprintln!("Not supported in \"lambda\" version.");
                            exit(1);
                        }
                    },
                    _ => {},
                }

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
        }
    } else {
        #[cfg(not(feature = "lambda"))]
        {
            let has_magic_number = match has_magic_number(&Platform::current()) {
                Ok(has_magic_number) => has_magic_number,
                Err(_) => false,
            };
            if has_magic_number {
                // extract code binary by libsui
                let code_binary = extract_code_binary();
                if let Some(code_binary) = code_binary {
                    vm.run_with(|ctx| {
                        let module = llrt_core::modules::require::loader::CustomLoader::load_bytecode_module(ctx.clone(), &code_binary)?;
                        module.eval()?;
                        Ok(())
                    }).await;
                }
            } else {
                repl::run_repl(&vm.ctx).await;
            }
        }

        #[cfg(feature = "lambda")]
        {
            eprintln!("REPL not supported in \"lambda\" version.");
            exit(1);
        }
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
        .map(|ext| [".test", ext].concat())
        .collect();

    let pwd = env::current_dir().map_err(|e| e.to_string())?;
    let pwd = pwd.to_string_lossy();
    while let Some((entry, _)) = directory_walker.walk().await.map_err(|e| e.to_string())? {
        if let Some(name) = entry.file_name() {
            let name = name.to_string_lossy();
            let name = name.as_ref();
            for ext_name in &test_js_extensions {
                if name.ends_with(ext_name)
                    && (!has_filters || filters.iter().any(|&f| name.contains(f)))
                {
                    entries.push([pwd.as_ref(), "/", entry.to_string_lossy().as_ref()].concat());
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
        import "llrt:test/index"
    "#,
        false,
        false,
    )
    .await;

    Ok(())
}