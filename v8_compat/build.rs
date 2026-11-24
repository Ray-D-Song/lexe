use std::env;
use std::path::PathBuf;
use std::process::Command;

fn node_version(include_dir: &std::path::Path) -> Option<(u32, u32)> {
    let header = std::fs::read_to_string(include_dir.join("node_version.h")).ok()?;
    let field = |name: &str| {
        header
            .lines()
            .find_map(|line| line.strip_prefix(&format!("#define {name} ")))
            .and_then(|value| value.trim().parse::<u32>().ok())
    };
    Some((field("NODE_MAJOR_VERSION")?, field("NODE_MINOR_VERSION")?))
}

/// Headers of the `node` found on PATH (`<prefix>/include/node`), used only when none of the
/// pinned locations exist. Any Node 24.x shares ABI 137; other majors are rejected.
fn include_dir_of_node_on_path() -> Option<PathBuf> {
    let output = Command::new("node")
        .args(["-p", "process.execPath"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let exec_path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let include = exec_path.parent()?.parent()?.join("include/node");
    if !include.join("node.h").exists() || !include.join("v8.h").exists() {
        return None;
    }
    match node_version(&include) {
        Some((24, minor)) => {
            if minor != 3 {
                println!(
                    "cargo:warning=building v8_compat against Node 24.{minor} headers from {}; \
                     the pinned version is 24.3.0 (CI uses refs/node)",
                    include.display()
                );
            }
            Some(include)
        },
        _ => None,
    }
}

fn node24_include_dir() -> PathBuf {
    if let Ok(path) = env::var("RASTER_NODE24_INCLUDE") {
        return PathBuf::from(path);
    }
    if let Ok(home) = env::var("HOME") {
        let nvm = PathBuf::from(home).join(".nvm/versions/node/v24.3.0/include/node");
        if nvm.join("v8.h").exists() {
            return nvm;
        }
    }
    let refs = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .join("refs/node/src");
    if !refs.join("node.h").exists() {
        if let Some(include) = include_dir_of_node_on_path() {
            return include;
        }
    }
    refs
}

fn apply_sanitizer_flags(build: &mut cc::Build) {
    let rust_sanitizer = env::var("CARGO_CFG_SANITIZE")
        .ok()
        .filter(|value| !value.is_empty());

    let fallback_sanitizer = env::var("RASTER_SANITIZE")
        .ok()
        .filter(|value| !value.is_empty());

    let sanitizer = rust_sanitizer.as_deref().or(fallback_sanitizer.as_deref());

    let Some(sanitizer) = sanitizer else {
        return;
    };

    match sanitizer {
        "address" | "undefined" | "thread" | "memory" => {
            build
                .flag(format!("-fsanitize={sanitizer}"))
                .flag("-fno-sanitize-recover=all")
                .flag("-fno-omit-frame-pointer");

            if rust_sanitizer.is_none() {
                println!("cargo:rustc-link-arg=-fsanitize={sanitizer}");
            }
        },
        x => {
            println!("cargo:warning=Unsupported sanitizer: {x}");
        },
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let node_root = node24_include_dir();
    let node_src = if node_root.join("node.h").exists() {
        node_root.clone()
    } else {
        node_root.parent().unwrap().join("src")
    };
    let v8_include = if node_root.join("v8.h").exists() {
        node_root.clone()
    } else {
        node_root.parent().unwrap().join("deps/v8/include")
    };

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++20")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-deprecated-declarations")
        .define("NODE_WANT_INTERNALS", "1")
        .define("NODE_MODULE_VERSION", "137")
        .include(manifest_dir.join("cpp/include"))
        .include(&node_src)
        .include(&v8_include);

    apply_sanitizer_flags(&mut build);

    if node_root.join("deps").exists() {
        let deps = node_root.join("deps");
        build
            .include(deps.join("uv/include"))
            .include(deps.join("zlib"))
            .include(deps.join("llhttp/include"))
            .include(deps.join("nbytes"))
            .include(deps.join("cares/include"))
            .include(deps.join("ncrypto"))
            .include(deps.join("openssl/openssl/include"));
    } else {
        let repo_node = manifest_dir.parent().unwrap().join("refs/node");
        build
            .include(repo_node.join("deps/uv/include"))
            .include(repo_node.join("deps/zlib"))
            .include(repo_node.join("deps/llhttp/include"))
            .include(repo_node.join("deps/nbytes"))
            .include(repo_node.join("deps/cares/include"))
            .include(repo_node.join("deps/ncrypto"))
            .include(repo_node.join("deps/openssl/openssl/include"));
    }

    for file in [
        "map.cc",
        "internal.cc",
        "bridge.cc",
        "node_module.cc",
        "exports.cc",
        "function_registry.cc",
        "accessor_registry.cc",
        "template_registry.cc",
        "registry_ownership.cc",
        "objectwrap_fixture.cc",
        "template_fixture.cc",
        "v8_handle_scope.cc",
        "v8_escapable_handle_scope.cc",
        "v8_api.cc",
        "v8_api_internal.cc",
        "weak_dispatch.cc",
        "v8_dispatch.cc",
        "v8_accessor_dispatch.cc",
        "v8_module_init.cc",
        "v8_object.cc",
        "v8_primitives.cc",
        "v8_context.cc",
        "v8_function.cc",
        "v8_template_ext.cc",
        "v8_exception.cc",
        "v8_external.cc",
        "v8_signature.cc",
        "v8_value.cc",
        "node_buffer.cc",
    ] {
        build.file(manifest_dir.join("cpp").join(file));
    }

    // Single owner of libraster_v8_shim.a: disable cc's default link metadata
    // (which packs the archive into libv8_compat.rlib) and emit whole-archive
    // ourselves so downstream crates do not re-link the same objects.
    build.cargo_metadata(false);
    build.compile("raster_v8_shim");

    let out_dir = env::var("OUT_DIR").unwrap();
    println!("cargo:rustc-link-search=native={out_dir}");
    println!("cargo:rustc-link-lib=static:+whole-archive=raster_v8_shim");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "macos" => println!("cargo:rustc-link-lib=c++"),
        "linux" => println!("cargo:rustc-link-lib=stdc++"),
        _ => {},
    }

    println!("cargo:OUT_DIR={out_dir}");
    println!("cargo:rerun-if-changed=cpp/");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RASTER_NODE24_INCLUDE");
    println!("cargo:rerun-if-env-changed=RASTER_SANITIZE");
}
