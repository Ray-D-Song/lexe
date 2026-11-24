// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::cell::RefCell;

use lexe_utils::result::ResultExt;
use rquickjs::{Ctx, Object, Result};

use super::RequireState;

pub struct CurrentModuleGuard<'js> {
    ctx: Ctx<'js>,
    previous: Option<Object<'js>>,
}

impl<'js> CurrentModuleGuard<'js> {
    pub fn push(ctx: Ctx<'js>, module: Object<'js>) -> Result<Self> {
        let ctx_for_userdata = ctx.clone();
        let binding = ctx_for_userdata
            .userdata::<RefCell<RequireState>>()
            .or_throw(&ctx)?;
        let previous = binding.borrow_mut().current_module.replace(module);
        Ok(Self { ctx, previous })
    }
}

impl Drop for CurrentModuleGuard<'_> {
    fn drop(&mut self) {
        if let Some(binding) = self.ctx.userdata::<RefCell<RequireState>>() {
            binding.borrow_mut().current_module = self.previous.take();
        }
    }
}
