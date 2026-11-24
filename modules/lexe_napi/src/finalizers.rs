// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::os::raw::c_void;

use rquickjs::qjs::{JSContext, JSValue};

use crate::gc_hook::{self, GcEntryKind};
use crate::types::{napi_env, napi_finalize};

pub struct FinalizerEntry {
    pub data: *mut c_void,
    pub finalize: napi_finalize,
    pub hint: *mut c_void,
}

pub struct FinalizerTable {
    by_id: HashMap<usize, FinalizerEntry>,
}

impl FinalizerTable {
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
        }
    }

    pub fn add(
        &mut self,
        ctx: *mut JSContext,
        obj: JSValue,
        data: *mut c_void,
        finalize: napi_finalize,
        hint: *mut c_void,
        env: napi_env,
    ) -> bool {
        let id =
            gc_hook::register_gc_entry(GcEntryKind::Finalizer, data, finalize, hint, env, None);
        if !gc_hook::attach_holder(ctx, obj, id) {
            gc_hook::remove_gc_entry(id);
            return false;
        }
        self.by_id.insert(
            id,
            FinalizerEntry {
                data,
                finalize,
                hint,
            },
        );
        true
    }

    pub fn remove_by_id(&mut self, id: usize) -> Option<FinalizerEntry> {
        self.by_id.remove(&id)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}
