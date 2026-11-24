#include "v8_bridge_helpers.h"

#include <v8.h>

namespace v8 {

#define V8_VALUE_PRED(name, fn)                     \
  bool Value::name() const {                        \
    bool out = false;                               \
    auto* ctx = lexe_v8::bridge_ctx();            \
    if (!ctx) return false;                         \
    if (lexe_v8_##fn(ctx, lexe_v8::root_from_object(this), &out) != LEXE_V8_OK) { \
      return false;                                 \
    }                                               \
    return out;                                     \
  }

V8_VALUE_PRED(IsObject, value_is_object)
V8_VALUE_PRED(IsArray, value_is_array)
V8_VALUE_PRED(IsFunction, value_is_function)
V8_VALUE_PRED(IsNumber, value_is_number)
V8_VALUE_PRED(IsInt32, value_is_int32)
V8_VALUE_PRED(IsBigInt, value_is_bigint)
V8_VALUE_PRED(IsBoolean, value_is_boolean)

#undef V8_VALUE_PRED

bool Value::StrictEquals(Local<Value> other) const {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return false;
  }
  bool out = false;
  if (lexe_v8_value_strict_equals(ctx, lexe_v8::root_from_object(this),
                                    lexe_v8::root_from_local(other), &out) != LEXE_V8_OK) {
    return false;
  }
  return out;
}

double Number::Value() const {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return 0.0;
  }
  double out = 0.0;
  if (lexe_v8_value_to_float64(ctx, lexe_v8::root_from_object(this), &out) != LEXE_V8_OK) {
    return 0.0;
  }
  return out;
}

int32_t Int32::Value() const {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return 0;
  }
  int32_t out = 0;
  if (lexe_v8_value_to_int32(ctx, lexe_v8::root_from_object(this), &out) != LEXE_V8_OK) {
    return 0;
  }
  return out;
}

bool Boolean::Value() const {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return false;
  }
  bool is_bool = false;
  uint64_t root = lexe_v8::root_from_object(this);
  if (lexe_v8_value_is_boolean(ctx, root, &is_bool) != LEXE_V8_OK || !is_bool) {
    return false;
  }
  bool out = false;
  if (lexe_v8_value_to_boolean(ctx, root, &out) != LEXE_V8_OK) {
    return false;
  }
  return out;
}

int64_t BigInt::Int64Value(bool* lossless) const {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    if (lossless) {
      *lossless = false;
    }
    return 0;
  }
  int64_t out = 0;
  bool ok = false;
  if (lexe_v8_value_to_int64(ctx, lexe_v8::root_from_object(this), &out, &ok) != LEXE_V8_OK) {
  }
  if (lossless) {
    *lossless = ok;
  }
  return out;
}

void* External::Value() const {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return nullptr;
  }
  void* out = nullptr;
  if (lexe_v8_internal_field_get(ctx, lexe_v8::root_from_object(this), 0, &out) != LEXE_V8_OK) {
    return nullptr;
  }
  return out;
}

uint32_t Array::Length() const {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return 0;
  }
  uint32_t out = 0;
  if (lexe_v8_array_length(ctx, lexe_v8::root_from_object(this), &out) != LEXE_V8_OK) {
    return 0;
  }
  return out;
}

MaybeLocal<Object> Function::NewInstance(Local<Context> context, int argc, Local<Value>* argv) const {
  (void)context;
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return MaybeLocal<Object>();
  }
  std::vector<uint64_t> args;
  args.reserve(static_cast<size_t>(argc));
  for (int i = 0; i < argc; ++i) {
    args.push_back(lexe_v8::root_from_local(argv[i]));
  }
  uint64_t out = 0;
  auto* layout_obj = lexe_v8::layout_from_slot(
      reinterpret_cast<v8::internal::Address*>(const_cast<void*>(static_cast<const void*>(this))));
  uint64_t func_root = lexe_v8::root_from_object(this);
  uint32_t function_id = layout_obj ? layout_obj->function_id : 0;
  if (function_id == 0) {
    lexe_v8_function_id_for_root(ctx, func_root, &function_id);
  }
  if (function_id != 0) {
    uint64_t cached_root = 0;
    if (lexe_v8_function_root_for_id(ctx, function_id, &cached_root) == LEXE_V8_OK) {
      func_root = cached_root;
    }
  }
  // Do not use persistent_root fallback: it can return the wrong root when multiple
  // persistents share a root_id or when layout_root was resolved via tagged_map chase.
  if (lexe_v8_function_new_instance(ctx, func_root, argc,
                                      args.empty() ? nullptr : args.data(), &out) != LEXE_V8_OK) {
    return MaybeLocal<Object>();
  }
  return lexe_v8::local_from_root<Object>(Isolate::GetCurrent(), out,
                                           &lexe_v8::shim::Map::object_map());
}

}  // namespace v8
