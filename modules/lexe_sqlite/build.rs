fn main() {
    let mut build = cc::Build::new();
    build
        .file("vendor/sqlite3.c")
        .file("src/shim.c")
        .include("vendor")
        .define("SQLITE_DEFAULT_MEMSTATUS", "0")
        .define("SQLITE_ENABLE_COLUMN_METADATA", None)
        .define("SQLITE_ENABLE_FTS5", None)
        .define("SQLITE_ENABLE_MATH_FUNCTIONS", None)
        .define("SQLITE_ENABLE_PREUPDATE_HOOK", None)
        .define("SQLITE_ENABLE_SESSION", None)
        .define("SQLITE_THREADSAFE", "1");

    if !cfg!(target_os = "windows") {
        build.flag("-fvisibility=hidden");
    }

    // Package-scoped C sanitizer: does not use global CFLAGS, so host
    // rquickjs-sys / proc-macros are not instrumented.
    match std::env::var("LEXE_SQLITE_SANITIZE") {
        Ok(value) if value == "address" => {
            build
                .flag("-fsanitize=address")
                .flag("-fno-omit-frame-pointer")
                .flag("-g");
        },
        Ok(value) if value.is_empty() || value == "0" || value == "none" => {},
        Ok(value) => {
            panic!("unsupported LEXE_SQLITE_SANITIZE={value} (expected address|none)");
        },
        Err(_) => {},
    }

    build.compile("sqlite3");
    println!("cargo:rerun-if-changed=vendor/sqlite3.c");
    println!("cargo:rerun-if-changed=vendor/sqlite3.h");
    println!("cargo:rerun-if-changed=src/shim.c");
    println!("cargo:rerun-if-env-changed=LEXE_SQLITE_SANITIZE");
}
