// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::result::Result as StdResult;

use lexe_events::Emitter;
use rquickjs::{Class, Ctx, IntoJs, Result, Value};
use tokio::sync::broadcast::error::RecvError;

pub mod readable;
pub mod writable;

pub use writable::WriteCommand;

pub fn set_destroyed_and_error<'js>(
    is_destroyed: &mut bool,
    error_value: &mut Option<Value<'js>>,
    error: StdResult<Option<Value<'js>>, RecvError>,
) {
    *is_destroyed = true;
    if let Ok(error) = error {
        // `stream.destroy()` with no argument broadcasts `Some(undefined)`.
        // Treating that as an error makes the read/write loops do
        // `ctx.throw(undefined)`, which reaches JS as a bare `undefined` (no
        // `message`, no `stack`) instead of a clean close.
        *error_value = error.filter(|v| !v.is_undefined() && !v.is_null());
    }
}
const DEFAULT_BUFFER_SIZE: usize = 1024 * 16;

pub trait SteamEvents<'js>
where
    Self: Emitter<'js>,
{
    fn emit_close(this: Class<'js, Self>, ctx: &Ctx<'js>, had_error: bool) -> Result<()> {
        Self::emit_str(this, ctx, "close", vec![had_error.into_js(ctx)?], false)
    }

    #[allow(dead_code)]
    fn emit_end(this: Class<'js, Self>, ctx: &Ctx<'js>) -> Result<()> {
        Self::emit_str(this, ctx, "end", vec![], false)
    }
}

#[macro_export]
macro_rules! impl_stream_events {

    ($($struct:ident),*) => {
        $(
            impl<'js> $crate::SteamEvents<'js> for $struct<'js> {}
        )*
    };
}
