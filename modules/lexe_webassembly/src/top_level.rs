// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Top-level `WebAssembly.compile`/`instantiate`/`validate`/`compileStreaming`/
//! `instantiateStreaming`.
//!
//! `validate` is genuinely synchronous per spec. The other four are
//! implemented as a synchronous native operation (`compile_sync`/
//! `instantiate_sync` below) wrapped in a thin *JS* `Promise`/`.then()` layer
//! (see [`GLUE_JS`]) rather than any Rust-side async machinery: per the
//! implementation plan, asynchrony here is solely the ordinary microtask
//! semantics every JS `Promise` already provides (attaching a `.then()` to an
//! already-settled promise is still always deferred to a microtask by the
//! engine itself), not a thread pool or a background Tokio runtime. Doing the
//! `Response`-awaiting/validation and `Promise` chaining in plain JS (calling
//! back into the two native sync primitives) is far simpler and less
//! error-prone than re-implementing `.then()` chaining by hand with nested
//! Rust closures, and is fully equivalent in observable behavior.

use rquickjs::prelude::{Func, Opt};
use rquickjs::{object::Property, Class, Ctx, IntoJs, Object, Result, Value};

use crate::module::WasmModule;

/// Native, synchronous helper backing the JS `compile()` defined in
/// [`GLUE_JS`]. Throws (as a `CompileError`/`TypeError`) on failure; the JS
/// wrapper turns that into a `Promise` rejection via a plain `try`/`catch`
/// inside a `new Promise` executor, with no change to the thrown value's
/// identity.
fn compile_sync<'js>(ctx: Ctx<'js>, source: Value<'js>) -> Result<Value<'js>> {
    let host = crate::realm::realm(&ctx)?.state.clone();
    let module = crate::module::compile_module(&ctx, &host, &source)?;
    let class = crate::module::wrap_module(&ctx, module)?;
    class.into_js(&ctx)
}

/// Native, synchronous helper backing the JS `instantiate()` defined in
/// [`GLUE_JS`]. Accepts either a `WebAssembly.Module` (resolving to an
/// `Instance`) or a `BufferSource` (compiling first, resolving to
/// `{module, instance}`), matching the two-overload behavior mandated by the
/// spec for `WebAssembly.instantiate`.
fn instantiate_sync<'js>(
    ctx: Ctx<'js>,
    source: Value<'js>,
    imports: Opt<Object<'js>>,
) -> Result<Value<'js>> {
    let realm = crate::realm::realm(&ctx)?;
    let host = realm.state.clone();

    if let Ok(module_class) = Class::<WasmModule>::from_value(&source) {
        let module_ref = module_class.borrow();
        let instance = crate::instance::instantiate_module(&ctx, &realm, &module_ref, imports.0)?;
        let instance_class = Class::instance(ctx.clone(), instance)?;
        return instance_class.into_js(&ctx);
    }

    let module = crate::module::compile_module(&ctx, &host, &source)?;
    let instance = crate::instance::instantiate_module(&ctx, &realm, &module, imports.0)?;
    let module_class = crate::module::wrap_module(&ctx, module)?;
    let instance_class = Class::instance(ctx.clone(), instance)?;

    let result = Object::new(ctx.clone())?;
    result.set("module", module_class)?;
    result.set("instance", instance_class)?;
    result.into_js(&ctx)
}

/// Hidden (NUL-prefixed, non-enumerable, non-configurable) property keys used
/// to hand the two native sync primitives above to [`GLUE_JS`] without ever
/// exposing them as a public, JS-reachable part of the `WebAssembly`
/// namespace's own surface (mirrors the `WasmRealmHolder` hidden-property
/// trick in `crate::realm`).
const COMPILE_SYNC_KEY: &str = "\0lexe:wasm_compile_sync";
const INSTANTIATE_SYNC_KEY: &str = "\0lexe:wasm_instantiate_sync";

/// Defines `compile`/`instantiate`/`compileStreaming`/`instantiateStreaming`
/// on `WebAssembly` purely in terms of `Promise`/`.then()` and the two hidden
/// native sync primitives above.
///
/// - `compile`/`instantiate` simply run the corresponding sync primitive
///   inside a `new Promise` executor.
/// - `compileStreaming`/`instantiateStreaming` first do `Promise.resolve(source)`
///   (uniformly handling a bare `Response` or a `Promise<Response>`, and
///   propagating an input rejection's *exact* value unchanged, since a
///   `.then()` with no rejection handler simply forwards the rejection),
///   then validate the resolved value is a successful `Response` whose
///   `Content-Type` MIME essence is `application/wasm`, then delegate to
///   `response.arrayBuffer()` followed by `compile`/`instantiate`.
///
/// All four are installed as non-enumerable, writable, configurable data
/// properties (`Object.defineProperty`), matching Node's own descriptors for
/// these methods.
const GLUE_JS: &str = r#"(function () {
    "use strict";
    var WebAssembly = globalThis.WebAssembly;
    var compileSync = WebAssembly["\0lexe:wasm_compile_sync"];
    var instantiateSync = WebAssembly["\0lexe:wasm_instantiate_sync"];
    var hasNativeResponseBrand = globalThis["\0lexe:has_native_response_brand"];

    function compile(source) {
        return new Promise(function (resolve, reject) {
            try {
                resolve(compileSync(source));
            } catch (e) {
                reject(e);
            }
        });
    }

    function instantiate(source, imports) {
        return new Promise(function (resolve, reject) {
            try {
                // Forward `imports` only when the caller actually supplied
                // it: `instantiateSync`'s native binding distinguishes "no
                // second argument" (-> no import object required) from "an
                // explicit `undefined` second argument" (-> tries to read
                // it as an import object and fails), so passing `undefined`
                // through unconditionally here would incorrectly turn
                // `WebAssembly.instantiate(module)` into a TypeError.
                resolve(imports === undefined ? instantiateSync(source) : instantiateSync(source, imports));
            } catch (e) {
                reject(e);
            }
        });
    }

    function checkWasmResponse(response) {
        // Do not use `response instanceof globalThis.Response`: both the
        // global constructor and its prototype chain are mutable from JS.
        // Fetch installs this immutable native-class predicate instead; it
        // checks QuickJS's class identity and cannot be satisfied by a plain
        // object, a forged prototype, or a replacement `Response` class.
        if (typeof hasNativeResponseBrand !== "function" || !hasNativeResponseBrand(response)) {
            throw new TypeError("WebAssembly streaming compilation failed: source was not a Response");
        }
        if (response.ok !== true) {
            throw new TypeError("WebAssembly streaming compilation failed: response was not a successful Response");
        }
        var contentType = null;
        if (response.headers && typeof response.headers.get === "function") {
            contentType = response.headers.get("content-type");
        }
        var essence = String(contentType || "").split(";")[0].trim().toLowerCase();
        if (essence !== "application/wasm") {
            throw new TypeError(
                "WebAssembly streaming compilation failed: response Content-Type was not 'application/wasm'"
            );
        }
        return response;
    }

    function compileStreaming(source) {
        return Promise.resolve(source)
            .then(function (response) {
                checkWasmResponse(response);
                return response.arrayBuffer();
            })
            .then(function (bytes) {
                return compile(bytes);
            });
    }

    function instantiateStreaming(source, imports) {
        return Promise.resolve(source)
            .then(function (response) {
                checkWasmResponse(response);
                return response.arrayBuffer();
            })
            .then(function (bytes) {
                return instantiate(bytes, imports);
            });
    }

    function defineMethod(name, fn) {
        Object.defineProperty(WebAssembly, name, {
            value: fn,
            writable: true,
            enumerable: false,
            configurable: true,
        });
    }

    defineMethod("compile", compile);
    defineMethod("instantiate", instantiate);
    defineMethod("compileStreaming", compileStreaming);
    defineMethod("instantiateStreaming", instantiateStreaming);
})();
"#;

/// `WebAssembly.validate`: genuinely synchronous per spec (unlike the other
/// four top-level methods), so it needs no `Promise` layer at all.
fn validate<'js>(ctx: Ctx<'js>, source: Value<'js>) -> Result<bool> {
    let host = crate::realm::realm(&ctx)?.state.clone();
    crate::module::validate_bytes(&ctx, &host, &source)
}

/// Installs `validate` (native, synchronous) plus `compile`/`instantiate`/
/// `compileStreaming`/`instantiateStreaming` (via [`GLUE_JS`]) onto
/// `namespace`.
///
/// Must be called only after `namespace` has already been published as
/// `globalThis.WebAssembly` (see `crate::init`), since [`GLUE_JS`] resolves
/// the namespace object by reading that global rather than receiving it as a
/// parameter.
pub fn install<'js>(ctx: &Ctx<'js>, namespace: &Object<'js>) -> Result<()> {
    namespace.prop(
        "validate",
        Property::from(Func::from(validate))
            .writable()
            .configurable(),
    )?;

    namespace.prop(COMPILE_SYNC_KEY, Property::from(Func::from(compile_sync)))?;
    namespace.prop(
        INSTANTIATE_SYNC_KEY,
        Property::from(Func::from(instantiate_sync)),
    )?;

    ctx.eval::<(), _>(GLUE_JS)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};
    use rquickjs::{prelude::Func, Class, Value};

    #[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
    #[rquickjs::class(rename = "Response")]
    struct TestResponse {}

    #[rquickjs::methods]
    impl TestResponse {
        #[qjs(constructor)]
        fn new() -> Self {
            Self {}
        }
    }

    const ADD_WAT: &str = r#"
        (module
            (func (export "add") (param i32 i32) (result i32)
                local.get 0
                local.get 1
                i32.add)
        )
    "#;

    fn set_wasm_bytes(ctx: &rquickjs::Ctx<'_>, wat: &str) {
        let bytes = wat::parse_str(wat).unwrap();
        let buffer = rquickjs::ArrayBuffer::new_copy(ctx.clone(), &bytes).unwrap();
        ctx.globals().set("__wasmBytes", buffer).unwrap();
    }

    fn init_with_native_response_brand(ctx: &rquickjs::Ctx<'_>) -> rquickjs::Result<()> {
        let globals = ctx.globals();
        Class::<TestResponse>::define(&globals)?;
        globals.prop(
            "\0lexe:has_native_response_brand",
            Func::from(|value: Value<'_>| Class::<TestResponse>::from_value(&value).is_ok()),
        )?;
        ctx.eval::<(), _>(
            r#"
            globalThis.__makeWasmResponse = function(ok, contentType, bytes) {
                const response = new Response();
                response.ok = ok;
                response.headers = { get: () => contentType };
                response.arrayBuffer = () => Promise.resolve(bytes);
                return response;
            };
            "#,
        )?;
        crate::init(ctx)
    }

    #[tokio::test]
    async fn validate_is_synchronous_and_accepts_valid_bytes() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let ok: bool = ctx
                    .eval("WebAssembly.validate(globalThis.__wasmBytes)")
                    .unwrap();
                assert!(ok);
                let bad: bool = ctx
                    .eval("WebAssembly.validate(new ArrayBuffer(4))")
                    .unwrap();
                assert!(!bad);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn compile_resolves_to_a_module_instance() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const module = await WebAssembly.compile(globalThis.__wasmBytes);
                        return module instanceof WebAssembly.Module;
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn compile_rejects_malformed_bytes_with_compile_error() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        try {
                            await WebAssembly.compile(new ArrayBuffer(4));
                            return false;
                        } catch (e) {
                            return e instanceof WebAssembly.CompileError;
                        }
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn instantiate_bytes_resolves_to_module_and_instance() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const { module, instance } = await WebAssembly.instantiate(globalThis.__wasmBytes, {});
                        if (!(module instanceof WebAssembly.Module)) return false;
                        if (!(instance instanceof WebAssembly.Instance)) return false;
                        return instance.exports.add(1, 2);
                    }
                    "#,
                )
                .await
                .unwrap();
                let sum: i32 = call_test(&ctx, &module, ()).await;
                assert_eq!(sum, 3);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn instantiate_module_resolves_to_instance() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const mod = await WebAssembly.compile(globalThis.__wasmBytes);
                        const instance = await WebAssembly.instantiate(mod, {});
                        return instance instanceof WebAssembly.Instance && instance.exports.add(4, 5);
                    }
                    "#,
                )
                .await
                .unwrap();
                let sum: i32 = call_test(&ctx, &module, ()).await;
                assert_eq!(sum, 9);
            })
        })
        .await;
    }

    /// Regression test: `WebAssembly.instantiate(module)`/`instantiate(bytes)`
    /// with the `importObject` argument omitted entirely (as opposed to
    /// passed explicitly as `{}`, which is what every other test in this
    /// module does) must not throw, for a module that declares no imports.
    /// `GLUE_JS`'s `instantiate` wrapper always forwards *some* second
    /// argument to the native `instantiateSync` trampoline; if it forwards a
    /// literal JS `undefined` instead of omitting the argument altogether,
    /// `rquickjs`'s `Opt<Object>` binding sees a present-but-unconvertible
    /// argument (arity-based, not value-based) and raises a spurious
    /// `TypeError` instead of treating it as "no imports supplied".
    #[tokio::test]
    async fn instantiate_without_imports_argument_does_not_throw() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const { instance: instanceFromBytes } = await WebAssembly.instantiate(globalThis.__wasmBytes);
                        const mod = await WebAssembly.compile(globalThis.__wasmBytes);
                        const instanceFromModule = await WebAssembly.instantiate(mod);
                        return instanceFromBytes.exports.add(1, 2) + instanceFromModule.exports.add(10, 20);
                    }
                    "#,
                )
                .await
                .unwrap();
                let sum: i32 = call_test(&ctx, &module, ()).await;
                assert_eq!(sum, 33);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn compile_streaming_rejects_non_wasm_mime_type() {
        test_async_with(|ctx| {
            Box::pin(async move {
                init_with_native_response_brand(&ctx).unwrap();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const response = globalThis.__makeWasmResponse(true, "text/plain", new ArrayBuffer(0));
                        try {
                            await WebAssembly.compileStreaming(response);
                            return false;
                        } catch (e) {
                            return e instanceof TypeError;
                        }
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn compile_streaming_rejects_non_ok_response() {
        test_async_with(|ctx| {
            Box::pin(async move {
                init_with_native_response_brand(&ctx).unwrap();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const response = globalThis.__makeWasmResponse(false, "application/wasm", new ArrayBuffer(0));
                        try {
                            await WebAssembly.compileStreaming(response);
                            return false;
                        } catch (e) {
                            return e instanceof TypeError;
                        }
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }

    /// Regression test for the reviewed bug: a plain object literal with
    /// the exact right shape (`ok: true`, a `Content-Type: application/wasm`
    /// `headers.get`, and an `arrayBuffer()` resolving to valid Wasm bytes)
    /// but that has no native `Response` class identity must still be rejected
    /// with a `TypeError`; duck-typing alone must never be sufficient.
    #[tokio::test]
    async fn compile_streaming_rejects_forged_response_object_with_correct_shape() {
        test_async_with(|ctx| {
            Box::pin(async move {
                init_with_native_response_brand(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const forged = {
                            ok: true,
                            headers: { get: () => "application/wasm" },
                            arrayBuffer: async () => globalThis.__wasmBytes,
                        };
                        try {
                            await WebAssembly.compileStreaming(forged);
                            return false;
                        } catch (e) {
                            return e instanceof TypeError;
                        }
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn compile_streaming_rejects_response_prototype_forgery() {
        test_async_with(|ctx| {
            Box::pin(async move {
                init_with_native_response_brand(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const forged = Object.create(Response.prototype);
                        Object.defineProperties(forged, {
                            ok: { value: true },
                            headers: { value: { get: () => "application/wasm" } },
                            arrayBuffer: { value: async () => globalThis.__wasmBytes },
                        });
                        try {
                            await WebAssembly.compileStreaming(forged);
                            return false;
                        } catch (e) {
                            return e instanceof TypeError;
                        }
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn compile_streaming_accepts_wasm_mime_with_parameters() {
        test_async_with(|ctx| {
            Box::pin(async move {
                init_with_native_response_brand(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const response = globalThis.__makeWasmResponse(true, "application/wasm; charset=utf-8", globalThis.__wasmBytes);
                        globalThis.Response = class ForgedResponse {};
                        const mod = await WebAssembly.compileStreaming(response);
                        return mod instanceof WebAssembly.Module;
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn instantiate_streaming_resolves_to_instance() {
        test_async_with(|ctx| {
            Box::pin(async move {
                init_with_native_response_brand(&ctx).unwrap();
                set_wasm_bytes(&ctx, ADD_WAT);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const response = globalThis.__makeWasmResponse(true, "application/wasm", globalThis.__wasmBytes);
                        const { instance } = await WebAssembly.instantiateStreaming(response, {});
                        return instance.exports.add(20, 22);
                    }
                    "#,
                )
                .await
                .unwrap();
                let sum: i32 = call_test(&ctx, &module, ()).await;
                assert_eq!(sum, 42);
            })
        })
        .await;
    }

    /// A rejected `source` promise passed to `compileStreaming` must
    /// propagate that exact rejection value unchanged (per the plan:
    /// "compileStreaming(source)：await source，保留 Promise rejection 原值").
    #[tokio::test]
    async fn compile_streaming_propagates_source_rejection_value_unchanged() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        const marker = { tag: "rejection-probe" };
                        const rejected = Promise.reject(marker);
                        try {
                            await WebAssembly.compileStreaming(rejected);
                            return false;
                        } catch (e) {
                            return e === marker;
                        }
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn instantiate_rejects_missing_import() {
        test_async_with(|ctx| {
            Box::pin(async move {
                crate::init(&ctx).unwrap();
                let wat = r#"
                    (module (import "env" "missing" (func)))
                "#;
                set_wasm_bytes(&ctx, wat);
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                    export async function test() {
                        try {
                            await WebAssembly.instantiate(globalThis.__wasmBytes, {});
                            return false;
                        } catch (e) {
                            return e instanceof WebAssembly.LinkError;
                        }
                    }
                    "#,
                )
                .await
                .unwrap();
                let ok: bool = call_test(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }
}
