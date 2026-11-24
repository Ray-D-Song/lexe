#include "v8_bridge_helpers.h"

#include <v8-exception.h>

namespace v8 {

Local<Value> Exception::Error(Local<String> message, Local<Value> options) {
  (void)options;
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->new_exception) {
    return Local<Value>();
  }
  uint64_t out = 0;
  if (b->new_exception(ctx, lexe_v8::root_from_local(message), 0, &out) != LEXE_V8_OK) {
    return Local<Value>();
  }
  return lexe_v8::local_value_from_root(Isolate::GetCurrent(), out);
}

Local<Value> Exception::TypeError(Local<String> message, Local<Value> options) {
  (void)options;
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->new_exception) {
    return Local<Value>();
  }
  uint64_t out = 0;
  if (b->new_exception(ctx, lexe_v8::root_from_local(message), 1, &out) != LEXE_V8_OK) {
    return Local<Value>();
  }
  return lexe_v8::local_value_from_root(Isolate::GetCurrent(), out);
}

Local<Value> Exception::RangeError(Local<String> message, Local<Value> options) {
  (void)options;
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->new_exception) {
    return Local<Value>();
  }
  uint64_t out = 0;
  if (b->new_exception(ctx, lexe_v8::root_from_local(message), 2, &out) != LEXE_V8_OK) {
    return Local<Value>();
  }
  return lexe_v8::local_value_from_root(Isolate::GetCurrent(), out);
}

Local<Value> Isolate::ThrowException(Local<Value> exception) {
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->throw_value) {
    return Local<Value>();
  }
  b->throw_value(ctx, lexe_v8::root_from_local(exception));
  return exception;
}

}  // namespace v8
