// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_buffer::Buffer;
use rquickjs::{Ctx, Object, Result, Value};

pub fn define_constants<'js>(ctx: &Ctx<'js>, target: &Object<'js>) -> Result<()> {
    let constants = Object::new(ctx.clone())?;
    constants.set("NGHTTP2_SESSION_SERVER", 0)?;
    constants.set("NGHTTP2_SESSION_CLIENT", 1)?;
    constants.set("NGHTTP2_STREAM_STATE_IDLE", 1)?;
    constants.set("HTTP2_HEADER_STATUS", ":status")?;
    constants.set("HTTP2_HEADER_METHOD", ":method")?;
    constants.set("HTTP2_HEADER_AUTHORITY", ":authority")?;
    constants.set("HTTP2_HEADER_SCHEME", ":scheme")?;
    constants.set("HTTP2_HEADER_PATH", ":path")?;
    constants.set("HTTP2_METHOD_GET", "GET")?;
    constants.set("HTTP2_METHOD_POST", "POST")?;
    target.set("constants", constants)?;
    Ok(())
}

pub fn get_default_settings<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let settings = Object::new(ctx.clone())?;
    settings.set("headerTableSize", 4096)?;
    settings.set("enablePush", true)?;
    settings.set("initialWindowSize", 65535)?;
    settings.set("maxFrameSize", 16384)?;
    settings.set("maxConcurrentStreams", 100)?;
    settings.set("maxHeaderListSize", 65535)?;
    Ok(settings)
}

pub fn get_packed_settings(_ctx: &Ctx<'_>, _settings: Value<'_>) -> Result<Buffer> {
    Ok(Buffer(Vec::new()))
}

pub fn get_unpacked_settings<'js>(ctx: Ctx<'js>, _buf: Value<'js>) -> Result<Object<'js>> {
    get_default_settings(ctx)
}
