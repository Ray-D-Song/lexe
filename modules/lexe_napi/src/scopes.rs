// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::qjs::{self, JSContext, JSValue};

use crate::value::NapiValue;

#[derive(Copy, Clone, PartialEq, Eq)]
enum ScopeKind {
    Handle,
    Escapable,
}

pub(crate) struct Scope {
    kind: ScopeKind,
    value_watermark: usize,
    handle_watermark: usize,
    escape_slot: Option<usize>,
    escaped: bool,
}

pub struct ScopeStack {
    values: Vec<JSValue>,
    handles: Vec<*mut NapiValue>,
    scopes: Vec<Scope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeCounts {
    pub scopes: usize,
    pub values: usize,
    pub handles: usize,
}

impl ScopeStack {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            handles: Vec::new(),
            scopes: Vec::new(),
        }
    }

    pub fn open(&mut self) {
        self.scopes.push(Scope {
            kind: ScopeKind::Handle,
            value_watermark: self.values.len(),
            handle_watermark: self.handles.len(),
            escape_slot: None,
            escaped: false,
        });
    }

    pub fn open_escapable(&mut self) {
        let escape_slot = self.values.len();
        self.values.push(qjs::JS_UNDEFINED);
        self.scopes.push(Scope {
            kind: ScopeKind::Escapable,
            value_watermark: self.values.len(),
            handle_watermark: self.handles.len(),
            escape_slot: Some(escape_slot),
            escaped: false,
        });
    }

    pub fn close_handle(&mut self, ctx: *mut JSContext) -> bool {
        if !matches!(self.scopes.last(), Some(s) if s.kind == ScopeKind::Handle) {
            return false;
        }
        if let Some(scope) = self.scopes.pop() {
            self.free_scope_tail(ctx, scope);
        }
        true
    }

    pub fn close_escapable(&mut self, ctx: *mut JSContext) -> bool {
        if !matches!(self.scopes.last(), Some(s) if s.kind == ScopeKind::Escapable) {
            return false;
        }
        if let Some(scope) = self.scopes.pop() {
            self.free_scope_tail(ctx, scope);
        }
        true
    }

    pub fn close(&mut self, ctx: *mut JSContext) {
        if let Some(scope) = self.scopes.pop() {
            self.free_scope_tail(ctx, scope);
        }
    }

    /// Push `value` into the global arena and return its absolute slot index.
    pub fn push_value(&mut self, value: JSValue) -> usize {
        let slot = self.values.len();
        self.values.push(value);
        slot
    }

    pub fn push_handle(&mut self, ptr: *mut NapiValue) {
        self.handles.push(ptr);
    }

    pub fn escapable_already_escaped(&self) -> bool {
        self.scopes
            .last()
            .filter(|s| s.kind == ScopeKind::Escapable)
            .is_some_and(|s| s.escaped)
    }

    pub fn depth(&self) -> usize {
        self.scopes
            .iter()
            .filter(|s| s.kind == ScopeKind::Handle)
            .count()
    }

    pub fn escapable_depth(&self) -> usize {
        self.scopes
            .iter()
            .filter(|s| s.kind == ScopeKind::Escapable)
            .count()
    }

    pub fn resolve_value(&self, slot: usize) -> Option<JSValue> {
        self.values.get(slot).copied()
    }

    /// Write an escaped value into the escapable scope's reserved parent slot.
    pub fn escape_into_slot(&mut self, value: JSValue) -> Option<usize> {
        let scope = self.scopes.last_mut()?;
        if scope.kind != ScopeKind::Escapable || scope.escaped {
            return None;
        }
        let escape_slot = scope.escape_slot?;
        if escape_slot >= self.values.len() {
            return None;
        }
        let old = self.values[escape_slot];
        if !unsafe { qjs::JS_IsUndefined(old) } {
            // Should not happen: placeholder is always undefined until escape.
        }
        self.values[escape_slot] = value;
        scope.escaped = true;
        Some(escape_slot)
    }

    pub fn close_all(&mut self, ctx: *mut JSContext) {
        while let Some(scope) = self.scopes.pop() {
            self.free_scope_tail(ctx, scope);
        }
        for value in self.values.drain(..) {
            unsafe {
                qjs::JS_FreeValue(ctx, value);
            }
        }
        for ptr in self.handles.drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    }

    pub fn counts(&self) -> ScopeCounts {
        ScopeCounts {
            scopes: self.scopes.len(),
            values: self.values.len(),
            handles: self.handles.len(),
        }
    }

    fn free_scope_tail(&mut self, ctx: *mut JSContext, scope: Scope) {
        for value in self.values.drain(scope.value_watermark..) {
            unsafe {
                qjs::JS_FreeValue(ctx, value);
            }
        }
        let handles_tail = self.handles.split_off(scope.handle_watermark);
        for ptr in handles_tail {
            let keep = unsafe {
                let nv = &*ptr;
                nv.slot < scope.value_watermark
            };
            if keep {
                self.handles.push(ptr);
            } else {
                unsafe {
                    let _ = Box::from_raw(ptr);
                }
            }
        }
        let _ = scope.escape_slot;
        let _ = scope.escaped;
    }
}
