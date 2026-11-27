// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Centralized low-level QuickJS helper wrappers (`ArrayBuffer` mutable slice
//! view for synchronous memory mirror sync and `JS_PreventExtensions` for
//! `Instance.exports`).

/// Returns a mutable view of a (non-detached) `ArrayBuffer`'s backing bytes.
///
/// # Safety
/// `buffer` must not be detached, and the returned slice must not be held past
/// any point where `buffer` could be detached or resized.
pub unsafe fn array_buffer_bytes_mut<'a>(
    buffer: &rquickjs::ArrayBuffer<'a>,
) -> Option<&'a mut [u8]> {
    let raw = buffer.as_raw()?;
    Some(unsafe { std::slice::from_raw_parts_mut(raw.ptr.as_ptr(), raw.len) })
}

/// Calls `JS_PreventExtensions` so `Instance.exports` is non-extensible per spec.
///
/// # Safety
/// `obj` must be a live QuickJS object owned by `ctx`.
pub unsafe fn prevent_extensions(
    ctx: &rquickjs::Ctx<'_>,
    obj: &rquickjs::Object<'_>,
) -> Result<(), ()> {
    let ret = unsafe {
        rquickjs::qjs::JS_PreventExtensions(ctx.as_raw().as_ptr(), obj.as_value().as_raw())
    };
    if ret < 0 {
        Err(())
    } else {
        Ok(())
    }
}
