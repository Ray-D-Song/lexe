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

  function validateFunction(fn, name) {
    if (typeof fn !== "function") {
      throw new TypeError(`The "${name}" argument must be of type function`);
    }
  }

  function markActive(channel) {
    Object.setPrototypeOf(channel, ActiveChannel.prototype);
    channel._subscribers = [];
  }

  function maybeMarkInactive(channel) {
    if (!channel._subscribers.length) {
      Object.setPrototypeOf(channel, Channel.prototype);
      channel._subscribers = undefined;
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

    get hasSubscribers() {
      return true;
    }

    publish(data) {
      const subscribers = this._subscribers.slice();
      for (let i = 0; i < subscribers.length; i++) {
        try {
          subscribers[i](data, this.name);
        } catch (err) {
          queueMicrotask(() => {
            throw err;
          });
        }
      }
    }
  }

  class Channel {
    constructor(name) {
      this.name = name;
      this._subscribers = undefined;
      channels.set(name, this);
    }

    static [Symbol.hasInstance](instance) {
      const prototype = Object.getPrototypeOf(instance);
      return prototype === Channel.prototype || prototype === ActiveChannel.prototype;
    }

    subscribe(subscription) {
      markActive(this);
      this.subscribe(subscription);
    }

    unsubscribe() {
      return false;
    }

    get hasSubscribers() {
      return false;
    }

    publish() {}
  }

  function channel(name) {
    const existing = channels.get(name);
    if (existing) {
      return existing;
    }

    if (typeof name !== "string" && typeof name !== "symbol") {
      throw new TypeError('The "channel" argument must be of type string or symbol');
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
    const existing = channels.get(name);
    if (!existing) {
      return false;
    }

    return existing.hasSubscribers;
  }

  return {
    Channel,
    channel,
    subscribe,
    unsubscribe,
    hasSubscribers,
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
    globals.set(STATE_KEY, state.clone())?;
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

            default.set("Channel", channel_ctor)?;
            default.set("channel", channel_fn)?;
            default.set("subscribe", subscribe_fn)?;
            default.set("unsubscribe", unsubscribe_fn)?;
            default.set("hasSubscribers", has_subscribers_fn)?;
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
