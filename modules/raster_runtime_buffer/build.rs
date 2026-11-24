use std::{env, fs, path::PathBuf};

use rquickjs::{module::WriteOptions, Context, Module, Runtime};

fn main() {
    println!("cargo:rerun-if-changed=src/methods.js");
    println!("cargo:rerun-if-changed=build.rs");

    let source = fs::read_to_string("src/methods.js").expect("read methods.js");
    let source = format!("export default {source}");

    let runtime = Runtime::new().expect("create runtime");
    let context = Context::full(&runtime).expect("create context");
    let bytes = context
        .with(|ctx| -> rquickjs::Result<Vec<u8>> {
            Module::declare(ctx, "raster:buffer-methods", source)?.write(WriteOptions::default())
        })
        .expect("compile methods.js");

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("methods.bc");
    fs::write(out, bytes).expect("write bytecode");
}
