// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `node:sea` built-in module for Single Executable Applications.

pub mod container;

use std::collections::HashMap;
use std::sync::OnceLock;

use container::StoredAsset;
use lexe_encoding::{
    bytes_to_b64_string, bytes_to_b64_url_safe_string, bytes_to_hex_string, bytes_to_utf16_string,
    bytes_to_utf8_string, Encoder, Endian,
};
use lexe_utils::{
    module::{export_default, ModuleInfo},
    result::ResultExt,
};
use rquickjs::{
    function::{Constructor, Opt},
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Array, ArrayBuffer, Ctx, Exception, IntoJs, Object, Result, Value,
};

pub const NODE_SEA_MODULE_NAME: &str = "node:sea";

struct SeaState {
    keys: Vec<String>,
    assets: HashMap<String, StoredAsset>,
}

static SEA_STATE: OnceLock<SeaState> = OnceLock::new();

pub fn init_sea(keys: Vec<String>, assets: HashMap<String, StoredAsset>) {
    let _ = SEA_STATE.set(SeaState { keys, assets });
}

fn throw_sea_error<T>(ctx: &Ctx<'_>, code: &str, msg: &str) -> Result<T> {
    let ctor: rquickjs::Function = ctx.globals().get("Error")?;
    let err: Object = ctor.call((msg,))?;
    err.set("code", code)?;
    Err(ctx.throw(err.into_value()))
}

fn get_sea_state<'a>(ctx: &Ctx<'_>) -> Result<&'a SeaState> {
    SEA_STATE.get().ok_or_else(|| {
        throw_sea_error::<()>(
            ctx,
            "ERR_NOT_IN_SINGLE_EXECUTABLE_APPLICATION",
            "This operation is only allowed in a single executable application",
        )
        .unwrap_err()
    })
}

pub fn is_sea() -> bool {
    SEA_STATE.get().is_some()
}

fn get_asset_keys(ctx: Ctx<'_>) -> Result<Vec<String>> {
    let state = get_sea_state(&ctx)?;
    Ok(state.keys.clone())
}

fn load_asset_bytes(ctx: &Ctx<'_>, key: &str) -> Result<Vec<u8>> {
    let state = get_sea_state(ctx)?;
    let Some(asset) = state.assets.get(key) else {
        return throw_sea_error(
            ctx,
            "ERR_SINGLE_EXECUTABLE_APPLICATION_ASSET_NOT_FOUND",
            &format!("Cannot find asset \"{key}\" in single executable application"),
        );
    };
    asset.decompress().or_throw(ctx)
}

fn get_raw_asset<'js>(ctx: Ctx<'js>, key: String) -> Result<ArrayBuffer<'js>> {
    let bytes = load_asset_bytes(&ctx, &key)?;
    ArrayBuffer::new(ctx, bytes)
}

fn get_asset<'js>(ctx: Ctx<'js>, key: String, encoding: Opt<Value<'js>>) -> Result<Value<'js>> {
    let bytes = load_asset_bytes(&ctx, &key)?;
    let Some(enc_val) = encoding.0 else {
        return Ok(ArrayBuffer::new(ctx, bytes)?.into_value());
    };
    if enc_val.is_undefined() || enc_val.is_null() {
        return Ok(ArrayBuffer::new(ctx, bytes)?.into_value());
    }
    let Some(enc_str) = enc_val.as_string() else {
        return Err(Exception::throw_type(&ctx, "encoding must be a string"));
    };
    let enc = enc_str.to_string()?;
    let decoded = match enc.trim().to_ascii_lowercase().as_str() {
        "utf8" | "utf-8" => bytes_to_utf8_string(&bytes, true).or_throw(&ctx)?,
        "hex" => bytes_to_hex_string(&bytes),
        "base64" => bytes_to_b64_string(&bytes),
        "base64url" => bytes_to_b64_url_safe_string(&bytes),
        "latin1" | "binary" => bytes.iter().map(|&b| b as char).collect(),
        "ascii" => bytes.iter().map(|&b| (b & 0x7f) as char).collect(),
        "ucs2" | "ucs-2" | "utf16le" | "utf-16le" => {
            bytes_to_utf16_string(&bytes, Endian::Little, true).or_throw(&ctx)?
        },
        _ => Encoder::from_str(&enc)
            .and_then(|e| e.encode_to_string(&bytes, true))
            .or_throw(&ctx)?,
    };
    decoded.into_js(&ctx)
}

fn get_asset_as_blob<'js>(
    ctx: Ctx<'js>,
    key: String,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let bytes = load_asset_bytes(&ctx, &key)?;
    let ab = ArrayBuffer::new(ctx.clone(), bytes)?;
    let parts = Array::new(ctx.clone())?;
    parts.set(0, ab)?;

    let blob_ctor: Constructor = ctx
        .globals()
        .get("Blob")
        .map_err(|_| Exception::throw_message(&ctx, "Global Blob constructor is not available"))?;
    match options.0 {
        Some(opt) if !opt.is_undefined() => blob_ctor.construct((parts, opt)),
        _ => blob_ctor.construct((parts,)),
    }
}

pub struct SeaModule;

impl ModuleDef for SeaModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("isSea")?;
        declare.declare("getAssetKeys")?;
        declare.declare("getRawAsset")?;
        declare.declare("getAsset")?;
        declare.declare("getAssetAsBlob")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            default.set("isSea", Func::from(is_sea))?;
            default.set("getAssetKeys", Func::from(get_asset_keys))?;
            default.set("getRawAsset", Func::from(get_raw_asset))?;
            default.set("getAsset", Func::from(get_asset))?;
            default.set("getAssetAsBlob", Func::from(get_asset_as_blob))?;
            Ok(())
        })
    }
}

impl From<SeaModule> for ModuleInfo<SeaModule> {
    fn from(val: SeaModule) -> Self {
        ModuleInfo {
            name: NODE_SEA_MODULE_NAME,
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    use super::*;

    #[tokio::test]
    async fn test_non_sea_runtime_throws_expected_code() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<SeaModule>(ctx.clone(), "node:sea")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test_sea",
                    r#"
                        import sea, { isSea, getAssetKeys, getRawAsset } from "node:sea";
                        export async function test() {
                            if (isSea() !== false || sea.isSea() !== false) {
                                throw new Error("expected isSea() === false");
                            }
                            let code1 = "";
                            try { getAssetKeys(); } catch (e) { code1 = e.code; }
                            if (code1 !== "ERR_NOT_IN_SINGLE_EXECUTABLE_APPLICATION") {
                                throw new Error("unexpected code1: " + code1);
                            }
                            let code2 = "";
                            try { getRawAsset("x"); } catch (e) { code2 = e.code; }
                            if (code2 !== "ERR_NOT_IN_SINGLE_EXECUTABLE_APPLICATION") {
                                throw new Error("unexpected code2: " + code2);
                            }
                            return true;
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<bool, _>(&ctx, &module, ()).await;
                assert!(result);
            })
        })
        .await;
    }
}
