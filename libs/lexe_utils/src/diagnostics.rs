// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{Ctx, Function, Object, Result};

const STATE_KEY: &str = "__diagnosticsChannelState";

/// Returns `true` if a `node:diagnostics_channel` channel currently has subscribers or bound stores.
pub fn has_subscribers<'js>(ctx: &Ctx<'js>, name: &str) -> bool {
    let Ok(state) = ctx.globals().get::<_, Object<'js>>(STATE_KEY) else {
        return false;
    };
    let Ok(has_fn) = state.get::<_, Function<'js>>("hasSubscribers") else {
        return false;
    };
    has_fn.call::<_, bool>((name,)).unwrap_or(false)
}

/// Publishes a diagnostic message if the named channel currently has subscribers.
///
/// The message builder closure only runs when `hasSubscribers(name)` is true,
/// keeping unsubscribed subsystems free of object-allocation overhead.
pub fn publish<'js, F>(ctx: &Ctx<'js>, name: &str, build_msg: F) -> Result<()>
where
    F: FnOnce(&Ctx<'js>) -> Result<Object<'js>>,
{
    let Ok(state) = ctx.globals().get::<_, Object<'js>>(STATE_KEY) else {
        return Ok(());
    };
    let Ok(has_fn) = state.get::<_, Function<'js>>("hasSubscribers") else {
        return Ok(());
    };
    if !has_fn.call::<_, bool>((name,)).unwrap_or(false) {
        return Ok(());
    }
    let Ok(pub_fn) = state.get::<_, Function<'js>>("publish") else {
        return Ok(());
    };
    let msg = build_msg(ctx)?;
    pub_fn.call::<_, ()>((name, msg))
}
