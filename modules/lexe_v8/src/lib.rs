// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Node-compatible `v8` surface backed by QuickJS heap statistics.
//!
//! Values come from QuickJS `JS_ComputeMemoryUsage`, not from Google V8.
//! Unsupported V8 features (serialization, snapshots, profilers) are not exposed.
//! `setFlagsFromString` is a compatibility no-op.

use lexe_utils::{
    clone_wire::{deserialize, serialize, CloneWirePayload},
    module::{export_default, ModuleInfo},
};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::{Func, Opt},
    qjs, Array, Ctx, Exception, Function, Object, Result, TypedArray, Value,
};

/// ECMAScript `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Convert a non-negative size into a finite JS number, clamping to MAX_SAFE_INTEGER.
fn safe_number_u64(value: u64) -> f64 {
    value.min(MAX_SAFE_INTEGER) as f64
}

/// Convert a QuickJS signed counter: negative values become 0, then clamp.
fn safe_number_i64(value: i64) -> f64 {
    if value <= 0 {
        0.0
    } else {
        safe_number_u64(value as u64)
    }
}

// SAFETY:
// - The associated QuickJS runtime must stay alive for the duration of this call.
// - QuickJS runtimes must not be accessed concurrently.
// - The runtime is not destroyed while this function runs.
unsafe fn compute_memory_usage(ctx: &Ctx<'_>) -> qjs::JSMemoryUsage {
    let mut usage = std::mem::zeroed();
    let runtime = qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
    qjs::JS_ComputeMemoryUsage(runtime, &mut usage);
    usage
}

fn heap_limit(usage: &qjs::JSMemoryUsage) -> f64 {
    // QuickJS uses malloc_limit == 0 when no limit is configured.
    if usage.malloc_limit > 0 {
        safe_number_i64(usage.malloc_limit)
    } else {
        MAX_SAFE_INTEGER as f64
    }
}

fn get_heap_statistics(ctx: Ctx<'_>) -> Result<Object<'_>> {
    let usage = unsafe { compute_memory_usage(&ctx) };
    let malloc_size = safe_number_i64(usage.malloc_size);
    let used = safe_number_i64(usage.memory_used_size);
    let limit = heap_limit(&usage);
    let available = (limit - used).max(0.0);

    let obj = Object::new(ctx.clone())?;
    obj.set("total_heap_size", malloc_size)?;
    obj.set(
        "total_heap_size_executable",
        safe_number_i64(usage.js_func_code_size),
    )?;
    obj.set("total_physical_size", malloc_size)?;
    obj.set("total_available_size", available)?;
    obj.set("used_heap_size", used)?;
    obj.set("heap_size_limit", limit)?;
    obj.set("malloced_memory", malloc_size)?;
    obj.set("peak_malloced_memory", malloc_size)?;
    obj.set("does_zap_garbage", 0)?;
    obj.set("number_of_native_contexts", 1)?;
    obj.set("number_of_detached_contexts", 0)?;
    obj.set("total_global_handles_size", 0)?;
    obj.set("used_global_handles_size", 0)?;
    obj.set("external_memory", safe_number_i64(usage.binary_object_size))?;
    obj.set("total_allocated_bytes", malloc_size)?;
    Ok(obj)
}

fn get_heap_space_statistics(ctx: Ctx<'_>) -> Result<Array<'_>> {
    let usage = unsafe { compute_memory_usage(&ctx) };
    let malloc_size = safe_number_i64(usage.malloc_size);
    let used = safe_number_i64(usage.memory_used_size);
    let limit = heap_limit(&usage);
    let available = (limit - used).max(0.0);

    let space = Object::new(ctx.clone())?;
    space.set("space_name", "quickjs")?;
    space.set("space_size", malloc_size)?;
    space.set("space_used_size", used)?;
    space.set("space_available_size", available)?;
    space.set("physical_space_size", malloc_size)?;

    let arr = Array::new(ctx)?;
    arr.set(0, space)?;
    Ok(arr)
}

fn get_heap_code_statistics(ctx: Ctx<'_>) -> Result<Object<'_>> {
    let usage = unsafe { compute_memory_usage(&ctx) };
    let bytecode = usage
        .js_func_code_size
        .saturating_add(usage.js_func_pc2line_size);

    let obj = Object::new(ctx.clone())?;
    obj.set(
        "code_and_metadata_size",
        safe_number_i64(usage.js_func_size),
    )?;
    obj.set("bytecode_and_metadata_size", safe_number_i64(bytecode))?;
    obj.set("external_script_source_size", 0)?;
    obj.set("cpu_profiler_metadata_size", 0)?;
    Ok(obj)
}

/// Compatibility no-op: Lexe does not run V8 and does not apply flags to QuickJS.
fn set_flags_from_string(ctx: Ctx<'_>, flags: Opt<Value<'_>>) -> Result<()> {
    let Some(value) = flags.0 else {
        return Err(Exception::throw_type(
            &ctx,
            "The \"flags\" argument must be of type string",
        ));
    };
    if value.as_string().is_none() {
        return Err(Exception::throw_type(
            &ctx,
            "The \"flags\" argument must be of type string",
        ));
    }
    Ok(())
}

/// Deterministic non-zero 32-bit cached-data version tag derived from runtime metadata.
fn cached_data_version_tag() -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in b"lexe-v8-cache:"
        .iter()
        .chain(env!("CARGO_PKG_VERSION").as_bytes())
    {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

fn encode_value_wire<'js>(ctx: Ctx<'js>, value: Value<'js>) -> Result<TypedArray<'js, u8>> {
    let wire = serialize(&ctx, value, Opt(None))?;
    TypedArray::new_copy(ctx, &wire.body)
}

fn decode_value_wire<'js>(ctx: Ctx<'js>, data: TypedArray<'js, u8>) -> Result<Value<'js>> {
    let bytes = data
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Serialized payload buffer is detached"))?;
    let wire = CloneWirePayload::from_wire(bytes.to_vec(), Vec::new());
    deserialize(&ctx, &wire)
}

const V8_SERIALIZER_FACTORY_JS: &str = r#"(function(encodeValueWire, decodeValueWire) {
  const WIRE_VERSION = 15;
  const TAG_WIRE = 0x00;
  const TAG_HOST = 0x5c;
  const TAG_TRANSFERRED_AB = 0x74;

  function toBuffer(u8) {
    if (typeof Buffer === "function") {
      return Buffer.from(u8.buffer, u8.byteOffset, u8.byteLength);
    }
    return u8;
  }

  function asUint8Array(buf, argName) {
    if (buf instanceof Uint8Array) return buf;
    if (ArrayBuffer.isView(buf)) {
      return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
    }
    if (buf instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && buf instanceof SharedArrayBuffer)) {
      return new Uint8Array(buf);
    }
    throw new TypeError(`The "${argName}" argument must be an instance of Buffer, TypedArray, or DataView.`);
  }

  class Serializer {
    constructor() {
      this._buf = new Uint8Array(256);
      this._len = 0;
      this._treatViewsAsHost = false;
      this._transferred = new Map();
    }

    _ensure(extra) {
      if (this._len + extra <= this._buf.length) return;
      let cap = this._buf.length;
      while (cap < this._len + extra) cap *= 2;
      const next = new Uint8Array(cap);
      next.set(this._buf.subarray(0, this._len));
      this._buf = next;
    }

    _pushByte(b) {
      this._ensure(1);
      this._buf[this._len++] = b & 0xff;
    }

    _pushVarint(v) {
      let val = v >>> 0;
      while (val >= 0x80) {
        this._pushByte((val & 0x7f) | 0x80);
        val >>>= 7;
      }
      this._pushByte(val);
    }

    _pushVarint64(hi, lo) {
      let big = (BigInt(hi >>> 0) << 32n) | BigInt(lo >>> 0);
      while (big >= 0x80n) {
        this._pushByte(Number(big & 0x7fn) | 0x80);
        big >>= 7n;
      }
      this._pushByte(Number(big));
    }

    writeHeader() {
      this._pushByte(0xff);
      this._pushByte(WIRE_VERSION);
    }

    writeValue(value) {
      if (value instanceof ArrayBuffer && this._transferred.has(value)) {
        this._pushByte(TAG_TRANSFERRED_AB);
        this._pushVarint(this._transferred.get(value));
        return;
      }
      const hasCustomHost = typeof this._writeHostObject === "function";
      if (this._treatViewsAsHost && ArrayBuffer.isView(value)) {
        if (!hasCustomHost) {
          throw this._getDataCloneError("Unsupported host object");
        }
        this._pushByte(TAG_HOST);
        this._writeHostObject(value);
        return;
      }
      try {
        const bytes = encodeValueWire(value);
        this._pushByte(TAG_WIRE);
        this._pushVarint(bytes.length);
        this.writeRawBytes(bytes);
      } catch (err) {
        if (hasCustomHost && value !== null && typeof value === "object") {
          this._pushByte(TAG_HOST);
          this._writeHostObject(value);
          return;
        }
        throw this._getDataCloneError(err && err.message ? err.message : String(err));
      }
    }

    releaseBuffer() {
      const slice = this._buf.slice(0, this._len);
      this._len = 0;
      return toBuffer(slice);
    }

    transferArrayBuffer(id, arrayBuffer) {
      if (!Number.isSafeInteger(id) || id < 0) {
        throw new RangeError('The "id" argument must be a non-negative safe integer.');
      }
      if (!(arrayBuffer instanceof ArrayBuffer)) {
        throw new TypeError('The "arrayBuffer" argument must be an instance of ArrayBuffer.');
      }
      this._transferred.set(arrayBuffer, id);
    }

    writeUint32(value) {
      if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > 0xffffffff) {
        throw new TypeError('The "value" argument must be an unsigned 32-bit integer.');
      }
      this._pushVarint(value);
    }

    writeUint64(hi, lo) {
      if (typeof hi !== "number" || !Number.isInteger(hi) || hi < 0 || hi > 0xffffffff) {
        throw new TypeError('The "hi" argument must be an unsigned 32-bit integer.');
      }
      if (typeof lo !== "number" || !Number.isInteger(lo) || lo < 0 || lo > 0xffffffff) {
        throw new TypeError('The "lo" argument must be an unsigned 32-bit integer.');
      }
      this._pushVarint64(hi, lo);
    }

    writeDouble(value) {
      if (typeof value !== "number") {
        throw new TypeError('The "value" argument must be of type number.');
      }
      this._ensure(8);
      const view = new DataView(this._buf.buffer, this._buf.byteOffset + this._len, 8);
      view.setFloat64(0, value, true);
      this._len += 8;
    }

    writeRawBytes(buffer) {
      const u8 = asUint8Array(buffer, "buffer");
      this._ensure(u8.length);
      this._buf.set(u8, this._len);
      this._len += u8.length;
    }

    _setTreatArrayBufferViewsAsHostObjects(value) {
      this._treatViewsAsHost = Boolean(value);
    }

    _getDataCloneError(message) {
      if (typeof DOMException === "function") {
        return new DOMException(String(message), "DataCloneError");
      }
      return new Error(String(message));
    }
  }

  class Deserializer {
    constructor(buffer) {
      this._buf = asUint8Array(buffer, "buffer");
      this._pos = 0;
      this._version = 0;
      this._transferred = new Map();
    }

    _readByte() {
      if (this._pos >= this._buf.length) {
        throw new Error("Unable to deserialize cloned data.");
      }
      return this._buf[this._pos++];
    }

    _readVarint() {
      let result = 0;
      let shift = 0;
      while (true) {
        const b = this._readByte();
        result |= (b & 0x7f) << shift;
        if ((b & 0x80) === 0) return result >>> 0;
        shift += 7;
        if (shift >= 35) throw new Error("Unable to deserialize cloned data.");
      }
    }

    _readVarint64() {
      let result = 0n;
      let shift = 0n;
      while (true) {
        const b = BigInt(this._readByte());
        result |= (b & 0x7fn) << shift;
        if ((b & 0x80n) === 0n) {
          const hi = Number((result >> 32n) & 0xffffffffn);
          const lo = Number(result & 0xffffffffn);
          return [hi, lo];
        }
        shift += 7n;
        if (shift >= 70n) throw new Error("Unable to deserialize cloned data.");
      }
    }

    readHeader() {
      if (this._pos + 2 > this._buf.length) {
        throw new Error("Unable to deserialize cloned data.");
      }
      const magic = this._readByte();
      if (magic !== 0xff) {
        throw new Error("Unable to deserialize cloned data.");
      }
      this._version = this._readByte();
      return true;
    }

    readValue() {
      const tag = this._readByte();
      if (tag === TAG_WIRE) {
        const len = this._readVarint();
        if (this._pos + len > this._buf.length) {
          throw new Error("Unable to deserialize cloned data.");
        }
        const slice = this._buf.subarray(this._pos, this._pos + len);
        this._pos += len;
        return decodeValueWire(slice);
      }
      if (tag === TAG_HOST) {
        if (typeof this._readHostObject !== "function") {
          throw new Error("Unable to deserialize host object.");
        }
        return this._readHostObject();
      }
      if (tag === TAG_TRANSFERRED_AB) {
        const id = this._readVarint();
        if (!this._transferred.has(id)) {
          throw new Error("Invalid transferred ArrayBuffer id.");
        }
        return this._transferred.get(id);
      }
      throw new Error("Unable to deserialize cloned data.");
    }

    getWireFormatVersion() {
      return this._version;
    }

    transferArrayBuffer(id, arrayBuffer) {
      if (!Number.isSafeInteger(id) || id < 0) {
        throw new RangeError('The "id" argument must be a non-negative safe integer.');
      }
      if (!(arrayBuffer instanceof ArrayBuffer) && (typeof SharedArrayBuffer === "undefined" || !(arrayBuffer instanceof SharedArrayBuffer))) {
        throw new TypeError('The "arrayBuffer" argument must be an instance of ArrayBuffer or SharedArrayBuffer.');
      }
      this._transferred.set(id, arrayBuffer);
    }

    readUint32() {
      return this._readVarint();
    }

    readUint64() {
      return this._readVarint64();
    }

    readDouble() {
      if (this._pos + 8 > this._buf.length) {
        throw new Error("Unable to deserialize cloned data.");
      }
      const view = new DataView(this._buf.buffer, this._buf.byteOffset + this._pos, 8);
      this._pos += 8;
      return view.getFloat64(0, true);
    }

    _readRawBytes(length) {
      if (typeof length !== "number" || !Number.isInteger(length) || length < 0) {
        throw new RangeError('The "length" argument must be a non-negative integer.');
      }
      if (this._pos + length > this._buf.length) {
        throw new Error("Unable to deserialize cloned data.");
      }
      const slice = this._buf.subarray(this._pos, this._pos + length);
      this._pos += length;
      return toBuffer(slice);
    }

    readRawBytes(length) {
      return this._readRawBytes(length);
    }
  }

  class DefaultSerializer extends Serializer {}
  class DefaultDeserializer extends Deserializer {}

  function serialize(value) {
    const ser = new DefaultSerializer();
    ser.writeHeader();
    ser.writeValue(value);
    return ser.releaseBuffer();
  }

  function deserialize(buffer) {
    const des = new DefaultDeserializer(buffer);
    des.readHeader();
    return des.readValue();
  }

  return {
    Serializer,
    Deserializer,
    DefaultSerializer,
    DefaultDeserializer,
    serialize,
    deserialize,
  };
})"#;

pub struct V8Module;

impl ModuleDef for V8Module {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("cachedDataVersionTag")?;
        declare.declare("getHeapStatistics")?;
        declare.declare("getHeapSpaceStatistics")?;
        declare.declare("getHeapCodeStatistics")?;
        declare.declare("setFlagsFromString")?;
        declare.declare("Serializer")?;
        declare.declare("Deserializer")?;
        declare.declare("DefaultSerializer")?;
        declare.declare("DefaultDeserializer")?;
        declare.declare("serialize")?;
        declare.declare("deserialize")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let factory: Function = ctx.eval(V8_SERIALIZER_FACTORY_JS)?;
        let encode_fn = Function::new(ctx.clone(), encode_value_wire)?;
        let decode_fn = Function::new(ctx.clone(), decode_value_wire)?;
        let ser_api: Object = factory.call((encode_fn, decode_fn))?;

        let serializer: Value = ser_api.get("Serializer")?;
        let deserializer: Value = ser_api.get("Deserializer")?;
        let default_serializer: Value = ser_api.get("DefaultSerializer")?;
        let default_deserializer: Value = ser_api.get("DefaultDeserializer")?;
        let serialize_fn: Value = ser_api.get("serialize")?;
        let deserialize_fn: Value = ser_api.get("deserialize")?;

        export_default(ctx, exports, |default| {
            default.set("cachedDataVersionTag", Func::from(cached_data_version_tag))?;
            default.set("getHeapStatistics", Func::from(get_heap_statistics))?;
            default.set(
                "getHeapSpaceStatistics",
                Func::from(get_heap_space_statistics),
            )?;
            default.set(
                "getHeapCodeStatistics",
                Func::from(get_heap_code_statistics),
            )?;
            default.set("setFlagsFromString", Func::from(set_flags_from_string))?;
            default.set("Serializer", serializer)?;
            default.set("Deserializer", deserializer)?;
            default.set("DefaultSerializer", default_serializer)?;
            default.set("DefaultDeserializer", default_deserializer)?;
            default.set("serialize", serialize_fn)?;
            default.set("deserialize", deserialize_fn)?;
            Ok(())
        })
    }
}

impl From<V8Module> for ModuleInfo<V8Module> {
    fn from(val: V8Module) -> Self {
        ModuleInfo {
            name: "v8",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    #[tokio::test]
    async fn test_v8_heap_statistics() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<V8Module>(ctx.clone(), "v8")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import {
                          getHeapStatistics,
                          getHeapSpaceStatistics,
                          getHeapCodeStatistics,
                          setFlagsFromString,
                        } from 'v8';

                        export async function test() {
                          const stats = getHeapStatistics();
                          const spaces = getHeapSpaceStatistics();
                          const code = getHeapCodeStatistics();
                          const flagsResult = setFlagsFromString('--detect-ineffective-gcs-near-heap-limit');

                          const numberFields = [
                            'total_heap_size',
                            'total_heap_size_executable',
                            'total_physical_size',
                            'total_available_size',
                            'used_heap_size',
                            'heap_size_limit',
                            'malloced_memory',
                            'peak_malloced_memory',
                            'does_zap_garbage',
                            'number_of_native_contexts',
                            'number_of_detached_contexts',
                            'total_global_handles_size',
                            'used_global_handles_size',
                            'external_memory',
                            'total_allocated_bytes',
                          ];

                          for (const key of numberFields) {
                            if (!(key in stats)) throw new Error('missing ' + key);
                            const v = stats[key];
                            if (!Number.isFinite(v) || v < 0 || v > Number.MAX_SAFE_INTEGER) {
                              throw new Error('bad value for ' + key + ': ' + v);
                            }
                          }

                          if (stats.heap_size_limit < stats.used_heap_size) {
                            throw new Error('heap_size_limit < used_heap_size');
                          }
                          if (stats.total_available_size < 0) {
                            throw new Error('total_available_size < 0');
                          }

                          if (!Array.isArray(spaces) || spaces.length !== 1) {
                            throw new Error('expected one heap space');
                          }
                          if (spaces[0].space_name !== 'quickjs') {
                            throw new Error('expected space_name quickjs');
                          }
                          for (const key of [
                            'space_size',
                            'space_used_size',
                            'space_available_size',
                            'physical_space_size',
                          ]) {
                            const v = spaces[0][key];
                            if (!Number.isFinite(v) || v < 0 || v > Number.MAX_SAFE_INTEGER) {
                              throw new Error('bad space field ' + key + ': ' + v);
                            }
                          }

                          for (const key of [
                            'code_and_metadata_size',
                            'bytecode_and_metadata_size',
                            'external_script_source_size',
                            'cpu_profiler_metadata_size',
                          ]) {
                            if (!(key in code)) throw new Error('missing code field ' + key);
                            const v = code[key];
                            if (!Number.isFinite(v) || v < 0 || v > Number.MAX_SAFE_INTEGER) {
                              throw new Error('bad code field ' + key + ': ' + v);
                            }
                          }

                          if (flagsResult !== undefined) {
                            throw new Error('setFlagsFromString should return undefined');
                          }

                          let typeError = false;
                          try {
                            setFlagsFromString(42);
                          } catch (e) {
                            typeError = e instanceof TypeError;
                          }
                          if (!typeError) throw new Error('expected TypeError for non-string flags');

                          return true;
                        }
                    "#,
                )
                .await
                .unwrap();

                let ok = call_test::<bool, _>(&ctx, &module, ()).await;
                assert!(ok);
            })
        })
        .await;
    }
}
