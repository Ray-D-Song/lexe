// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{prelude::Func, Ctx, Object, Result};
use tracing::trace;

use super::{dispatch_destroy_hooks, remove_id_map, remove_id_map_if_matches};

pub(crate) fn init_finalization_registry(ctx: &Ctx<'_>) -> Result<()> {
    let global = ctx.globals();

    global.set(
        "__invokeFinalizationHook",
        Func::from(invoke_finalization_hook),
    )?;

    let _: () = ctx.eval(
        r#"
        globalThis.asyncFinalizationRegistry = (() => {
            const registry = new FinalizationRegistry(__invokeFinalizationHook);
            return {
                register(target, heldValue) {
                    registry.register(target, heldValue);
                }
            };
        })();
        "#,
    )?;

    global.remove("__invokeFinalizationHook")?;

    Ok(())
}

fn invoke_finalization_hook<'js>(ctx: Ctx<'js>, held: Object<'js>) -> Result<()> {
    let uid: usize = held.get("uid")?;
    // When the caller recorded which async id it registered for, only remove
    // the id map entry if it still belongs to that id: the raw pointer used
    // as `uid` can be reused by an unrelated object before this (deferred)
    // finalizer runs, and an unconditional removal would delete that new
    // object's live mapping instead.
    let expected_async_id: Option<u64> = held.get("asyncId")?;

    let resource_id = match expected_async_id {
        Some(async_id) => remove_id_map_if_matches(&ctx, uid, async_id)?,
        None => remove_id_map(&ctx, uid)?,
    };
    if resource_id.0 == 0 {
        return Ok(());
    }

    // Destroy must not change current_id.
    trace!("Destroy[{}](async_id, trigger_id): {:?}", uid, resource_id);

    dispatch_destroy_hooks(&ctx, resource_id.0)?;
    Ok(())
}
