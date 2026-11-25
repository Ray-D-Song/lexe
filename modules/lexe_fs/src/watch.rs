// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

use lexe_context::CtxExtension;
use lexe_events::{Emitter, EventEmitter, EventList};
use lexe_utils::event_loop::{self, EventLoopRef};
use notify::{
    event::ModifyKind, Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use rquickjs::{
    class::{Trace, Tracer},
    prelude::{Opt, This},
    Class, Ctx, Exception, Function, IntoJs, JsLifetime, Result, Value,
};
use tokio::sync::mpsc;

enum WatchMessage {
    Event {
        event_type: &'static str,
        filename: Option<String>,
    },
    Error(String),
}

#[rquickjs::class]
pub struct FSWatcher<'js> {
    emitter: EventEmitter<'js>,
    watcher: Option<RecommendedWatcher>,
    closed: Arc<AtomicBool>,
    loop_ref: Arc<EventLoopRef>,
}

unsafe impl<'js> JsLifetime<'js> for FSWatcher<'js> {
    type Changed<'to> = FSWatcher<'to>;
}

impl<'js> Trace<'js> for FSWatcher<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

impl<'js> Emitter<'js> for FSWatcher<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

#[rquickjs::methods]
impl<'js> FSWatcher<'js> {
    #[qjs(constructor)]
    pub fn constructor(ctx: Ctx<'js>) -> Result<Class<'js, Self>> {
        Err(Exception::throw_type(
            &ctx,
            "FSWatcher cannot be constructed directly.",
        ))
    }

    pub fn close(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        let should_emit = Self::close_inner(&this);
        if should_emit {
            Self::emit_str(this.0, &ctx, "close", vec![], false)?;
        }
        Ok(())
    }

    #[qjs(rename = "ref")]
    pub fn r#ref(this: This<Class<'js, Self>>) -> Result<Class<'js, Self>> {
        this.borrow().loop_ref.reference();
        Ok(this.0)
    }

    pub fn unref(this: This<Class<'js, Self>>) -> Result<Class<'js, Self>> {
        this.borrow().loop_ref.unreference();
        Ok(this.0)
    }
}

impl<'js> FSWatcher<'js> {
    fn new(
        ctx: Ctx<'js>,
        path: String,
        options: WatchOptions,
        listener: Option<Function<'js>>,
    ) -> Result<Class<'js, Self>> {
        let watch_path = PathBuf::from(path);
        let base_path = base_path(&watch_path);
        let closed = Arc::new(AtomicBool::new(false));
        let closed_callback = closed.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();

        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| {
                if closed_callback.load(Ordering::SeqCst) {
                    return;
                }

                let message = match result {
                    Ok(event) => map_event(event, &base_path),
                    Err(err) => WatchMessage::Error(err.to_string()),
                };
                let _ = tx.send(message);
            },
            Config::default(),
        )
        .map_err(|err| Exception::throw_message(&ctx, &err.to_string()))?;

        watcher
            .watch(
                &watch_path,
                if options.recursive {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                },
            )
            .map_err(|err| Exception::throw_message(&ctx, &err.to_string()))?;

        let rt_ptr = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let rt_key = event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
        let loop_ref = EventLoopRef::new_referenced_named(rt_key, "FSWatcher");
        if !options.persistent {
            loop_ref.unreference();
        }
        let encoding_is_buffer = options.encoding.as_deref() == Some("buffer");

        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                watcher: Some(watcher),
                closed,
                loop_ref,
            },
        )?;

        if let Some(listener) = listener {
            Self::add_event_listener_str(instance.clone(), &ctx, "change", listener, false, false)?;
        }

        let instance_for_task = instance.clone();
        let ctx_for_task = ctx.clone();
        let loop_ref = instance.borrow().loop_ref.clone();
        ctx.spawn_exit_simple(async move {
            loop {
                if !event_loop::keeps_event_loop_alive(&loop_ref) {
                    break;
                }

                tokio::select! {
                    _ = event_loop::wait_until_io_unref(loop_ref.clone()) => {
                        break;
                    }
                    message = rx.recv() => {
                        let Some(message) = message else {
                            break;
                        };

                        if instance_for_task.borrow().closed.load(Ordering::SeqCst) {
                            break;
                        }

                        match message {
                            WatchMessage::Event {
                                event_type,
                                filename,
                            } => {
                                let event_type = event_type.into_js(&ctx_for_task)?;
                                let filename = match (encoding_is_buffer, filename) {
                                    (true, Some(name)) => {
                                        lexe_buffer::Buffer(name.into_bytes()).into_js(&ctx_for_task)?
                                    },
                                    (_, other) => other.into_js(&ctx_for_task)?,
                                };
                                Self::emit_str(
                                    instance_for_task.clone(),
                                    &ctx_for_task,
                                    "change",
                                    vec![event_type, filename],
                                    false,
                                )?;
                            },
                            WatchMessage::Error(message) => {
                                let error = Exception::from_message(ctx_for_task.clone(), &message)?;
                                Self::emit_str(
                                    instance_for_task.clone(),
                                    &ctx_for_task,
                                    "error",
                                    vec![error.into()],
                                    false,
                                )?;
                            },
                        }
                    }
                }
            }
            Ok(())
        });

        Ok(instance)
    }

    fn close_inner(this: &This<Class<'js, Self>>) -> bool {
        let mut borrow = this.borrow_mut();
        if borrow.closed.swap(true, Ordering::SeqCst) {
            return false;
        }
        borrow.watcher.take();
        borrow.loop_ref.unreference();
        true
    }
}

struct WatchOptions {
    recursive: bool,
    persistent: bool,
    encoding: Option<String>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            recursive: false,
            persistent: true,
            encoding: None,
        }
    }
}

pub fn watch<'js>(
    ctx: Ctx<'js>,
    path: Value<'js>,
    options_or_listener: Opt<Value<'js>>,
    listener: Opt<Function<'js>>,
) -> Result<Class<'js, FSWatcher<'js>>> {
    let path = crate::realpath::path_from_value(&ctx, path)?;
    let mut options = WatchOptions::default();
    let mut listener = listener.0;
    let mut signal: Option<rquickjs::Object<'js>> = None;

    if let Some(value) = options_or_listener.0 {
        if let Some(function) = value.as_function() {
            listener = Some(function.clone());
        } else if let Some(s) = value.as_string() {
            options.encoding = Some(s.to_string()?);
        } else if let Some(object) = value.as_object() {
            options.recursive = object.get("recursive").unwrap_or_default();
            if let Ok(Some(persistent)) = object.get::<_, Option<bool>>("persistent") {
                options.persistent = persistent;
            }
            if let Ok(Some(enc)) = object.get::<_, Option<String>>("encoding") {
                options.encoding = Some(enc);
            }
            if let Ok(Some(sig)) = object.get::<_, Option<rquickjs::Object>>("signal") {
                signal = Some(sig);
            }
        } else if !value.is_undefined() && !value.is_null() {
            return Err(Exception::throw_type(
                &ctx,
                "The \"options\" argument must be an object, string, or function.",
            ));
        }
    }

    let watcher = FSWatcher::new(ctx.clone(), path, options, listener)?;
    if let Some(sig) = signal {
        let wire: Function<'js> = ctx.eval(
            r#"(function(watcher, signal) {
  function abortErr() {
    const err = new Error("The operation was aborted", { cause: signal.reason });
    err.name = "AbortError";
    err.code = "ABORT_ERR";
    return err;
  }
  if (signal.aborted) {
    queueMicrotask(() => {
      watcher.emit("error", abortErr());
      watcher.close();
    });
    return;
  }
  const onAbort = () => {
    watcher.emit("error", abortErr());
    watcher.close();
  };
  if (typeof signal.addEventListener === "function") {
    signal.addEventListener("abort", onAbort, { once: true });
    watcher.once("close", () => {
      if (typeof signal.removeEventListener === "function") {
        signal.removeEventListener("abort", onAbort);
      }
    });
  }
})"#,
        )?;
        wire.call::<_, ()>((watcher.clone(), sig))?;
    }
    Ok(watcher)
}

fn base_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or_else(|| Path::new("")).to_path_buf()
    }
}

fn map_event(event: Event, base_path: &Path) -> WatchMessage {
    let event_type = match event.kind {
        EventKind::Create(_) | EventKind::Remove(_) => "rename",
        EventKind::Modify(ModifyKind::Name(_))
        | EventKind::Modify(ModifyKind::Any)
        | EventKind::Modify(ModifyKind::Other) => "rename",
        EventKind::Modify(_) => "change",
        EventKind::Any | EventKind::Access(_) | EventKind::Other => "change",
    };

    let filename = event.paths.first().and_then(|path| {
        if let Ok(path) = path.strip_prefix(base_path) {
            Some(path.to_string_lossy().to_string())
        } else {
            path.file_name()
                .map(|filename| filename.to_string_lossy().to_string())
        }
    });

    WatchMessage::Event {
        event_type,
        filename,
    }
}
