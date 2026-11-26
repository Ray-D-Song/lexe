// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "macos" {
        // Reserve enough Mach-O header padding for `libsui::Macho::write_section`
        // to insert an extra segment/section load command without overwriting `__TEXT,__text`.
        println!("cargo:rustc-link-arg=-Wl,-headerpad,0x1000");
    }

    let v8_compat = env::var("CARGO_FEATURE_V8_COMPAT").is_ok();
    let napi = env::var("CARGO_FEATURE_NAPI").is_ok();
    if napi || v8_compat {
        // Export dynamic symbols so native .node addons can resolve V8 ABI
        // symbols from the host executable. The V8 shim archive itself is
        // linked solely by v8_compat's build.rs (static:+whole-archive).
        if target_os == "macos" {
            println!("cargo:rustc-link-arg=-Wl,-export_dynamic");
        } else if target_family == "unix" && target_env != "musl" {
            println!("cargo:rustc-link-arg=-rdynamic");
        }
    }
}
