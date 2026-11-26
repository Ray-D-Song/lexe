// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use rquickjs::{Ctx, JsLifetime};

pub type EnvMap = Arc<RwLock<HashMap<String, String>>>;

#[derive(Clone)]
pub struct EnvStoreHandle(pub Arc<RwLock<EnvMap>>);

unsafe impl<'js> JsLifetime<'js> for EnvStoreHandle {
    type Changed<'to> = EnvStoreHandle;
}

/// Returns this context's `EnvStoreHandle`, lazily initializing it from the OS
/// environment if none has been attached yet.
pub fn env_store_handle(ctx: &Ctx<'_>) -> EnvStoreHandle {
    if let Some(handle) = ctx.userdata::<EnvStoreHandle>() {
        return handle.clone();
    }
    let handle = EnvStoreHandle(Arc::new(RwLock::new(Arc::new(RwLock::new(
        std::env::vars().collect(),
    )))));
    let _ = ctx.store_userdata(handle.clone());
    handle
}

/// Returns the shared `Arc<RwLock<HashMap<...>>>` backing this context's `process.env`.
pub fn current_env_map(ctx: &Ctx<'_>) -> EnvMap {
    let handle = env_store_handle(ctx);
    let map = handle.0.read().unwrap().clone();
    map
}

/// Returns an isolated snapshot clone of this context's current `process.env`.
pub fn snapshot_env_map(ctx: &Ctx<'_>) -> HashMap<String, String> {
    current_env_map(ctx).read().unwrap().clone()
}

/// Replaces the `EnvMap` backing this context's `process.env`.
pub fn replace_env_map(ctx: &Ctx<'_>, map: EnvMap) {
    let handle = env_store_handle(ctx);
    *handle.0.write().unwrap() = map;
}
