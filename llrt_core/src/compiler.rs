// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{fs, io, path::Path};

use rquickjs::{CatchResultExt, Context, Module, Runtime, WriteOptions};
use tracing::trace;
use zstd::bulk::Compressor;

use crate::bytecode::add_bytecode_header;
use crate::compiler_common::{human_file_size, DummyLoader, DummyResolver};
use crate::libs::utils::result::ResultExt;
use crate::vm::{Vm, COMPRESSION_DICT};

fn compress_module(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut compressor = Compressor::with_dictionary(22, COMPRESSION_DICT)?;
    let compressed_bytes = compressor.compress(bytes)?;
    let uncompressed_len = bytes.len() as u32;

    let compressed = add_bytecode_header(compressed_bytes, Some(uncompressed_len));
    Ok(compressed)
}

pub async fn compile_file(
    input_filename: &Path,
    output_filename: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let resolver = (DummyResolver,);
    let loader = (DummyLoader,);

    let rt = Runtime::new()?;
    rt.set_loader(resolver, loader);
    let ctx = Context::full(&rt)?;

    let mut total_bytes: usize = 0;
    let mut compressed_bytes: usize = 0;
    let mut js_bytes: usize = 0;

    ctx.with(|ctx| {
        (|| {
            let source = fs::read_to_string(input_filename).or_throw_msg(
                &ctx,
                &["Unable to load: ", &input_filename.to_string_lossy()].concat(),
            )?;
            js_bytes = source.len();

            let module_name = input_filename
                .with_extension("")
                .to_string_lossy()
                .to_string();

            trace!("Compiling module: {}", module_name);

            let module = Module::declare(ctx.clone(), module_name, source)?;
            let bytes = module.write(WriteOptions::default())?;
            let compressed = compress_module(&bytes)?;

            total_bytes += bytes.len();
            compressed_bytes += compressed.len();

            fs::write(output_filename, &compressed)?;

            Ok(())
        })()
        .catch(&ctx)
        .unwrap_or_else(|err| Vm::print_error_and_exit(&ctx, err))
    });

    trace!("JS size: {}", human_file_size(js_bytes));
    trace!("Bytecode size: {}", human_file_size(total_bytes));
    trace!(
        "Compressed bytecode size: {}",
        human_file_size(compressed_bytes)
    );

    Ok(())
}

pub async fn compile_string(
    input_str: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let resolver = (DummyResolver,);
    let loader = (DummyLoader,);

    let rt = Runtime::new()?;
    rt.set_loader(resolver, loader);
    let ctx = Context::full(&rt)?;

    let mut total_bytes: usize = 0;
    let mut compressed_bytes: usize = 0;
    let js_bytes: usize = input_str.len();
    let mut result_bytes = Vec::new();

    ctx.with(|ctx| {
        (|| {
            let source = input_str;
            let module_name = "string_module";

            trace!("Compiling module: {}", module_name);

            let module = Module::declare(ctx.clone(), module_name, source)?;
            let bytes = module.write(WriteOptions::default())?;
            let compressed = compress_module(&bytes)?;

            total_bytes += bytes.len();
            compressed_bytes += compressed.len();

            result_bytes = compressed;
            Ok(())
        })()
        .catch(&ctx)
        .unwrap_or_else(|err| Vm::print_error_and_exit(&ctx, err))
    });

    trace!("JS size: {}", human_file_size(js_bytes));
    trace!("Bytecode size: {}", human_file_size(total_bytes));
    trace!(
        "Compressed bytecode size: {}",
        human_file_size(compressed_bytes)
    );

    Ok(result_bytes)
}
