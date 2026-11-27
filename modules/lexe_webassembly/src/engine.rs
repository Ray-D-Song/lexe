// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Per-realm `wasmz::Engine` construction and `wasmparser::WasmFeatures`
//! mirror for metadata pre-validation.

/// Creates a fresh [`wasmz::Engine`] for a realm.
pub fn new_engine() -> wasmz::Result<wasmz::Engine> {
    wasmz::Engine::new()
}

/// Returns the [`wasmparser::WasmFeatures`] bitset matching the WebAssembly
/// proposals supported by `wasmz`.
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
    features.set(WasmFeatures::GC_TYPES, true);
    features.set(WasmFeatures::SIMD, true);
    features.set(WasmFeatures::RELAXED_SIMD, false);
    features.set(WasmFeatures::FLOATS, true);
    features.set(WasmFeatures::TAIL_CALL, true);
    features.set(WasmFeatures::EXTENDED_CONST, true);
    features.set(WasmFeatures::CUSTOM_PAGE_SIZES, false);
    features.set(WasmFeatures::MEMORY64, true);
    features.set(WasmFeatures::WIDE_ARITHMETIC, false);
    features.set(WasmFeatures::THREADS, true);
    features.set(WasmFeatures::SHARED_EVERYTHING_THREADS, false);
    features.set(WasmFeatures::EXCEPTIONS, true);
    features.set(WasmFeatures::LEGACY_EXCEPTIONS, false);
    features.set(WasmFeatures::FUNCTION_REFERENCES, true);
    features.set(WasmFeatures::GC, true);
    features.set(WasmFeatures::COMPONENT_MODEL, false);
    features.set(WasmFeatures::STACK_SWITCHING, false);
    features.set(WasmFeatures::MEMORY_CONTROL, false);
    features
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_engine_cleanly() {
        assert!(new_engine().is_ok());
    }
}
