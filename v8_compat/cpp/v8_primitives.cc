#include "v8_bridge_helpers.h"

#include <cstring>

#include <v8.h>

namespace v8 {

Local<Array> Array::New(Isolate* isolate, int length) {
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->array_new) {
    return Local<Array>();
  }
  uint64_t root_id = 0;
  if (b->array_new(ctx, length, &root_id) != LEXE_V8_OK) {
    return Local<Array>();
  }
  return lexe_v8::local_from_root<Array>(isolate, root_id, &lexe_v8::shim::Map::object_map());
}

Local<Number> Number::New(Isolate* isolate, double value) {
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->number_new) {
    return Local<Number>();
  }
  uint64_t root_id = 0;
  if (b->number_new(ctx, value, &root_id) != LEXE_V8_OK) {
    return Local<Number>();
  }
  return lexe_v8::local_from_root<Number>(isolate, root_id, &lexe_v8::shim::Map::object_map());
}

Local<BigInt> BigInt::New(Isolate* isolate, int64_t value) {
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->bigint_new) {
    return Local<BigInt>();
  }
  uint64_t root_id = 0;
  if (b->bigint_new(ctx, value, &root_id) != LEXE_V8_OK) {
    return Local<BigInt>();
  }
  return lexe_v8::local_from_root<BigInt>(isolate, root_id, &lexe_v8::shim::Map::object_map());
}

Local<Integer> Integer::New(Isolate* isolate, int32_t value) {
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->integer_new) {
    return Local<Integer>();
  }
  uint64_t root_id = 0;
  if (b->integer_new(ctx, value, &root_id) != LEXE_V8_OK) {
    return Local<Integer>();
  }
  return lexe_v8::local_from_root<Integer>(isolate, root_id,
                                             &lexe_v8::shim::Map::object_map());
}

MaybeLocal<String> String::NewFromOneByte(Isolate* isolate,
                                          const uint8_t* data,
                                          NewStringType type,
                                          int length) {
  (void)type;
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->string_new_latin1 || !data) {
    return MaybeLocal<String>();
  }
  uint64_t root_id = 0;
  if (b->string_new_latin1(ctx, data, length, &root_id) != LEXE_V8_OK) {
    return MaybeLocal<String>();
  }
  return lexe_v8::local_from_root<String>(isolate, root_id, &lexe_v8::shim::Map::string_map());
}

String::Utf8Value::Utf8Value(Isolate* isolate, Local<v8::Value> obj, WriteOptions options) {
  (void)isolate;
  (void)options;
  str_ = nullptr;
  length_ = 0;
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->string_to_utf8) {
    return;
  }
  char* ptr = nullptr;
  size_t len = 0;
  if (b->string_to_utf8(ctx, lexe_v8::root_from_object(obj.operator->()), &ptr, &len) !=
      LEXE_V8_OK) {
    return;
  }
  str_ = ptr;
  length_ = len;
}

String::Utf8Value::~Utf8Value() {
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (b && ctx && b->string_free_utf8 && str_) {
    b->string_free_utf8(ctx, str_);
  }
}

Local<Symbol> Symbol::GetIterator(Isolate* isolate) {
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->symbol_iterator) {
    return Local<Symbol>();
  }
  uint64_t root_id = 0;
  if (b->symbol_iterator(ctx, &root_id) != LEXE_V8_OK) {
    return Local<Symbol>();
  }
  return lexe_v8::local_from_root<Symbol>(isolate, root_id, &lexe_v8::shim::Map::object_map());
}

}  // namespace v8
