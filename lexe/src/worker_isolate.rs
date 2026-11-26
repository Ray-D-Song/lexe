// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_core::modules::module_builder::{ModuleBuilder, ModuleBuilderConfig};
use lexe_core::vm::{Vm, VmOptions};
use lexe_utils::clone_wire::deserialize;
use lexe_worker_threads::{
    set_worker_isolate_runner, set_worker_thread_id, MainToWorker, WorkerIsolateStart, WorkerToMain,
};
use rquickjs::{prelude::Func, CatchResultExt, CaughtError, Ctx, Function, Promise};

use url::Url;

pub fn register() {
    set_worker_isolate_runner(run_worker_isolate);
}

fn run_worker_isolate(start: WorkerIsolateStart) {
    set_worker_thread_id(start.thread_id);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("worker tokio runtime");
    if let Err(err) = rt.block_on(run_worker_isolate_async(start)) {
        eprintln!("worker isolate error: {err}");
    }
}

async fn run_worker_isolate_async(start: WorkerIsolateStart) -> Result<(), String> {
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
    .await
    .map_err(|e| e.to_string())?;

    let to_main = start.to_main.clone();
    let from_main = start.from_main;

    let run_result: Result<(), String> = vm
        .ctx
        .async_with(async move |ctx| {
            lexe_worker_threads::init(&ctx).map_err(|e| e.to_string())?;

            let worker_data = deserialize(&ctx, &start.worker_data).map_err(|e| e.to_string())?;
            let env_data = deserialize(&ctx, &start.env_snapshot).map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__lexeWorkerBootstrapData", worker_data)
                .map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__lexeWorkerEnvironmentSnapshot", env_data)
                .map_err(|e| e.to_string())?;

            lexe_worker_threads::set_worker_parent_channel(to_main.clone());
            ctx.globals()
                .set(
                    "__lexeWorkerParentPostMessage",
                    Func::from(lexe_worker_threads::worker_parent_post_message_hook),
                )
                .map_err(|e| e.to_string())?;

            let body = if start.eval {
                // `new Worker(code, { eval: true })`: run the string directly.
                start.script.clone()
            } else if start.script.len() >= 5 && start.script[..5].eq_ignore_ascii_case("data:") {
                format!("await import({:?});", start.script)
            } else {
                let script_path = resolve_script(&ctx, &start.script)?;
                let import_url = Url::from_file_path(&script_path)
                    .map_err(|_| format!("Path is not absolute: {script_path}"))?;
                format!("await import({:?});", import_url.as_str())
            };

            let code = format!(
                r#"
globalThis.workerData = globalThis.__lexeWorkerBootstrapData;
delete globalThis.__lexeWorkerBootstrapData;
(function () {{
  const snap = globalThis.__lexeWorkerEnvironmentSnapshot;
  if (snap && typeof snap === "object") {{
    const m = (globalThis.__lexeWorkerEnvBootstrap = new Map());
    for (const [k, v] of Object.entries(snap)) m.set(k, v);
  }}
  delete globalThis.__lexeWorkerEnvironmentSnapshot;
}})();
{body}
"#
            );

            let mut opts = rquickjs::context::EvalOptions::default();
            opts.promise = true;
            let promise = ctx
                .eval_with_options::<Promise, _>(code, opts)
                .map_err(|e| e.to_string())?;
            if let Err(err) = promise.into_future::<()>().await.catch(&ctx) {
                let message = match err {
                    CaughtError::Exception(exc) => exc.to_string(),
                    CaughtError::Value(val) => format!("Uncaught {val:?}"),
                    CaughtError::Error(e) => e.to_string(),
                };
                let _ = to_main.send(WorkerToMain::Error(message.clone()));
                return Err(message);
            }

            let _ = to_main.send(WorkerToMain::Online);

            // `eval` workers have no channel listener by construction, so they
            // finish as soon as the script settles (Node semantics). A file
            // worker keeps waiting for messages until told to terminate.
            if start.eval {
                let _ = to_main.send(WorkerToMain::Exit {
                    code: 0,
                    signal: None,
                });
                return Ok(());
            }

            loop {
                match from_main.recv() {
                    Ok(MainToWorker::Terminate) => {
                        let _ = to_main.send(WorkerToMain::Exit {
                            code: 0,
                            signal: None,
                        });
                        break;
                    },
                    Ok(MainToWorker::Message(wire)) => {
                        if let Ok(v) = deserialize(&ctx, &wire) {
                            let _ = ctx
                                .globals()
                                .get::<_, Function>("__lexeWorkerDispatchParentMessage")
                                .and_then(|f| f.call::<_, ()>((v,)));
                        }
                    },
                    Ok(MainToWorker::EnvPatch { key, value }) => {
                        if let (Ok(k), Ok(v)) = (deserialize(&ctx, &key), deserialize(&ctx, &value))
                        {
                            let _ = ctx
                                .globals()
                                .get::<_, Function>("__lexeWorkerApplyEnvPatch")
                                .and_then(|f| f.call::<_, ()>((k, v)));
                        }
                    },
                    Err(_) => break,
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string());

    if run_result.is_ok() {
        let _ = vm.idle().await;
    }
    vm.shutdown().await;
    run_result
}

fn resolve_script(_ctx: &Ctx<'_>, script: &str) -> Result<String, String> {
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
