// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{cell::RefCell, collections::HashMap, marker::PhantomData, rc::Rc};

use lexe_hooking::{register_finalization_registry, set_async_tracking_active, AsyncTrackingState};
use lexe_utils::{
    module::{export_default, ModuleInfo},
    result::ResultExt,
};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, This},
    promise::PromiseHookType,
    qjs,
    runtime::PromiseHook,
    Ctx, Exception, Function, JsLifetime, Object, Result, Value,
};
use tracing::trace;

mod finalization_registry;

use crate::finalization_registry::init_finalization_registry;

type AsyncId = u64;
type TriggerAsyncId = i64;

const BOOTSTRAP_CONTEXT: (AsyncId, TriggerAsyncId) = (1, 1);

/// User-facing hooks from `createHook()`. Dispatched whenever at least one
/// hook has been enabled via `.enable()`.
struct Hook<'js> {
    enabled: Rc<RefCell<bool>>,
    init: Option<Function<'js>>,
    before: Option<Function<'js>>,
    after: Option<Function<'js>>,
    promise_resolve: Option<Function<'js>>,
    destroy: Option<Function<'js>>,
}

/// Internal hooks for AsyncLocalStorage (and other runtime consumers).
/// Always dispatched when tracking is active.
struct InternalHook<'js> {
    init: Option<Function<'js>>,
    before: Option<Function<'js>>,
    after: Option<Function<'js>>,
    destroy: Option<Function<'js>>,
}

struct AsyncHookState<'js> {
    hooks: Vec<Hook<'js>>,
    internal_hooks: Vec<InternalHook<'js>>,
    /// Number of enabled AsyncLocalStorage (or other internal) consumers.
    internal_consumer_count: usize,
    /// Number of currently enabled user hooks from `createHook()`.
    enabled_user_hook_count: usize,
}

impl Default for AsyncHookState<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncHookState<'_> {
    fn new() -> Self {
        Self {
            hooks: Vec::new(),
            internal_hooks: Vec::new(),
            internal_consumer_count: 0,
            enabled_user_hook_count: 0,
        }
    }
}

unsafe impl<'js> JsLifetime<'js> for AsyncHookState<'js> {
    type Changed<'to> = AsyncHookState<'to>;
}

struct AsyncHookIds<'js> {
    next_async_id: AsyncId,
    id_map: HashMap<usize, (AsyncId, TriggerAsyncId)>,
    current_id: (AsyncId, TriggerAsyncId),
    scope_stack: Vec<(AsyncId, TriggerAsyncId, Option<Object<'js>>)>,
    current_resource: Option<Object<'js>>,
    bootstrap_resource: Option<Object<'js>>,
    fallback_resources: HashMap<AsyncId, Object<'js>>,
    _marker: PhantomData<&'js ()>,
}

impl Default for AsyncHookIds<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncHookIds<'_> {
    fn new() -> Self {
        Self {
            next_async_id: 1,
            id_map: HashMap::new(),
            current_id: BOOTSTRAP_CONTEXT,
            scope_stack: Vec::new(),
            current_resource: None,
            bootstrap_resource: None,
            fallback_resources: HashMap::new(),
            _marker: PhantomData,
        }
    }
}

unsafe impl<'js> JsLifetime<'js> for AsyncHookIds<'js> {
    type Changed<'to> = AsyncHookIds<'to>;
}

/// Shared ID allocator for promises, timers, DNS, and AsyncResource.
/// Skips 0 and the bootstrap ID 1.
fn allocate_async_id(ids: &mut AsyncHookIds<'_>) -> AsyncId {
    loop {
        ids.next_async_id = ids.next_async_id.wrapping_add(1);
        let id = ids.next_async_id;
        if id > 1 {
            return id;
        }
    }
}

/// Tracking is active when internal consumers or enabled `createHook()`
/// instances exist. When both counts are zero, promise/async Init fast-paths
/// skip tracking completely.
fn should_track(state: &AsyncHookState<'_>) -> bool {
    state.internal_consumer_count > 0 || state.enabled_user_hook_count > 0
}

fn refresh_tracking_flag(ctx: &Ctx<'_>) -> Result<()> {
    let active = {
        let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(ctx)?;
        let state = bind_state.borrow();
        should_track(&state)
    };
    set_async_tracking_active(ctx, active);
    Ok(())
}

fn create_hook<'js>(ctx: Ctx<'js>, hooks_obj: Object<'js>) -> Result<Value<'js>> {
    let init = hooks_obj.get::<_, Function>("init").ok();
    let before = hooks_obj.get::<_, Function>("before").ok();
    let after = hooks_obj.get::<_, Function>("after").ok();
    let promise_resolve = hooks_obj.get::<_, Function>("promiseResolve").ok();
    let destroy = hooks_obj.get::<_, Function>("destroy").ok();
    let enabled = Rc::new(RefCell::new(false));

    let hook = Hook {
        enabled: enabled.clone(),
        init,
        before,
        after,
        promise_resolve,
        destroy,
    };

    {
        let binding = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
        let mut state = binding.borrow_mut();
        state.hooks.push(hook);
    }

    let obj = Object::new(ctx.clone())?;
    {
        let enabled_clone = enabled.clone();
        obj.set(
            "enable",
            Function::new(
                ctx.clone(),
                move |this: This<Value<'js>>, ctx: Ctx<'js>| -> Result<Value<'js>> {
                    let mut is_enabled = enabled_clone.borrow_mut();
                    if !*is_enabled {
                        *is_enabled = true;
                        drop(is_enabled);
                        {
                            let binding =
                                ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
                            binding.borrow_mut().enabled_user_hook_count += 1;
                        }
                        refresh_tracking_flag(&ctx)?;
                    }
                    Ok(this.0)
                },
            ),
        )?;
    }
    {
        let enabled_clone = enabled.clone();
        obj.set(
            "disable",
            Function::new(
                ctx.clone(),
                move |this: This<Value<'js>>, ctx: Ctx<'js>| -> Result<Value<'js>> {
                    let mut is_enabled = enabled_clone.borrow_mut();
                    if *is_enabled {
                        *is_enabled = false;
                        drop(is_enabled);
                        {
                            let binding =
                                ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
                            let mut state = binding.borrow_mut();
                            state.enabled_user_hook_count =
                                state.enabled_user_hook_count.saturating_sub(1);
                        }
                        refresh_tracking_flag(&ctx)?;
                    }
                    Ok(this.0)
                },
            ),
        )?;
    }

    Ok(obj.into())
}

/// Register an internal hook used by AsyncLocalStorage.
fn register_internal_hook<'js>(ctx: Ctx<'js>, hooks_obj: Object<'js>) -> Result<()> {
    let hook = InternalHook {
        init: hooks_obj.get::<_, Function>("init").ok(),
        before: hooks_obj.get::<_, Function>("before").ok(),
        after: hooks_obj.get::<_, Function>("after").ok(),
        destroy: hooks_obj.get::<_, Function>("destroy").ok(),
    };
    {
        let binding = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
        let mut state = binding.borrow_mut();
        state.internal_hooks.push(hook);
    }
    Ok(())
}

/// Adjust internal consumer count (AsyncLocalStorage enable/disable).
fn set_internal_consumer_count(ctx: Ctx<'_>, count: usize) -> Result<()> {
    {
        let binding = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
        let mut state = binding.borrow_mut();
        state.internal_consumer_count = count;
    }
    refresh_tracking_flag(&ctx)?;
    Ok(())
}

fn current_id(ctx: Ctx<'_>) -> Result<u64> {
    execution_async_id(ctx)
}

fn execution_async_id(ctx: Ctx<'_>) -> Result<u64> {
    let bind_ids = ctx.userdata::<RefCell<AsyncHookIds>>().or_throw(&ctx)?;
    let ids = bind_ids.borrow();
    Ok(ids.current_id.0)
}

fn execution_async_resource<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let bind_ids = ctx
        .userdata::<RefCell<AsyncHookIds<'js>>>()
        .or_throw(&ctx)?;
    let mut ids = bind_ids.borrow_mut();
    if let Some(res) = ids.current_resource.as_ref() {
        return Ok(res.clone());
    }
    let async_id = ids.current_id.0;
    if async_id <= 1 {
        if let Some(res) = ids.bootstrap_resource.as_ref() {
            return Ok(res.clone());
        }
        let res = Object::new(ctx.clone())?;
        ids.bootstrap_resource = Some(res.clone());
        return Ok(res);
    }
    if let Some(res) = ids.fallback_resources.get(&async_id) {
        return Ok(res.clone());
    }
    let res = Object::new(ctx.clone())?;
    ids.fallback_resources.insert(async_id, res.clone());
    Ok(res)
}

fn trigger_async_id(ctx: Ctx<'_>) -> Result<i64> {
    let bind_ids = ctx.userdata::<RefCell<AsyncHookIds>>().or_throw(&ctx)?;
    let ids = bind_ids.borrow();
    Ok(ids.current_id.1)
}

#[derive(Clone, Copy)]
enum HookKind {
    Init,
    Before,
    After,
    PromiseResolve,
    Destroy,
}

/// Collect enabled user hook callbacks of one kind.
fn collect_enabled_user_hooks<'js>(
    state: &AsyncHookState<'js>,
    kind: HookKind,
) -> Vec<Function<'js>> {
    let mut funcs = Vec::new();
    for hook in &state.hooks {
        if !*hook.enabled.as_ref().borrow() {
            continue;
        }
        let func = match kind {
            HookKind::Init => hook.init.as_ref(),
            HookKind::Before => hook.before.as_ref(),
            HookKind::After => hook.after.as_ref(),
            HookKind::PromiseResolve => hook.promise_resolve.as_ref(),
            HookKind::Destroy => hook.destroy.as_ref(),
        };
        if let Some(func) = func {
            funcs.push(func.clone());
        }
    }
    funcs
}

fn collect_internal_hooks<'js>(state: &AsyncHookState<'js>, kind: HookKind) -> Vec<Function<'js>> {
    let mut funcs = Vec::new();
    for hook in &state.internal_hooks {
        let func = match kind {
            HookKind::Init => hook.init.as_ref(),
            HookKind::Before => hook.before.as_ref(),
            HookKind::After => hook.after.as_ref(),
            HookKind::PromiseResolve => None,
            HookKind::Destroy => hook.destroy.as_ref(),
        };
        if let Some(func) = func {
            funcs.push(func.clone());
        }
    }
    funcs
}

fn has_enabled_destroy_hooks(ctx: &Ctx<'_>) -> Result<bool> {
    let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(ctx)?;
    let state = bind_state.borrow();
    if state.internal_consumer_count > 0 && state.internal_hooks.iter().any(|h| h.destroy.is_some())
    {
        return Ok(true);
    }
    Ok(state.enabled_user_hook_count > 0
        && state
            .hooks
            .iter()
            .any(|hook| *hook.enabled.as_ref().borrow() && hook.destroy.is_some()))
}

fn has_enabled_init_hooks(ctx: &Ctx<'_>) -> Result<bool> {
    let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(ctx)?;
    let state = bind_state.borrow();
    if state.internal_consumer_count > 0 && state.internal_hooks.iter().any(|h| h.init.is_some()) {
        return Ok(true);
    }
    Ok(state.enabled_user_hook_count > 0
        && state
            .hooks
            .iter()
            .any(|hook| *hook.enabled.as_ref().borrow() && hook.init.is_some()))
}

fn push_async_context<'js>(
    ctx: &Ctx<'js>,
    id: (AsyncId, TriggerAsyncId),
    resource: Option<Object<'js>>,
) -> Result<()> {
    let bind_ids = ctx.userdata::<RefCell<AsyncHookIds<'js>>>().or_throw(ctx)?;
    let mut ids = bind_ids.borrow_mut();
    let prev_id = ids.current_id;
    let prev_res = ids.current_resource.take();
    ids.scope_stack.push((prev_id.0, prev_id.1, prev_res));
    ids.current_id = id;
    ids.current_resource = resource;
    Ok(())
}

fn pop_async_context<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let bind_ids = ctx.userdata::<RefCell<AsyncHookIds<'js>>>().or_throw(ctx)?;
    let mut ids = bind_ids.borrow_mut();
    if let Some((pid, ptid, pres)) = ids.scope_stack.pop() {
        ids.current_id = (pid, ptid);
        ids.current_resource = pres;
    } else {
        ids.current_id = BOOTSTRAP_CONTEXT;
        ids.current_resource = None;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// AsyncResource native bridges (not exported on globalThis)
// ---------------------------------------------------------------------------

fn async_resource_allocate(ctx: Ctx<'_>) -> Result<Object<'_>> {
    let async_id = {
        let bind_ids = ctx.userdata::<RefCell<AsyncHookIds>>().or_throw(&ctx)?;
        let mut ids = bind_ids.borrow_mut();
        allocate_async_id(&mut ids)
    };

    let out = Object::new(ctx.clone())?;
    out.set("asyncId", async_id)?;
    Ok(out)
}

fn async_resource_emit_init<'js>(
    ctx: Ctx<'js>,
    resource: Object<'js>,
    type_name: String,
    async_id: u64,
    trigger_async_id: i64,
) -> Result<bool> {
    let tracking = {
        let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
        let track = should_track(&bind_state.borrow());
        track
    };
    if !tracking {
        return Ok(false);
    }

    if type_name.is_empty() && has_enabled_init_hooks(&ctx)? {
        return Err(Exception::throw_type(
            &ctx,
            "The \"type\" argument must be of type string and non-empty when init hooks exist.",
        ));
    }

    let (internal_fns, user_fns) = {
        let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
        let state = bind_state.borrow();
        let internal = collect_internal_hooks(&state, HookKind::Init);
        let user = if state.enabled_user_hook_count > 0 {
            collect_enabled_user_hooks(&state, HookKind::Init)
        } else {
            Vec::new()
        };
        (internal, user)
    };

    let resource_val: Value = resource.into_value();
    for func in internal_fns {
        let _ = func.call::<_, ()>((
            async_id,
            type_name.as_str(),
            trigger_async_id,
            resource_val.clone(),
        ));
    }
    for func in user_fns {
        let _ = func.call::<_, ()>((
            async_id,
            type_name.as_str(),
            trigger_async_id,
            resource_val.clone(),
        ));
    }

    has_enabled_destroy_hooks(&ctx)
}

fn async_resource_before<'js>(
    ctx: Ctx<'js>,
    async_id: u64,
    trigger_async_id: i64,
    resource: Option<Object<'js>>,
) -> Result<()> {
    push_async_context(&ctx, (async_id, trigger_async_id), resource)?;

    let (internal_fns, user_fns) = {
        let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
        let state = bind_state.borrow();
        let internal = collect_internal_hooks(&state, HookKind::Before);
        let user = if state.enabled_user_hook_count > 0 {
            collect_enabled_user_hooks(&state, HookKind::Before)
        } else {
            Vec::new()
        };
        (internal, user)
    };
    for func in internal_fns {
        let _ = func.call::<_, ()>((async_id,));
    }
    for func in user_fns {
        let _ = func.call::<_, ()>((async_id,));
    }
    Ok(())
}

fn async_resource_after(ctx: Ctx<'_>, async_id: u64) -> Result<()> {
    let (internal_fns, user_fns) = {
        let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
        let state = bind_state.borrow();
        let internal = collect_internal_hooks(&state, HookKind::After);
        let user = if state.enabled_user_hook_count > 0 {
            collect_enabled_user_hooks(&state, HookKind::After)
        } else {
            Vec::new()
        };
        (internal, user)
    };
    for func in internal_fns {
        let _ = func.call::<_, ()>((async_id,));
    }
    for func in user_fns {
        let _ = func.call::<_, ()>((async_id,));
    }

    pop_async_context(&ctx)?;
    Ok(())
}

/// Per-request `HTTPINCOMINGMESSAGE` async context (Node `Server` request scope).
pub struct HttpIncomingMessageScope<'js> {
    ctx: Ctx<'js>,
    async_id: AsyncId,
    needs_destroy: bool,
}

pub fn begin_http_incoming_message_scope<'js>(
    ctx: Ctx<'js>,
    incoming: Object<'js>,
) -> Result<HttpIncomingMessageScope<'js>> {
    let (async_id, trigger_async_id) = {
        let bind_ids = ctx.userdata::<RefCell<AsyncHookIds>>().or_throw(&ctx)?;
        let mut ids = bind_ids.borrow_mut();
        let trigger_async_id = ids.current_id.0 as TriggerAsyncId;
        let async_id = allocate_async_id(&mut ids);
        (async_id, trigger_async_id)
    };

    let needs_destroy = async_resource_emit_init(
        ctx.clone(),
        incoming.clone(),
        "HTTPINCOMINGMESSAGE".into(),
        async_id,
        trigger_async_id,
    )?;
    async_resource_before(ctx.clone(), async_id, trigger_async_id, Some(incoming))?;

    Ok(HttpIncomingMessageScope {
        ctx,
        async_id,
        needs_destroy,
    })
}

impl Drop for HttpIncomingMessageScope<'_> {
    fn drop(&mut self) {
        let ctx = self.ctx.clone();
        let async_id = self.async_id;
        let needs_destroy = self.needs_destroy;
        let _ = async_resource_after(ctx.clone(), async_id);
        if needs_destroy {
            let _ = async_resource_destroy(ctx, async_id);
        }
    }
}

fn async_resource_destroy(ctx: Ctx<'_>, async_id: u64) -> Result<()> {
    let (internal_fns, user_fns) = {
        let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(&ctx)?;
        let state = bind_state.borrow();
        let internal = collect_internal_hooks(&state, HookKind::Destroy);
        let user = if state.enabled_user_hook_count > 0 {
            collect_enabled_user_hooks(&state, HookKind::Destroy)
        } else {
            Vec::new()
        };
        (internal, user)
    };
    for func in internal_fns {
        let _ = func.call::<_, ()>((async_id,));
    }
    for func in user_fns {
        let _ = func.call::<_, ()>((async_id,));
    }
    Ok(())
}

const ASYNC_RESOURCE_FACTORY_JS: &str = r#"(function (
  allocateNative,
  emitInitNative,
  beforeNative,
  afterNative,
  destroyNative,
  executionAsyncId
) {
  const kAsyncId = Symbol("asyncId");
  const kTriggerAsyncId = Symbol("triggerAsyncId");
  const kDestroyed = Symbol("destroyed");

  const destroyRegistry = new FinalizationRegistry((held) => {
    if (held && !held.destroyed) {
      held.destroyed = true;
      destroyNative(held.asyncId);
    }
  });

  function validateType(type) {
    if (typeof type !== "string") {
      throw new TypeError('The "type" argument must be of type string. Received type ' + typeof type);
    }
  }

  function validateFunction(fn, name) {
    if (typeof fn !== "function") {
      throw new TypeError(
        'The "' + name + '" argument must be of type function. Received type ' + typeof fn
      );
    }
  }

  function validateTriggerAsyncId(triggerAsyncId) {
    if (!Number.isSafeInteger(triggerAsyncId) || triggerAsyncId < -1) {
      throw new RangeError(
        'The "triggerAsyncId" argument must be a safe integer >= -1. Received ' + triggerAsyncId
      );
    }
  }

  class AsyncResource {
    constructor(type, options) {
      validateType(type);

      if (options === undefined) {
        options = {};
      }

      let triggerAsyncId;
      let requireManualDestroy = false;

      if (typeof options === "number") {
        triggerAsyncId = options;
      } else {
        if (options === null || typeof options !== "object") {
          throw new TypeError(
            'The "options" argument must be of type object. Received type ' + typeof options
          );
        }
        triggerAsyncId =
          options.triggerAsyncId === undefined
            ? executionAsyncId()
            : options.triggerAsyncId;
        requireManualDestroy = Boolean(options.requireManualDestroy);
      }

      validateTriggerAsyncId(triggerAsyncId);

      const state = allocateNative();
      this[kAsyncId] = state.asyncId;
      this[kTriggerAsyncId] = triggerAsyncId;
      const registerDestroy = emitInitNative(
        this,
        type,
        state.asyncId,
        triggerAsyncId
      );

      if (!requireManualDestroy && registerDestroy) {
        const destroyed = { destroyed: false, asyncId: state.asyncId };
        this[kDestroyed] = destroyed;
        destroyRegistry.register(this, destroyed);
      }
    }

    runInAsyncScope(fn, thisArg, ...args) {
      validateFunction(fn, "fn");

      beforeNative(this[kAsyncId], this[kTriggerAsyncId], this);

      try {
        return Reflect.apply(fn, thisArg, args);
      } finally {
        afterNative(this[kAsyncId]);
      }
    }

    asyncId() {
      return this[kAsyncId];
    }

    triggerAsyncId() {
      return this[kTriggerAsyncId];
    }

    emitDestroy() {
      const destroyed = this[kDestroyed];
      if (destroyed !== undefined) {
        destroyed.destroyed = true;
      }
      destroyNative(this[kAsyncId]);
      return this;
    }

    bind(fn, thisArg) {
      validateFunction(fn, "fn");

      const resource = this;
      const dynamicThis = thisArg === undefined;

      const bound = function (...args) {
        return resource.runInAsyncScope(
          fn,
          dynamicThis ? this : thisArg,
          ...args
        );
      };

      let exposedResource = resource;

      Object.defineProperties(bound, {
        length: {
          configurable: true,
          enumerable: false,
          writable: false,
          value: fn.length,
        },
        asyncResource: {
          configurable: true,
          enumerable: true,
          get() {
            return exposedResource;
          },
          set(value) {
            exposedResource = value;
          },
        },
      });

      return bound;
    }

    static bind(fn, type, thisArg) {
      validateFunction(fn, "fn");
      type ||= fn.name;
      return new AsyncResource(type || "bound-anonymous-fn").bind(fn, thisArg);
    }
  }

  return AsyncResource;
})"#;

// Map-by-asyncId ALS. Requires Promise BEFORE/AFTER hooks (provided by the
// vendored QuickJS patch in vendor/rquickjs-sys) so await continuations switch
// executionAsyncId. Timer/DNS paths use the same id map via invokeAsyncHook.
const ASYNC_LOCAL_STORAGE_FACTORY_JS: &str = r#"(function (
  executionAsyncId,
  registerInternalHook,
  setInternalConsumerCount
) {
  const activeSet = new Set();
  let hookRegistered = false;

  function ensureInternalHook() {
    if (hookRegistered) return;
    hookRegistered = true;
    registerInternalHook({
      init(asyncId, type, triggerAsyncId) {
        for (const als of activeSet) {
          if (!als.enabled) continue;
          let scope = als.storeMap.get(executionAsyncId());
          if (
            (!scope || !scope.active) &&
            triggerAsyncId > 1 &&
            (type === "Timeout" ||
              type === "Immediate" ||
              type === "TickObject" ||
              type === "Microtask" ||
              type === "HTTPINCOMINGMESSAGE")
          ) {
            scope = als.storeMap.get(triggerAsyncId);
          }
          if (scope && scope.active) {
            als.storeMap.set(asyncId, scope);
          }
        }
      },
      destroy(asyncId) {
        for (const als of activeSet) {
          als.storeMap.delete(asyncId);
        }
      },
    });
  }

  function refreshConsumerCount() {
    setInternalConsumerCount(activeSet.size);
  }

  function validateFunction(fn, name) {
    if (typeof fn !== "function") {
      throw new TypeError(
        'The "' + name + '" argument must be of type function. Received type ' + typeof fn
      );
    }
  }

  function readStore(als) {
    const scope = als.storeMap.get(executionAsyncId());
    if (scope && scope.active) return scope.store;
    return als._defaultValue;
  }

  function writeScope(als, store) {
    const scope = { store: store, active: true };
    als.storeMap.set(executionAsyncId(), scope);
    return scope;
  }

  class AsyncLocalStorage {
    constructor(options) {
      if (options !== undefined && (options === null || typeof options !== "object")) {
        throw new TypeError(
          'The "options" argument must be of type object. Received type ' + typeof options
        );
      }
      this.enabled = false;
      this._defaultValue = options !== undefined ? options.defaultValue : undefined;
      this._name =
        options !== undefined && options.name !== undefined ? String(options.name) : "";
      // asyncId -> { store, active }
      this.storeMap = new Map();
    }

    get name() {
      return this._name;
    }

    _enable() {
      if (!this.enabled) {
        ensureInternalHook();
        this.enabled = true;
        activeSet.add(this);
        refreshConsumerCount();
      }
    }

    disable() {
      if (this.enabled) {
        this.enabled = false;
        activeSet.delete(this);
        this.storeMap.clear();
        refreshConsumerCount();
      }
    }

    enterWith(store) {
      this._enable();
      writeScope(this, store);
    }

    getStore() {
      return readStore(this);
    }

    run(store, callback, ...args) {
      validateFunction(callback, "callback");
      this._enable();
      const id = executionAsyncId();
      const had = this.storeMap.has(id);
      const previous = this.storeMap.get(id);
      const scope = { store: store, active: true };
      this.storeMap.set(id, scope);
      try {
        return Reflect.apply(callback, null, args);
      } finally {
        if (had) {
          this.storeMap.set(id, previous);
        } else {
          this.storeMap.delete(id);
        }
      }
    }

    exit(callback, ...args) {
      validateFunction(callback, "callback");
      if (!this.enabled && this._defaultValue === undefined) {
        return Reflect.apply(callback, null, args);
      }
      const id = executionAsyncId();
      const had = this.storeMap.has(id);
      const previous = this.storeMap.get(id);
      this.storeMap.set(id, { store: undefined, active: true });
      try {
        return Reflect.apply(callback, null, args);
      } finally {
        if (had) {
          this.storeMap.set(id, previous);
        } else {
          this.storeMap.delete(id);
        }
      }
    }

    // Node 22/24: only static AsyncLocalStorage.bind(fn). No instance bind.

    static bind(fn) {
      validateFunction(fn, "fn");
      const snapshot = AsyncLocalStorage.snapshot();
      return function (...args) {
        const self = this;
        return snapshot(function () {
          return Reflect.apply(fn, self, args);
        });
      };
    }

    static snapshot() {
      const captures = [];
      for (const als of activeSet) {
        const id = executionAsyncId();
        const scope = als.storeMap.get(id);
        captures.push({
          als: als,
          had: !!(scope && scope.active),
          value: scope && scope.active ? scope.store : undefined,
        });
      }
      return function (callback, ...args) {
        validateFunction(callback, "callback");
        const restores = [];
        for (const cap of captures) {
          const id = executionAsyncId();
          const cur = cap.als.storeMap.get(id);
          restores.push({
            als: cap.als,
            had: !!(cur && cur.active),
            value: cur && cur.active ? cur.store : undefined,
            raw: cur,
          });
          if (cap.had) {
            cap.als.storeMap.set(id, { store: cap.value, active: true });
          } else {
            cap.als.storeMap.delete(id);
          }
        }
        for (const als of activeSet) {
          if (!captures.some(function (c) { return c.als === als; })) {
            const id = executionAsyncId();
            const cur = als.storeMap.get(id);
            restores.push({
              als: als,
              had: !!(cur && cur.active),
              value: cur && cur.active ? cur.store : undefined,
              raw: cur,
            });
            als.storeMap.delete(id);
          }
        }
        try {
          return Reflect.apply(callback, null, args);
        } finally {
          for (const r of restores) {
            const id = executionAsyncId();
            if (r.had) {
              r.als.storeMap.set(id, { store: r.value, active: true });
            } else {
              r.als.storeMap.delete(id);
            }
          }
        }
      };
    }
  }

  return AsyncLocalStorage;
})"#;

fn create_async_resource_constructor<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let factory: Function = ctx.eval(ASYNC_RESOURCE_FACTORY_JS)?;
    let allocate_native = Function::new(ctx.clone(), async_resource_allocate)?;
    let emit_init_native = Function::new(ctx.clone(), async_resource_emit_init)?;
    let before_native = Function::new(ctx.clone(), async_resource_before)?;
    let after_native = Function::new(ctx.clone(), async_resource_after)?;
    let destroy_native = Function::new(ctx.clone(), async_resource_destroy)?;
    let execution_fn = Function::new(ctx.clone(), execution_async_id)?;

    factory.call((
        allocate_native,
        emit_init_native,
        before_native,
        after_native,
        destroy_native,
        execution_fn,
    ))
}

fn create_async_local_storage_constructor<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let factory: Function = ctx.eval(ASYNC_LOCAL_STORAGE_FACTORY_JS)?;
    let execution_fn = Function::new(ctx.clone(), execution_async_id)?;
    let register_fn = Function::new(ctx.clone(), register_internal_hook)?;
    let consumer_fn = Function::new(ctx.clone(), set_internal_consumer_count)?;
    factory.call((execution_fn, register_fn, consumer_fn))
}

const ASYNC_WRAP_PROVIDERS: &[(&str, i32)] = &[
    ("NONE", 0),
    ("DIRHANDLE", 1),
    ("DNSCHANNEL", 2),
    ("ELDHISTOGRAM", 3),
    ("FILEHANDLE", 4),
    ("FILEHANDLECLOSEREQ", 5),
    ("BLOBREADER", 6),
    ("FSEVENTWRAP", 7),
    ("FSREQCALLBACK", 8),
    ("FSREQPROMISE", 9),
    ("GETADDRINFOREQWRAP", 10),
    ("GETNAMEINFOREQWRAP", 11),
    ("HEAPSNAPSHOT", 12),
    ("HTTP2SESSION", 13),
    ("HTTP2STREAM", 14),
    ("HTTP2PING", 15),
    ("HTTP2SETTINGS", 16),
    ("HTTPINCOMINGMESSAGE", 17),
    ("HTTPCLIENTREQUEST", 18),
    ("JSSTREAM", 19),
    ("JSUDPWRAP", 20),
    ("MESSAGEPORT", 21),
    ("PIPECONNECTWRAP", 22),
    ("PIPESERVERWRAP", 23),
    ("PIPEWRAP", 24),
    ("PROCESSWRAP", 25),
    ("PROMISE", 26),
    ("QUERYWRAP", 27),
    ("QUIC_ENDPOINT", 28),
    ("QUIC_LOGSTREAM", 29),
    ("QUIC_PACKET", 30),
    ("QUIC_SESSION", 31),
    ("QUIC_STREAM", 32),
    ("QUIC_UDP", 33),
    ("SHUTDOWNWRAP", 34),
    ("SIGNALWRAP", 35),
    ("STATWATCHER", 36),
    ("STREAMPIPE", 37),
    ("TCPCONNECTWRAP", 38),
    ("TCPSERVERWRAP", 39),
    ("TCPWRAP", 40),
    ("TTYWRAP", 41),
    ("UDPSENDWRAP", 42),
    ("UDPWRAP", 43),
    ("SIGINTWATCHDOG", 44),
    ("WORKER", 45),
    ("WORKERHEAPSNAPSHOT", 46),
    ("WORKERHEAPSTATISTICS", 47),
    ("WRITEWRAP", 48),
    ("ZLIB", 49),
    ("CHECKPRIMEREQUEST", 50),
    ("PBKDF2REQUEST", 51),
    ("KEYPAIRGENREQUEST", 52),
    ("KEYGENREQUEST", 53),
    ("KEYEXPORTREQUEST", 54),
    ("CIPHERREQUEST", 55),
    ("DERIVEBITSREQUEST", 56),
    ("HASHREQUEST", 57),
    ("RANDOMBYTESREQUEST", 58),
    ("RANDOMPRIMEREQUEST", 59),
    ("SCRYPTREQUEST", 60),
    ("SIGNREQUEST", 61),
    ("TLSWRAP", 62),
    ("VERIFYREQUEST", 63),
];

fn create_async_wrap_providers<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let entries: Vec<String> = ASYNC_WRAP_PROVIDERS
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect();
    let script = format!(
        "Object.freeze(Object.assign(Object.create(null), {{ {} }}))",
        entries.join(", ")
    );
    ctx.eval(script)
}

pub struct AsyncHooksModule;

impl ModuleDef for AsyncHooksModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("asyncWrapProviders")?;
        declare.declare("createHook")?;
        declare.declare("currentId")?;
        declare.declare("executionAsyncId")?;
        declare.declare("executionAsyncResource")?;
        declare.declare("triggerAsyncId")?;
        declare.declare("AsyncResource")?;
        declare.declare("AsyncLocalStorage")?;
        declare.declare("default")?;

        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let async_resource = create_async_resource_constructor(ctx)?;
        let async_local_storage = create_async_local_storage_constructor(ctx)?;
        let wrap_providers = create_async_wrap_providers(ctx)?;

        export_default(ctx, exports, |default| {
            default.set("asyncWrapProviders", wrap_providers)?;
            default.set("AsyncResource", async_resource)?;
            default.set("AsyncLocalStorage", async_local_storage)?;
            default.set("createHook", Func::from(create_hook))?;
            default.set("currentId", Func::from(current_id))?;
            default.set("executionAsyncId", Func::from(execution_async_id))?;
            default.set(
                "executionAsyncResource",
                Func::from(execution_async_resource),
            )?;
            default.set("triggerAsyncId", Func::from(trigger_async_id))?;

            Ok(())
        })?;

        Ok(())
    }
}

impl From<AsyncHooksModule> for ModuleInfo<AsyncHooksModule> {
    fn from(val: AsyncHooksModule) -> Self {
        ModuleInfo {
            name: "async_hooks",
            module: val,
        }
    }
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let global = ctx.globals();

    let _ = ctx.store_userdata(RefCell::new(AsyncHookState::default()));
    let _ = ctx.store_userdata(RefCell::new(AsyncHookIds::default()));
    let _ = ctx.store_userdata(AsyncTrackingState::default());

    global.set(
        "invokeAsyncHook",
        Func::from(
            move |ctx: Ctx<'_>, type_: String, async_type: String, uid: usize| {
                if type_ == "destroy" {
                    // Timer/microtask/tick cleanup: remove id_map entry and fire destroy hooks.
                    if let Ok(resource_id) = remove_id_map(&ctx, uid) {
                        if resource_id.0 != 0 {
                            let _ = dispatch_destroy_hooks(&ctx, resource_id.0);
                        }
                    }
                    return;
                }
                let type_ = match type_.as_ref() {
                    "init" => PromiseHookType::Init,
                    "before" => PromiseHookType::Before,
                    "after" => PromiseHookType::After,
                    "resolve" => PromiseHookType::Resolve,
                    _ => return,
                };

                let _ = invoke_async_hook(&ctx, type_, async_type.as_ref(), uid, None, None);
            },
        ),
    )?;

    init_finalization_registry(ctx)?;

    Ok(())
}

/// Always install the Promise hook when the async-hooks feature is enabled.
/// The hook itself has a fast path when no consumers need tracking.
pub fn promise_hook_tracker() -> PromiseHook {
    Box::new(
        |ctx: Ctx<'_>, type_: PromiseHookType, promise: Value<'_>, parent: Value<'_>| {
            // Fast path for Init only: skip allocation when nothing is consuming context.
            // Before/After for already-mapped promises must still run so the
            // execution stack cannot leak if consumers disable mid-flight.
            if type_ == PromiseHookType::Init && !lexe_hooking::is_async_tracking_active(&ctx) {
                return;
            }

            // SAFETY: Since it checks in advance whether it is an Object type, we can always get a pointer to the object.
            let object = promise
                .as_object()
                .map(|v| unsafe { qjs::JS_VALUE_GET_PTR(v.as_raw()) } as usize)
                .unwrap();
            let parent = parent
                .as_object()
                .map(|v| unsafe { qjs::JS_VALUE_GET_PTR(v.as_raw()) } as usize);

            let is_init = type_ == PromiseHookType::Init;
            let promise_for_registry = promise.clone();
            let resource_id =
                invoke_async_hook(&ctx, type_, "PROMISE", object, parent, Some(promise))
                    .unwrap_or((0, 0));

            // Registering after the async id is assigned (rather than before,
            // as previously) lets the finalizer verify the id map entry for
            // `object` still belongs to this async id when it eventually
            // runs. See `register_finalization_registry` for why that matters.
            if is_init && resource_id.0 != 0 {
                let _ = register_finalization_registry(
                    &ctx,
                    promise_for_registry,
                    object,
                    Some(resource_id.0),
                );
            }
        },
    )
}

fn invoke_async_hook<'js>(
    ctx: &Ctx<'js>,
    type_: PromiseHookType,
    async_type: &str,
    object: usize,
    parent: Option<usize>,
    resource: Option<Value<'js>>,
) -> Result<(AsyncId, TriggerAsyncId)> {
    // Init requires active consumers; Before/After/Resolve may still need stack
    // maintenance for resources created while tracking was on.
    if type_ == PromiseHookType::Init {
        let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(ctx)?;
        if !should_track(&bind_state.borrow()) {
            return Ok((0, 0));
        }
    }

    let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(ctx)?;

    // Snapshot callbacks, then release the state borrow before calling JS.
    let (
        internal_init,
        internal_before,
        internal_after,
        user_init,
        user_before,
        user_after,
        user_resolve,
    ) = {
        let state = bind_state.borrow();
        let user = state.enabled_user_hook_count > 0;
        (
            collect_internal_hooks(&state, HookKind::Init),
            collect_internal_hooks(&state, HookKind::Before),
            collect_internal_hooks(&state, HookKind::After),
            if user {
                collect_enabled_user_hooks(&state, HookKind::Init)
            } else {
                Vec::new()
            },
            if user {
                collect_enabled_user_hooks(&state, HookKind::Before)
            } else {
                Vec::new()
            },
            if user {
                collect_enabled_user_hooks(&state, HookKind::After)
            } else {
                Vec::new()
            },
            if user {
                collect_enabled_user_hooks(&state, HookKind::PromiseResolve)
            } else {
                Vec::new()
            },
        )
    };

    match type_ {
        PromiseHookType::Init => {
            let resource_id = insert_id_map(ctx, object, parent)?;
            trace!("Init(async_id, trigger_id): {:?}", resource_id);

            let resource = resource.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
            for func in internal_init {
                let _ = func.call::<_, ()>((
                    resource_id.0,
                    async_type,
                    resource_id.1,
                    resource.clone(),
                ));
            }
            for func in user_init {
                let _ = func.call::<_, ()>((
                    resource_id.0,
                    async_type,
                    resource_id.1,
                    resource.clone(),
                ));
            }
            Ok(resource_id)
        },
        PromiseHookType::Before => {
            let resource_id = get_id_map(ctx, object)?;
            if resource_id.0 == 0 {
                return Ok((0, 0));
            }

            trace!("Before(async_id, trigger_id): {:?}", resource_id);
            push_async_context(ctx, resource_id, None)?;

            for func in internal_before {
                let _ = func.call::<_, ()>((resource_id.0,));
            }
            for func in user_before {
                let _ = func.call::<_, ()>((resource_id.0,));
            }
            Ok(resource_id)
        },
        PromiseHookType::After => {
            let resource_id = get_id_map(ctx, object)?;
            if resource_id.0 == 0 {
                return Ok((0, 0));
            }

            trace!("After(async_id, trigger_id): {:?}", resource_id);
            for func in internal_after {
                let _ = func.call::<_, ()>((resource_id.0,));
            }
            for func in user_after {
                let _ = func.call::<_, ()>((resource_id.0,));
            }

            pop_async_context(ctx)?;
            Ok(resource_id)
        },
        PromiseHookType::Resolve => {
            let resource_id = get_id_map(ctx, object)?;
            if resource_id.0 == 0 {
                return Ok((0, 0));
            }

            trace!("Resolve(async_id, trigger_id): {:?}", resource_id);
            for func in user_resolve {
                let _ = func.call::<_, ()>((resource_id.0,));
            }
            Ok(resource_id)
        },
    }
}

fn insert_id_map(
    ctx: &Ctx<'_>,
    target: usize,
    parent: Option<usize>,
) -> Result<(AsyncId, TriggerAsyncId)> {
    let bind_ids = ctx.userdata::<RefCell<AsyncHookIds>>().or_throw(ctx)?;
    let mut ids = bind_ids.borrow_mut();
    let async_id = allocate_async_id(&mut ids);
    let trigger_id = parent
        .and_then(|tid| ids.id_map.get(&tid).map(|id| id.0 as TriggerAsyncId))
        .unwrap_or(ids.current_id.0 as TriggerAsyncId);
    ids.id_map.insert(target, (async_id, trigger_id));
    Ok((async_id, trigger_id))
}

fn get_id_map(ctx: &Ctx<'_>, target: usize) -> Result<(AsyncId, TriggerAsyncId)> {
    let bind_ids = ctx.userdata::<RefCell<AsyncHookIds>>().or_throw(ctx)?;
    let ids = bind_ids.borrow();
    Ok(*ids.id_map.get(&target).unwrap_or(&(0, 0)))
}

pub(crate) fn remove_id_map(ctx: &Ctx<'_>, target: usize) -> Result<(AsyncId, TriggerAsyncId)> {
    let bind_ids = ctx.userdata::<RefCell<AsyncHookIds>>().or_throw(ctx)?;
    let mut ids = bind_ids.borrow_mut();
    let removed = ids
        .id_map
        .remove_entry(&target)
        .map(|(_, (async_id, trigger_id))| (async_id, trigger_id))
        .unwrap_or((0, 0));
    if removed.0 != 0 {
        ids.fallback_resources.remove(&removed.0);
    }
    Ok(removed)
}

/// Like `remove_id_map`, but only removes the entry when it still maps to
/// `expected_async_id`. Used by finalizers so a stale (delayed) GC callback
/// for a freed object cannot delete a different, unrelated object's mapping
/// after the underlying pointer has been reused.
pub(crate) fn remove_id_map_if_matches(
    ctx: &Ctx<'_>,
    target: usize,
    expected_async_id: AsyncId,
) -> Result<(AsyncId, TriggerAsyncId)> {
    let bind_ids = ctx.userdata::<RefCell<AsyncHookIds>>().or_throw(ctx)?;
    let mut ids = bind_ids.borrow_mut();
    let matches = ids
        .id_map
        .get(&target)
        .is_some_and(|(async_id, _)| *async_id == expected_async_id);
    if !matches {
        return Ok((0, 0));
    }
    let removed = ids
        .id_map
        .remove_entry(&target)
        .map(|(_, (async_id, trigger_id))| (async_id, trigger_id))
        .unwrap_or((0, 0));
    if removed.0 != 0 {
        ids.fallback_resources.remove(&removed.0);
    }
    Ok(removed)
}

pub(crate) fn dispatch_destroy_hooks(ctx: &Ctx<'_>, async_id: AsyncId) -> Result<()> {
    if let Some(bind_ids) = ctx.userdata::<RefCell<AsyncHookIds>>() {
        bind_ids.borrow_mut().fallback_resources.remove(&async_id);
    }
    let (internal_fns, user_fns) = {
        let bind_state = ctx.userdata::<RefCell<AsyncHookState>>().or_throw(ctx)?;
        let state = bind_state.borrow();
        let internal = collect_internal_hooks(&state, HookKind::Destroy);
        let user = if state.enabled_user_hook_count > 0 {
            collect_enabled_user_hooks(&state, HookKind::Destroy)
        } else {
            Vec::new()
        };
        (internal, user)
    };
    for func in internal_fns {
        let _ = func.call::<_, ()>((async_id,));
    }
    for func in user_fns {
        let _ = func.call::<_, ()>((async_id,));
    }
    Ok(())
}
