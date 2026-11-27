// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::env;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(lexe_asan)");
    if env::var("CARGO_CFG_SANITIZE")
        .map(|s| s.split(',').any(|f| f == "address"))
        .unwrap_or(false)
    {
        // Under ASan/LSan, keep the system/ASan allocator active so Rust heap
        // allocations are instrumented and visible to LeakSanitizer instead of
        // hidden inside snmalloc's mmap arenas.
        println!("cargo::rustc-cfg=lexe_asan");
    }

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
        } else if target_os == "windows" && target_env == "msvc" && napi {
            // Native .node addons resolve napi_* from the executable. MSVC does not
            // export #[no_mangle] symbols from an EXE unless told to, so pass a
            // generated .def file listing every N-API entry point.
            let exports = napi_export_names();
            let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
            let def_path = std::path::Path::new(&out_dir).join("napi_exports.def");
            let mut def = String::from("EXPORTS\n");
            for name in &exports {
                def.push_str("    ");
                def.push_str(name);
                def.push('\n');
            }
            std::fs::write(&def_path, def).expect("write napi_exports.def");
            println!("cargo:rustc-link-arg=/DEF:{}", def_path.display());
            println!("cargo:rustc-link-arg=/EXPORT:napi_module_register");
            println!("cargo:rerun-if-changed=../modules/lexe_napi/src");
        }
    }
}

/// Collect the names of every `#[no_mangle] pub extern "C" fn napi_*` / `node_api_*`
/// in `lexe_napi`, so the Windows export list never drifts from the implementation.
fn napi_export_names() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    let src = std::path::Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../modules/lexe_napi/src");
    let entries = std::fs::read_dir(&src).expect("read lexe_napi/src");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            if line.trim() != "#[no_mangle]" {
                continue;
            }
            // Skip attribute lines between #[no_mangle] and the fn.
            let mut fn_line = None;
            for next in lines.by_ref() {
                let t = next.trim();
                if t.starts_with("#[") || t.is_empty() {
                    continue;
                }
                fn_line = Some(t.to_string());
                break;
            }
            if let Some(fl) = fn_line {
                if let Some(rest) = fl.split("fn ").nth(1) {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if name.starts_with("napi_") || name.starts_with("node_api_") {
                        names.insert(name);
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}
