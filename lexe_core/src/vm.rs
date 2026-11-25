// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{env, result::Result as StdResult};

use rquickjs::{
    context::EvalOptions, loader::FileResolver, prelude::Func, AsyncContext, AsyncRuntime,
    CatchResultExt, Ctx, Error, Function, Result, Value,
};

use crate::libs::{
    context::set_spawn_error_handler,
    json,
    logging::print_error_and_exit,
    numbers,
    utils::{
        clone::structured_clone,
        primordials::{BasePrimordials, Primordial},
        time,
    },
};
#[cfg(feature = "async-hooks")]
use crate::modules::async_hooks::promise_hook_tracker;
use crate::modules::{
    embedded::{loader::EmbeddedLoader, resolver::EmbeddedResolver, BYTECODE_CACHE},
    module_builder::{ModuleBuilder, ModuleBuilderConfig},
    package::{loader::PackageLoader, resolver::PackageResolver},
};
use crate::{environment, http, security};

pub struct Vm {
    pub runtime: AsyncRuntime,
    pub ctx: AsyncContext,
    rt_key: usize,
}

pub struct VmOptions {
    pub module_builder: ModuleBuilder,
    pub sqlite_enabled: bool,
    pub max_stack_size: usize,
    pub gc_threshold_mb: usize,
}

impl Default for VmOptions {
    fn default() -> Self {
        let sqlite_enabled = true;
        let module_builder = ModuleBuilder::with_config(ModuleBuilderConfig { sqlite_enabled })
            .with_global(crate::modules::embedded::init)
            .with_global(crate::builtins_inspect::init)
            .with_module(crate::modules::lexe::hex::LexeHexModule)
            .with_module(crate::modules::lexe::qjs::LexeQjsModule)
            .with_module(crate::modules::lexe::util::LexeUtilModule)
            .with_module(crate::modules::lexe::xml::LexeXmlModule);

        Self {
            module_builder,
            sqlite_enabled,
            max_stack_size: 2 * 1024 * 1024,
            gc_threshold_mb: {
                const DEFAULT_GC_THRESHOLD_MB: usize = 20;

                let gc_threshold_mb: usize = env::var(environment::ENV_LEXE_GC_THRESHOLD_MB)
                    .map(|threshold| threshold.parse().unwrap_or(DEFAULT_GC_THRESHOLD_MB))
                    .unwrap_or(DEFAULT_GC_THRESHOLD_MB);

                gc_threshold_mb * 1024 * 1024
            },
        }
    }
}

impl Vm {
    pub async fn from_options(
        vm_options: VmOptions,
    ) -> StdResult<Self, Box<dyn std::error::Error + Send + Sync>> {
        time::init();
        let node_start_ns = time::monotonic_elapsed_nanos();
        http::init()?;
        security::init()?;

        let mut file_resolver = FileResolver::default();
        let mut paths: Vec<&str> = Vec::with_capacity(10);

        paths.push(".");

        if cfg!(debug_assertions) {
            paths.push("bundle");
        } else {
            paths.push("/opt");
        }

        for path in paths.iter() {
            file_resolver.add_path(*path);
        }

        let (module_resolver, module_loader, mut global_attachment) =
            vm_options.module_builder.build();

        // Public embedded bytecode modules (e.g. stream) must be ModuleNames
        // builtins so require() uses load_via_import instead of disk paths.
        // Keep lexe:* private and out of Module.builtinModules.
        for &name in BYTECODE_CACHE.keys() {
            if !name.starts_with("lexe:") {
                global_attachment = global_attachment.add_name(name);
            }
        }

        let resolver = (
            module_resolver,
            EmbeddedResolver,
            PackageResolver,
            file_resolver,
        );
        let loader = (module_loader, EmbeddedLoader, PackageLoader);

        let v8_start_ns = time::monotonic_elapsed_nanos();
        let runtime = AsyncRuntime::new()?;
        runtime.set_max_stack_size(vm_options.max_stack_size).await;
        runtime.set_gc_threshold(vm_options.gc_threshold_mb).await;
        runtime.set_loader(resolver, loader).await;

        let environment_ns = time::monotonic_elapsed_nanos();
        let ctx = AsyncContext::full(&runtime).await?;
        ctx.with(|ctx| {
            let rt = unsafe {
                let rt = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                lexe_utils::shared_array_buffer::install_on_runtime(rt);
                lexe_utils::perf::install_gc_hook(rt);
                rt
            };
            (|| {
                BasePrimordials::init(&ctx)?;
                global_attachment.attach(&ctx)?;
                self::init(&ctx)?;
                let bootstrap_complete_ns = time::monotonic_elapsed_nanos();
                lexe_utils::perf::set_boot_milestones(
                    rt as usize,
                    node_start_ns,
                    v8_start_ns,
                    environment_ns,
                    bootstrap_complete_ns,
                );
                Ok(())
            })()
            .catch(&ctx)
            .unwrap_or_else(|err| print_error_and_exit(&ctx, err));
            Ok::<_, Error>(())
        })
        .await?;

        // Always register the Promise hook when async-hooks is built in.
        // The hook fast-paths when no AsyncLocalStorage consumers exist and
        // user createHook tracking is inactive (LEXE_ASYNC_HOOKS).
        #[cfg(feature = "async-hooks")]
        runtime.set_promise_hook(Some(promise_hook_tracker())).await;

        // Unhandled promise rejections must not look like a successful idle exit.
        // Defer emit + default handling to the next macrotask so same-turn
        // `.catch()` can cancel before `unhandledRejection` fires (Node-like).
        // `rejectionHandled` is not emitted yet (follow-up compat item).
        runtime
            .set_host_promise_rejection_tracker(Some(Box::new(
                |ctx, promise, reason, is_handled| {
                    let _ = track_promise_rejection(ctx, promise, reason, is_handled);
                },
            )))
            .await;

        let rt_key = ctx
            .with(|c| {
                Ok::<_, Error>(lexe_utils::event_loop::rt_key_from_ptr(unsafe {
                    rquickjs::qjs::JS_GetRuntime(c.as_raw().as_ptr()) as *mut std::ffi::c_void
                }))
            })
            .await?;

        Ok(Vm {
            runtime,
            ctx,
            rt_key,
        })
    }

    pub fn js_runtime_key(&self) -> usize {
        self.rt_key
    }

    pub async fn new() -> StdResult<Self, Box<dyn std::error::Error + Send + Sync>> {
        let vm = Self::from_options(VmOptions::default()).await?;
        Ok(vm)
    }

    pub async fn run_with<F>(&self, f: F)
    where
        F: for<'js> FnOnce(&Ctx<'js>) -> Result<()> + std::marker::Send,
    {
        let rt_key = self.rt_key;
        self.ctx
            .with(|ctx| {
                let _active = lexe_utils::perf::ActiveTurnGuard::enter(rt_key);
                if let Err(err) = f(&ctx).catch(&ctx) {
                    handle_uncaught_error(&ctx, err);
                }
            })
            .await;
    }

    pub async fn run<S: Into<Vec<u8>> + Send>(&self, source: S, strict: bool, global: bool) {
        self.run_with(|ctx| {
            let mut options = EvalOptions::default();
            options.strict = strict;
            options.promise = true;
            options.global = global;
            let res = ctx.eval_with_options::<Value, _>(source, options)?;
            if res.is_promise() {
                let suppress = rejection_tracker_cache(ctx)?.suppress_reason;
                if let Ok(catcher) = ctx.eval::<Function, _>(
                    r#"(p, suppress) => {
  if (p && typeof p.then === "function") {
    p.then(undefined, (e) => {
      suppress(e);
      if (typeof process !== "undefined" && process?.[Symbol.for("lexe.invokeUncaughtCapture")]?.(e)) {
        return;
      }
      console.error(e);
      if (typeof process !== "undefined") process.exitCode = 1;
    });
  }
}"#,
                ) {
                    let _ = catcher.call::<_, ()>((res, suppress));
                }
            }
            Ok::<_, Error>(())
        })
        .await;
    }

    pub async fn run_file(&self, filename: impl AsRef<str>, strict: bool, global: bool) {
        // Await the dynamic import so top-level evaluation errors reject this
        // promise (handled below). Nested async work still relies on the host
        // promise rejection tracker for unhandled rejections.
        let source = [
            r#"await import(""#,
            &filename.as_ref().replace('\\', "/"),
            r#"").catch((e) => { if (typeof process !== "undefined" && process?.[Symbol.for("lexe.invokeUncaughtCapture")]?.(e)) return; console.error(e); process.exit(1); })"#,
        ]
        .concat();

        self.run(source, strict, global).await;
    }

    pub async fn run_bytecode(&self, bytecode: &[u8]) {
        let bytecode = bytecode.to_vec();
        let rt_key = self.rt_key;
        self.ctx
            .async_with(async move |ctx| {
                let result = async {
                    let promise = {
                        let _active = lexe_utils::perf::ActiveTurnGuard::enter(rt_key);
                        let module = EmbeddedLoader::load_bytecode_module(ctx.clone(), &bytecode)
                            .map_err(|err| {
                            eprintln!("Failed to evaluate module: {err:?}");
                            err
                        })?;
                        let (_module, promise) = module.eval()?;
                        promise
                    };
                    promise.into_future::<()>().await?;
                    Ok::<_, Error>(())
                }
                .await
                .catch(&ctx);

                if let Err(err) = result {
                    handle_uncaught_error(&ctx, err);
                }
            })
            .await;
    }

    pub async fn idle(&self) -> StdResult<(), Box<dyn std::error::Error + Sync + Send>> {
        IdleInstrumentedFuture {
            rt_key: self.rt_key,
            inner: self.runtime.idle(),
            pending_since: None,
        }
        .await;
        Ok(())
    }

    /// Tear the VM down in the only safe order.
    ///
    /// QuickJS asserts that every GC object has been released when the runtime
    /// is freed, so context userdata that caches JS values (module/require
    /// state, primordials, the rejection tracker) must be released, and the
    /// context must be dropped before the runtime. Field order alone would drop
    /// the runtime first.
    pub async fn shutdown(self) {
        let rt_key = self.rt_key;
        let _ = self
            .ctx
            .with(|ctx| {
                unsafe {
                    let rt = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                    lexe_utils::perf::uninstall_gc_hook(rt);
                }
                lexe_utils::event_loop::unregister_runtime(rt_key);
                let _ = shutdown_module_state(&ctx);
                let _ = shutdown_context_userdata(&ctx);
                Ok::<(), Error>(())
            })
            .await;

        let Vm { ctx, runtime, .. } = self;
        drop(ctx);
        drop(runtime);
    }
}

struct IdleInstrumentedFuture<F> {
    rt_key: usize,
    inner: F,
    pending_since: Option<u64>,
}

impl<F> Drop for IdleInstrumentedFuture<F> {
    fn drop(&mut self) {
        if let Some(since) = self.pending_since.take() {
            let elapsed = time::monotonic_elapsed_nanos().saturating_sub(since);
            lexe_utils::perf::add_idle_ns(self.rt_key, elapsed);
        }
    }
}

impl<F: std::future::Future<Output = ()>> std::future::Future for IdleInstrumentedFuture<F> {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // SAFETY: `rt_key` and `pending_since` are Unpin; `inner` remains pinned in place.
        let this = unsafe { self.get_unchecked_mut() };
        lexe_utils::perf::mark_loop_start_once(this.rt_key);
        if let Some(since) = this.pending_since.take() {
            let elapsed = time::monotonic_elapsed_nanos().saturating_sub(since);
            lexe_utils::perf::add_idle_ns(this.rt_key, elapsed);
        }
        lexe_utils::perf::inc_loop_count(this.rt_key);
        let poll_res = {
            let _active = lexe_utils::perf::ActiveTurnGuard::enter(this.rt_key);
            let inner = unsafe { std::pin::Pin::new_unchecked(&mut this.inner) };
            inner.poll(cx)
        };
        if poll_res.is_pending() {
            this.pending_since = Some(time::monotonic_elapsed_nanos());
        }
        poll_res
    }
}

fn track_promise_rejection<'js>(
    ctx: Ctx<'js>,
    promise: Value<'js>,
    reason: Value<'js>,
    is_handled: bool,
) -> Result<()> {
    let cache = rejection_tracker_cache(&ctx)?;
    cache.track.call::<_, ()>((promise, reason, is_handled))?;
    Ok(())
}

/// Private ctx userdata holding the deferred rejection tracker functions.
/// Pending map / schedule state live in the JS closure — not on `globalThis`.
#[derive(Clone, rquickjs::JsLifetime)]
struct RejectionTrackerCache<'js> {
    track: Function<'js>,
    suppress_reason: Function<'js>,
}

fn rejection_tracker_cache<'js>(ctx: &Ctx<'js>) -> Result<RejectionTrackerCache<'js>> {
    if let Some(cached) = ctx.userdata::<RejectionTrackerCache>() {
        return Ok(cached.clone());
    }

    // Node-like: only emit / default-log after a macrotask, so same-turn `.catch()`
    // can cancel. `rejectionHandled` is intentionally not emitted yet.
    let obj: rquickjs::Object = ctx.eval(
        r#"
(() => {
  const pending = new Map();
  const suppressed = new Set();
  let scheduled = false;
  const flush = () => {
    scheduled = false;
    for (const [promise, reason] of pending) {
      pending.delete(promise);
      if (suppressed.has(reason)) {
        suppressed.delete(reason);
        continue;
      }
      let handled = false;
      try {
        handled = process.emit("unhandledRejection", reason, promise) === true;
      } catch (_) {}
      if (handled) continue;
      try {
        console.error("UnhandledPromiseRejection:", reason);
      } catch (_) {}
      try {
        process.exitCode = 1;
      } catch (_) {}
    }
    suppressed.clear();
  };
  const schedule = () => {
    if (scheduled) return;
    scheduled = true;
    if (typeof setImmediate === "function") setImmediate(flush);
    else if (typeof setTimeout === "function") setTimeout(flush, 0);
    else Promise.resolve().then(flush);
  };
  return {
    track(promise, reason, isHandled) {
      if (isHandled || suppressed.has(reason)) {
        pending.delete(promise);
        return;
      }
      pending.set(promise, reason);
      schedule();
    },
    suppressReason(reason) {
      suppressed.add(reason);
      for (const [promise, r] of pending) {
        if (r === reason) pending.delete(promise);
      }
      schedule();
    },
  };
})()
"#,
    )?;

    let cache = RejectionTrackerCache {
        track: obj.get("track")?,
        suppress_reason: obj.get("suppressReason")?,
    };
    let _ = ctx.store_userdata(cache.clone());
    if let Some(cached) = ctx.userdata::<RejectionTrackerCache>() {
        return Ok(cached.clone());
    }
    Ok(cache)
}

fn handle_uncaught_error<'js>(ctx: &Ctx<'js>, err: rquickjs::CaughtError<'js>) {
    if let rquickjs::CaughtError::Exception(ref ex) = err {
        #[cfg(feature = "process")]
        if crate::modules::process::try_invoke_uncaught_exception_capture(ctx, ex.clone().into()) {
            return;
        }
    }
    print_error_and_exit(ctx, err);
}

fn init(ctx: &Ctx<'_>) -> Result<()> {
    set_spawn_error_handler(|ctx, err| {
        handle_uncaught_error(ctx, err);
    });

    let globals = ctx.globals();

    globals.set(
        "__gc",
        Func::from(|ctx: Ctx| {
            ctx.run_gc();
            lexe_utils::perf::flush_pending_gc(&ctx);
        }),
    )?;
    globals.set("global", ctx.globals())?;
    globals.set("self", ctx.globals())?;
    globals.set(
        "structuredClone",
        Func::from(|ctx, value, options| structured_clone(&ctx, value, options)),
    )?;

    numbers::redefine_prototype(ctx)?;
    json::redefine_static_methods(ctx)?;

    Ok(())
}

/// Tear down module-related JS/Rust state before V8 bridge teardown.
pub fn shutdown_module_state<'js>(ctx: &Ctx<'js>) -> Result<()> {
    crate::modules::module::shutdown_context_state(ctx)
}

/// Tear down non-module context userdata before V8 bridge teardown.
pub fn shutdown_context_userdata<'js>(ctx: &Ctx<'js>) -> Result<()> {
    if ctx.userdata::<BasePrimordials>().is_some() {
        ctx.remove_userdata::<BasePrimordials>()
            .map_err(|_| Error::Unknown)?;
        if ctx.userdata::<BasePrimordials>().is_some() {
            return Err(Error::Unknown);
        }
    }
    if ctx.userdata::<RejectionTrackerCache>().is_some() {
        ctx.remove_userdata::<RejectionTrackerCache>()
            .map_err(|_| Error::Unknown)?;
        if ctx.userdata::<RejectionTrackerCache>().is_some() {
            return Err(Error::Unknown);
        }
    }
    if ctx
        .userdata::<crate::modules::module::ModuleNames>()
        .is_some()
    {
        ctx.remove_userdata::<crate::modules::module::ModuleNames>()
            .map_err(|_| Error::Unknown)?;
        if ctx
            .userdata::<crate::modules::module::ModuleNames>()
            .is_some()
        {
            return Err(Error::Unknown);
        }
    }
    if ctx.userdata::<lexe_hooking::AsyncTrackingState>().is_some() {
        ctx.remove_userdata::<lexe_hooking::AsyncTrackingState>()
            .map_err(|_| Error::Unknown)?;
        if ctx.userdata::<lexe_hooking::AsyncTrackingState>().is_some() {
            return Err(Error::Unknown);
        }
    }

    Ok(())
}

/// Tear down context userdata that retains `Ctx` roots before `AsyncContext` drop.
pub fn shutdown_context<'js>(ctx: &Ctx<'js>) -> Result<()> {
    use std::cell::RefCell;

    use crate::modules::module::RequireState;

    if ctx.userdata::<RefCell<RequireState>>().is_some() {
        shutdown_module_state(ctx)?;
    }

    shutdown_context_userdata(ctx)
}

/// Final GC sweep before V8 bridge teardown (prefer [`lexe_napi::shutdown_environment`]).
pub fn run_final_context_gc<'js>(ctx: &Ctx<'js>) {
    let rt = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    for _ in 0..16 {
        ctx.run_gc();
        unsafe {
            rquickjs::qjs::JS_RunGC(rt);
        }
    }
}
