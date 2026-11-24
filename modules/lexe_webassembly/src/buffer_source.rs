// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Strict `BufferSource` (`ArrayBuffer` or `ArrayBufferView`) extraction,
//! shared by `WebAssembly.Module`/`compile`/`validate`/`instantiate`(bytes
//! overload).
//!
//! Per the implementation plan this is intentionally *not* the same as
//! `lexe_utils::ObjectBytes`'s general-purpose `FromJs` conversion,
//! which also accepts plain JS strings/arrays (used elsewhere in the project
//! for Node `Buffer`-like coercions). The WebAssembly JS API must reject
//! those with a `TypeError` and only ever accept an `ArrayBuffer` or a
//! `TypedArray`/`DataView` view over one (sliced to that view's
//! `byteOffset`/`byteLength`).

use lexe_utils::primordials::{BasePrimordials, Primordial};
use rquickjs::{ArrayBuffer, Ctx, Result, Value};

use crate::host_state::HostState;

/// Extracts the bytes of a `BufferSource` (`ArrayBuffer` or any
/// `ArrayBufferView`), copying only the view's own `byteOffset`/`byteLength`
/// window. Throws `TypeError` for anything else (including plain strings and
/// arrays).
///
/// Deliberately does **not** delegate to
/// `lexe_utils::bytes::ObjectBytes`'s general-purpose `FromJs`/
/// `from_array_buffer` conversion (used elsewhere in the project for
/// Node-`Buffer`-like coercions). That converter has a structured fallback of
/// `obj.get::<_, ArrayBuffer>("buffer")` for anything that isn't itself an
/// `ArrayBuffer` or one of the concrete `TypedArray` kinds it special-cases:
///
/// - it duck-types on a `.buffer` property, so a forged plain object like
///   `{ buffer: someValidWasmArrayBuffer }` (which is not an
///   `ArrayBufferView` at all) is incorrectly accepted;
/// - for a real `DataView` (which rquickjs has no dedicated Rust binding
///   for), it reads the *entire* underlying `.buffer`, ignoring the
///   `DataView`'s own `byteOffset`/`byteLength` window.
///
/// This function instead classifies strictly:
/// 1. A genuine `ArrayBuffer`, verified via the real QuickJS
///    `JS_GetArrayBuffer` internal-slot accessor (`ArrayBuffer::from_object`,
///    not duck-typing) -- takes the whole buffer.
/// 2. Any `ArrayBufferView` (every concrete `TypedArray` kind *and*
///    `DataView` uniformly), verified via the primordial `ArrayBuffer.isView`
///    (itself an internal-slot check that a forged `{ buffer }`-shaped
///    object cannot satisfy) -- takes exactly that view's own
///    `byteOffset`/`byteLength` window of its backing buffer.
/// 3. Anything else -- `TypeError`.
pub fn extract_buffer_source<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    value: &Value<'js>,
) -> Result<Vec<u8>> {
    let obj = value
        .as_object()
        .ok_or_else(|| host.throw_type_error(ctx, "expected an ArrayBuffer or ArrayBufferView"))?;

    if let Some(array_buffer) = ArrayBuffer::from_object(obj.clone()) {
        let bytes = array_buffer
            .as_bytes()
            .ok_or_else(|| host.throw_type_error(ctx, "ArrayBuffer is detached"))?;
        return Ok(bytes.to_vec());
    }

    // Idempotent: cheap no-op if `crate::init` (or anything else sharing this
    // context) already initialized it.
    BasePrimordials::init(ctx)?;
    let primordials = BasePrimordials::get(ctx)?;
    let is_view = primordials
        .function_array_buffer_is_view
        .call::<_, bool>((obj.clone(),))?;
    if is_view {
        let buffer: ArrayBuffer = obj.get("buffer")?;
        let byte_offset: usize = obj.get("byteOffset")?;
        let byte_length: usize = obj.get("byteLength")?;
        let bytes = buffer
            .as_bytes()
            .ok_or_else(|| host.throw_type_error(ctx, "underlying ArrayBuffer is detached"))?;
        let end = byte_offset
            .checked_add(byte_length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| {
                host.throw_type_error(ctx, "ArrayBufferView is out of bounds of its buffer")
            })?;
        return Ok(bytes[byte_offset..end].to_vec());
    }

    Err(host.throw_type_error(ctx, "expected an ArrayBuffer or ArrayBufferView"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_sync_with;

    #[tokio::test]
    async fn rejects_plain_string() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let value: Value = ctx.eval("'not a buffer'")?;
            let result = extract_buffer_source(&ctx, &realm.state, &value);
            assert!(result.is_err());
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_plain_array() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let value: Value = ctx.eval("[0, 1, 2, 3]")?;
            let result = extract_buffer_source(&ctx, &realm.state, &value);
            assert!(result.is_err());
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn slices_typed_array_view_by_byte_offset_and_length() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let value: Value = ctx.eval(
                "(() => { const buf = new Uint8Array([0,1,2,3,4,5,6,7]).buffer; return new Uint8Array(buf, 2, 3); })()",
            )?;
            let bytes = extract_buffer_source(&ctx, &realm.state, &value)?;
            assert_eq!(bytes, vec![2, 3, 4]);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_forged_object_with_buffer_property() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            // Not an `ArrayBufferView` at all (`ArrayBuffer.isView` returns
            // `false` for it) -- must be rejected even though it structurally
            // has a `.buffer` property pointing at a valid `ArrayBuffer`.
            let value: Value = ctx.eval("({ buffer: new Uint8Array([0,1,2,3]).buffer })")?;
            let result = extract_buffer_source(&ctx, &realm.state, &value);
            assert!(result.is_err());
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn data_view_uses_its_own_byte_offset_and_length_window() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let value: Value = ctx.eval(
                "(() => { const buf = new Uint8Array([0,1,2,3,4,5,6,7]).buffer; return new DataView(buf, 2, 3); })()",
            )?;
            let bytes = extract_buffer_source(&ctx, &realm.state, &value)?;
            assert_eq!(bytes, vec![2, 3, 4]);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn data_view_with_zero_offset_still_respects_its_length() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let value: Value = ctx.eval(
                "(() => { const buf = new Uint8Array([0,1,2,3,4,5,6,7]).buffer; return new DataView(buf, 0, 4); })()",
            )?;
            let bytes = extract_buffer_source(&ctx, &realm.state, &value)?;
            assert_eq!(bytes, vec![0, 1, 2, 3]);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_detached_array_buffer() {
        test_sync_with(|ctx| {
            let namespace = rquickjs::Object::new(ctx.clone())?;
            let errors = crate::errors::install(&ctx, &namespace)?;
            let realm = crate::realm::install(&ctx, errors)?;
            let value: Value = ctx.eval("new ArrayBuffer(8)")?;
            let mut array_buffer = rquickjs::ArrayBuffer::from_value(value.clone())
                .expect("eval result must be an ArrayBuffer");
            array_buffer.detach();
            let result = extract_buffer_source(&ctx, &realm.state, &value);
            assert!(result.is_err());
            Ok(())
        })
        .await;
    }
}
