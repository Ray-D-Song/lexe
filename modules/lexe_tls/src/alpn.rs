// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{Array, Ctx, Exception, Result, Value};

use crate::error::throw_invalid_arg_value;

/// Convert a JS ALPNProtocols value to a length-prefixed wire buffer.
pub fn convert_alpn_protocols<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Vec<u8>> {
    if let Some(array) = value.as_array() {
        return convert_alpn_protocol_array(ctx, array);
    }

    let bytes = lexe_utils::bytes::ObjectBytes::from(ctx, &value)?;
    let data = bytes
        .try_into()
        .map_err(|_| throw_invalid_arg_value(ctx, "ALPNProtocols", "must be an array or buffer"))?;
    Ok(data)
}

/// Node `tls.convertALPNProtocols` export logic.
pub fn convert_alpn_protocols_export<'js>(
    ctx: &Ctx<'js>,
    protocols: Value<'js>,
    out: &rquickjs::Object<'js>,
) -> Result<()> {
    let converted = convert_alpn_protocols(ctx, protocols)?;
    let buffer = lexe_buffer::Buffer(converted);
    out.set("ALPNProtocols", buffer.into_js(ctx)?)?;
    Ok(())
}

fn convert_alpn_protocol_array<'js>(ctx: &Ctx<'js>, array: &Array<'js>) -> Result<Vec<u8>> {
    let mut lens = Vec::new();
    let mut total = 0usize;

    for (i, item) in array.iter::<Value>().enumerate() {
        let item = item?;
        let protocol = item
            .as_string()
            .ok_or_else(|| {
                Exception::throw_type(ctx, "ALPNProtocols array elements must be strings")
            })?
            .to_string()?;
        let len = protocol.len();
        if len == 0 || len > 255 {
            return Err(Exception::throw_range(
                ctx,
                &format!(
                    "The byte length of the protocol at index {i} exceeds the maximum length."
                ),
            ));
        }
        lens.push((protocol, len));
        total += 1 + len;
    }

    let mut buf = Vec::with_capacity(total);
    for (protocol, len) in lens {
        buf.push(len as u8);
        buf.extend_from_slice(protocol.as_bytes());
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};

    fn with_ctx(f: impl FnOnce(&Ctx<'_>)) {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| f(&ctx));
    }

    #[test]
    fn converts_protocol_array() {
        with_ctx(|ctx| {
            let array = Array::new(ctx.clone()).unwrap();
            array.set(0, "h2").unwrap();
            array.set(1, "http/1.1").unwrap();
            let buf = convert_alpn_protocol_array(ctx, &array).unwrap();
            assert_eq!(buf, b"\x02h2\x08http/1.1");
        });
    }

    #[test]
    fn rejects_empty_protocol() {
        with_ctx(|ctx| {
            let array = Array::new(ctx.clone()).unwrap();
            array.set(0, "").unwrap();
            assert!(convert_alpn_protocol_array(ctx, &array).is_err());
        });
    }

    #[test]
    fn rejects_oversized_protocol() {
        with_ctx(|ctx| {
            let array = Array::new(ctx.clone()).unwrap();
            array.set(0, "x".repeat(256)).unwrap();
            assert!(convert_alpn_protocol_array(ctx, &array).is_err());
        });
    }
}

use rquickjs::IntoJs;
