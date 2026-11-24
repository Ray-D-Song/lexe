// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::env;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream as StdUnixStream;

use lexe_child_process::install_child_process_ipc;
use lexe_core::modules::module_builder::{ModuleBuilder, ModuleBuilderConfig};
use lexe_core::vm::{Vm, VmOptions};
use tokio::net::UnixStream;
use url::Url;

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let module = env::var("LEXE_FORK_MODULE").map_err(|_| "missing LEXE_FORK_MODULE")?;
    let fd: i32 = env::var("NODE_CHANNEL_FD")
        .map_err(|_| "missing NODE_CHANNEL_FD")?
        .parse()
        .map_err(|_| "invalid NODE_CHANNEL_FD")?;

    let vm = Vm::from_options(VmOptions {
        module_builder: ModuleBuilder::with_config(ModuleBuilderConfig {
            sqlite_enabled: false,
        })
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

    let std_stream = unsafe { StdUnixStream::from_raw_fd(fd) };
    std_stream.set_nonblocking(true)?;
    let stream = UnixStream::from_std(std_stream)?;

    // Install the IPC channel first: the script may use `process.send` and
    // `process.on("message")` during evaluation.
    vm.ctx
        .with(|ctx| install_child_process_ipc(&ctx, stream))
        .await
        .map_err(|e| e.to_string())?;

    let script_path = resolve_script(&module)?;
    let import_url = Url::from_file_path(&script_path)
        .map_err(|_| format!("Path is not absolute: {script_path}"))?;

    // Run the script through the same drain loop the CLI uses, so pending
    // promises, IPC reads, and timers all get driven to completion.
    vm.run_file(import_url.as_str(), true, false).await;
    crate::drain::run_until_process_exit(&vm).await?;

    Ok(())
}

fn resolve_script(script: &str) -> Result<String, String> {
    if let Some(path) = script.strip_prefix("file://") {
        return Ok(path.to_string());
    }
    let path = std::path::PathBuf::from(script);
    if path.is_absolute() {
        return Ok(script.to_string());
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    Ok(cwd.join(path).to_string_lossy().into_owned())
}
