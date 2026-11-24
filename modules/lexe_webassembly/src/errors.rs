// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.CompileError` / `WebAssembly.LinkError` / `WebAssembly.RuntimeError`
//! and the classification rules that decide, for any failure surfaced while
//! implementing the WebAssembly JS API, which JS error type it becomes:
//!
//! - Illegal JS-level arguments -> `TypeError` / `RangeError` (via the
//!   captured primordial constructors, never a re-read of `globalThis.Error`).
//! - Invalid or unsupported Wasm bytes -> `CompileError`.
//! - Missing imports / import type, realm or signature mismatch -> `LinkError`.
//! - Wasm traps (including a trapping `start` function) -> `RuntimeError`.
//! - A value thrown by a JS import callback is re-thrown as-is (identity
//!   preserved), never wrapped in a `RuntimeError`.

use rquickjs::{
    atom::PredefinedAtom, function::Constructor, Coerced, Ctx, Exception, FromJs, Object, Result,
    Value,
};

use crate::host_state::{ErrorConstructors, HostState};

/// Constructs a `new Error(message)` purely to harvest QuickJS's own
/// auto-generated `.stack` string, mirroring the approach used by
/// `lexe_exceptions::DOMException`. The temporary `Error` instance
/// is discarded; only the resulting stack string is kept.
///
/// This helper backs the *public*, directly-JS-invocable
/// `new WebAssembly.CompileError(...)` style constructors, where a plain
/// lookup of `globalThis.Error` is harmless (the caller explicitly asked for
/// JS-visible construction semantics). Internal error classification that
/// must be immune to a user-replaced `globalThis.Error` instead goes through
/// [`HostState::throw_compile_error`] and friends, which use the primordial
/// constructor captured once at realm-init time.
/// `CompileError`/`LinkError`/`RuntimeError`'s constructor `message`
/// argument, per spec (`NativeError(message)`), is `ToString`-coerced, not
/// required to already be a JS string: an omitted argument or explicit
/// `undefined` yields `""`; every other value (numbers, `null`, objects with
/// a `toString`/`valueOf`) goes through ordinary JS `ToString`. A thrown
/// `toString`/`valueOf` (or a `Symbol`, which `ToString` itself rejects)
/// keeps its exact original identity, propagated via `?`.
fn to_message_string<'js>(ctx: &Ctx<'js>, message: Option<Value<'js>>) -> Result<String> {
    match message {
        None => Ok(String::new()),
        Some(v) if v.is_undefined() => Ok(String::new()),
        Some(v) => Ok(Coerced::<String>::from_js(ctx, v)?.0),
    }
}

fn capture_stack(ctx: &Ctx<'_>, name: &str, message: &str) -> Result<String> {
    let ctor: Constructor = ctx.globals().get(PredefinedAtom::Error)?;
    let err: Object = ctor.construct((message.to_string(),))?;
    let stack: String = err.get(PredefinedAtom::Stack).unwrap_or_default();
    Ok(format!("{name}: {message}\n{stack}"))
}

macro_rules! define_error_class {
    ($name:ident, $tag:literal) => {
        #[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
        #[rquickjs::class]
        pub struct $name {
            message: String,
            stack: String,
        }

        #[rquickjs::methods]
        impl $name {
            #[qjs(constructor)]
            pub fn new<'js>(
                ctx: Ctx<'js>,
                message: rquickjs::prelude::Opt<Value<'js>>,
            ) -> Result<Self> {
                let message = to_message_string(&ctx, message.0)?;
                let stack = capture_stack(&ctx, $tag, &message)?;
                Ok(Self { message, stack })
            }

            #[qjs(get)]
            pub fn message(&self) -> String {
                self.message.clone()
            }

            #[qjs(get)]
            pub fn stack(&self) -> String {
                self.stack.clone()
            }

            #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
            pub fn to_string_tag(&self) -> &'static str {
                $tag
            }

            #[qjs(rename = "toString")]
            pub fn to_string(&self) -> String {
                if self.message.is_empty() {
                    $tag.to_string()
                } else {
                    format!("{}: {}", $tag, self.message)
                }
            }
        }

        impl $name {
            pub fn create<'js>(ctx: &Ctx<'js>, message: impl Into<String>) -> Result<Self> {
                let message = message.into();
                let stack = capture_stack(ctx, $tag, &message)?;
                Ok(Self { message, stack })
            }
        }
    };
}

define_error_class!(CompileError, "CompileError");
define_error_class!(LinkError, "LinkError");
define_error_class!(RuntimeError, "RuntimeError");

/// Registers `WebAssembly.CompileError`/`LinkError`/`RuntimeError` onto the
/// `WebAssembly` namespace object, wires their prototype chain to `Error`, and
/// returns the captured primordial constructors for [`ErrorConstructors`].
pub fn install<'js>(ctx: &Ctx<'js>, namespace: &Object<'js>) -> Result<ErrorConstructors> {
    use rquickjs::{class::Class, object::Property, Persistent};

    let globals = ctx.globals();
    let error_ctor: Constructor = globals.get(PredefinedAtom::Error)?;
    let error_proto: Object = error_ctor.get(PredefinedAtom::Prototype)?;

    macro_rules! install_one {
        ($ty:ty, $name:literal) => {{
            let ctor = Class::<$ty>::create_constructor(ctx)?
                .expect(concat!($name, " must have a constructor"));
            let proto: Object = ctor.get(PredefinedAtom::Prototype)?;
            proto.set_prototype(Some(&error_proto))?;
            // Matches every built-in `Error` subclass (e.g.
            // `TypeError.prototype.name === "TypeError"`): `.name` lives on
            // the *prototype*, inherited by instances, not on each instance
            // itself -- `Symbol.toStringTag` alone (defined as a per-instance
            // getter above) does not affect `Error.prototype.toString()` or
            // any code that reads `err.name` directly.
            proto.prop(
                PredefinedAtom::Name,
                Property::from($name).writable().configurable(),
            )?;
            namespace.prop(
                $name,
                Property::from(ctor.clone()).writable().configurable(),
            )?;
            Persistent::save(ctx, ctor)
        }};
    }

    let compile_error = install_one!(CompileError, "CompileError");
    let link_error = install_one!(LinkError, "LinkError");
    let runtime_error = install_one!(RuntimeError, "RuntimeError");

    Ok(ErrorConstructors {
        compile_error,
        link_error,
        runtime_error,
    })
}

impl HostState {
    fn construct_error<'js>(
        &self,
        ctx: &Ctx<'js>,
        ctor: &rquickjs::Persistent<Constructor<'static>>,
        message: impl Into<String>,
    ) -> Result<Value<'js>> {
        let ctor: Constructor = ctor.clone().restore(ctx)?;
        let value: Value = ctor.construct((message.into(),))?;
        Ok(value)
    }

    pub fn compile_error<'js>(
        &self,
        ctx: &Ctx<'js>,
        message: impl Into<String>,
    ) -> Result<Value<'js>> {
        self.construct_error(ctx, &self.errors.compile_error, message)
    }

    pub fn link_error<'js>(
        &self,
        ctx: &Ctx<'js>,
        message: impl Into<String>,
    ) -> Result<Value<'js>> {
        self.construct_error(ctx, &self.errors.link_error, message)
    }

    pub fn runtime_error<'js>(
        &self,
        ctx: &Ctx<'js>,
        message: impl Into<String>,
    ) -> Result<Value<'js>> {
        self.construct_error(ctx, &self.errors.runtime_error, message)
    }

    pub fn throw_compile_error(
        &self,
        ctx: &Ctx<'_>,
        message: impl Into<String>,
    ) -> rquickjs::Error {
        match self.compile_error(ctx, message) {
            Ok(value) => ctx.throw(value),
            Err(err) => err,
        }
    }

    pub fn throw_link_error(&self, ctx: &Ctx<'_>, message: impl Into<String>) -> rquickjs::Error {
        match self.link_error(ctx, message) {
            Ok(value) => ctx.throw(value),
            Err(err) => err,
        }
    }

    pub fn throw_runtime_error(
        &self,
        ctx: &Ctx<'_>,
        message: impl Into<String>,
    ) -> rquickjs::Error {
        match self.runtime_error(ctx, message) {
            Ok(value) => ctx.throw(value),
            Err(err) => err,
        }
    }

    /// Throws a `TypeError` via QuickJS's own internal `JS_ThrowTypeError`
    /// (through [`rquickjs::Exception::throw_type`]), which constructs the
    /// exception directly against the engine's internal error-class
    /// machinery rather than looking up `globalThis.TypeError` as a property
    /// -- i.e. it is *already* immune to a user replacing that global,
    /// exactly as the plan requires, with no captured `Persistent`
    /// constructor needed.
    pub fn throw_type_error(&self, ctx: &Ctx<'_>, message: impl Into<String>) -> rquickjs::Error {
        Exception::throw_type(ctx, &message.into())
    }

    /// See [`HostState::throw_type_error`]; same rationale for `RangeError`.
    pub fn throw_range_error(&self, ctx: &Ctx<'_>, message: impl Into<String>) -> rquickjs::Error {
        Exception::throw_range(ctx, &message.into())
    }
}

/// Classifies a `wasmi::Error` produced while instantiating or running a
/// module into the appropriate WebAssembly JS error category. A JS import
/// callback exception is *not* handled here: it must be intercepted via the
/// pending-exception sentinel before it ever reaches this classification (see
/// `crate::realm::sentinel`).
pub enum ErrorClass {
    Link,
    Runtime,
    Compile,
}

pub fn classify_wasmi_error(error: &wasmi::Error) -> ErrorClass {
    use wasmi::errors::ErrorKind;
    if error.as_trap_code().is_some() {
        return ErrorClass::Runtime;
    }
    match error.kind() {
        ErrorKind::Linker(_) | ErrorKind::Instantiation(_) => ErrorClass::Link,
        ErrorKind::Wasm(_) | ErrorKind::Translation(_) | ErrorKind::Read(_) => ErrorClass::Compile,
        ErrorKind::TrapCode(_) => ErrorClass::Runtime,
        ErrorKind::Global(_) | ErrorKind::Memory(_) | ErrorKind::Table(_) => ErrorClass::Link,
        _ => ErrorClass::Runtime,
    }
}

pub fn throw_for_wasmi_error(
    ctx: &Ctx<'_>,
    host: &HostState,
    error: wasmi::Error,
) -> rquickjs::Error {
    let class = classify_wasmi_error(&error);
    let message = error.to_string();
    match class {
        ErrorClass::Compile => host.throw_compile_error(ctx, message),
        ErrorClass::Link => host.throw_link_error(ctx, message),
        ErrorClass::Runtime => host.throw_runtime_error(ctx, message),
    }
}

/// Private sentinel [`wasmi::errors::HostError`] used to smuggle a JS import
/// callback's original thrown value (or a wrapped internal validation error)
/// back across the `wasmi::Func::call` boundary without wasmi ever seeing --
/// or this crate ever classifying -- the real payload as a `RuntimeError`.
/// See `crate::instance::call_js_import` (where it is produced) and
/// [`throw_for_wasmi_error_or_sentinel`] (where it is unwrapped again).
#[derive(Debug)]
pub struct PendingJsExceptionSentinel;

impl std::fmt::Display for PendingJsExceptionSentinel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "pending JS exception (should never be observed directly)"
        )
    }
}

impl wasmi::errors::HostError for PendingJsExceptionSentinel {}

/// Converts an `rquickjs::Result` failure produced while running JS code on
/// behalf of a host callback (import trampoline, or memory/table/global sync
/// pass) into a `wasmi::Error` suitable for returning from that callback.
///
/// If `err` already represents a live JS exception (i.e. `ctx.throw(..)` was
/// called, including by every `HostState::throw_*` helper) its exact value is
/// captured into [`HostState::set_pending_exception`] and a
/// [`PendingJsExceptionSentinel`] is returned so the original value/identity
/// can be restored and re-thrown by [`throw_for_wasmi_error_or_sentinel`] once
/// control returns to the outer `wasmi` call. Any other (rare, purely
/// internal) `rquickjs::Error` is downgraded to a plain message-only
/// `wasmi::Error` instead, since there is no JS value to preserve.
pub fn to_wasmi_error(ctx: &Ctx<'_>, host: &HostState, err: rquickjs::Error) -> wasmi::Error {
    if err.is_exception() {
        host.set_pending_exception(ctx.catch());
        return wasmi::Error::host(PendingJsExceptionSentinel);
    }
    wasmi::Error::new(err.to_string())
}

/// The inverse of [`throw_for_wasmi_error`]: checks for the private
/// [`PendingJsExceptionSentinel`] first and, if found, restores and re-throws
/// the original JS value/identity exactly as thrown (never wrapped in a
/// `RuntimeError`). Otherwise classifies and throws normally.
pub fn throw_for_wasmi_error_or_sentinel(
    ctx: &Ctx<'_>,
    host: &HostState,
    mut error: wasmi::Error,
) -> rquickjs::Error {
    if error.downcast_mut::<PendingJsExceptionSentinel>().is_some() {
        if let Some(value) = host.take_pending_exception(ctx) {
            return ctx.throw(value);
        }
        // The sentinel fired but no pending exception was recorded (should
        // not happen); fall through to generic classification rather than
        // silently losing the failure.
    }
    throw_for_wasmi_error(ctx, host, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_sync_with;

    #[test]
    fn out_of_bounds_memory_access_classifies_as_runtime() {
        let error = wasmi::Error::from(wasmi::TrapCode::MemoryOutOfBounds);
        assert!(matches!(classify_wasmi_error(&error), ErrorClass::Runtime));
    }

    /// Regression test for the reviewed bug: `new
    /// WebAssembly.CompileError(message)` (and `LinkError`/`RuntimeError`)
    /// must `ToString`-coerce `message` like every built-in `Error`
    /// subclass constructor, not require it to already be a JS string --
    /// an omitted argument or explicit `undefined` yields `""`, and every
    /// other value (including `null`, which stringifies to `"null"`) goes
    /// through ordinary JS `ToString`.
    #[tokio::test]
    async fn error_constructor_message_uses_to_string_coercion() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    if (new WebAssembly.CompileError(123).message !== "123") return false;
                    if (new WebAssembly.CompileError(null).message !== "null") return false;
                    if (new WebAssembly.CompileError(undefined).message !== "") return false;
                    if (new WebAssembly.CompileError().message !== "") return false;
                    if (new WebAssembly.CompileError("hi").message !== "hi") return false;
                    return true;
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }

    /// A `toString()`/`valueOf()` that itself throws must propagate with
    /// its exact original identity, not get swallowed or replaced by a
    /// synthetic `TypeError`.
    #[tokio::test]
    async fn error_constructor_message_thrown_value_identity_is_preserved() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let ok: bool = ctx.eval(
                r#"
                (() => {
                    const thrown = {};
                    const message = { toString() { throw thrown; } };
                    try {
                        new WebAssembly.CompileError(message);
                        return false;
                    } catch (e) {
                        return e === thrown;
                    }
                })()
                "#,
            )?;
            assert!(ok);
            Ok(())
        })
        .await;
    }
}
