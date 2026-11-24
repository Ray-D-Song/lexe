// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shared, exception-identity-preserving reading of `Memory`/`Table`/
//! `Global` descriptor dictionaries (`{ initial, maximum, shared, address }`,
//! `{ element, initial, maximum }`, `{ value, mutable }`).
//!
//! `descriptor[key]` is an ordinary JS `Get`: if `descriptor` (or its
//! prototype chain) is a `Proxy`, or `key` is an accessor property, whatever
//! it throws must propagate to the caller with its exact original identity.
//! Reading it via `descriptor.get(key).ok()` or `.unwrap_or(default)` (as
//! this module's callers originally did) silently discards *any* such
//! exception -- along with every other conversion failure -- and treats the
//! property as merely absent, letting construction spuriously succeed.
//!
//! [`get_optional`]/[`get_required`] fix that by reading the property as a
//! plain [`Value`] first (via `?`, so a thrown exception simply propagates)
//! and only ever treating a literal JS `undefined` as "not provided".
//! [`to_string`]/[`to_bool`]/[`to_u32_enforce_range`] (and their
//! `optional_*` convenience wrappers) then convert that `Value` using the
//! exact WebIDL coercion each field actually specifies -- `ToString`,
//! `ToBoolean`, and `[EnforceRange] unsigned long` respectively -- rather
//! than `rquickjs`'s generic, strict `FromJs`-derived conversions (which
//! reject e.g. `{ initial: "1" }` and silently accept `{ initial: NaN }` as
//! `0`, both wrong per spec). A thrown exception during coercion
//! (`rquickjs::Error::Exception`, e.g. from a `valueOf`/`toString`) always
//! keeps its original identity untouched.

use rquickjs::{Coerced, Ctx, FromJs, Object, Result, Value};

use crate::host_state::HostState;

/// Reads `descriptor[key]` as a raw [`Value`], preserving the exact identity
/// of any exception a getter/Proxy trap throws. Returns `None` only when the
/// property is genuinely the JS value `undefined` -- WebAssembly descriptor
/// dictionaries treat that (and only that) as "not provided"; `null` and
/// every other value are passed through untouched for the caller to convert
/// or reject.
pub fn get_optional<'js>(descriptor: &Object<'js>, key: &str) -> Result<Option<Value<'js>>> {
    let value: Value = descriptor.get(key)?;
    Ok(if value.is_undefined() {
        None
    } else {
        Some(value)
    })
}

/// Same as [`get_optional`], but throws a descriptive `TypeError` (crediting
/// `owner`, e.g. `"WebAssembly.Memory"`, and `key` in the message) if the
/// property is missing.
pub fn get_required<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    descriptor: &Object<'js>,
    key: &str,
    owner: &str,
) -> Result<Value<'js>> {
    get_optional(descriptor, key)?.ok_or_else(|| {
        host.throw_type_error(
            ctx,
            format!("{owner} descriptor must have a '{key}' property"),
        )
    })
}

/// WebIDL `ToString` (i.e. plain JS `ToString`) coercion for a descriptor
/// string-valued field (`element`, `value`, `address`): *any* JS value
/// converts (numbers/booleans stringify; objects go through
/// `toString`/`valueOf`), not just an already-a-JS-string one. Only a
/// `Symbol` (which `ToString` itself rejects) or a `toString`/`valueOf` that
/// itself throws produces an error, and that error keeps its exact original
/// identity -- there is no separate "structural mismatch" case to
/// synthesize a `TypeError` for here, unlike [`coerce`].
pub fn to_string<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    Ok(Coerced::<String>::from_js(ctx, value)?.0)
}

/// WebIDL `ToBoolean` coercion for a descriptor boolean-valued field
/// (`shared`, `mutable`): every JS value converts, and per spec `ToBoolean`
/// itself can never throw.
pub fn to_bool<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<bool> {
    Ok(Coerced::<bool>::from_js(ctx, value)?.0)
}

/// WebIDL `[EnforceRange] unsigned long` coercion, used for every
/// WebAssembly size/index descriptor field (`initial`, `maximum`): `x =
/// ToNumber(value)`; if `x` is `NaN`/`+Infinity`/`-Infinity`, or
/// `IntegerPart(x)` is outside `[0, 2**32 - 1]`, throw a `TypeError`;
/// otherwise return `IntegerPart(x)`.
///
/// Deliberately *not* the ECMAScript `ToIndex` operation (which maps `NaN`
/// to `0` rather than throwing -- observably wrong here, e.g. Node throws
/// for `new WebAssembly.Memory({ initial: NaN })`) and *not* a saturating or
/// wrapping Rust numeric cast (which would silently accept `2**32 + 1` as
/// some in-range value instead of rejecting it as out of range). A thrown
/// `ToNumber` exception (`Symbol`, or a `valueOf`/`toString` that itself
/// throws) keeps its exact original identity.
pub fn to_u32_enforce_range<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    value: Value<'js>,
    key: &str,
) -> Result<u32> {
    let n = Coerced::<f64>::from_js(ctx, value)?.0;
    if !n.is_finite() {
        return Err(host.throw_type_error(
            ctx,
            format!("descriptor property '{key}' must be a finite number"),
        ));
    }
    let truncated = n.trunc();
    if truncated < 0.0 || truncated > f64::from(u32::MAX) {
        return Err(
            host.throw_type_error(ctx, format!("descriptor property '{key}' is out of range"))
        );
    }
    Ok(truncated as u32)
}

/// Convenience for an optional [`to_string`]-coerced field: `None`/
/// `undefined` yields `None`; any other value is [`to_string`]-coerced.
pub fn optional_string<'js>(
    ctx: &Ctx<'js>,
    descriptor: &Object<'js>,
    key: &str,
) -> Result<Option<String>> {
    match get_optional(descriptor, key)? {
        Some(value) => Ok(Some(to_string(ctx, value)?)),
        None => Ok(None),
    }
}

/// Convenience for an optional [`to_u32_enforce_range`]-coerced field:
/// `None`/`undefined` yields `None`; any other value is coerced.
pub fn optional_u32_enforce_range<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    descriptor: &Object<'js>,
    key: &str,
) -> Result<Option<u32>> {
    match get_optional(descriptor, key)? {
        Some(value) => Ok(Some(to_u32_enforce_range(ctx, host, value, key)?)),
        None => Ok(None),
    }
}

/// Convenience for an optional [`to_bool`]-coerced field (`shared`,
/// `mutable`) with a default: `None`/`undefined` yields `default`; any
/// other value is [`to_bool`]-coerced.
pub fn optional_bool<'js>(
    ctx: &Ctx<'js>,
    descriptor: &Object<'js>,
    key: &str,
    default: bool,
) -> Result<bool> {
    match get_optional(descriptor, key)? {
        Some(value) => to_bool(ctx, value),
        None => Ok(default),
    }
}
