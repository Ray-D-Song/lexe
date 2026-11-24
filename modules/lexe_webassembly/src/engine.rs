// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Process-wide, cloneable [`wasmi::Engine`] configuration.
//!
//! `wasmi::Engine` is intentionally created exactly once per process and then
//! cheaply cloned (an `Engine` is an `Arc`-like handle) for every realm. All
//! realms therefore share the same compiled-code cache semantics and Wasm
//! feature gating, which keeps `Module` instances (immutable, engine-scoped)
//! usable independently of any particular `Store`.
//!
//! The enabled feature set intentionally matches the "in scope" list from the
//! implementation plan: core WebAssembly, bulk-memory, reference-types,
//! multi-value, mutable-globals and SIMD. Everything else (Memory64, threads /
//! shared memory, tail-call, extended-const, custom-page-sizes, wide-arithmetic,
//! relaxed-simd, exceptions, function-references, GC, component-model, ...) is
//! explicitly disabled so that unsupported modules are rejected by validation
//! with a `CompileError` instead of silently behaving differently than V8/Node.

use once_cell::sync::Lazy;
use wasmi::{Config, Engine};

fn build_config() -> Config {
    let mut config = Config::default();
    config
        // Explicit even though this is wasmi's own default: `Module::custom_sections()`
        // (used by `Module.customSections()`, see `crate::module`) yields nothing if
        // this is ever flipped to `true`.
        .ignore_custom_sections(false)
        .wasm_mutable_global(true)
        .wasm_sign_extension(true)
        .wasm_saturating_float_to_int(true)
        .wasm_multi_value(true)
        .wasm_multi_memory(false)
        .wasm_bulk_memory(true)
        .wasm_reference_types(true)
        .wasm_tail_call(false)
        .wasm_extended_const(false)
        .wasm_custom_page_sizes(false)
        .wasm_memory64(false)
        .wasm_wide_arithmetic(false)
        .wasm_simd(true)
        .wasm_relaxed_simd(false)
        .floats(true);
    config
}

/// The single process-level [`Engine`] used by every realm. `Engine` is cheap
/// to clone (internally reference-counted), so callers should call
/// [`shared_engine`] and clone the result rather than trying to cache it
/// themselves across realm boundaries.
static ENGINE: Lazy<Engine> = Lazy::new(|| Engine::new(&build_config()));

/// Returns a clone of the process-wide [`Engine`] handle.
pub fn shared_engine() -> Engine {
    ENGINE.clone()
}

/// Returns the [`wasmparser::WasmFeatures`] bitset that mirrors [`build_config`].
///
/// This is used by [`crate::module::precheck`] to run an explicit
/// `wasmparser::Validator` pass ahead of `wasmi::Module::new`, so that
/// unsupported proposals (shared memory, Memory64, tags, GC types, ...) are
/// rejected with a clean, typed `CompileError` before any bytes reach wasmi's
/// own compilation pipeline.
pub fn wasm_features() -> wasmparser::WasmFeatures {
    use wasmparser::WasmFeatures;
    let mut features = WasmFeatures::empty();
    features.set(WasmFeatures::MUTABLE_GLOBAL, true);
    features.set(WasmFeatures::SIGN_EXTENSION, true);
    features.set(WasmFeatures::SATURATING_FLOAT_TO_INT, true);
    features.set(WasmFeatures::MULTI_VALUE, true);
    features.set(WasmFeatures::MULTI_MEMORY, false);
    features.set(WasmFeatures::BULK_MEMORY, true);
    features.set(WasmFeatures::REFERENCE_TYPES, true);
    // `GC_TYPES` only gates the `externref`/`anyref` *type encoding* used by the
    // reference-types proposal; it is unrelated to the full `GC` proposal
    // (struct/array types), which we keep disabled below.
    features.set(WasmFeatures::GC_TYPES, true);
    features.set(WasmFeatures::SIMD, true);
    features.set(WasmFeatures::RELAXED_SIMD, false);
    features.set(WasmFeatures::FLOATS, true);
    features.set(WasmFeatures::TAIL_CALL, false);
    features.set(WasmFeatures::EXTENDED_CONST, false);
    features.set(WasmFeatures::CUSTOM_PAGE_SIZES, false);
    features.set(WasmFeatures::MEMORY64, false);
    features.set(WasmFeatures::WIDE_ARITHMETIC, false);
    features.set(WasmFeatures::THREADS, false);
    features.set(WasmFeatures::SHARED_EVERYTHING_THREADS, false);
    features.set(WasmFeatures::EXCEPTIONS, false);
    features.set(WasmFeatures::LEGACY_EXCEPTIONS, false);
    features.set(WasmFeatures::FUNCTION_REFERENCES, false);
    features.set(WasmFeatures::GC, false);
    features.set(WasmFeatures::COMPONENT_MODEL, false);
    features.set(WasmFeatures::STACK_SWITCHING, false);
    features.set(WasmFeatures::MEMORY_CONTROL, false);
    features
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_engine_is_stable_and_cloneable() {
        let a = shared_engine();
        let b = shared_engine();
        assert!(Engine::same(&a, &b));
    }

    #[test]
    fn feature_set_excludes_out_of_scope_proposals() {
        let features = wasm_features();
        assert!(features.contains(wasmparser::WasmFeatures::REFERENCE_TYPES));
        assert!(features.contains(wasmparser::WasmFeatures::SIMD));
        assert!(features.contains(wasmparser::WasmFeatures::BULK_MEMORY));
        assert!(features.contains(wasmparser::WasmFeatures::MULTI_VALUE));
        assert!(features.contains(wasmparser::WasmFeatures::MUTABLE_GLOBAL));
        assert!(!features.contains(wasmparser::WasmFeatures::MEMORY64));
        assert!(!features.contains(wasmparser::WasmFeatures::THREADS));
        assert!(!features.contains(wasmparser::WasmFeatures::EXCEPTIONS));
        assert!(!features.contains(wasmparser::WasmFeatures::GC));
        assert!(!features.contains(wasmparser::WasmFeatures::FUNCTION_REFERENCES));
        assert!(!features.contains(wasmparser::WasmFeatures::COMPONENT_MODEL));
    }
}
