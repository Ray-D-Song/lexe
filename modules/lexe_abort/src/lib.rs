// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::new_without_default)]
use lexe_events::Emitter;
use lexe_utils::primordials::{BasePrimordials, Primordial};
use rquickjs::{prelude::Func, Class, Ctx, Result, Value};

pub use self::{
    abort_controller::AbortController,
    abort_signal::{make_signal_transferable, register_codecs, AbortSignal},
};

mod abort_controller;
mod abort_signal;

fn transferable_abort_controller<'js>(ctx: Ctx<'js>) -> Result<Class<'js, AbortController<'js>>> {
    let controller = AbortController::new(ctx.clone())?;
    let _ = make_signal_transferable(&ctx, controller.signal())?;
    Class::instance(ctx, controller)
}

fn transferable_abort_signal<'js>(
    ctx: Ctx<'js>,
    signal: Value<'js>,
) -> Result<Class<'js, AbortSignal<'js>>> {
    let cls = Class::<AbortSignal<'js>>::from_value(&signal).map_err(|_| {
        rquickjs::Exception::throw_type(
            &ctx,
            "The \"signal\" argument must be an instance of AbortSignal",
        )
    })?;
    make_signal_transferable(&ctx, cls)
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    register_codecs();
    let globals = ctx.globals();

    BasePrimordials::init(ctx)?;

    Class::<AbortController>::define(&globals)?;
    Class::<AbortSignal>::define(&globals)?;

    AbortSignal::add_event_emitter_prototype(ctx)?;
    AbortSignal::add_event_target_prototype(ctx)?;

    globals.set(
        "__lexeTransferableAbortController",
        Func::from(transferable_abort_controller),
    )?;
    globals.set(
        "__lexeTransferableAbortSignal",
        Func::from(transferable_abort_signal),
    )?;

    Ok(())
}
