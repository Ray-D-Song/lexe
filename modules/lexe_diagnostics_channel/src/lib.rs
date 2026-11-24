// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    Ctx, Function, Object, Result, Value,
};

const STATE_KEY: &str = "__diagnosticsChannelState";

const FACTORY_JS: &str = r#"(function () {
  const channels = new Map();
  const activeChannels = new Map();
  const channelRegistry = new FinalizationRegistry(({ name, ref }) => {
    if (channels.get(name) === ref) {
      channels.delete(name);
    }
  });

  const kTracingChannelKeys = ["start", "end", "asyncStart", "asyncEnd", "error"];

  function validateFunction(fn, name) {
    if (typeof fn !== "function") {
      const err = new TypeError(`The "${name}" argument must be of type function`);
      err.code = "ERR_INVALID_ARG_TYPE";
      throw err;
    }
  }

  function makeInvalidArgType(name, expected, actual) {
    const desc =
      actual === null
        ? "null"
        : actual === undefined
          ? "undefined"
          : `type ${typeof actual} (${String(actual)})`;
    const err = new TypeError(
      `The "${name}" argument must be of ${expected}. Received ${desc}`
    );
    err.code = "ERR_INVALID_ARG_TYPE";
    return err;
  }

  function scheduleRethrow(err) {
    if (
      typeof globalThis.process !== "undefined" &&
      globalThis.process !== null &&
      typeof globalThis.process.nextTick === "function"
    ) {
      globalThis.process.nextTick(() => {
        throw err;
      });
    } else {
      queueMicrotask(() => {
        throw err;
      });
    }
  }

  function markActive(channel) {
    Object.setPrototypeOf(channel, ActiveChannel.prototype);
    channel._subscribers = [];
    channel._stores = new Map();
    activeChannels.set(channel.name, channel);
  }

  function maybeMarkInactive(channel) {
    if (!channel._subscribers.length && !channel._stores.size) {
      Object.setPrototypeOf(channel, Channel.prototype);
      channel._subscribers = undefined;
      channel._stores = undefined;
      activeChannels.delete(channel.name);
    }
  }

  class ActiveChannel {
    subscribe(subscription) {
      validateFunction(subscription, "subscription");
      this._subscribers.push(subscription);
    }

    unsubscribe(subscription) {
      const index = this._subscribers.indexOf(subscription);
      if (index === -1) {
        return false;
      }

      this._subscribers.splice(index, 1);
      maybeMarkInactive(this);
      return true;
    }

    bindStore(store, transform) {
      this._stores.set(store, transform);
    }

    unbindStore(store) {
      if (!this._stores.delete(store)) {
        return false;
      }
      maybeMarkInactive(this);
      return true;
    }

    get hasSubscribers() {
      return true;
    }

    publish(data) {
      const subscribers = this._subscribers.slice();
      for (let i = 0; i < subscribers.length; i++) {
        try {
          subscribers[i](data, this.name);
        } catch (err) {
          scheduleRethrow(err);
        }
      }
    }

    runStores(context, fn, thisArg, ...args) {
      let run = () => {
        this.publish(context);
        return Reflect.apply(fn, thisArg, args);
      };

      if (this._stores.size > 0) {
        const entries = Array.from(this._stores.entries());
        for (let i = entries.length - 1; i >= 0; i--) {
          const [store, transform] = entries[i];
          const next = run;
          run = () => {
            let storeContext = context;
            if (transform !== undefined) {
              try {
                storeContext = transform(context);
              } catch (err) {
                scheduleRethrow(err);
                return next();
              }
            }
            return store.run(storeContext, next);
          };
        }
      }

      return run();
    }
  }

  class Channel {
    constructor(name) {
      this._subscribers = undefined;
      this._stores = undefined;
      this.name = name;
      const ref = new WeakRef(this);
      channels.set(name, ref);
      channelRegistry.register(this, { name, ref });
    }

    static [Symbol.hasInstance](instance) {
      if (typeof instance !== "object" || instance === null) {
        return false;
      }
      const prototype = Object.getPrototypeOf(instance);
      return (
        prototype === Channel.prototype || prototype === ActiveChannel.prototype
      );
    }

    subscribe(subscription) {
      markActive(this);
      this.subscribe(subscription);
    }

    unsubscribe() {
      return false;
    }

    bindStore(store, transform) {
      markActive(this);
      this.bindStore(store, transform);
    }

    unbindStore() {
      return false;
    }

    get hasSubscribers() {
      return false;
    }

    publish() {}

    runStores(context, fn, thisArg, ...args) {
      return Reflect.apply(fn, thisArg, args);
    }
  }

  function channel(name) {
    const existing = channels.get(name)?.deref();
    if (existing) {
      return existing;
    }

    if (typeof name !== "string" && typeof name !== "symbol") {
      const err = new TypeError(
        'The "channel" argument must be of type string or symbol'
      );
      err.code = "ERR_INVALID_ARG_TYPE";
      throw err;
    }

    return new Channel(name);
  }

  function subscribe(name, subscription) {
    channel(name).subscribe(subscription);
  }

  function unsubscribe(name, subscription) {
    return channel(name).unsubscribe(subscription);
  }

  function hasSubscribers(name) {
    const existing = channels.get(name)?.deref();
    if (!existing) {
      return false;
    }

    return existing.hasSubscribers;
  }

  function publish(name, message) {
    const existing = channels.get(name)?.deref();
    if (existing && existing.hasSubscribers) {
      existing.publish(message);
    }
  }

  class TracingChannel {
    constructor(nameOrChannels) {
      if (typeof nameOrChannels === "string") {
        for (const key of kTracingChannelKeys) {
          Object.defineProperty(this, key, {
            value: channel(`tracing:${nameOrChannels}:${key}`),
            writable: false,
            enumerable: false,
            configurable: false,
          });
        }
        return;
      }

      if (typeof nameOrChannels !== "object" || nameOrChannels === null) {
        throw makeInvalidArgType(
          "nameOrChannels",
          "type string or an instance of TracingChannel or Object",
          nameOrChannels
        );
      }

      for (const key of kTracingChannelKeys) {
        const ch = nameOrChannels[key];
        if (!(ch instanceof Channel)) {
          if (ch === undefined) {
            throw new TypeError(
              `Channel "${key}" must be an instance of Channel`
            );
          }
          const err = new TypeError(
            `The "nameOrChannels.${key}" property must be an instance of Channel. Received type ${typeof ch} (${String(ch)})`
          );
          err.code = "ERR_INVALID_ARG_TYPE";
          throw err;
        }
        Object.defineProperty(this, key, {
          value: ch,
          writable: false,
          enumerable: false,
          configurable: false,
        });
      }
    }

    get hasSubscribers() {
      return Boolean(
        this.start.hasSubscribers ||
          this.end.hasSubscribers ||
          this.asyncStart.hasSubscribers ||
          this.asyncEnd.hasSubscribers ||
          this.error.hasSubscribers
      );
    }

    subscribe(subscribers) {
      if (typeof subscribers !== "object" || subscribers === null) {
        return;
      }
      for (const key of kTracingChannelKeys) {
        const handler = subscribers[key];
        if (handler !== undefined) {
          this[key].subscribe(handler);
        }
      }
    }

    unsubscribe(subscribers) {
      if (typeof subscribers !== "object" || subscribers === null) {
        return false;
      }
      let done = true;
      for (const key of kTracingChannelKeys) {
        const handler = subscribers[key];
        if (handler !== undefined && !this[key].unsubscribe(handler)) {
          done = false;
        }
      }
      return done;
    }

    traceSync(fn, context = {}, thisArg, ...args) {
      if (!this.hasSubscribers) {
        return Reflect.apply(fn, thisArg, args);
      }
      const { start, end, error } = this;
      return start.runStores(context, () => {
        try {
          const result = Reflect.apply(fn, thisArg, args);
          context.result = result;
          return result;
        } catch (err) {
          context.error = err;
          error.publish(context);
          throw err;
        } finally {
          end.publish(context);
        }
      });
    }

    tracePromise(fn, context = {}, thisArg, ...args) {
      if (!this.hasSubscribers) {
        return Reflect.apply(fn, thisArg, args);
      }
      const { start, end, asyncStart, asyncEnd, error } = this;

      function reject(err) {
        context.error = err;
        error.publish(context);
        return asyncStart.runStores(context, () => {
          try {
            throw err;
          } finally {
            asyncEnd.publish(context);
          }
        });
      }

      function resolve(result) {
        context.result = result;
        return asyncStart.runStores(context, () => {
          try {
            return result;
          } finally {
            asyncEnd.publish(context);
          }
        });
      }

      const promise = start.runStores(context, () => {
        try {
          return Reflect.apply(fn, thisArg, args);
        } catch (err) {
          context.error = err;
          error.publish(context);
          throw err;
        } finally {
          end.publish(context);
        }
      });

      return Promise.resolve(promise).then(resolve, reject);
    }

    traceCallback(fn, position = -1, context = {}, thisArg, ...args) {
      if (!this.hasSubscribers) {
        return Reflect.apply(fn, thisArg, args);
      }
      const { start, end, asyncStart, asyncEnd, error } = this;

      function wrappedCallback(err, res) {
        if (err) {
          context.error = err;
          error.publish(context);
        } else {
          context.result = res;
        }

        return asyncStart.runStores(context, () => {
          try {
            return Reflect.apply(callback, this, arguments);
          } finally {
            asyncEnd.publish(context);
          }
        });
      }

      const callbackAt = position < 0 ? args.length + position : position;
      const callback = args[callbackAt];
      validateFunction(callback, "callback");
      args.splice(callbackAt, 1, wrappedCallback);

      return start.runStores(context, () => {
        try {
          return Reflect.apply(fn, thisArg, args);
        } catch (err) {
          context.error = err;
          error.publish(context);
          throw err;
        } finally {
          end.publish(context);
        }
      });
    }
  }

  function tracingChannel(nameOrChannels) {
    return new TracingChannel(nameOrChannels);
  }

  return {
    Channel,
    channel,
    subscribe,
    unsubscribe,
    hasSubscribers,
    tracingChannel,
    publish,
  };
})"#;

fn install_module<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let globals = ctx.globals();
    if let Ok(existing) = globals.get::<_, Object>(STATE_KEY) {
        return Ok(existing);
    }

    let factory: Function = ctx.eval(FACTORY_JS)?;
    let state_value: Value = factory.call(())?;
    let state = state_value
        .as_object()
        .expect("diagnostics channel state")
        .clone();
    let installer: Function = ctx.eval(
        r#"(function (key, state) {
  Object.defineProperty(globalThis, key, {
    value: state,
    writable: true,
    enumerable: false,
    configurable: true,
  });
})"#,
    )?;
    installer.call::<_, ()>((STATE_KEY, state.clone()))?;
    Ok(state)
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    install_module(ctx)?;
    Ok(())
}

pub struct DiagnosticsChannelModule;

impl ModuleDef for DiagnosticsChannelModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("Channel")?;
        declare.declare("channel")?;
        declare.declare("subscribe")?;
        declare.declare("unsubscribe")?;
        declare.declare("hasSubscribers")?;
        declare.declare("tracingChannel")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let state = install_module(ctx)?;

        export_default(ctx, exports, |default| {
            let channel_ctor: Value = state.get("Channel")?;
            let channel_fn: Value = state.get("channel")?;
            let subscribe_fn: Value = state.get("subscribe")?;
            let unsubscribe_fn: Value = state.get("unsubscribe")?;
            let has_subscribers_fn: Value = state.get("hasSubscribers")?;
            let tracing_channel_fn: Value = state.get("tracingChannel")?;

            default.set("Channel", channel_ctor)?;
            default.set("channel", channel_fn)?;
            default.set("subscribe", subscribe_fn)?;
            default.set("unsubscribe", unsubscribe_fn)?;
            default.set("hasSubscribers", has_subscribers_fn)?;
            default.set("tracingChannel", tracing_channel_fn)?;
            Ok(())
        })
    }
}

impl From<DiagnosticsChannelModule> for ModuleInfo<DiagnosticsChannelModule> {
    fn from(val: DiagnosticsChannelModule) -> Self {
        ModuleInfo {
            name: "diagnostics_channel",
            module: val,
        }
    }
}
