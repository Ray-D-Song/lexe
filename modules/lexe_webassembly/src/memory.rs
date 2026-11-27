// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `WebAssembly.Memory` backed by `wasmz::Memory` and a synchronous
//! QuickJS `ArrayBuffer` mirror that automatically detaches whenever
//! `grow_seq` or byte length changes (including `grow(0)` and Wasm-internal
//! `memory.grow`).

use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::{
    atom::PredefinedAtom, class::Trace, ArrayBuffer, Class, Ctx, IntoJs, Object, Persistent,
    Result, Value,
};

use crate::host_state::{HostState, WrapKind};

const MAX_WASM32_PAGES: u64 = 1 << 16;

pub struct MemoryMirrorEntry {
    buffer: Persistent<Value<'static>>,
    byte_len: usize,
    grow_seq: u64,
    memory: wasmz::Memory,
}

#[derive(rquickjs::JsLifetime)]
#[rquickjs::class(rename = "Memory")]
pub struct WasmMemory {
    pub(crate) realm_id: u64,
    pub(crate) handle: RefCell<wasmz::Memory>,
}

impl<'js> Trace<'js> for WasmMemory {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

struct ParsedMemoryDesc {
    initial: u64,
    maximum: Option<u64>,
    shared: bool,
    is_64: bool,
}

fn parse_memory_descriptor<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    descriptor: &Object<'js>,
) -> Result<ParsedMemoryDesc> {
    let initial_value =
        crate::descriptor::get_required(ctx, host, descriptor, "initial", "WebAssembly.Memory")?;
    let initial = crate::descriptor::to_u32_enforce_range(ctx, host, initial_value, "initial")?;
    let maximum = crate::descriptor::optional_u32_enforce_range(ctx, host, descriptor, "maximum")?;
    let shared = crate::descriptor::optional_bool(ctx, descriptor, "shared", false)?;
    let address = crate::descriptor::optional_string(ctx, descriptor, "address")?;

    let is_64 = match address.as_deref() {
        None | Some("i32") => false,
        Some("i64") => true,
        Some(other) => {
            return Err(
                host.throw_type_error(ctx, format!("unsupported memory address type '{other}'"))
            );
        },
    };

    if shared && maximum.is_none() {
        return Err(host.throw_type_error(ctx, "shared WebAssembly.Memory must specify maximum"));
    }

    if !is_64 && u64::from(initial) > MAX_WASM32_PAGES {
        return Err(host.throw_range_error(ctx, "initial memory size exceeds the wasm32 limit"));
    }
    if let Some(max) = maximum {
        if !is_64 && u64::from(max) > MAX_WASM32_PAGES {
            return Err(host.throw_range_error(ctx, "maximum memory size exceeds the wasm32 limit"));
        }
        if max < initial {
            return Err(
                host.throw_range_error(ctx, "maximum memory size is smaller than initial size")
            );
        }
    }

    Ok(ParsedMemoryDesc {
        initial: u64::from(initial),
        maximum: maximum.map(u64::from),
        shared,
        is_64,
    })
}

#[rquickjs::methods]
impl WasmMemory {
    #[qjs(constructor)]
    pub fn new<'js>(ctx: Ctx<'js>, descriptor: Object<'js>) -> Result<Self> {
        let realm = crate::realm::realm(&ctx)?;
        let host = realm.state.clone();
        let desc = parse_memory_descriptor(&ctx, &host, &descriptor)?;
        let mem = wasmz::Memory::new(desc.initial, desc.maximum, desc.shared, desc.is_64)
            .map_err(|err| host.throw_range_error(&ctx, err.to_string()))?;
        Ok(Self {
            realm_id: host.realm_id,
            handle: RefCell::new(mem),
        })
    }

    #[qjs(get)]
    pub fn buffer<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let mem = self.handle.borrow().clone();
        materialize(&ctx, &realm.state, &mem)
    }

    pub fn grow(&self, ctx: Ctx<'_>, delta: u32) -> Result<u32> {
        let realm = require_same_realm(&ctx, self.realm_id)?;
        let mut mem = self.handle.borrow().clone();
        grow(&ctx, &realm.state, &mut mem, u64::from(delta))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "WebAssembly.Memory"
    }
}

fn require_same_realm(ctx: &Ctx<'_>, realm_id: u64) -> Result<Rc<crate::realm::WasmRealm>> {
    let realm = crate::realm::realm(ctx)?;
    if realm.state.realm_id != realm_id {
        return Err(realm
            .state
            .throw_link_error(ctx, "Memory belongs to a different realm"));
    }
    Ok(realm)
}

pub fn wrap_memory<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    memory: wasmz::Memory,
) -> Result<Class<'js, WasmMemory>> {
    let bits = memory.identity();
    if let Some(existing) = host.cached_wrapper(ctx, WrapKind::Memory, bits) {
        if let Ok(class) = Class::<WasmMemory>::from_value(&existing) {
            return Ok(class);
        }
    }
    let instance = Class::instance(
        ctx.clone(),
        WasmMemory {
            realm_id: host.realm_id,
            handle: RefCell::new(memory),
        },
    )?;
    host.cache_wrapper(WrapKind::Memory, bits, instance.clone().into_js(ctx)?);
    Ok(instance)
}

#[cfg(test)]
pub(crate) fn is_materialized(host: &HostState, memory: &wasmz::Memory) -> bool {
    let bits = memory.identity();
    host.memory_mirrors.borrow().contains_key(&bits)
}

#[cfg(test)]
pub(crate) fn corrupt_mirror_for_test(ctx: &Ctx<'_>, host: &HostState, memory: &wasmz::Memory) {
    let bits = memory.identity();
    let not_an_array_buffer: Value = Object::new(ctx.clone()).unwrap().into_value();
    let mut mirrors = host.memory_mirrors.borrow_mut();
    let entry = mirrors
        .get_mut(&bits)
        .expect("memory must already be materialized before injecting fault");
    entry.buffer = Persistent::save(ctx, not_an_array_buffer);
}

pub fn materialize<'js>(
    ctx: &Ctx<'js>,
    host: &HostState,
    memory: &wasmz::Memory,
) -> Result<Value<'js>> {
    let bits = memory.identity();
    let current_len = memory.data_len();
    let current_seq = memory.grow_seq();

    let existing = host
        .memory_mirrors
        .borrow()
        .get(&bits)
        .map(|entry| (entry.byte_len, entry.grow_seq));
    if let Some((byte_len, grow_seq)) = existing {
        if byte_len == current_len && grow_seq == current_seq {
            copy_wasm_to_existing_mirror(ctx, host, bits, memory)?;
        } else {
            recreate_mirror(ctx, host, bits, memory, current_len, current_seq)?;
        }
    } else {
        recreate_mirror(ctx, host, bits, memory, current_len, current_seq)?;
    }

    let persistent = host
        .memory_mirrors
        .borrow()
        .get(&bits)
        .expect("mirror was just created")
        .buffer
        .clone();
    persistent.restore(ctx)
}

fn copy_wasm_to_existing_mirror(
    ctx: &Ctx<'_>,
    host: &HostState,
    bits: u64,
    memory: &wasmz::Memory,
) -> Result<()> {
    let persistent = host
        .memory_mirrors
        .borrow()
        .get(&bits)
        .expect("checked by caller")
        .buffer
        .clone();
    let value = persistent.restore(ctx)?;
    let buffer = ArrayBuffer::from_value(value)
        .ok_or_else(|| host.throw_runtime_error(ctx, "Memory mirror was not an ArrayBuffer"))?;
    let wasm_bytes = unsafe { memory.as_slice() };
    if let Some(mirror_bytes) = unsafe { crate::store_access::array_buffer_bytes_mut(&buffer) } {
        mirror_bytes.copy_from_slice(wasm_bytes);
    }
    Ok(())
}

fn recreate_mirror(
    ctx: &Ctx<'_>,
    host: &HostState,
    bits: u64,
    memory: &wasmz::Memory,
    new_len: usize,
    new_seq: u64,
) -> Result<()> {
    if let Some(entry) = host.memory_mirrors.borrow_mut().remove(&bits) {
        if let Ok(value) = entry.buffer.restore(ctx) {
            if let Some(mut old) = ArrayBuffer::from_value(value) {
                old.detach();
            }
        }
    }
    let bytes = unsafe { memory.as_slice() };
    debug_assert_eq!(bytes.len(), new_len);
    let buffer = ArrayBuffer::new_copy(ctx.clone(), bytes)?;
    unsafe {
        rquickjs::qjs::JS_SetNonDetachableArrayBuffer(buffer.as_value().as_raw(), true);
    }
    let value: Value = buffer.into_value();
    host.memory_mirrors.borrow_mut().insert(
        bits,
        MemoryMirrorEntry {
            buffer: Persistent::save(ctx, value),
            byte_len: new_len,
            grow_seq: new_seq,
            memory: memory.clone(),
        },
    );
    Ok(())
}

pub fn grow(
    ctx: &Ctx<'_>,
    host: &HostState,
    memory: &mut wasmz::Memory,
    delta: u64,
) -> Result<u32> {
    let bits = memory.identity();
    if host.memory_mirrors.borrow().contains_key(&bits) {
        copy_js_to_wasm_one(ctx, host, bits, memory)?;
    }
    let prev = memory
        .grow(delta)
        .ok_or_else(|| host.throw_range_error(ctx, "failed to grow WebAssembly.Memory"))?;
    let new_len = memory.data_len();
    let new_seq = memory.grow_seq();
    recreate_mirror(ctx, host, bits, memory, new_len, new_seq)?;
    Ok(prev as u32)
}

fn copy_js_to_wasm_one(
    ctx: &Ctx<'_>,
    host: &HostState,
    bits: u64,
    memory: &mut wasmz::Memory,
) -> Result<()> {
    let persistent = host
        .memory_mirrors
        .borrow()
        .get(&bits)
        .expect("checked by caller")
        .buffer
        .clone();
    let value = persistent.restore(ctx)?;
    let buffer = ArrayBuffer::from_value(value)
        .ok_or_else(|| host.throw_runtime_error(ctx, "Memory mirror was not an ArrayBuffer"))?;
    if let Some(bytes) = buffer.as_bytes() {
        let dst = unsafe { memory.as_slice_mut() };
        if bytes.len() <= dst.len() {
            dst[..bytes.len()].copy_from_slice(bytes);
        }
    }
    Ok(())
}

pub fn sync_all_js_to_wasm(ctx: &Ctx<'_>, host: &HostState) -> Result<()> {
    let entries: Vec<(u64, wasmz::Memory)> = host
        .memory_mirrors
        .borrow()
        .iter()
        .map(|(bits, entry)| (*bits, entry.memory.clone()))
        .collect();
    for (bits, mut memory) in entries {
        copy_js_to_wasm_one(ctx, host, bits, &mut memory)?;
    }
    Ok(())
}

pub fn sync_all_wasm_to_js(ctx: &Ctx<'_>, host: &HostState) -> Result<()> {
    let entries: Vec<(u64, wasmz::Memory)> = host
        .memory_mirrors
        .borrow()
        .iter()
        .map(|(bits, entry)| (*bits, entry.memory.clone()))
        .collect();
    for (bits, memory) in entries {
        let current_len = memory.data_len();
        let current_seq = memory.grow_seq();
        let existing = host
            .memory_mirrors
            .borrow()
            .get(&bits)
            .map(|e| (e.byte_len, e.grow_seq));
        if existing == Some((current_len, current_seq)) {
            copy_wasm_to_existing_mirror(ctx, host, bits, &memory)?;
        } else {
            recreate_mirror(ctx, host, bits, &memory, current_len, current_seq)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::test_sync_with;

    #[tokio::test]
    async fn buffer_getter_is_lazy_and_corrupt_mirror_propagates_runtime_error() {
        test_sync_with(|ctx| {
            crate::init(&ctx)?;
            let realm = crate::realm::realm(&ctx)?;
            let mem = wasmz::Memory::new(1, None, false, false).unwrap();
            assert!(!is_materialized(&realm.state, &mem));

            let wrapped = wrap_memory(&ctx, &realm.state, mem.clone())?;
            let wrapped_val = wrapped.into_js(&ctx)?;
            ctx.globals().set("__testMem", wrapped_val)?;
            let byte_len: usize = ctx.eval("globalThis.__testMem.buffer.byteLength")?;
            assert_eq!(byte_len, 65_536);
            assert!(is_materialized(&realm.state, &mem));

            corrupt_mirror_for_test(&ctx, &realm.state, &mem);
            let res: Result<Value<'_>> = ctx.eval("globalThis.__testMem.buffer");
            assert!(res.is_err());
            let _ = ctx.catch();
            Ok(())
        })
        .await;
    }
}
